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
static ASSETS: OnceLock<Assets> = OnceLock::new();

struct PinnedFile {
    path: PathBuf,
    lease: std::fs::File,
}

impl PinnedFile {
    fn open(path: &Path, expected: &str) -> Result<Self> {
        use std::os::windows::fs::OpenOptionsExt as _;
        let path = path.canonicalize()?;
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
            .open(&path)?;
        verify_open_file(&mut file, &path, expected)?;
        Ok(Self { path, lease: file })
    }
}

struct Assets {
    core: PinnedFile,
    rules: [PinnedFile; 2],
}

fn assets() -> Result<&'static Assets> {
    ASSETS.get().context("verified product files are not initialized")
}

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

pub(crate) fn available_port() -> Result<u16> {
    Ok(TcpListener::bind(("127.0.0.1", 0))
        .context("could not reserve a loopback controller port")?
        .local_addr()?
        .port())
}

pub(crate) fn initialize() -> Result<()> {
    let root = &paths()?.root;
    ASSETS
        .set(Assets {
            core: PinnedFile::open(&root.join("core/mihomo.exe"), CORE_SHA256)?,
            rules: [
                PinnedFile::open(
                    &root.join("resources/rules/geoip.dat"),
                    "90b78ab861c1cdc3f871f305f9ceedfb8d6d6329e3a40295e6773e057297384c",
                )?,
                PinnedFile::open(
                    &root.join("resources/rules/geosite.dat"),
                    "85073a00a3c3eb80ac76b97df2f97f7dab00cb36c8b5e7ed2354cbd016f8b088",
                )?,
            ],
        })
        .map_err(|_| anyhow::anyhow!("product files are already initialized"))?;
    CONTROLLER
        .set(create_controller()?)
        .map_err(|_| anyhow::anyhow!("portable core is already initialized"))?;
    Ok(())
}

pub(crate) fn create_controller() -> Result<Controller> {
    let paths = paths()?;
    let assets = assets()?;
    let directory = paths.data.join("runtime").join(random_token()?);
    super::owner_identity::create_private_current_user_directory(&directory)?;
    crate::utils::help::save_private_bytes(
        &directory.join("owner.json"),
        &serde_json::to_vec(&super::process_identity::Process::open(std::process::id())?.identity)?,
    )?;
    for asset in &assets.rules {
        let destination = directory.join(asset.path.file_name().context("rule asset has no filename")?);
        if std::fs::copy(&asset.path, &destination)? != asset.lease.metadata()?.len() {
            bail!("a verified rule asset was not copied completely");
        }
    }
    Ok(Controller {
        port: AtomicU16::new(available_port()?),
        secret: random_token()?,
        directory,
    })
}

pub(crate) fn record_child(controller: &Controller, pid: u32) -> Result<()> {
    let process = super::process_identity::Process::open(pid)?;
    record_process(controller, &process.identity)
}

pub(crate) fn record_process(controller: &Controller, identity: &super::process_identity::Identity) -> Result<()> {
    crate::utils::help::save_private_bytes(
        &controller.directory.join("process.json"),
        &serde_json::to_vec(identity)?,
    )
}

pub(crate) fn cleanup_orphaned_runtime() -> Result<()> {
    use super::process_identity::{Identity, Process};
    let cleanup_lock =
        super::owner_identity::open_or_create_current_user_lock(&paths()?.data.join("runtime-cleanup.lock"))?;
    cleanup_lock
        .lock()
        .context("could not serialize stopped runtime cleanup")?;
    use std::os::windows::fs::MetadataExt as _;
    let runtime = paths()?.data.join("runtime");
    for entry in std::fs::read_dir(&runtime)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.len() != 64 || !name.bytes().all(|byte| byte.is_ascii_hexdigit()) || !entry.file_type()?.is_dir() {
            continue;
        }
        let directory = entry.path();
        if entry.metadata()?.file_attributes() & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            continue;
        }
        let owner = directory.join("owner.json");
        let child = directory.join("process.json");
        if !owner.try_exists()? || !child.try_exists()? {
            continue;
        }
        let owner: Identity = serde_json::from_slice(&crate::utils::help::read_private_bytes(&owner, 65536)?)?;
        if Process::matching(&owner)?.map(|parent| parent.exited(0)).transpose()? == Some(false) {
            continue;
        }
        let child: Identity = serde_json::from_slice(&crate::utils::help::read_private_bytes(&child, 65536)?)?;
        if Process::matching(&child)?.map(|child| child.exited(5000)).transpose()? == Some(false) {
            continue;
        }
        std::fs::remove_dir_all(&directory).context("could not remove a verified stopped core session")?;
    }
    drop(cleanup_lock);
    Ok(())
}

pub(crate) fn verify_file(path: &Path, expected: &str) -> Result<()> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("required product file is unavailable: {}", path.display()))?;
    verify_open_file(&mut file, path, expected)
}

fn verify_open_file(file: &mut std::fs::File, path: &Path, expected: &str) -> Result<()> {
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
    let core = &assets()?.core.path;
    let mut command = super::handle::Handle::app_handle()
        .shell()
        .command(core)
        .current_dir(&paths.root)
        .env_clear();
    for (key, value) in environment()? {
        command = command.env(key, value);
    }
    Ok(command)
}

pub(crate) fn native_command() -> Result<std::process::Command> {
    use std::os::windows::process::CommandExt as _;
    let paths = paths()?;
    let core = &assets()?.core.path;
    let mut command = std::process::Command::new(core);
    command
        .current_dir(&paths.root)
        .env_clear()
        .envs(environment()?)
        .creation_flags(0x08000000);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    Ok(command)
}

pub(crate) fn environment() -> Result<Vec<(&'static str, std::ffi::OsString)>> {
    let mut environment = Vec::new();
    for key in ["SystemRoot", "WINDIR", "SystemDrive", "COMSPEC"] {
        if let Some(value) = std::env::var_os(key) {
            environment.push((key, value));
        }
    }
    if let Some(system) = std::env::var_os("SystemRoot") {
        environment.push(("PATH", PathBuf::from(system).join("System32").into_os_string()));
    }
    for key in ["TEMP", "TMP", "APPDATA", "LOCALAPPDATA"] {
        environment.push((key, paths()?.data.join("temp").into_os_string()));
    }
    Ok(environment)
}

pub(crate) fn constrain_config(config: &mut Mapping) -> Result<()> {
    let controller = controller()?;
    constrain_with_controller(config, controller);
    if let Some(port) = crate::config::MixedPort::session_fallback() {
        config.insert("mixed-port".into(), port.into());
    }
    if crate::config::chain::store::active().is_none()
        && let Some(dns) = config.get_mut("dns").and_then(Value::as_mapping_mut)
    {
        dns.insert("enable".into(), false.into());
    }
    Ok(())
}

pub(crate) fn constrain_with_controller(config: &mut Mapping, controller: &Controller) {
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
            | "read_profile_file"
            | "create_local_backup"
            | "restore_local_backup"
            | "import_local_backup"
            | "export_local_backup"
            | "create_webdav_backup"
            | "list_webdav_backup"
            | "delete_webdav_backup"
            | "restore_webdav_backup"
            | "save_webdav_config"
            | "test_delay"
            | "save_dns_config"
            | "apply_dns_config"
            | "set_dns_override"
            | "check_media_unlock"
            | "check_media_unlock_item"
    )
}

#[cfg(test)]
mod tests {
    use super::PinnedFile;
    use sha2::{Digest as _, Sha256};

    #[test]
    fn a_verified_asset_cannot_be_written_or_replaced_until_its_lease_is_released() -> anyhow::Result<()> {
        let root = std::env::temp_dir()
            .canonicalize()?
            .join(format!("chaindesk-pin-{}", crate::utils::server::random_token()?));
        crate::core::owner_identity::create_private_current_user_directory(&root)?;
        let root = scopeguard::guard(root, |root| {
            let _ = std::fs::remove_dir_all(root);
        });
        let path = root.join("fixture.dat");
        let bytes = b"verified fixture";
        std::fs::write(&path, bytes)?;
        let hash = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert!(PinnedFile::open(&path, &"0".repeat(64)).is_err());
        let asset = PinnedFile::open(&path, &hash)?;
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
        assert!(std::fs::remove_file(&path).is_err());
        let copy = root.join("copy.dat");
        std::fs::copy(&asset.path, &copy)?;
        assert_eq!(std::fs::read(copy)?, bytes);
        drop(asset);
        std::fs::remove_file(path)?;
        Ok(())
    }
}
