use super::{ChainErrorView, ChainFailure, ChainPhase, ProbeObservation};
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
    pub error: Option<ChainErrorView>,
}

static STATUS: LazyLock<Mutex<Status>> = LazyLock::new(|| Mutex::new(Status::default()));
static CANCELLATION: Mutex<Option<CancellationToken>> = Mutex::new(None);
static PROXY_PERMITTED: AtomicBool = AtomicBool::new(false);

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
}

pub(crate) fn begin() -> CancellationToken {
    let token = CancellationToken::new();
    *CANCELLATION.lock() = Some(token.clone());
    let mut status = STATUS.lock();
    status.phase = ChainPhase::Connecting;
    status.error = None;
    token
}

pub(crate) fn cancel() {
    if let Some(token) = CANCELLATION.lock().as_ref() {
        token.cancel();
    }
}

pub(crate) fn connected(observation: ProbeObservation) {
    let mut status = STATUS.lock();
    status.phase = ChainPhase::Connected;
    status.observation = Some(observation);
    status.error = None;
    drop(status);
    CANCELLATION.lock().take();
}

pub(crate) fn failed(error: &anyhow::Error, fallback: ChainPhase) {
    let detail = error.downcast_ref::<ChainFailure>();
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
