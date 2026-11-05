//! Page views (ports of `src/views/*.jsx`).

pub mod archive;
pub mod briefing;
pub mod empty_state;
pub mod onboarding;
pub mod saved;

#[cfg(test)]
#[path = "saved_archive_tests.rs"]
mod saved_archive_tests;
