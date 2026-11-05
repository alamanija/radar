//! App updates (stub until Task 15, which ports the `useUpdater` states).

/// Where the update flow stands. Only `Idle` exists until Task 15.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum State {
    #[default]
    Idle,
}

pub fn state() -> State {
    State::Idle
}

/// "Check for updates". A no-op for now.
pub fn check() {}
