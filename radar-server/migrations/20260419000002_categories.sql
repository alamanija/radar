-- Categories are a per-user ordered list; the client is authoritative on
-- ids (slugs), labels, descriptions and accents. Stored as a single jsonb
-- blob rather than a row-per-category table because sync uses full-list
-- replace semantics and the list is small (usually 6–20 entries).
create table user_categories (
    user_id     uuid primary key references users(id) on delete cascade,
    categories  jsonb not null default '[]'::jsonb,
    updated_at  timestamptz not null default now()
);
