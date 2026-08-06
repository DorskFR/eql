create table if not exists fights (
    id bigserial primary key,
    character_id bigint not null references characters (id) on delete cascade,
    start_wall double precision not null,
    started_at timestamptz not null,
    zone text,
    span double precision not null,
    active_secs double precision not null,
    dmg_out bigint not null,
    dmg_in bigint not null,
    heal_out bigint not null,
    kills integer not null,
    deaths integer not null,
    enemies text[] not null default '{}',
    fight jsonb not null,
    created_at timestamptz not null default now(),
    unique (character_id, start_wall)
);

create index if not exists fights_character_started_idx
    on fights (character_id, started_at desc);
