//! OS integrations: the login item (`auto-launch`), system notifications,
//! the tray icon (`tray-icon`), and the `updater` stub (Task 15). The model
//! reaches them only through `app_model::Hooks`, which `main.rs` wires to
//! these modules; tests use fakes.

pub mod autostart;
pub mod notify;
pub mod tray;
pub mod updater;
