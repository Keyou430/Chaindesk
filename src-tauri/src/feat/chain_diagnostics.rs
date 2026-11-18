use crate::config::chain::{self, ChainPhase, ProbeObservation, RulePreset, session, store};
use anyhow::Result;
use serde::Serialize;
use sha2::{Digest as _, Sha256};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    schema_version: u32,
    application_version: &'static str,
    core_version: &'static str,
    capability_version: u32,
    phase: ChainPhase,
    health_checked_at: Option<String>,
    error_code: Option<String>,
    front_protocol: Option<String>,
    preset: RulePreset,
    probe_consent: bool,
    background_checks: bool,
    automatic_front: bool,
    expected_ipv4: Option<String>,
    observation: Option<ProbeObservation>,
    candidate_observation: Option<ProbeObservation>,
    proxy_recovery_pending: bool,
}

pub(crate) async fn preview() -> Result<chain::DiagnosticPreview> {
    let data = store::snapshot().await?;
    let status = session::snapshot();
    let report = Report {
        schema_version: 1,
        application_version: env!("CARGO_PKG_VERSION"),
        core_version: "1.19.33",
        capability_version: chain::CAPABILITY_VERSION,
        phase: status.phase,
        health_checked_at: status.health_checked_at,
        error_code: status.error.map(|error| error.code),
        front_protocol: data
            .secrets
            .subscription
            .as_ref()
            .and_then(|source| {
                source
                    .nodes
                    .iter()
                    .find(|node| Some(&node.id) == data.settings.selected_front_id.as_ref())
            })
            .map(|node| node.protocol.clone()),
        preset: data.settings.preset,
        probe_consent: data.settings.probe_consent,
        background_checks: data.settings.background_checks,
        automatic_front: data.settings.automatic_front,
        expected_ipv4: data.settings.expected_ipv4.clone(),
        observation: status.observation,
        candidate_observation: status.candidate_observation,
        proxy_recovery_pending: crate::utils::portable::paths()?
            .data
            .join("proxy-session.bin")
            .try_exists()?,
    };
    let json = serde_json::to_string_pretty(&report)?;
    let sha256 = Sha256::digest(json.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(chain::DiagnosticPreview { json, sha256 })
}

pub(crate) async fn export(path: String, expected_sha256: String) -> Result<()> {
    use std::io::Write as _;
    let preview = preview().await?;
    if preview.sha256 != expected_sha256 {
        return Err(chain::failure(
            "DIAGNOSTIC_CHANGED",
            "The diagnostic snapshot changed. Preview it again before exporting.",
        ));
    }
    let path = std::path::PathBuf::from(path);
    if !path.is_absolute()
        || path
            .extension()
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("json"))
    {
        return Err(chain::failure(
            "DIAGNOSTIC_PATH_INVALID",
            "Choose an absolute path for a new JSON diagnostic file.",
        ));
    }
    let mut file = crate::core::owner_identity::create_private_current_user_file(&path)?;
    let result = file.write_all(preview.json.as_bytes()).and_then(|()| file.sync_all());
    drop(file);
    if result.is_err() {
        let _ = std::fs::remove_file(&path);
    }
    result?;
    Ok(())
}
