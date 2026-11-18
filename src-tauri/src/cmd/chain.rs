use super::{CmdResult, CommandFailure};
use crate::config::chain::{ChainView, ConnectInput, DiagnosticPreview, ExitInput};

#[cfg(windows)]
fn report(error: anyhow::Error) -> CommandFailure {
    if let Some(failure) = error.downcast_ref::<crate::config::chain::ChainFailure>() {
        CommandFailure::coded(failure.code, &failure.message)
    } else {
        CommandFailure::coded(
            "CHAIN_OPERATION_FAILED",
            "The operation failed. Check the writable data directory and try again; previous saved inputs are retained.",
        )
    }
}

macro_rules! portable {
    ($operation:expr) => {{
        #[cfg(windows)]
        {
            $operation.await.map_err(report)
        }
        #[cfg(not(windows))]
        {
            Err(CommandFailure::coded(
                "PLATFORM_UNSUPPORTED",
                "ChainDesk currently supports Windows x64.",
            ))
        }
    }};
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn get_chain_state() -> CmdResult<ChainView> {
    portable!(crate::feat::chain::view())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn import_chain_file(path: String) -> CmdResult<ChainView> {
    portable!(crate::feat::chain::import_file(path))
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn import_chain_https(url: String, local_proxy_port: Option<u16>) -> CmdResult<ChainView> {
    portable!(crate::feat::chain::import_https(url, local_proxy_port))
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn update_chain_subscription(local_proxy_port: Option<u16>) -> CmdResult<ChainView> {
    portable!(crate::feat::chain::update_subscription(local_proxy_port))
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn save_chain_exit(input: ExitInput) -> CmdResult<ChainView> {
    portable!(crate::feat::chain::save_exit(input))
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn forget_chain_authentication() -> CmdResult<ChainView> {
    portable!(crate::feat::chain::forget_authentication())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn exclude_chain_front(id: String, excluded: bool) -> CmdResult<ChainView> {
    portable!(crate::feat::chain::exclude_front(id, excluded))
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn connect_chain(input: ConnectInput) -> CmdResult<ChainView> {
    portable!(crate::feat::chain::connect(input))
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn disconnect_chain() -> CmdResult<ChainView> {
    portable!(crate::feat::chain::disconnect())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn cancel_chain_connection() -> CmdResult {
    #[cfg(windows)]
    crate::feat::chain::cancel();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn restore_chain_last_good() -> CmdResult<ChainView> {
    portable!(crate::feat::chain::restore_last_good())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn confirm_chain_ipv4(ipv4: String) -> CmdResult<ChainView> {
    portable!(crate::feat::chain::confirm_ipv4(ipv4))
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn preview_chain_diagnostics() -> CmdResult<DiagnosticPreview> {
    portable!(crate::feat::chain_diagnostics::preview())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn export_chain_diagnostics(path: String, expected_sha256: String) -> CmdResult {
    portable!(crate::feat::chain_diagnostics::export(path, expected_sha256))
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn keep_chain_current_proxy() -> CmdResult<ChainView> {
    portable!(crate::feat::chain::keep_current_proxy())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn set_chain_background_checks(enabled: bool) -> CmdResult<ChainView> {
    portable!(crate::feat::chain::set_background_checks(enabled))
}
