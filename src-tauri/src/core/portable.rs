use crate::utils::{portable::paths, server::random_token};
use anyhow::{Context as _, Result, bail};
use serde_yaml_ng::{Mapping, Value};
use sha2::{Digest as _, Sha256};
use std::{
    io::Read as _,
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicU16, Ordering},
    },
};
use tauri_plugin_shell::{ShellExt as _, process::Command};

const CORE_SHA256: &str = "0b4c7353057a78de8a09db445351aadec7d72a4f48adc7d99691f3c5ff74d3da";

#[derive(Debug)]
pub(crate) struct PortConflict;

impl std::fmt::Display for PortConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a core listener port was occupied during startup")
    }
}

impl std::error::Error for PortConflict {}

pub(crate) fn is_port_conflict(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("address already in use") || message.contains("only one usage of each socket address")
}

pub(crate) struct Controller {
    port: AtomicU16,
    pub secret: String,
    pub directory: PathBuf,
}

static CONTROLLER: OnceLock<Controller> = OnceLock::new();

pub(crate) fn controller() -> Result<&'static Controller> {
    CONTROLLER.get().context("portable core is not initialized")
}

impl Controller {
    pub fn port(&self) -> u16 {
        self.port.load(Ordering::Acquire)
    }

    pub fn renew_port(&self) -> Result<()> {
        self.port.store(available_port()?, Ordering::Release);
        Ok(())
    }
}

fn available_port() -> Result<u16> {
    Ok(TcpListener::bind(("127.0.0.1", 0))
        .context("could not reserve a loopback controller port")?
        .local_addr()?
        .port())
}

pub(crate) fn initialize() -> Result<()> {
    let paths = paths()?;
    verify_file(&paths.root.join("core/mihomo.exe"), CORE_SHA256)?;
    let directory = paths.data.join("runtime").join(random_token()?);
    super::owner_identity::create_private_current_user_directory(&directory)?;
    for (name, hash) in [
        (
            "geoip.dat",
            "90b78ab861c1cdc3f871f305f9ceedfb8d6d6329e3a40295e6773e057297384c",
        ),
        (
            "geosite.dat",
            "85073a00a3c3eb80ac76b97df2f97f7dab00cb36c8b5e7ed2354cbd016f8b088",
        ),
    ] {
        let source = paths.root.join("resources/rules").join(name);
        verify_file(&source, hash)?;
        let destination = directory.join(name);
        std::fs::copy(&source, &destination)?;
        verify_file(&destination, hash)?;
    }
    CONTROLLER
        .set(Controller {
            port: AtomicU16::new(available_port()?),
            secret: random_token()?,
            directory,
        })
        .map_err(|_| anyhow::anyhow!("portable core is already initialized"))?;
    Ok(())
}

pub(crate) fn verify_file(path: &Path, expected: &str) -> Result<()> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("required product file is unavailable: {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16384];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let actual: String = digest.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
    if actual != expected {
        bail!(
            "product file checksum mismatch: {}; restore it from the verified ChainDesk package",
            path.display()
        );
    }
    Ok(())
}

pub(crate) fn command() -> Result<Command> {
    let paths = paths()?;
    let core = paths.root.join("core/mihomo.exe");
    verify_file(&core, CORE_SHA256)?;
    let mut command = super::handle::Handle::app_handle()
        .shell()
        .command(core)
        .current_dir(&paths.root)
        .env_clear();
    for key in ["SystemRoot", "WINDIR", "SystemDrive", "COMSPEC"] {
        if let Some(value) = std::env::var_os(key) {
            command = command.env(key, value);
        }
    }
    if let Some(system) = std::env::var_os("SystemRoot") {
        command = command.env("PATH", PathBuf::from(system).join("System32"));
    }
    for key in ["TEMP", "TMP", "APPDATA", "LOCALAPPDATA"] {
        command = command.env(key, paths.data.join("temp"));
    }
    Ok(command)
}

pub(crate) fn constrain_config(config: &mut Mapping) -> Result<()> {
    let controller = controller()?;
    for key in [
        "external-controller-pipe",
        "external-controller-unix",
        "external-controller-tls",
        "external-ui",
        "external-ui-url",
        "external-ui-name",
        "external-ui-download-url",
        "listeners",
        "geox-url",
    ] {
        config.remove(key);
    }
    config.insert(
        "external-controller".into(),
        format!("127.0.0.1:{}", controller.port()).into(),
    );
    config.insert("secret".into(), controller.secret.clone().into());
    config.insert(
        "external-controller-cors".into(),
        Value::Mapping(Mapping::from_iter([
            ("allow-private-network".into(), false.into()),
            ("allow-origins".into(), Value::Sequence(Vec::new())),
        ])),
    );
    config.insert("allow-lan".into(), false.into());
    config.insert("ipv6".into(), false.into());
    config.insert("bind-address".into(), "127.0.0.1".into());
    if let Some(port) = crate::config::MixedPort::session_fallback() {
        config.insert("mixed-port".into(), port.into());
    }
    config.insert("geodata-mode".into(), true.into());
    config.insert("geo-auto-update".into(), false.into());
    for key in ["port", "socks-port", "redir-port", "tproxy-port"] {
        config.insert(key.into(), 0.into());
    }
    config.insert(
        "tun".into(),
        Value::Mapping(Mapping::from_iter([("enable".into(), false.into())])),
    );
    config.insert("mode".into(), "rule".into());
    if crate::config::chain::store::active().is_none()
        && let Some(dns) = config.get_mut("dns").and_then(Value::as_mapping_mut)
    {
        dns.insert("enable".into(), false.into());
    }
    Ok(())
}

pub(crate) fn redact_config(config: &mut Mapping) {
    config.retain(|key, value| {
        if key.as_str().is_some_and(|key| {
            matches!(
                key,
                "secret" | "password" | "username" | "uuid" | "private-key" | "token" | "headers"
            )
        }) {
            return false;
        }
        redact_value(value);
        true
    });
}

fn redact_value(value: &mut Value) {
    match value {
        Value::Mapping(map) => redact_config(map),
        Value::Sequence(sequence) => sequence.iter_mut().for_each(redact_value),
        _ => {}
    }
}

pub(crate) fn clear_runtime_configuration() -> Result<()> {
    for name in [
        crate::constants::files::RUNTIME_CONFIG,
        crate::constants::files::CHECK_CONFIG,
    ] {
        let path = controller()?.directory.join(name);
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(crate) fn cleanup() -> Result<()> {
    let directory = &controller()?.directory;
    if directory.exists() {
        std::fs::remove_dir_all(directory).context("could not remove the stopped core's session directory")?;
    }
    Ok(())
}

pub(crate) fn unsupported_command(command: &str) -> bool {
    matches!(
        command,
        "install_service"
            | "patch_clash_config"
            | "patch_clash_mode"
            | "update_proxy_chain_config_in_runtime"
            | "create_profile"
            | "import_profile"
            | "update_profile"
            | "save_profile_file"
            | "patch_profiles_config"
            | "patch_profile"
            | "delete_profile"
            | "enhance_profiles"
            | "uninstall_service"
            | "reinstall_service"
            | "repair_service"
            | "install_update"
            | "upgrade_clash_core"
            | "change_clash_core"
            | "invoke_uwp_tool"
    )
}
