use crate::{
    config::{
        Config, IVerge, MixedPort,
        chain::{
            self, CAPABILITY_VERSION, ChainPhase, ChainView, ConnectInput, ExitAuthentication, ExitEndpoint, ExitInput,
            FrontView, MAX_SUBSCRIPTION_BYTES, builder, download, failure, importer, session, store,
        },
    },
    core::{CoreManager, chain_diagnostic::Diagnostic, chain_probe},
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
    if crate::core::portable_proxy::has_conflict() {
        status.phase = ChainPhase::RecoveryRequired;
        status.error = Some(chain::ChainErrorView { code: "RESTORE_CONFLICT".into(), message: "Another application changed the system proxy. Its current settings were preserved. Keep the current settings to acknowledge this recovery conflict.".into() });
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
        expected_ipv4: data.settings.expected_ipv4.clone(),
        can_restore_last_good: data.last_good.is_some(),
        automatic_front: data.settings.automatic_front,
        observation: status.observation,
        candidate_observation: status.candidate_observation,
        health_checked_at: status.health_checked_at,
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
    apply_exit(&mut next, input)?;
    store::commit(next).await?;
    session::inputs_changed();
    view().await
}

fn apply_exit(next: &mut store::ChainData, input: ExitInput) -> Result<()> {
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
    Ok(())
}

pub(crate) async fn forget_authentication() -> Result<ChainView> {
    let _operation = store::OPERATION.lock().await;
    require_offline()?;
    let mut next = (*store::snapshot().await?).clone();
    next.secrets.authentication = None;
    next.session_authentication = (!next.settings.authentication_required).then(ExitAuthentication::default);
    next.settings.remember_authentication = false;
    if let Some(last) = next.last_good.as_mut() {
        last.secrets.authentication = None;
        last.session_authentication = (!last.inputs.authentication_required).then(ExitAuthentication::default);
        last.inputs.remember_authentication = false;
    }
    if let Some(last) = next.settings.last_good.as_mut() {
        last.remember_authentication = false;
    }
    store::commit(next).await?;
    session::inputs_changed();
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
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let _operation = tokio::time::timeout_at(deadline, store::OPERATION.lock())
        .await
        .map_err(|_| {
            failure(
                "CHAIN_TIMEOUT",
                "The 30-second connection budget expired while waiting for the active operation.",
            )
        })?;
    if matches!(
        session::snapshot().phase,
        ChainPhase::Connecting | ChainPhase::Probing | ChainPhase::Disconnecting
    ) {
        return Err(failure(
            "CHAIN_BUSY",
            "Wait for the current connection operation or cancel it.",
        ));
    }
    if !input.probe_consent {
        return Err(failure(
            "PROBE_CONSENT_REQUIRED",
            "Allow the disclosed HTTPS and IPv4 services before testing the chain.",
        ));
    }
    let previous_data = store::snapshot().await?;
    let mut next = (*previous_data).clone();
    if next.cleanup_pending {
        return Err(store::cleanup_failure());
    }
    if let Some(exit) = input.exit {
        apply_exit(&mut next, exit)?;
    }
    if let Some(expected) = input.expected_ipv4 {
        next.settings.expected_ipv4 = if expected.trim().is_empty() {
            None
        } else {
            Some(
                expected
                    .trim()
                    .parse::<std::net::Ipv4Addr>()
                    .map_err(|_| failure("EXIT_ADDRESS_INVALID", "The expected exit must be an IPv4 address."))?
                    .to_string(),
            )
        };
    }
    next.settings.preset = input.preset;
    next.settings.probe_consent = true;
    next.settings.background_checks = input.background_checks;
    next.settings.automatic_front = input.automatic_front;
    let candidates = candidates(&next, &input.front_id, input.automatic_front)?;
    let previous = store::active();
    let close_existing = input.close_existing_connections && previous.is_some();
    let previous_phase = session::snapshot().phase;
    let token = session::begin();
    session::stop_monitor();
    session::phase(ChainPhase::Probing);
    let applied = std::sync::atomic::AtomicBool::new(false);
    let operation = apply_candidate(&mut next, candidates, &applied);
    let result = tokio::select! {
        _ = token.cancelled() => Err(failure("CHAIN_CANCELLED", "Connection testing was cancelled.")),
        result = tokio::time::timeout_at(deadline, operation) => {
            result.unwrap_or_else(|_| Err(failure("CHAIN_TIMEOUT", "The 30-second connection budget expired. The previous working chain is retained.")))
        }
    };
    let result = match result {
        Ok(observation) => match store::commit_good(next).await {
            Ok(_) => Ok(observation),
            Err(error)
                if error
                    .downcast_ref::<chain::ChainFailure>()
                    .is_some_and(|error| error.code == "CREDENTIAL_CLEANUP_PENDING") =>
            {
                let cleanup = cleanup_failed_connection(None).await;
                session::failed(cleanup.as_ref().err().unwrap_or(&error), ChainPhase::RecoveryRequired);
                return Err(cleanup.err().unwrap_or(error));
            }
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    match result {
        Ok(observation) => {
            session::candidate_observe(None);
            session::connected(observation);
            start_monitor();
            if close_existing
                && crate::core::handle::Handle::mihomo()
                    .close_all_connections()
                    .await
                    .is_err()
            {
                let error = failure(
                    "CONNECTION_CLOSE_FAILED",
                    "The new working chain was committed, but existing connections could not all be closed.",
                );
                session::failed(&error, ChainPhase::Connected);
                return Err(error);
            }
            view().await
        }
        Err(error) => {
            if let Some(previous) = previous {
                if applied.load(std::sync::atomic::Ordering::Acquire) {
                    rollback_working(previous, &previous_data, &error).await?;
                } else {
                    session::failed(&error, previous_phase);
                }
                start_monitor();
            } else {
                if let Err(cleanup) = cleanup_failed_connection(None).await {
                    session::failed(&cleanup, ChainPhase::RecoveryRequired);
                    return Err(cleanup);
                }
                session::failed(&error, ChainPhase::Disconnected);
            }
            Err(error)
        }
    }
}

async fn apply_candidate(
    next: &mut store::ChainData,
    candidates: Vec<chain::FrontNode>,
    applied: &std::sync::atomic::AtomicBool,
) -> Result<Option<chain::ProbeObservation>> {
    let (front_id, config) = select_candidate(next, candidates, next.settings.automatic_front).await?;
    next.settings.selected_front_id = Some(front_id);
    session::phase(ChainPhase::Connecting);
    applied.store(true, std::sync::atomic::Ordering::Release);
    store::set_active(Some(Arc::new(config)));
    apply_working().await?;
    let observation = chain_probe::probe(MixedPort::effective().await).await?;
    session::observe(observation.clone());
    if let Err(error) = chain_probe::verify(observation.as_ref(), next.settings.expected_ipv4.as_deref()) {
        session::candidate_observe(observation);
        return Err(error);
    }
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

async fn rollback_working(
    previous: Arc<serde_yaml_ng::Mapping>,
    data: &store::ChainData,
    error: &anyhow::Error,
) -> Result<()> {
    store::set_active(Some(previous));
    let recovered = tokio::time::timeout(Duration::from_secs(30), async {
        apply_working().await?;
        let observation = chain_probe::probe(MixedPort::effective().await).await?;
        chain_probe::verify(observation.as_ref(), data.settings.expected_ipv4.as_deref())?;
        super::patch_verge(
            &IVerge {
                enable_system_proxy: Some(true),
                ..IVerge::default()
            },
            false,
        )
        .await?;
        Ok::<_, anyhow::Error>(observation)
    })
    .await
    .unwrap_or_else(|_| {
        Err(failure(
            "RECOVERY_REQUIRED",
            "Restoring the previous working chain timed out.",
        ))
    });
    match recovered {
        Ok(observation) => {
            session::connected(observation);
            session::failed(error, ChainPhase::Connected);
        }
        Err(rollback) => {
            let port = MixedPort::effective().await;
            store::set_active(Some(Arc::new(builder::blocked(port))));
            if apply_working().await.is_err() {
                CoreManager::global().stop_core().await?;
            }
            session::failed(&rollback, ChainPhase::Degraded);
        }
    }
    Ok(())
}

fn candidates(data: &store::ChainData, selected: &str, automatic: bool) -> Result<Vec<chain::FrontNode>> {
    let endpoint = data
        .settings
        .endpoint
        .as_ref()
        .ok_or_else(|| failure("EXIT_ADDRESS_INVALID", "Configure the fixed SOCKS5 exit first."))?;
    let subscription = data
        .secrets
        .subscription
        .as_ref()
        .ok_or_else(|| failure("NO_VALID_FRONT", "Import a subscription first."))?;
    let mut nodes = subscription
        .nodes
        .iter()
        .filter(|node| {
            node.rejection.is_none()
                && !data.settings.excluded_front_ids.contains(&node.id)
                && !endpoint.matches(&node.server, node.port)
                && (automatic || node.id == selected)
        })
        .cloned()
        .collect::<Vec<_>>();
    nodes.sort_by_key(|node| node.id != selected);
    nodes.truncate(if automatic { 3 } else { 1 });
    if nodes.is_empty() {
        return Err(failure(
            "NO_VALID_FRONT",
            "Choose an eligible front with a different endpoint from the exit.",
        ));
    }
    Ok(nodes)
}

async fn select_candidate(
    data: &store::ChainData,
    candidates: Vec<chain::FrontNode>,
    automatic: bool,
) -> Result<(String, serde_yaml_ng::Mapping)> {
    let endpoint = data
        .settings
        .endpoint
        .as_ref()
        .ok_or_else(|| failure("EXIT_ADDRESS_INVALID", "The exit is missing."))?;
    let auth = data
        .session_authentication
        .as_ref()
        .ok_or_else(|| failure("EXIT_AUTH_REQUIRED", "Re-enter exit authentication after restarting."))?;
    let port = MixedPort::desired().await;
    let configurations = candidates
        .into_iter()
        .map(|front| {
            builder::build(&front, endpoint, auth, data.settings.preset, port).map(|config| (front.id, config))
        })
        .collect::<Result<Vec<_>>>()?;
    let screened = if automatic {
        futures::future::join_all(configurations.into_iter().map(|(id, config)| async move {
            let diagnostic = Diagnostic::start(builder::front_probe(config.clone())).await?;
            chain_probe::https(diagnostic.port).await?;
            Ok::<_, anyhow::Error>((id, config))
        }))
        .await
        .into_iter()
        .filter_map(Result::ok)
        .collect::<Vec<_>>()
    } else {
        configurations
    };
    if screened.is_empty() {
        return Err(failure(
            "FRONT_PROBE_FAILED",
            "None of the three selected front candidates completed a verified HTTPS request.",
        ));
    }
    let results = futures::future::join_all(screened.into_iter().map(|(id, config)| async move {
        let diagnostic = Diagnostic::start(config.clone()).await?;
        let started = std::time::Instant::now();
        let observation = chain_probe::probe(diagnostic.port).await?;
        if let Err(error) = chain_probe::verify(observation.as_ref(), data.settings.expected_ipv4.as_deref()) {
            session::candidate_observe(observation);
            return Err(error);
        }
        Ok::<_, anyhow::Error>((
            id,
            config,
            started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
        ))
    }))
    .await;
    let mut best = None;
    let mut last_error = failure(
        "FRONT_TO_EXIT_FAILED",
        "The candidate fronts could not complete the chain through the fixed exit.",
    );
    for result in results {
        match result {
            Ok(candidate)
                if best
                    .as_ref()
                    .is_none_or(|best: &(String, serde_yaml_ng::Mapping, u32)| candidate.2 < best.2) =>
            {
                best = Some(candidate)
            }
            Ok(_) => {}
            Err(error) => last_error = error,
        }
    }
    best.map(|(id, config, _)| (id, config)).ok_or(last_error)
}

async fn apply_working() -> Result<()> {
    let _configuration = Config::lock_config_write().await;
    let result = CoreManager::global().update_config_forced().await?;
    if !result.is_valid() {
        return Err(failure(
            "CORE_CONFIG_INVALID",
            "The fixed core rejected the generated chain configuration.",
        ));
    }
    Ok(())
}

pub(crate) fn working_core_terminated() {
    crate::AsyncHandler::spawn(|| async {
        let _operation = store::OPERATION.lock().await;
        if store::active().is_none()
            || !matches!(
                *CoreManager::global().get_running_mode(),
                crate::core::manager::RunningMode::NotRunning
            )
        {
            return;
        }
        session::stop_monitor();
        session::permit_proxy(false);
        if clear_runtime().await.is_err() {
            session::failed(
                &failure(
                    "RECOVERY_REQUIRED",
                    "The working core stopped, but its private temporary configuration could not be cleared. Disconnect and review recovery.",
                ),
                ChainPhase::RecoveryRequired,
            );
        } else if matches!(session::snapshot().phase, ChainPhase::Connected | ChainPhase::Degraded) {
            session::failed(
                &failure(
                    "CORE_EXITED",
                    "The working core exited. The owned proxy path was retained; reconnect or disconnect to restore the previous proxy.",
                ),
                ChainPhase::Degraded,
            );
        }
    });
}

fn start_monitor() {
    if !session::proxy_permitted() || crate::core::portable_proxy::has_conflict() {
        session::stop_monitor();
        return;
    }
    let token = session::monitor_token();
    crate::AsyncHandler::spawn(move || async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_probe = std::time::Instant::now();
        let mut last_front = last_probe;
        let mut last_ipv4 = last_probe;
        let mut last_tick = std::time::SystemTime::now();
        let mut network = network_fingerprint();
        let network_watch = crate::core::portable_network::NetworkWatch::start().ok();
        let mut failures = 0_u32;
        loop {
            tokio::select! { _ = token.cancelled() => break, _ = interval.tick() => {} }
            let Ok(_operation) = store::OPERATION.try_lock() else {
                continue;
            };
            let Ok(data) = store::snapshot().await else {
                break;
            };
            if !data.settings.background_checks
                || store::active().is_none()
                || !session::proxy_permitted()
                || crate::core::portable_proxy::has_conflict()
                || !crate::core::proxy_watchdog::is_available().await
            {
                break;
            }
            let current = network_fingerprint();
            let woke = last_tick
                .elapsed()
                .is_ok_and(|elapsed| elapsed > Duration::from_secs(20));
            last_tick = std::time::SystemTime::now();
            let changed =
                current != network || woke || network_watch.as_ref().is_some_and(|watch| watch.take_changed());
            network = current;
            if !changed && last_probe.elapsed() < Duration::from_secs(60) {
                continue;
            }
            if changed {
                session::failed(
                    &failure(
                        "NETWORK_CHANGED",
                        "The network changed or resumed. Rechecking the fixed chain.",
                    ),
                    ChainPhase::Degraded,
                );
            }
            let check = async {
                let status = session::snapshot();
                let observe_ipv4 = changed
                    || last_ipv4.elapsed() >= Duration::from_secs(300)
                    || status.phase == ChainPhase::Degraded
                    || status
                        .error
                        .as_ref()
                        .is_some_and(|error| error.code.starts_with("EXIT_IP_"));
                let port = MixedPort::effective().await;
                let observation = if observe_ipv4 {
                    let observation = chain_probe::probe(port).await?;
                    session::observe(observation.clone());
                    if let Err(error) =
                        chain_probe::verify(observation.as_ref(), data.settings.expected_ipv4.as_deref())
                    {
                        session::candidate_observe(observation);
                        return Err(error);
                    }
                    observation
                } else {
                    chain_probe::https(port).await?;
                    None
                };
                if last_front.elapsed() >= Duration::from_secs(300) {
                    if let Some(active) = store::active() {
                        let diagnostic = Diagnostic::start(builder::front_probe((*active).clone())).await?;
                        chain_probe::https(diagnostic.port).await?;
                    }
                    last_front = std::time::Instant::now();
                }
                Ok::<_, anyhow::Error>((observe_ipv4, observation))
            };
            let result = tokio::select! {
                _ = token.cancelled() => break,
                result = tokio::time::timeout(Duration::from_secs(30), check) => result.unwrap_or_else(|_| Err(failure("CHAIN_TIMEOUT", "The background chain check timed out."))),
            };
            if !session::proxy_permitted() || !crate::core::proxy_watchdog::is_available().await {
                break;
            }
            last_probe = std::time::Instant::now();
            match result {
                Ok((observed_ipv4, observation)) => {
                    failures = 0;
                    if observed_ipv4 {
                        last_ipv4 = std::time::Instant::now();
                        session::connected(observation);
                    } else {
                        session::healthy();
                    }
                }
                Err(error) => {
                    failures = failures.saturating_add(1);
                    let changed_ip = error.downcast_ref::<chain::ChainFailure>().is_some_and(|error| {
                        matches!(
                            error.code,
                            "EXIT_IP_CHANGED" | "EXIT_IP_INCONSISTENT" | "EXIT_IP_UNAVAILABLE"
                        )
                    });
                    let phase = if changed
                        || changed_ip
                        || failures >= 3
                        || session::snapshot().phase == ChainPhase::Degraded
                    {
                        ChainPhase::Degraded
                    } else {
                        ChainPhase::Connected
                    };
                    session::failed(&error, phase);
                    let ip_service_failure = error
                        .downcast_ref::<chain::ChainFailure>()
                        .is_some_and(|error| error.code.starts_with("EXIT_IP_"));
                    if data.settings.automatic_front && failures >= 3 && !ip_service_failure {
                        match repair_front(&data, &token).await {
                            Ok(true) => {
                                failures = 0;
                                last_front = std::time::Instant::now();
                                last_ipv4 = last_front;
                            }
                            Ok(false) => {}
                            Err(error) => {
                                session::failed(&error, ChainPhase::RecoveryRequired);
                                break;
                            }
                        }
                    }
                }
            }
        }
    });
}

async fn repair_front(data: &store::ChainData, monitor: &tokio_util::sync::CancellationToken) -> Result<bool> {
    let Some(previous) = store::active() else {
        return Ok(false);
    };
    let mut next = data.clone();
    let nodes = candidates(
        data,
        data.settings.selected_front_id.as_deref().unwrap_or_default(),
        true,
    )?;
    let applied = std::sync::atomic::AtomicBool::new(false);
    let cancellation = session::begin();
    session::phase(ChainPhase::Probing);
    let result = tokio::select! {
        _ = monitor.cancelled() => Err(failure("CHAIN_CANCELLED", "Background connection testing was stopped.")),
        _ = cancellation.cancelled() => Err(failure("CHAIN_CANCELLED", "Automatic front testing was cancelled.")),
        result = tokio::time::timeout(Duration::from_secs(30), apply_candidate(&mut next, nodes, &applied)) => result.unwrap_or_else(|_| Err(failure("CHAIN_TIMEOUT", "Automatic front testing exceeded its 30-second budget."))),
    };
    let result = match result {
        Ok(observation) => match store::commit_good(next).await {
            Ok(_) => Ok(observation),
            Err(error) => {
                if store::snapshot().await?.cleanup_pending {
                    cleanup_failed_connection(None).await?;
                    return Err(error);
                }
                Err(error)
            }
        },
        Err(error) => Err(error),
    };
    match result {
        Ok(observation) => {
            session::candidate_observe(None);
            session::connected(observation);
            Ok(true)
        }
        Err(error) => {
            if applied.load(std::sync::atomic::Ordering::Acquire) {
                rollback_working(previous, data, &error).await?;
            } else {
                session::failed(&error, ChainPhase::Degraded);
            }
            Ok(false)
        }
    }
}

fn network_fingerprint() -> Vec<String> {
    use network_interface::NetworkInterfaceConfig as _;
    let mut addresses = network_interface::NetworkInterface::show()
        .unwrap_or_default()
        .into_iter()
        .filter(|interface| !interface.addr.iter().all(|address| address.ip().is_loopback()))
        .map(|interface| format!("{}:{:?}", interface.name, interface.addr))
        .collect::<Vec<_>>();
    addresses.sort();
    addresses
}

pub(crate) async fn restore_last_good() -> Result<ChainView> {
    let _operation = store::OPERATION.lock().await;
    require_offline()?;
    let current = store::snapshot().await?;
    let next = store::restored(&current)?;
    store::commit(next).await?;
    session::inputs_changed();
    view().await
}

pub(crate) async fn confirm_ipv4(ipv4: String) -> Result<ChainView> {
    let _operation = store::OPERATION.lock().await;
    let observation = session::snapshot()
        .observation
        .ok_or_else(|| failure("EXIT_IP_UNAVAILABLE", "Test the chain before confirming its IPv4."))?;
    if !observation.consistent || observation.sources.len() != 2 || observation.ipv4 != ipv4 {
        return Err(failure(
            "EXIT_IP_INCONSISTENT",
            "Confirm only the latest IPv4 agreed by both disclosed services.",
        ));
    }
    let mut next = (*store::snapshot().await?).clone();
    next.settings.expected_ipv4 = Some(ipv4);
    if session::snapshot().phase == ChainPhase::Connected {
        store::commit_good(next).await?;
    } else {
        store::commit(next).await?;
    }
    view().await
}

pub(crate) async fn set_background_checks(enabled: bool) -> Result<ChainView> {
    if !enabled {
        session::stop_monitor();
    }
    let _operation = store::OPERATION.lock().await;
    let mut next = (*store::snapshot().await?).clone();
    if enabled && !next.settings.probe_consent {
        return Err(failure(
            "PROBE_CONSENT_REQUIRED",
            "Allow the disclosed checks before enabling background probes.",
        ));
    }
    next.settings.background_checks = enabled;
    if let Some(last) = next.last_good.as_mut() {
        last.inputs.background_checks = enabled;
    }
    if let Some(last) = next.settings.last_good.as_mut() {
        last.background_checks = enabled;
    }
    store::commit(next).await?;
    session::stop_monitor();
    if enabled && store::active().is_some() {
        start_monitor();
    }
    view().await
}

pub(crate) async fn keep_current_proxy() -> Result<ChainView> {
    session::cancel();
    session::stop_monitor();
    let _operation = store::OPERATION.lock().await;
    crate::core::portable_proxy::keep_current()?;
    session::permit_proxy(false);
    super::patch_verge(
        &IVerge {
            enable_system_proxy: Some(false),
            ..IVerge::default()
        },
        false,
    )
    .await?;
    CoreManager::global().stop_core().await?;
    store::set_active(None);
    clear_runtime().await?;
    session::inputs_changed();
    session::phase(ChainPhase::Disconnected);
    view().await
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
    let cleared = if core.is_ok() { clear_runtime().await } else { Ok(()) };
    if proxy.is_err() || core.is_err() {
        return Err(failure(
            "RECOVERY_REQUIRED",
            "The connection failed and cleanup could not finish. Check system proxy ownership before retrying.",
        ));
    }
    cleared
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
    session::stop_monitor();
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
    let cleared = if stopped.is_ok() { clear_runtime().await } else { Ok(()) };
    if restored.is_err() || stopped.is_err() {
        let error = failure(
            "RECOVERY_REQUIRED",
            "Disconnect could not finish. Check system proxy ownership before retrying.",
        );
        session::failed(&error, ChainPhase::RecoveryRequired);
        return Err(error);
    }
    if let Err(error) = cleared {
        session::failed(&error, ChainPhase::RecoveryRequired);
        return Err(error);
    }
    session::phase(ChainPhase::Disconnected);
    view().await
}

pub(crate) fn cancel() {
    session::cancel();
}

#[cfg(test)]
mod tests {
    use super::{candidates, chain, importer, store};

    #[test]
    fn automatic_candidates_are_capped_and_never_include_excluded_or_exit_endpoints() -> anyhow::Result<()> {
        let mut data = store::ChainData::default();
        data.settings.endpoint = Some(chain::ExitEndpoint {
            host: "exit.example".into(),
            port: 1080,
        });
        let source = "proxies:\n- {name: excluded, type: ss, server: front0.example, port: 443, cipher: aes-128-gcm, password: fixture}\n- {name: exit, type: ss, server: exit.example, port: 1080, cipher: aes-128-gcm, password: fixture}\n- {name: one, type: ss, server: front1.example, port: 443, cipher: aes-128-gcm, password: fixture}\n- {name: two, type: ss, server: front2.example, port: 443, cipher: aes-128-gcm, password: fixture}\n- {name: three, type: ss, server: front3.example, port: 443, cipher: aes-128-gcm, password: fixture}\n- {name: selected, type: ss, server: front4.example, port: 443, cipher: aes-128-gcm, password: fixture}\n";
        let subscription = importer::parse(source.into(), "fixture".into(), None)?;
        let excluded = subscription.nodes[0].id.clone();
        let selected = subscription.nodes[5].id.clone();
        data.settings.excluded_front_ids.push(excluded.clone());
        data.secrets.subscription = Some(subscription);
        let automatic = candidates(&data, &selected, true)?;
        assert_eq!(automatic.len(), 3);
        assert_eq!(automatic[0].id, selected);
        assert!(
            automatic
                .iter()
                .all(|front| front.id != excluded && front.server != "exit.example")
        );
        assert!(candidates(&data, &excluded, false).is_err());
        Ok(())
    }
}
