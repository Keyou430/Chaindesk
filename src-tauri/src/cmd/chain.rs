use super::{CmdResult, CommandFailure};
use crate::config::chain::{ChainView, ConnectInput, ExitInput};

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
