use super::{
    listener::loopback_tcp_listener_owner, manager::create_and_assign_sidecar_job, portable, process_identity::Process,
};
use crate::{
    config::chain::failure,
    utils::{dirs, help},
};
use anyhow::{Context as _, Result};
use clash_verge_logging::{Type, logging};
use serde_yaml_ng::Mapping;
use std::{os::windows::io::OwnedHandle, path::Path, process::Child, time::Duration};

struct OwnedCore {
    child: Child,
    process: Process,
    job: Option<OwnedHandle>,
}

impl OwnedCore {
    fn spawn(controller: &portable::Controller, path: &Path, validate: bool) -> Result<Self> {
        let mut command = portable::native_command()?;
        command.args([
            "-d",
            dirs::path_to_str(&controller.directory)?,
            "-f",
            dirs::path_to_str(path)?,
        ]);
        if validate {
            command.arg("-t");
        }
        let mut child = command
            .spawn()
            .context("could not start the isolated diagnostic core")?;
        let owned = (|| {
            let process = Process::open(child.id())?;
            let job = create_and_assign_sidecar_job(child.id())?;
            Ok((process, job))
        })();
        match owned {
            Ok((process, job)) => Ok(Self {
                child,
                process,
                job: Some(job),
            }),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }

    fn stop(&mut self) -> bool {
        self.job.take();
        if self.process.exited(5000).unwrap_or(false) {
            let _ = self.child.wait();
            true
        } else {
            false
        }
    }

    async fn validation_status(&mut self) -> Result<bool> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(status) = self.child.try_wait()? {
                    return Ok::<_, anyhow::Error>(status.success());
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap_or(Ok(false))
    }
}

pub(crate) async fn validate_existing(path: &Path) -> Result<bool> {
    OwnedCore::spawn(portable::controller()?, path, true)?
        .validation_status()
        .await
}

impl Drop for OwnedCore {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) struct Diagnostic {
    core: Option<OwnedCore>,
    controller: portable::Controller,
    pub port: u16,
}

impl Diagnostic {
    pub async fn start(config: Mapping) -> Result<Self> {
        use crate::utils::retry::{RetryError, RetryPolicy, retry};
        retry(
            RetryPolicy::fixed(
                std::num::NonZeroUsize::new(5).unwrap_or(std::num::NonZeroUsize::MIN),
                Duration::from_millis(100),
            ),
            |_| {
                let config = config.clone();
                async move {
                    Self::start_once(config).await.map_err(|error| {
                        if error.is::<portable::PortConflict>() {
                            RetryError::Retry(error)
                        } else {
                            RetryError::Stop(error)
                        }
                    })
                }
            },
        )
        .await
    }

    async fn start_once(mut config: Mapping) -> Result<Self> {
        let started = std::time::Instant::now();
        let controller = portable::create_controller()?;
        logging!(
            info,
            Type::Core,
            "{}",
            serde_json::json!({"event":"CHAIN_DIAGNOSTIC_STAGE", "stage":"prepared", "elapsedMs":started.elapsed().as_millis()})
        );
        let port = portable::available_port()?;
        let mut diagnostic = Self {
            core: None,
            controller,
            port,
        };
        portable::constrain_with_controller(&mut config, &diagnostic.controller);
        config.insert("mixed-port".into(), port.into());
        let path = diagnostic.controller.directory.join("diagnostic.yaml");
        help::save_private_bytes(&path, serde_yaml_ng::to_string(&config)?.as_bytes())?;
        diagnostic.core = Some(OwnedCore::spawn(&diagnostic.controller, &path, true)?);
        let validator = diagnostic.core.as_mut().context("diagnostic validator is missing")?;
        portable::record_process(&diagnostic.controller, &validator.process.identity)?;
        let valid = validator.validation_status().await?;
        logging!(
            info,
            Type::Core,
            "{}",
            serde_json::json!({"event":"CHAIN_DIAGNOSTIC_STAGE", "stage":"validated", "elapsedMs":started.elapsed().as_millis(), "valid":valid})
        );
        diagnostic.core.take();
        if !valid {
            return Err(failure(
                "CORE_CONFIG_INVALID",
                "The isolated core rejected the generated configuration.",
            ));
        }
        diagnostic.core = Some(OwnedCore::spawn(&diagnostic.controller, &path, false)?);
        portable::record_process(
            &diagnostic.controller,
            &diagnostic
                .core
                .as_ref()
                .context("diagnostic core is missing")?
                .process
                .identity,
        )?;
        let pid = diagnostic
            .core
            .as_ref()
            .context("diagnostic core is missing")?
            .child
            .id();
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(500))
            .build()?;
        let ready = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                for listener in [port, diagnostic.controller.port()] {
                    if loopback_tcp_listener_owner(listener)?.is_some_and(|owner| owner != pid) {
                        return Err(portable::PortConflict.into());
                    }
                }
                if diagnostic
                    .core
                    .as_mut()
                    .context("diagnostic core is missing")?
                    .child
                    .try_wait()?
                    .is_some()
                {
                    return Err(failure(
                        "CORE_START_FAILED",
                        "The isolated diagnostic core exited before readiness.",
                    ));
                }
                if loopback_tcp_listener_owner(port)? == Some(pid)
                    && loopback_tcp_listener_owner(diagnostic.controller.port())? == Some(pid)
                    && client
                        .get(format!("http://127.0.0.1:{}/version", diagnostic.controller.port()))
                        .bearer_auth(&diagnostic.controller.secret)
                        .send()
                        .await
                        .is_ok_and(|response| response.status().is_success())
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            // The version API and listeners precede the core's inbound Running state.
            client
                .put(format!(
                    "http://127.0.0.1:{}/configs?force=true",
                    diagnostic.controller.port()
                ))
                .bearer_auth(&diagnostic.controller.secret)
                .timeout(Duration::from_secs(5))
                .json(&serde_json::json!({"path": dirs::path_to_str(&path)?}))
                .send()
                .await?
                .error_for_status()?;
            Ok(())
        })
        .await
        .map_err(|_| {
            failure(
                "CORE_START_FAILED",
                "The isolated diagnostic core did not become ready.",
            )
        })?;
        ready?;
        logging!(
            info,
            Type::Core,
            "{}",
            serde_json::json!({"event":"CHAIN_DIAGNOSTIC_STAGE", "stage":"ready", "elapsedMs":started.elapsed().as_millis()})
        );
        Ok(diagnostic)
    }
}

impl Drop for Diagnostic {
    fn drop(&mut self) {
        if self.core.take().is_none_or(|mut core| core.stop()) {
            let _ = std::fs::remove_dir_all(&self.controller.directory);
        }
    }
}
