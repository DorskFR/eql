create table if not exists harvest_docs (
    id bigserial primary key,
    character_id bigint not null references characters (id) on delete cascade,
    kind text not null,
    captured_at timestamptz not null,
    doc jsonb not null,
    created_at timestamptz not null default now(),
    unique (character_id, kind)
);
