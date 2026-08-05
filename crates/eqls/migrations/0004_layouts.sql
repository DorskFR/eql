create table if not exists layouts (
    id bigserial primary key,
    name text not null unique,
    screen_w int not null,
    screen_h int not null,
    layout jsonb not null,
    updated_at timestamptz not null default now()
);
