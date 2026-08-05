create table if not exists items (
    id bigserial primary key,
    game_id bigint unique,
    name text not null unique,
    stats jsonb not null,
    wikitext text not null,
    scraped_at timestamptz not null default now()
);

create index if not exists items_name_lower_idx on items (lower(name));
