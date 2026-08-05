create table if not exists log_events (
    id bigserial primary key,
    character_id bigint not null references characters (id) on delete cascade,
    at timestamptz not null,
    kind text not null,
    payload jsonb not null default '{}'::jsonb,
    created_at timestamptz not null default now()
);

create index if not exists log_events_character_at_idx
    on log_events (character_id, at desc);
