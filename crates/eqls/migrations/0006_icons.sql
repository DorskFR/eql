create table if not exists item_icons (
    icon       integer primary key,
    png        bytea not null,
    updated_at timestamptz not null default now()
);
