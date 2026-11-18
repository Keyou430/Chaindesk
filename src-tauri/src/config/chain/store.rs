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
    #[serde(default)]
    pub automatic_front: bool,
    pub excluded_front_ids: Vec<String>,
    #[serde(default)]
    pub last_good: Option<GoodInputs>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 2,
            credential_revision: String::new(),
            endpoint: None,
            remember_authentication: false,
            authentication_required: false,
            selected_front_id: None,
            preset: RulePreset::default(),
            expected_ipv4: None,
            probe_consent: false,
            background_checks: true,
            automatic_front: false,
            excluded_front_ids: Vec::new(),
            last_good: None,
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Secrets {
    pub subscription: Option<Subscription>,
    pub authentication: Option<ExitAuthentication>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GoodInputs {
    pub credential_revision: String,
    pub endpoint: ExitEndpoint,
    pub front_id: String,
    pub preset: RulePreset,
    pub expected_ipv4: Option<String>,
    pub authentication_required: bool,
    pub remember_authentication: bool,
    pub background_checks: bool,
    #[serde(default)]
    pub automatic_front: bool,
}

#[derive(Clone)]
pub(crate) struct GoodData {
    pub inputs: GoodInputs,
    pub secrets: Secrets,
    pub session_authentication: Option<ExitAuthentication>,
}

#[derive(Clone, Default)]
pub(crate) struct ChainData {
    pub settings: Settings,
    pub secrets: Secrets,
    pub session_authentication: Option<ExitAuthentication>,
    pub cleanup_pending: bool,
    pub last_good: Option<GoodData>,
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
    if !matches!(settings.schema_version, 1 | 2) {
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
    let mut secrets = records[desired].secrets.clone();
    parse_cached(&mut secrets)?;
    let last_good = settings
        .last_good
        .as_ref()
        .map(|inputs| {
            let mut secrets = records
                .iter()
                .find(|record| record.revision == inputs.credential_revision)
                .ok_or_else(|| failure("RECOVERY_REQUIRED", "The last working credential revision is missing."))?
                .secrets
                .clone();
            parse_cached(&mut secrets)?;
            let session_authentication = if inputs.authentication_required {
                secrets.authentication.clone()
            } else {
                Some(ExitAuthentication::default())
            };
            Ok::<_, anyhow::Error>(GoodData {
                inputs: inputs.clone(),
                secrets,
                session_authentication,
            })
        })
        .transpose()?;
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
        last_good,
    })
}

fn parse_cached(secrets: &mut Secrets) -> Result<()> {
    if let Some(cached) = secrets.subscription.take() {
        let imported_at = cached.imported_at;
        let mut subscription = importer::parse(cached.source, cached.name, cached.update_url)?;
        subscription.imported_at = imported_at;
        secrets.subscription = Some(subscription);
    }
    Ok(())
}

pub(crate) async fn commit(next: ChainData) -> Result<Arc<ChainData>> {
    commit_revision(next, false).await
}

pub(crate) async fn commit_good(next: ChainData) -> Result<Arc<ChainData>> {
    commit_revision(next, true).await
}

async fn commit_revision(mut next: ChainData, good: bool) -> Result<Arc<ChainData>> {
    let previous = snapshot().await?;
    next.settings.schema_version = 2;
    next.settings.credential_revision = random_token()?;
    if !next.settings.remember_authentication || !next.settings.authentication_required {
        next.secrets.authentication = None;
        if let Some(last) = next.last_good.as_mut() {
            last.secrets.authentication = None;
        }
    }
    if good {
        let inputs = GoodInputs {
            credential_revision: next.settings.credential_revision.clone(),
            endpoint: next
                .settings
                .endpoint
                .clone()
                .ok_or_else(|| failure("EXIT_ADDRESS_INVALID", "The working exit is missing."))?,
            front_id: next
                .settings
                .selected_front_id
                .clone()
                .ok_or_else(|| failure("NO_VALID_FRONT", "The working front is missing."))?,
            preset: next.settings.preset,
            expected_ipv4: next.settings.expected_ipv4.clone(),
            authentication_required: next.settings.authentication_required,
            remember_authentication: next.settings.remember_authentication,
            background_checks: next.settings.background_checks,
            automatic_front: next.settings.automatic_front,
        };
        next.last_good = Some(GoodData {
            inputs: inputs.clone(),
            secrets: next.secrets.clone(),
            session_authentication: next.session_authentication.clone(),
        });
        next.settings.last_good = Some(inputs);
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
    if previous.settings.schema_version == 1 && !previous.settings.credential_revision.is_empty() {
        let backup = root.join("backups/schema-1-migration.bin");
        if !backup.exists() {
            crate::core::owner_identity::create_private_current_user_directory(&root.join("backups"))?;
            let mut secrets = previous.secrets.clone();
            secrets.authentication = None;
            let mut settings = previous.settings.clone();
            settings.remember_authentication = false;
            help::save_private_bytes(
                &backup,
                &protection::protect(&serde_json::to_vec(&(settings, secrets))?)?,
            )?;
        }
    }
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
    if let Some(last) = next.last_good.as_ref()
        && !records
            .iter()
            .any(|record| record.revision == last.inputs.credential_revision)
    {
        records.push(CredentialRecord {
            revision: last.inputs.credential_revision.clone(),
            secrets: last.secrets.clone(),
        });
    }
    if let Some(last) = previous.last_good.as_ref()
        && !records
            .iter()
            .any(|record| record.revision == last.inputs.credential_revision)
    {
        records.push(CredentialRecord {
            revision: last.inputs.credential_revision.clone(),
            secrets: last.secrets.clone(),
        });
    }
    // The previous revision stays complete until the settings pointer commits.
    write_records(root, &records)?;
    help::save_private_bytes(&root.join("settings.json"), &serde_json::to_vec_pretty(&next.settings)?)?;
    scrub_unremembered_authentication(&mut records, &next.settings);
    records.retain(|record| {
        record.revision == next.settings.credential_revision
            || next
                .settings
                .last_good
                .as_ref()
                .is_some_and(|last| last.credential_revision == record.revision)
    });
    if write_records(root, &records).is_err() {
        return Ok(SaveOutcome::CleanupPending);
    }
    Ok(SaveOutcome::Complete)
}

pub(crate) fn restored(current: &ChainData) -> Result<ChainData> {
    let last = current
        .last_good
        .as_ref()
        .ok_or_else(|| failure("RECOVERY_REQUIRED", "No last working configuration is available."))?;
    let mut next = current.clone();
    next.settings.endpoint = Some(last.inputs.endpoint.clone());
    next.settings.selected_front_id = Some(last.inputs.front_id.clone());
    next.settings.preset = last.inputs.preset;
    next.settings.expected_ipv4 = last.inputs.expected_ipv4.clone();
    next.settings.authentication_required = last.inputs.authentication_required;
    next.settings.remember_authentication =
        last.inputs.remember_authentication && last.secrets.authentication.is_some();
    next.settings.background_checks = last.inputs.background_checks;
    next.settings.automatic_front = last.inputs.automatic_front;
    next.settings.probe_consent = false;
    next.settings.excluded_front_ids.clear();
    next.secrets = last.secrets.clone();
    next.session_authentication = last.session_authentication.clone();
    Ok(next)
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
    fn a_failed_good_revision_commit_retains_the_old_good_inputs_and_secret() -> anyhow::Result<()> {
        let root = std::env::temp_dir()
            .canonicalize()?
            .join(format!("chaindesk-good-{}", random_token()?));
        create_private_current_user_directory(&root)?;
        let root = scopeguard::guard(root, |root| {
            let _ = std::fs::remove_dir_all(root);
        });
        let mut good = ChainData::default();
        good.settings.credential_revision = "good".into();
        good.settings.remember_authentication = true;
        good.settings.authentication_required = true;
        good.secrets.authentication = Some(ExitAuthentication {
            username: "fixture".into(),
            password: "last-good-sentinel".into(),
        });
        let inputs = super::GoodInputs {
            credential_revision: "good".into(),
            endpoint: super::ExitEndpoint {
                host: "exit.example".into(),
                port: 1080,
            },
            front_id: "front".into(),
            preset: super::RulePreset::AllProxy,
            expected_ipv4: Some("203.0.113.1".into()),
            authentication_required: true,
            remember_authentication: true,
            background_checks: false,
            automatic_front: false,
        };
        good.settings.last_good = Some(inputs.clone());
        good.last_good = Some(super::GoodData {
            inputs,
            secrets: good.secrets.clone(),
            session_authentication: good.secrets.authentication.clone(),
        });
        save_at(&root, &ChainData::default(), &good)?;
        let mut desired = good.clone();
        desired.settings.credential_revision = "desired".into();
        desired.secrets.authentication = Some(ExitAuthentication {
            username: "fixture".into(),
            password: "unverified-desired-sentinel".into(),
        });
        save_at(&root, &good, &desired)?;
        let mut candidate = desired.clone();
        candidate.settings.credential_revision = "candidate".into();
        let mut inputs = candidate
            .settings
            .last_good
            .clone()
            .ok_or_else(|| anyhow::anyhow!("missing fixture good inputs"))?;
        inputs.credential_revision = "candidate".into();
        candidate.settings.last_good = Some(inputs.clone());
        candidate.last_good = Some(super::GoodData {
            inputs,
            secrets: candidate.secrets.clone(),
            session_authentication: candidate.secrets.authentication.clone(),
        });
        let settings = root.join("settings.json");
        let permissions = settings.metadata()?.permissions();
        let restore = scopeguard::guard((settings.clone(), permissions.clone()), |(path, permissions)| {
            let _ = std::fs::set_permissions(path, permissions);
        });
        let mut readonly = permissions.clone();
        readonly.set_readonly(true);
        std::fs::set_permissions(&settings, readonly)?;
        assert!(save_at(&root, &desired, &candidate).is_err());
        let reloaded = load_at(&root)?;
        assert_eq!(reloaded.settings.credential_revision, "desired");
        let restored = super::restored(&reloaded)?;
        assert_eq!(
            restored
                .secrets
                .authentication
                .as_ref()
                .map(|auth| auth.password.as_str()),
            Some("last-good-sentinel")
        );
        assert_eq!(restored.settings.expected_ipv4.as_deref(), Some("203.0.113.1"));
        assert!(!restored.settings.probe_consent);
        std::fs::set_permissions(&settings, permissions)?;
        drop(restore);
        candidate.settings.remember_authentication = false;
        candidate.secrets.authentication = None;
        save_at(&root, &desired, &candidate)?;
        assert!(super::restored(&load_at(&root)?)?.session_authentication.is_none());
        Ok(())
    }

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
