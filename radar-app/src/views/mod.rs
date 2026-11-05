//! Page views (ports of `src/views/*.jsx`).

pub mod archive;
pub mod briefing;
pub mod categories;
pub mod empty_state;
pub mod form;
pub mod onboarding;
pub mod saved;
pub mod settings;
pub mod sources;

#[cfg(test)]
#[path = "saved_archive_tests.rs"]
mod saved_archive_tests;

#[cfg(test)]
#[path = "sources_categories_tests.rs"]
mod sources_categories_tests;

#[cfg(test)]
#[path = "settings_tests.rs"]
mod settings_tests;
