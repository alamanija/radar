create extension if not exists "pgcrypto";

create table users (
    id            uuid primary key default gen_random_uuid(),
    google_sub    text unique not null,
    email         text not null,
    name          text,
    created_at    timestamptz not null default now(),
    updated_at    timestamptz not null default now()
);

create table sessions (
    token         text primary key,
    user_id       uuid not null references users(id) on delete cascade,
    created_at    timestamptz not null default now(),
    expires_at    timestamptz not null,
    last_used_at  timestamptz not null default now()
);

create index sessions_user_id_idx on sessions(user_id);
create index sessions_expires_at_idx on sessions(expires_at);
