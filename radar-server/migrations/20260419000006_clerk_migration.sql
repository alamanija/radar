-- Switch auth from server-owned Google-OAuth sessions to Clerk-issued JWTs.
-- The server no longer holds identity; Clerk's `sub` is the user key, so
-- there's no `users` table to FK against. Destructive — pre-release only;
-- any dev data is wiped. Run `docker compose down -v` to start clean.

drop table if exists user_article_states;
drop table if exists user_archives;
drop table if exists user_sources;
drop table if exists user_categories;
drop table if exists profiles;
drop table if exists sessions;
drop table if exists users;

create table profiles (
    clerk_user_id text primary key,
    name          text,
    role          text,
    lens          text,
    updated_at    timestamptz not null default now()
);

create table user_categories (
    clerk_user_id text primary key,
    categories    jsonb not null default '[]'::jsonb,
    updated_at    timestamptz not null default now()
);

create table user_sources (
    clerk_user_id text primary key,
    sources       jsonb not null default '[]'::jsonb,
    updated_at    timestamptz not null default now()
);

create table user_archives (
    clerk_user_id text primary key,
    archives      jsonb not null default '[]'::jsonb,
    updated_at    timestamptz not null default now()
);

create table user_article_states (
    clerk_user_id text not null,
    article_id    text not null,
    read          boolean not null default false,
    bookmarked    boolean not null default false,
    updated_at    timestamptz not null default now(),
    primary key (clerk_user_id, article_id)
);
