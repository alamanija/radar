//! UI-free backend for Radar: feed ingestion, Claude summarization,
//! keychain access, schedule logic and shared wire types.

pub mod ingest;
pub mod keychain;
pub mod schedule;
pub mod schedule_wake;
pub mod summarize;
pub mod types;

use std::sync::LazyLock;
use tokio::runtime::Runtime;

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime")
});

/// Shared multi-thread tokio runtime for the app.
pub fn runtime() -> &'static Runtime {
    &RUNTIME
}
