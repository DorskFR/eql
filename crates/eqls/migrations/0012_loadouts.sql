-- A character swaps between class combinations; each one gears differently, so
-- snapshots belong to a loadout rather than piling onto one profile.

create or replace function loadout_key(classes text[]) returns text
    language sql immutable strict
    as $$
    select coalesce(string_agg(upper(class), '/' order by upper(class)), '')
    from unnest(classes) as class
$$;

create table if not exists character_loadouts (
    id bigserial primary key,
    character_id bigint not null references characters (id) on delete cascade,
    classes text[] not null,
    class_key text generated always as (loadout_key(classes)) stored,
    level int,
    first_seen_at timestamptz not null,
    last_seen_at timestamptz not null,
    unique (character_id, class_key)
);

alter table inventory_snapshots
    add column if not exists loadout_id bigint
        references character_loadouts (id) on delete set null;

create index if not exists inventory_snapshots_loadout_captured_idx
    on inventory_snapshots (loadout_id, captured_at desc);

create index if not exists log_events_who_idx
    on log_events (character_id, at) where kind = 'who';

-- The `/who` nearest the dump names the loadout it was taken in; the EQLD
-- social types one immediately before the export, so the gap is seconds.
create or replace function attribute_snapshots(target bigint) returns void
    language sql
    as $$
    update inventory_snapshots s
    set loadout_id = matched.loadout_id
    from (
        select snap.id, nearest.id as loadout_id
        from inventory_snapshots snap
        cross join lateral (
            select loadout.id
            from log_events who
            join character_loadouts loadout
                on loadout.character_id = who.character_id
               and loadout.class_key = loadout_key(
                       array(select jsonb_array_elements_text(who.payload -> 'classes')))
            where who.character_id = snap.character_id and who.kind = 'who'
            order by abs(extract(epoch from (who.at - snap.captured_at))),
                     (who.at > snap.captured_at)
            limit 1
        ) nearest
        where target is null or snap.character_id = target
    ) matched
    where matched.id = s.id and s.loadout_id is distinct from matched.loadout_id
$$;

with seen as (
    select e.character_id,
           array(select jsonb_array_elements_text(e.payload -> 'classes')) as classes,
           (e.payload ->> 'level')::int as level,
           e.at
    from log_events e
    where e.kind = 'who' and jsonb_typeof(e.payload -> 'classes') = 'array'
),
spans as (
    select character_id, loadout_key(classes) as class_key,
           min(at) as first_seen_at, max(at) as last_seen_at
    from seen
    where loadout_key(classes) <> ''
    group by character_id, loadout_key(classes)
),
newest as (
    select distinct on (character_id, loadout_key(classes))
           character_id, loadout_key(classes) as class_key, classes, level
    from seen
    where loadout_key(classes) <> ''
    order by character_id, loadout_key(classes), at desc
)
insert into character_loadouts (character_id, classes, level, first_seen_at, last_seen_at)
select n.character_id, n.classes, n.level, s.first_seen_at, s.last_seen_at
from newest n
join spans s on s.character_id = n.character_id and s.class_key = n.class_key
on conflict do nothing;

select attribute_snapshots(null::bigint);
