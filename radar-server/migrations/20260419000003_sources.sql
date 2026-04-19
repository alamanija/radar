-- Sources are a per-user list, same jsonb-blob-per-user shape as
-- user_categories. Only the user-editable fields travel over sync;
-- `lastFetchAt` and `health` stay device-local (what device A has observed
-- about a feed's freshness isn't ground truth for device B).
create table user_sources (
    user_id     uuid primary key references users(id) on delete cascade,
    sources     jsonb not null default '[]'::jsonb,
    updated_at  timestamptz not null default now()
);
