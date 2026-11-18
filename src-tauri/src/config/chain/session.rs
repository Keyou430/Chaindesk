use super::{ChainErrorView, ChainFailure, ChainPhase, ProbeObservation};
use clash_verge_logging::{Type, logging};
use parking_lot::Mutex;
use std::sync::{
    LazyLock,
    atomic::{AtomicBool, Ordering},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Default)]
pub(crate) struct Status {
    pub phase: ChainPhase,
    pub observation: Option<ProbeObservation>,
    pub candidate_observation: Option<ProbeObservation>,
    pub health_checked_at: Option<String>,
    pub error: Option<ChainErrorView>,
}

static STATUS: LazyLock<Mutex<Status>> = LazyLock::new(|| Mutex::new(Status::default()));
static CANCELLATION: Mutex<Option<CancellationToken>> = Mutex::new(None);
static PROXY_PERMITTED: AtomicBool = AtomicBool::new(false);
static MONITOR: Mutex<Option<CancellationToken>> = Mutex::new(None);

pub(crate) fn stop_monitor() {
    let token = MONITOR.lock().take();
    if let Some(token) = token {
        token.cancel();
    }
}

pub(crate) fn monitor_token() -> CancellationToken {
    stop_monitor();
    let token = CancellationToken::new();
    *MONITOR.lock() = Some(token.clone());
    token
}

pub(crate) fn observe(observation: Option<ProbeObservation>) {
    STATUS.lock().observation = observation;
}

pub(crate) fn candidate_observe(observation: Option<ProbeObservation>) {
    STATUS.lock().candidate_observation = observation;
}

pub(crate) fn proxy_permitted() -> bool {
    PROXY_PERMITTED.load(Ordering::Acquire)
}
pub(crate) fn permit_proxy(permitted: bool) {
    PROXY_PERMITTED.store(permitted, Ordering::Release);
}

pub(crate) fn snapshot() -> Status {
    STATUS.lock().clone()
}

pub(crate) fn inputs_changed() {
    *STATUS.lock() = Status::default();
}

pub(crate) fn phase(phase: ChainPhase) {
    STATUS.lock().phase = phase;
    logging!(
        debug,
        Type::Core,
        "{}",
        serde_json::json!({"event":"CHAIN_PHASE", "phase":phase})
    );
}

pub(crate) fn begin() -> CancellationToken {
    let token = CancellationToken::new();
    *CANCELLATION.lock() = Some(token.clone());
    let mut status = STATUS.lock();
    status.phase = ChainPhase::Connecting;
    status.error = None;
    status.candidate_observation = None;
    token
}

pub(crate) fn cancel() {
    if let Some(token) = CANCELLATION.lock().as_ref() {
        token.cancel();
    }
}

pub(crate) fn connected(observation: Option<ProbeObservation>) {
    let warning = observation_warning(observation.as_ref());
    let mut status = STATUS.lock();
    status.phase = ChainPhase::Connected;
    status.observation = observation;
    status.health_checked_at = Some(chrono::Utc::now().to_rfc3339());
    status.error = warning;
    drop(status);
    CANCELLATION.lock().take();
}

pub(crate) fn failed(error: &anyhow::Error, fallback: ChainPhase) {
    let detail = error.downcast_ref::<ChainFailure>();
    logging!(
        warn,
        Type::Core,
        "{}",
        serde_json::json!({"event":"CHAIN_FAILURE", "code":detail.map_or("CHAIN_OPERATION_FAILED", |error| error.code), "phase":fallback})
    );
    let mut status = STATUS.lock();
    status.phase = fallback;
    status.error = Some(ChainErrorView {
        code: detail.map_or("CHAIN_OPERATION_FAILED", |failure| failure.code).into(),
        message: detail.map_or_else(
            || "The operation failed. The previous settings were retained.".into(),
            |failure| failure.message.clone(),
        ),
    });
    drop(status);
    CANCELLATION.lock().take();
}

pub(crate) fn healthy() {
    let mut status = STATUS.lock();
    status.phase = ChainPhase::Connected;
    status.health_checked_at = Some(chrono::Utc::now().to_rfc3339());
    status.error = observation_warning(status.observation.as_ref());
}

fn observation_warning(observation: Option<&ProbeObservation>) -> Option<ChainErrorView> {
    let Some(observation) = observation else {
        return Some(ChainErrorView { code: "EXIT_IP_UNAVAILABLE".into(), message: "The complete HTTPS chain works, but the IPv4 observation services are unavailable. No public IP or baseline was inferred.".into() });
    };
    (!observation.consistent).then(|| ChainErrorView {
        code: "EXIT_IP_INCONSISTENT".into(),
        message: "The complete HTTPS chain works, but the IPv4 services disagree. Both observations are retained; no baseline was inferred.".into(),
    })
}
