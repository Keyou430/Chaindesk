use super::{ExitAuthentication, ExitEndpoint, RulePreset, Subscription, failure, importer, protection};
use crate::utils::{dirs, help, server::random_token};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, LazyLock};
use tokio::sync::{Mutex, OnceCell, RwLock};

const MAX_PROTECTED_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const CLEANUP_MESSAGE: &str = "Settings were saved, but old protected credentials could not be removed. Retry deleting saved authentication before connecting.";

pub(crate) fn cleanup_failure() -> anyhow::Error {
    failure("CREDENTIAL_CLEANUP_PENDING", CLEANUP_MESSAGE)
}

enum SaveOutcome {
    Complete,
    CleanupPending,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Settings {
    pub schema_version: u32,
    pub credential_revision: String,
    pub endpoint: Option<ExitEndpoint>,
    pub remember_authentication: bool,
    pub authentication_required: bool,
    pub selected_front_id: Option<String>,
    pub preset: RulePreset,
    pub expected_ipv4: Option<String>,
    pub probe_consent: bool,
    pub background_checks: bool,
    pub excluded_front_ids: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            credential_revision: String::new(),
            endpoint: None,
            remember_authentication: false,
            authentication_required: false,
            selected_front_id: None,
            preset: RulePreset::default(),
            expected_ipv4: None,
            probe_consent: false,
            background_checks: true,
            excluded_front_ids: Vec::new(),
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Secrets {
    pub subscription: Option<Subscription>,
    pub authentication: Option<ExitAuthentication>,
}

#[derive(Clone, Default)]
pub(crate) struct ChainData {
    pub settings: Settings,
    pub secrets: Secrets,
    pub session_authentication: Option<ExitAuthentication>,
    pub cleanup_pending: bool,
}

#[derive(Serialize, Deserialize)]
struct CredentialRecord {
    revision: String,
    secrets: Secrets,
}

static DATA: OnceCell<RwLock<Arc<ChainData>>> = OnceCell::const_new();
pub(crate) static OPERATION: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static ACTIVE: arc_swap::ArcSwapOption<serde_yaml_ng::Mapping> = arc_swap::ArcSwapOption::const_empty();

pub(crate) fn active() -> Option<Arc<serde_yaml_ng::Mapping>> {
    ACTIVE.load_full()
}
pub(crate) fn set_active(mapping: Option<Arc<serde_yaml_ng::Mapping>>) {
    ACTIVE.store(mapping);
}

async fn data() -> Result<&'static RwLock<Arc<ChainData>>> {
    DATA.get_or_try_init(|| async {
        let value = crate::AsyncHandler::spawn_blocking(load).await??;
        Ok(RwLock::new(Arc::new(value)))
    })
    .await
}

pub(crate) async fn snapshot() -> Result<Arc<ChainData>> {
    Ok(Arc::clone(&*data().await?.read().await))
}

fn load() -> Result<ChainData> {
    let root = dirs::app_home_dir()?;
    load_at(&root)
}

fn load_at(root: &std::path::Path) -> Result<ChainData> {
    let settings_path = root.join("settings.json");
    if !settings_path.exists() {
        return Ok(ChainData::default());
    }
    let settings: Settings =
        serde_json::from_slice(&help::read_private_bytes(&settings_path, 1024 * 1024)?).map_err(|_| {
            failure(
                "RECOVERY_REQUIRED",
                "Saved settings cannot be read. Keep a copy of the data directory before recovery.",
            )
        })?;
    if settings.schema_version != 1 {
        return Err(failure(
            "RECOVERY_REQUIRED",
            "This settings version requires a compatible application version.",
        ));
    }
    let mut records: Vec<CredentialRecord> = serde_json::from_slice(&protection::unprotect(
        &help::read_private_bytes(&root.join("credentials.bin"), MAX_PROTECTED_BYTES)?,
    )?)
    .map_err(|_| {
        failure(
            "RECOVERY_REQUIRED",
            "The protected subscription or credential data is incomplete.",
        )
    })?;
    let desired = records
        .iter()
        .position(|record| record.revision == settings.credential_revision)
        .ok_or_else(|| {
            failure(
                "RECOVERY_REQUIRED",
                "Settings and credentials refer to different revisions.",
            )
        })?;
    if scrub_unremembered_authentication(&mut records, &settings) {
        write_records(root, &records).map_err(|_| cleanup_failure())?;
    }
    let mut secrets = records.swap_remove(desired).secrets;
    if let Some(cached) = secrets.subscription.take() {
        let imported_at = cached.imported_at;
        let mut subscription = importer::parse(cached.source, cached.name, cached.update_url)?;
        subscription.imported_at = imported_at;
        secrets.subscription = Some(subscription);
    }
    let session_authentication = if settings.authentication_required {
        secrets.authentication.clone()
    } else {
        Some(ExitAuthentication::default())
    };
    Ok(ChainData {
        settings,
        secrets,
        session_authentication,
        cleanup_pending: false,
    })
}

pub(crate) async fn commit(mut next: ChainData) -> Result<Arc<ChainData>> {
    let previous = snapshot().await?;
    next.settings.credential_revision = random_token()?;
    if !next.settings.remember_authentication || !next.settings.authentication_required {
        next.secrets.authentication = None;
    }
    let current = next.clone();
    let outcome = crate::AsyncHandler::spawn_blocking(move || save(&previous, &current)).await??;
    next.cleanup_pending = matches!(outcome, SaveOutcome::CleanupPending);
    let next = Arc::new(next);
    *data().await?.write().await = Arc::clone(&next);
    if next.cleanup_pending {
        return Err(cleanup_failure());
    }
    Ok(next)
}

fn save(previous: &ChainData, next: &ChainData) -> Result<SaveOutcome> {
    let root = dirs::app_home_dir()?;
    save_at(&root, previous, next)
}

fn save_at(root: &std::path::Path, previous: &ChainData, next: &ChainData) -> Result<SaveOutcome> {
    let mut records = Vec::new();
    if !previous.settings.credential_revision.is_empty() {
        records.push(CredentialRecord {
            revision: previous.settings.credential_revision.clone(),
            secrets: previous.secrets.clone(),
        });
    }
    records.push(CredentialRecord {
        revision: next.settings.credential_revision.clone(),
        secrets: next.secrets.clone(),
    });
    // The previous revision stays complete until the settings pointer commits.
    write_records(root, &records)?;
    help::save_private_bytes(&root.join("settings.json"), &serde_json::to_vec_pretty(&next.settings)?)?;
    if scrub_unremembered_authentication(&mut records, &next.settings) && write_records(root, &records).is_err() {
        return Ok(SaveOutcome::CleanupPending);
    }
    Ok(SaveOutcome::Complete)
}

fn scrub_unremembered_authentication(records: &mut [CredentialRecord], settings: &Settings) -> bool {
    let mut changed = false;
    if !settings.remember_authentication || !settings.authentication_required {
        for record in records {
            changed |= record.secrets.authentication.take().is_some();
        }
    }
    changed
}

fn write_records(root: &std::path::Path, records: &[CredentialRecord]) -> Result<()> {
    let protected = protection::protect(&serde_json::to_vec(records)?)?;
    if protected.len() > MAX_PROTECTED_BYTES {
        return Err(failure(
            "SUBSCRIPTION_TOO_LARGE",
            "The protected subscription cache is too large.",
        ));
    }
    help::save_private_bytes(&root.join("credentials.bin"), &protected)
}

#[cfg(test)]
mod tests {
    use super::{ChainData, CredentialRecord, ExitAuthentication, load_at, protection, save_at, write_records};
    use crate::{
        core::owner_identity::create_private_current_user_directory,
        utils::{help, server::random_token},
    };

    #[test]
    fn failed_settings_replace_keeps_the_previous_secret_and_forgetting_removes_it() -> anyhow::Result<()> {
        let parent = std::env::temp_dir().canonicalize()?;
        let root = parent.join(format!("chaindesk-state-{}", random_token()?));
        create_private_current_user_directory(&root)?;
        let cleanup = scopeguard::guard(root, |root| {
            if root.parent() == Some(parent.as_path())
                && root
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("chaindesk-state-"))
            {
                let _ = std::fs::remove_dir_all(root);
            }
        });
        let mut previous = ChainData::default();
        previous.settings.credential_revision = "previous".into();
        previous.settings.remember_authentication = true;
        previous.settings.authentication_required = true;
        previous.secrets.authentication = Some(ExitAuthentication {
            username: "fixture-user".into(),
            password: "previous-secret-sentinel".into(),
        });
        save_at(&cleanup, &ChainData::default(), &previous)?;
        let mut next = previous.clone();
        next.settings.credential_revision = "next".into();
        next.secrets
            .authentication
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("missing fixture"))?
            .password = "next-secret-sentinel".into();
        let settings = cleanup.join("settings.json");
        let original_permissions = settings.metadata()?.permissions();
        let restore_permissions = scopeguard::guard(
            (settings.clone(), original_permissions.clone()),
            |(path, permissions)| {
                let _ = std::fs::set_permissions(path, permissions);
            },
        );
        let mut permissions = original_permissions.clone();
        permissions.set_readonly(true);
        std::fs::set_permissions(&settings, permissions)?;
        assert!(save_at(&cleanup, &previous, &next).is_err());
        let reloaded = load_at(&cleanup)?;
        assert_eq!(
            reloaded
                .secrets
                .authentication
                .as_ref()
                .map(|auth| auth.password.as_str()),
            Some("previous-secret-sentinel")
        );
        let mut forgotten = next.clone();
        forgotten.settings.remember_authentication = false;
        forgotten.secrets.authentication = None;
        assert!(save_at(&cleanup, &previous, &forgotten).is_err());
        assert_eq!(
            load_at(&cleanup)?
                .secrets
                .authentication
                .as_ref()
                .map(|auth| auth.password.as_str()),
            Some("previous-secret-sentinel")
        );
        std::fs::set_permissions(&settings, original_permissions)?;
        drop(restore_permissions);
        next.settings.remember_authentication = false;
        next.secrets.authentication = None;
        save_at(&cleanup, &previous, &next)?;
        let raw = help::read_private_bytes(&cleanup.join("credentials.bin"), 1024 * 1024)?;
        assert!(
            !raw.windows(b"secret-sentinel".len())
                .any(|window| window == b"secret-sentinel")
        );
        let mut records: Vec<CredentialRecord> = serde_json::from_slice(&protection::unprotect(&raw)?)?;
        assert!(records.iter().all(|record| record.secrets.authentication.is_none()));
        records[0].secrets.authentication = previous.secrets.authentication;
        write_records(&cleanup, &records)?;
        assert!(load_at(&cleanup)?.secrets.authentication.is_none());
        let recovered: Vec<CredentialRecord> = serde_json::from_slice(&protection::unprotect(
            &help::read_private_bytes(&cleanup.join("credentials.bin"), 1024 * 1024)?,
        )?)?;
        assert!(recovered.iter().all(|record| record.secrets.authentication.is_none()));
        Ok(())
    }
}
