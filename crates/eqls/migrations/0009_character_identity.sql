alter table characters
    add column if not exists level int,
    add column if not exists race text,
    add column if not exists classes text[],
    add column if not exists identity_at timestamptz;
