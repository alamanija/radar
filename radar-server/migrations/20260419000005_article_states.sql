-- Per-(user, article) read/bookmark state. Per-row PK so rapid toggles
-- don't have to round-trip the full article list — unlike categories/
-- sources/archives, this is a diff-based sync.
create table user_article_states (
    user_id     uuid not null references users(id) on delete cascade,
    article_id  text not null,
    read        boolean not null default false,
    bookmarked  boolean not null default false,
    updated_at  timestamptz not null default now(),
    primary key (user_id, article_id)
);
