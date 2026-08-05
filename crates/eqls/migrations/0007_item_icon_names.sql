create table if not exists item_icon_names (
    name_key   text primary key,
    name       text not null,
    game_id    bigint,
    icon       integer not null,
    updated_at timestamptz not null default now()
);
