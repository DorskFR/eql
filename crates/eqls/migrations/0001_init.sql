create table if not exists characters (
    id bigserial primary key,
    name text not null,
    server text not null,
    created_at timestamptz not null default now(),
    unique (name, server)
);

create table if not exists inventory_snapshots (
    id bigserial primary key,
    character_id bigint not null references characters (id) on delete cascade,
    captured_at timestamptz not null,
    entries jsonb not null,
    raw text,
    created_at timestamptz not null default now()
);

create index if not exists inventory_snapshots_character_captured_idx
    on inventory_snapshots (character_id, captured_at desc);
