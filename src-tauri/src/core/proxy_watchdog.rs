use super::{
    owner_identity, portable, portable_proxy,
    process_identity::{Identity, Process},
};
use crate::{
    config::chain::{ChainPhase, failure, protection, session, store},
    utils::{help, portable::paths, server::random_token},
};
use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    os::windows::{io::AsRawHandle as _, process::CommandExt as _},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::windows::named_pipe::{ClientOptions, NamedPipeServer},
    sync::Mutex,
};
use windows_sys::Win32::System::Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId};

const READY: u8 = 1;
const HEARTBEAT: u8 = 2;
const RELEASE: u8 = 3;
const RECOVER: u8 = 4;

#[derive(Serialize, Deserialize)]
struct Invitation {
    version: u32,
    session: String,
    parent: Identity,
}

pub(crate) struct Watchdog {
    pub session: String,
    pub parent: Identity,
    process: Process,
    healthy: AtomicBool,
}

impl Watchdog {
    pub fn verify_alive(&self) -> Result<()> {
        if !self.healthy.load(Ordering::Acquire) || self.process.exited(0)? {
            return Err(failure(
                "WATCHDOG_UNAVAILABLE",
                "The verified proxy recovery process is unavailable. Reconnect after reviewing recovery.",
            ));
        }
        Ok(())
    }
}

static WATCHDOG: Mutex<Option<Arc<Watchdog>>> = Mutex::const_new(None);

pub(crate) async fn is_available() -> bool {
    WATCHDOG
        .lock()
        .await
        .as_ref()
        .is_some_and(|watchdog| watchdog.verify_alive().is_ok())
}

fn pipe_name(session: &str) -> String {
    format!(r"\\.\pipe\ChainDesk.Recovery.{session}")
}

pub(crate) async fn ensure() -> Result<Arc<Watchdog>> {
    let mut slot = WATCHDOG.lock().await;
    if let Some(watchdog) = slot.as_ref()
        && watchdog.verify_alive().is_ok()
    {
        return Ok(Arc::clone(watchdog));
    }
    let session = random_token()?;
    let parent = Process::open(std::process::id())?.identity;
    let mut pipe = owner_identity::create_private_pipe(&pipe_name(&session))?;
    let invitation = Invitation {
        version: 1,
        session: session.clone(),
        parent: parent.clone(),
    };
    help::save_private_bytes(
        &paths()?.data.join("proxy-watchdog.bin"),
        &protection::protect(&serde_json::to_vec(&invitation)?)?,
    )?;
    let mut child = std::process::Command::new(std::env::current_exe()?)
        .args(["--proxy-watchdog", &session])
        .current_dir(&paths()?.root)
        .env_clear()
        .envs(portable::environment()?)
        .creation_flags(0x08000000)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("could not launch the independent proxy recovery process")?;
    let result = tokio::time::timeout(Duration::from_secs(8), async {
        let process = Process::open(child.id())?;
        if process.identity.sid != parent.sid || process.identity.executable != parent.executable {
            bail!("the recovery executable identity differs from the parent");
        }
        pipe.connect().await?;
        verify_client(&pipe, &process.identity)?;
        if pipe.read_u8().await? != READY {
            bail!("the recovery process sent an invalid handshake");
        }
        pipe.write_u8(READY).await?;
        Ok::<_, anyhow::Error>(Arc::new(Watchdog {
            session,
            parent,
            process,
            healthy: AtomicBool::new(true),
        }))
    })
    .await
    .context("proxy recovery process handshake timed out")
    .and_then(|result| result);
    let watchdog = match result {
        Ok(watchdog) => watchdog,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let monitor = Arc::clone(&watchdog);
    crate::AsyncHandler::spawn(move || async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let sent = tokio::time::timeout(Duration::from_secs(2), pipe.write_u8(HEARTBEAT)).await;
            if !matches!(sent, Ok(Ok(()))) || monitor.process.exited(0).unwrap_or(true) {
                break;
            }
        }
        monitor.healthy.store(false, Ordering::Release);
        let _operation = store::OPERATION.lock().await;
        let mut slot = WATCHDOG.lock().await;
        if slot.as_ref().is_some_and(|current| Arc::ptr_eq(current, &monitor)) {
            session::stop_monitor();
            session::permit_proxy(false);
            let restored = portable_proxy::restore().await;
            let error = if restored.is_ok() {
                failure(
                    "WATCHDOG_UNAVAILABLE",
                    "The recovery process exited. The owned system proxy was restored; reconnect to establish a new recovery session.",
                )
            } else {
                failure(
                    "RECOVERY_REQUIRED",
                    "The recovery process exited and proxy restoration did not finish. Review the independent recovery entry.",
                )
            };
            session::failed(&error, ChainPhase::RecoveryRequired);
            *slot = None;
        }
        drop(slot);
        if child.try_wait().is_ok_and(|status| status.is_none()) {
            let _ = child.kill();
            let _ = child.wait();
        }
    });
    *slot = Some(Arc::clone(&watchdog));
    drop(slot);
    Ok(watchdog)
}

fn verify_client(pipe: &NamedPipeServer, expected: &Identity) -> Result<()> {
    let mut pid = 0;
    if unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle(), &mut pid) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if pid != expected.pid {
        bail!("the named pipe client is not the launched recovery process");
    }
    Process::verify(expected)?;
    Ok(())
}

async fn run_helper(session: &str) -> Result<()> {
    let invitation: Invitation = serde_json::from_slice(&protection::unprotect(&help::read_private_bytes(
        &paths()?.data.join("proxy-watchdog.bin"),
        65536,
    )?)?)?;
    if invitation.version != 1 || invitation.session != session {
        bail!("the recovery invitation is invalid");
    }
    let current = Process::open(std::process::id())?;
    let parent = Process::verify(&invitation.parent)?;
    if parent.identity.executable != current.identity.executable || parent.identity.sid != current.identity.sid {
        bail!("the recovery parent identity is not trusted");
    }
    let mut pipe = ClientOptions::new().open(pipe_name(session))?;
    let mut server_pid = 0;
    if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut server_pid) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if server_pid != parent.identity.pid {
        bail!("the recovery pipe server is not the verified parent");
    }
    Process::verify(&invitation.parent)?;
    pipe.write_u8(READY).await?;
    if tokio::time::timeout(Duration::from_secs(5), pipe.read_u8()).await?? != READY {
        bail!("the parent did not complete recovery authentication");
    }
    let mut connected = true;
    loop {
        if parent.exited(0)? {
            let restored = portable_proxy::recover_orphaned();
            portable::cleanup_orphaned_runtime()?;
            return restored;
        }
        if !connected {
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(250)) => {},
            message = pipe.read_u8() => match message {
                Ok(HEARTBEAT) => {},
                Ok(RELEASE) if !paths()?.data.join("proxy-session.bin").try_exists()? => return Ok(()),
                Ok(RECOVER) if parent.exited(0)? => return portable_proxy::recover_orphaned(),
                _ => connected = false,
            }
        }
    }
}

pub(crate) fn entrypoint() -> Option<std::process::ExitCode> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let mode = arguments.first()?.as_str();
    if !matches!(mode, "--proxy-watchdog" | "--recover-proxy") {
        return None;
    }
    let result = (|| {
        crate::utils::portable::initialize_paths()?;
        match arguments.as_slice() {
            [mode, session]
                if mode == "--proxy-watchdog"
                    && session.len() == 64
                    && session.bytes().all(|byte| byte.is_ascii_hexdigit()) =>
            {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?
                    .block_on(run_helper(session))
            }
            [mode] if mode == "--recover-proxy" => {
                portable_proxy::recover_orphaned()?;
                portable::cleanup_orphaned_runtime()
            }
            [mode, keep] if mode == "--recover-proxy" && keep == "--keep-current" => portable_proxy::keep_current(),
            _ => Err(anyhow::anyhow!("invalid ChainDesk recovery arguments")),
        }
    })();
    Some(match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ChainDesk recovery: {error:#}");
            std::process::ExitCode::FAILURE
        }
    })
}
