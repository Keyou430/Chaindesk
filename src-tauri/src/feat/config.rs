use crate::config::{Config, IVerge};
use anyhow::Result;
use clash_verge_draft::SharedDraft;
use serde_yaml_ng::Mapping;
use tokio::sync::MutexGuard;

pub async fn patch_clash(patch: &Mapping) -> Result<()> {
    let config_write = Config::try_lock_config_write()?;
    super::executor::apply(
        &config_write,
        super::executor::Patch::Clash(patch),
        super::effects::clash_effects(patch),
    )
    .await
}

/// Apply a patch, then reconcile TUN when its setting changes.
///
/// TUN patches do not always produce a Run State transition, so reconciliation is explicit.
pub async fn patch_verge(patch: &IVerge, not_save_file: bool) -> Result<()> {
    apply_verge_patch(patch, not_save_file).await?;
    if patch.enable_tun_mode.is_some() {
        super::reconcile_tun_availability().await;
    }
    Ok(())
}

/// Apply a patch without post-update reconciliation.
pub(super) async fn apply_verge_patch(patch: &IVerge, not_save_file: bool) -> Result<()> {
    let config_write = Config::lock_config_write().await;
    apply_verge_patch_locked(&config_write, patch, not_save_file).await
}

/// Apply a patch with the shared configuration write lock already held.
/// Callers must pass the guard returned by [`Config::lock_config_write`].
pub(super) async fn apply_verge_patch_locked(
    _config_write: &MutexGuard<'_, ()>,
    patch: &IVerge,
    not_save_file: bool,
) -> Result<()> {
    #[cfg(windows)]
    if patch.enable_system_proxy == Some(true) && !crate::config::chain::session::proxy_permitted() {
        return Err(crate::config::chain::failure(
            "CHAIN_NOT_VALIDATED",
            "Use Detect and connect to validate the complete chain before enabling the system proxy.",
        ));
    }
    if cfg!(windows)
        && (patch.enable_tun_mode == Some(true)
            || patch.enable_auto_launch == Some(true)
            || patch.enable_proxy_guard == Some(true)
            || patch.auto_check_update == Some(true)
            || patch.enable_external_controller == Some(true)
            || patch.enable_auto_backup_schedule == Some(true)
            || patch.auto_backup_on_change == Some(true)
            || patch.enable_global_hotkey == Some(true)
            || patch.pac_file_content.is_some()
            || patch.system_proxy_bypass.is_some()
            || patch.use_default_bypass == Some(false)
            || patch.startup_script.as_ref().is_some_and(|script| !script.is_empty())
            || patch.proxy_host.as_ref().is_some_and(|host| host != "127.0.0.1")
            || patch.clash_core.as_ref().is_some_and(|core| core != "verge-mihomo"))
    {
        anyhow::bail!("This setting is unavailable in ChainDesk portable mode");
    }
    super::executor::apply(
        _config_write,
        super::executor::Patch::Verge {
            patch,
            persist: !not_save_file,
        },
        super::effects::verge_effects(patch),
    )
    .await
}

pub async fn fetch_verge_config() -> Result<SharedDraft<IVerge>> {
    let draft = Config::verge().await;
    let data = draft.data_arc();
    Ok(data)
}
