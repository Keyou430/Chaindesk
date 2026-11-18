pub mod auto_backup;
pub mod autostart;
pub mod backup;
#[cfg(windows)]
pub(crate) mod chain_diagnostic;
pub(crate) mod chain_probe;
pub mod hotkey;
pub mod lightweight;
pub mod listener;
pub mod logger;
pub mod manager;
#[cfg(target_os = "macos")]
pub mod network_watch;
pub mod notify;
pub(crate) mod owner_identity;
#[cfg(windows)]
pub(crate) mod portable;
#[cfg(windows)]
pub(crate) mod portable_network;
#[cfg(windows)]
pub(crate) mod portable_proxy;
#[cfg(windows)]
pub(crate) mod process_identity;
pub mod proxy_control;
pub mod proxy_view;
#[cfg(windows)]
pub(crate) mod proxy_watchdog;
pub mod runstate;
pub(crate) mod runtime_bundle;
pub mod service;
pub mod sysopt;
pub mod timer;
pub mod tray;
pub mod updater;
pub mod validate;
pub mod win_uwp;

pub use self::{manager::CoreManager, timer::Timer, updater::SilentUpdater};
pub use notify::{handle, notification};
