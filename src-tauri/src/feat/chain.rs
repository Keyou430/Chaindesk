use crate::{
    config::{
        Config, IVerge, MixedPort,
        chain::{
            self, CAPABILITY_VERSION, ChainPhase, ChainView, ConnectInput, ExitAuthentication, ExitEndpoint, ExitInput,
            FrontView, MAX_SUBSCRIPTION_BYTES, builder, download, failure, importer, session, store,
        },
    },
    core::{CoreManager, chain_probe},
};
use anyhow::Result;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::io::AsyncReadExt as _;

pub(crate) async fn view() -> Result<ChainView> {
    let data = store::snapshot().await?;
    let mut status = session::snapshot();
    if data.cleanup_pending {
        status.phase = ChainPhase::RecoveryRequired;
        status.error = Some(chain::ChainErrorView {
            code: "CREDENTIAL_CLEANUP_PENDING".into(),
            message: store::CLEANUP_MESSAGE.into(),
        });
    }
    let subscription = data.secrets.subscription.as_ref();
    let fronts = subscription.map_or_else(Vec::new, |subscription| {
        subscription
            .nodes
            .iter()
            .map(|node| {
                let rejection = if data.settings.excluded_front_ids.contains(&node.id) {
                    Some("Excluded by you.".into())
                } else if data
                    .settings
                    .endpoint
                    .as_ref()
                    .is_some_and(|exit| exit.matches(&node.server, node.port))
                {
                    Some("This endpoint is also the configured exit.".into())
                } else {
                    node.rejection.clone()
                };
                FrontView {
                    id: node.id.clone(),
                    name: node.name.clone(),
                    protocol: node.protocol.clone(),
                    endpoint: format!("{}:{}", node.server, node.port),
                    rejection,
                    excluded: data.settings.excluded_front_ids.contains(&node.id),
                }
            })
            .collect()
    });
    let phase = if status.phase == ChainPhase::Unconfigured
        && data.settings.endpoint.is_some()
        && data.session_authentication.is_some()
        && fronts.iter().any(|node: &FrontView| node.rejection.is_none())
    {
        ChainPhase::Ready
    } else {
        status.phase
    };
    Ok(ChainView {
        phase,
        fronts,
        subscription_name: subscription.map(|s| s.name.clone()),
        subscription_updated_at: subscription.map(|s| s.imported_at.clone()),
        subscription_host: subscription
            .and_then(|s| s.update_url.as_ref())
            .and_then(|s| reqwest::Url::parse(s).ok())
            .and_then(|url| url.host_str().map(str::to_owned)),
        can_update_subscription: subscription.is_some_and(|s| s.update_url.is_some()),
        duplicates: subscription.map_or(0, |s| s.duplicates as u32),
        host: data
            .settings
            .endpoint
            .as_ref()
            .map_or_else(String::new, |exit| exit.host.clone()),
        port: data.settings.endpoint.as_ref().map_or(1080, |exit| exit.port),
        remember_authentication: data.settings.remember_authentication,
        authentication_required: data.settings.authentication_required,
        has_session_authentication: data.session_authentication.is_some(),
        has_saved_authentication: data
            .secrets
            .authentication
            .as_ref()
            .is_some_and(|auth| !auth.username.is_empty()),
        selected_front_id: data.settings.selected_front_id.clone(),
        preset: data.settings.preset,
        probe_consent: data.settings.probe_consent,
        background_checks: data.settings.background_checks,
        observation: status.observation,
        error: status.error,
        capability_version: CAPABILITY_VERSION,
    })
}

fn require_offline() -> Result<()> {
    if matches!(
        session::snapshot().phase,
        ChainPhase::Connected
            | ChainPhase::Degraded
            | ChainPhase::Connecting
            | ChainPhase::Probing
            | ChainPhase::Disconnecting
    ) {
        return Err(failure(
            "CHAIN_BUSY",
            "Disconnect before changing the subscription or exit credentials.",
        ));
    }
    Ok(())
}

pub(crate) async fn import_file(path: String) -> Result<ChainView> {
    let _operation = store::OPERATION.lock().await;
    require_offline()?;
    let mut bytes = Vec::new();
    tokio::fs::File::open(&path)
        .await
        .map_err(|_| {
            failure(
                "SUBSCRIPTION_READ_FAILED",
                "The selected subscription file could not be opened.",
            )
        })?
        .take(MAX_SUBSCRIPTION_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| {
            failure(
                "SUBSCRIPTION_READ_FAILED",
                "The selected subscription file could not be read.",
            )
        })?;
    if bytes.len() > MAX_SUBSCRIPTION_BYTES {
        return Err(failure(
            "SUBSCRIPTION_TOO_LARGE",
            "Subscriptions are limited to 10 MiB.",
        ));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| failure("SUBSCRIPTION_UNSUPPORTED", "The subscription must contain UTF-8 YAML."))?;
    let name = Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Subscription")
        .to_owned();
    import(text, name, None).await?;
    view().await
}

pub(crate) async fn import_https(url: String, local_proxy_port: Option<u16>) -> Result<ChainView> {
    let _operation = store::OPERATION.lock().await;
    require_offline()?;
    let text = download::fetch(&url, local_proxy_port).await?;
    let name = reqwest::Url::parse(&url)?
        .host_str()
        .unwrap_or("Subscription")
        .to_owned();
    import(text, name, Some(url)).await?;
    view().await
}

pub(crate) async fn update_subscription(local_proxy_port: Option<u16>) -> Result<ChainView> {
    let _operation = store::OPERATION.lock().await;
    require_offline()?;
    let data = store::snapshot().await?;
    let subscription = data
        .secrets
        .subscription
        .as_ref()
        .ok_or_else(|| failure("SUBSCRIPTION_UNSUPPORTED", "Import a subscription first."))?;
    let url = subscription.update_url.as_ref().ok_or_else(|| {
        failure(
            "SUBSCRIPTION_UNSUPPORTED",
            "Import a new local file to update this subscription.",
        )
    })?;
    let text = download::fetch(url, local_proxy_port).await?;
    import(text, subscription.name.clone(), Some(url.clone())).await?;
    view().await
}

async fn import(source: String, name: String, url: Option<String>) -> Result<()> {
    let subscription = crate::AsyncHandler::spawn_blocking(move || importer::parse(source, name, url)).await??;
    let mut next = (*store::snapshot().await?).clone();
    next.secrets.subscription = Some(subscription);
    next.settings.selected_front_id = None;
    next.settings.excluded_front_ids.clear();
    store::commit(next).await?;
    session::inputs_changed();
    Ok(())
}

pub(crate) async fn save_exit(input: ExitInput) -> Result<ChainView> {
    let _operation = store::OPERATION.lock().await;
    require_offline()?;
    let mut next = (*store::snapshot().await?).clone();
    let endpoint = ExitEndpoint {
        host: input.host,
        port: input.port,
    }
    .normalized()?;
    let authentication = match (input.username, input.password) {
        (Some(username), Some(password)) => ExitAuthentication { username, password },
        (None, None)
            if next
                .settings
                .endpoint
                .as_ref()
                .is_some_and(|old| old.matches(&endpoint.host, endpoint.port)) =>
        {
            next.session_authentication
                .clone()
                .ok_or_else(|| failure("EXIT_AUTH_REQUIRED", "Re-enter the exit username and password."))?
        }
        _ => {
            return Err(failure(
                "EXIT_AUTH_REQUIRED",
                "Enter both authentication fields, or leave both empty for no authentication.",
            ));
        }
    };
    authentication.validate()?;
    next.settings.authentication_required = !authentication.username.is_empty();
    next.settings.endpoint = Some(endpoint);
    next.settings.remember_authentication = input.remember;
    next.session_authentication = Some(authentication.clone());
    next.secrets.authentication = input.remember.then_some(authentication);
    store::commit(next).await?;
    session::inputs_changed();
    view().await
}

pub(crate) async fn forget_authentication() -> Result<ChainView> {
    let _operation = store::OPERATION.lock().await;
    require_offline()?;
    let mut next = (*store::snapshot().await?).clone();
    next.secrets.authentication = None;
    next.session_authentication = (!next.settings.authentication_required).then(ExitAuthentication::default);
    next.settings.remember_authentication = false;
    store::commit(next).await?;
    view().await
}

pub(crate) async fn exclude_front(id: String, excluded: bool) -> Result<ChainView> {
    let _operation = store::OPERATION.lock().await;
    require_offline()?;
    let mut next = (*store::snapshot().await?).clone();
    if !next
        .secrets
        .subscription
        .as_ref()
        .is_some_and(|s| s.nodes.iter().any(|node| node.id == id))
    {
        return Err(failure(
            "NO_VALID_FRONT",
            "Choose a node from the current subscription.",
        ));
    }
    next.settings.excluded_front_ids.retain(|current| current != &id);
    if excluded {
        next.settings.excluded_front_ids.push(id);
    }
    store::commit(next).await?;
    view().await
}

pub(crate) async fn connect(input: ConnectInput) -> Result<ChainView> {
    let _operation = store::OPERATION.lock().await;
    require_offline()?;
    if !input.probe_consent {
        return Err(failure(
            "PROBE_CONSENT_REQUIRED",
            "Allow the disclosed HTTPS and IPv4 services before testing the chain.",
        ));
    }
    let mut next = (*store::snapshot().await?).clone();
    if next.cleanup_pending {
        return Err(store::cleanup_failure());
    }
    let subscription = next
        .secrets
        .subscription
        .as_ref()
        .ok_or_else(|| failure("NO_VALID_FRONT", "Import a subscription first."))?;
    let front = subscription
        .nodes
        .iter()
        .find(|node| node.id == input.front_id)
        .ok_or_else(|| failure("NO_VALID_FRONT", "Choose a front from the current subscription."))?;
    if next.settings.excluded_front_ids.contains(&front.id) {
        return Err(failure("NO_VALID_FRONT", "This front is excluded."));
    }
    let endpoint = next
        .settings
        .endpoint
        .as_ref()
        .ok_or_else(|| failure("EXIT_ADDRESS_INVALID", "Configure the fixed SOCKS5 exit first."))?;
    let auth = next.session_authentication.as_ref().ok_or_else(|| {
        failure(
            "EXIT_AUTH_REQUIRED",
            "Re-enter the exit authentication after restarting.",
        )
    })?;
    let config = Arc::new(builder::build(
        front,
        endpoint,
        auth,
        input.preset,
        MixedPort::desired().await,
    )?);
    next.settings.selected_front_id = Some(input.front_id);
    next.settings.preset = input.preset;
    next.settings.probe_consent = true;
    next.settings.background_checks = input.background_checks;
    let token = session::begin();
    let previous = store::active();
    store::set_active(Some(config));
    let result = tokio::select! {
        _ = token.cancelled() => Err(failure("CHAIN_CANCELLED", "Connection testing was cancelled.")),
        result = tokio::time::timeout(Duration::from_secs(30), connect_working()) => {
            result.unwrap_or_else(|_| Err(failure("CHAIN_TIMEOUT", "The 30-second connection budget expired. Check the selected front and exit.")))
        }
    };
    match result {
        Ok(observation) => {
            if let Err(error) = store::commit(next).await {
                if let Err(cleanup) = cleanup_failed_connection(previous).await {
                    session::failed(&cleanup, ChainPhase::RecoveryRequired);
                    return Err(cleanup);
                }
                session::failed(&error, ChainPhase::Disconnected);
                return Err(error);
            }
            session::connected(observation);
            view().await
        }
        Err(error) => {
            if let Err(cleanup) = cleanup_failed_connection(previous).await {
                session::failed(&cleanup, ChainPhase::RecoveryRequired);
                return Err(cleanup);
            }
            session::failed(&error, ChainPhase::Disconnected);
            Err(error)
        }
    }
}

async fn connect_working() -> Result<chain::ProbeObservation> {
    {
        let _configuration = Config::lock_config_write().await;
        let result = CoreManager::global().update_config_forced().await?;
        if !result.is_valid() {
            return Err(failure(
                "CORE_CONFIG_INVALID",
                "The fixed core rejected the generated chain configuration.",
            ));
        }
    }
    let observation = chain_probe::probe(MixedPort::effective().await).await?;
    session::permit_proxy(true);
    super::patch_verge(
        &IVerge {
            enable_system_proxy: Some(true),
            ..IVerge::default()
        },
        false,
    )
    .await?;
    Ok(observation)
}

async fn cleanup_failed_connection(previous: Option<Arc<serde_yaml_ng::Mapping>>) -> Result<()> {
    session::permit_proxy(false);
    let proxy = super::patch_verge(
        &IVerge {
            enable_system_proxy: Some(false),
            ..IVerge::default()
        },
        false,
    )
    .await;
    store::set_active(previous);
    let core = CoreManager::global().stop_core().await;
    if proxy.is_err() || core.is_err() {
        return Err(failure(
            "RECOVERY_REQUIRED",
            "The connection failed and cleanup could not finish. Check system proxy ownership before retrying.",
        ));
    }
    clear_runtime().await
}

async fn clear_runtime() -> Result<()> {
    crate::core::portable::clear_runtime_configuration()?;
    let port = MixedPort::desired().await;
    let _configuration = Config::lock_config_write().await;
    let runtime = Config::runtime().await;
    runtime.edit_draft(|draft| {
        draft.config = Some(builder::blocked(port));
    });
    runtime.apply();
    Ok(())
}

pub(crate) async fn disconnect() -> Result<ChainView> {
    session::cancel();
    let _operation = store::OPERATION.lock().await;
    session::phase(ChainPhase::Disconnecting);
    session::permit_proxy(false);
    let restored = super::patch_verge(
        &IVerge {
            enable_system_proxy: Some(false),
            ..IVerge::default()
        },
        false,
    )
    .await;
    let stopped = CoreManager::global().stop_core().await;
    store::set_active(None);
    if restored.is_err() || stopped.is_err() {
        let error = failure(
            "RECOVERY_REQUIRED",
            "Disconnect could not finish. Check system proxy ownership before retrying.",
        );
        session::failed(&error, ChainPhase::RecoveryRequired);
        return Err(error);
    }
    if let Err(error) = clear_runtime().await {
        session::failed(&error, ChainPhase::RecoveryRequired);
        return Err(error);
    }
    session::phase(ChainPhase::Disconnected);
    view().await
}

pub(crate) fn cancel() {
    session::cancel();
}
