create table profiles (
    user_id     uuid primary key references users(id) on delete cascade,
    name        text,
    role        text,
    lens        text,
    updated_at  timestamptz not null default now()
);
