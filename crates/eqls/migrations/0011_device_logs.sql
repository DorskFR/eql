create table if not exists device_logs (
    id bigserial primary key,
    device text not null,
    session text not null,
    seq bigint not null,
    at timestamptz not null default now(),
    dropped bigint not null default 0,
    lines jsonb not null,
    unique (device, session, seq)
);

create index if not exists device_logs_session_idx on device_logs (device, session, seq);
create index if not exists device_logs_at_idx on device_logs (at desc);
