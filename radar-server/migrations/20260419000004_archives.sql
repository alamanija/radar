-- Archives are append-only briefing snapshots, capped at 90 on the client
-- and treated as opaque JSON here — the server never interprets the inner
-- article/error shapes, it just round-trips them.
create table user_archives (
    user_id     uuid primary key references users(id) on delete cascade,
    archives    jsonb not null default '[]'::jsonb,
    updated_at  timestamptz not null default now()
);
