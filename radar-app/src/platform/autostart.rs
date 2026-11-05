//! "Open at login" (stub until Task 14). The OS owns the truth, so callers
//! read [`is_enabled`] instead of mirroring it.

/// Whether Radar is registered to start at login. Always off for now.
pub fn is_enabled() -> bool {
    false
}

/// Registers (or removes) the login item. A no-op for now.
pub fn set(_enabled: bool) -> anyhow::Result<()> {
    Ok(())
}
