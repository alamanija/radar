-- Per-user app preferences (visual taste + briefing schedule). Stored as a
-- single jsonb blob rather than typed columns because the shape is small,
-- the client is authoritative, and full-replace sync semantics make per-key
-- columns pointless churn.
--
-- Per-device UI state (sidebarCollapsed, current view) stays in local
-- storage only and is not part of this blob.
create table user_prefs (
    clerk_user_id text primary key,
    prefs         jsonb not null default '{}'::jsonb,
    updated_at    timestamptz not null default now()
);
