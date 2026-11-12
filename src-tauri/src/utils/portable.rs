use crate::core::owner_identity::{create_private_current_user_directory, create_private_current_user_file};
use anyhow::{Context as _, Result, bail};
use std::{os::windows::ffi::OsStrExt as _, path::PathBuf, sync::OnceLock};
use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumeInformationW, GetVolumePathNameW};

pub(crate) struct PortablePaths {
    pub root: PathBuf,
    pub data: PathBuf,
}

static PATHS: OnceLock<PortablePaths> = OnceLock::new();

pub(crate) fn paths() -> Result<&'static PortablePaths> {
    PATHS
        .get()
        .context("portable directory initialization did not finish; move ChainDesk to a writable local NTFS directory")
}

pub(crate) fn initialize() -> Result<()> {
    let executable = std::env::current_exe().context("could not locate ChainDesk.exe")?;
    let root = executable
        .parent()
        .context("executable has no parent directory")?
        .to_path_buf();
    let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut volume = vec![0_u16; 32768];
    if unsafe { GetVolumePathNameW(wide.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) } == 0 {
        return Err(std::io::Error::last_os_error()).context("could not inspect the portable volume");
    }
    let mut filesystem = [0_u16; 32];
    if unsafe {
        GetVolumeInformationW(
            volume.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            filesystem.as_mut_ptr(),
            filesystem.len() as u32,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error()).context("could not inspect the portable filesystem");
    }
    let end = filesystem
        .iter()
        .position(|&value| value == 0)
        .unwrap_or(filesystem.len());
    if unsafe { GetDriveTypeW(volume.as_ptr()) } != 3 || String::from_utf16_lossy(&filesystem[..end]) != "NTFS" {
        bail!("ChainDesk requires a writable local NTFS directory; network and removable locations are unsupported");
    }
    let probe = root.join(format!(".chaindesk-write-{}", super::server::random_token()?));
    drop(
        create_private_current_user_file(&probe).context(
            "ChainDesk's directory is not writable; move the entire folder to a writable local NTFS directory",
        )?,
    );
    std::fs::remove_file(&probe).context("could not remove portable write probe")?;
    let data = root.join("data");
    create_private_current_user_directory(&data)?;
    for name in ["logs", "backups", "runtime", "temp", "webview"] {
        create_private_current_user_directory(&data.join(name))?;
    }
    let runtime = root.join("runtime");
    if runtime.exists() {
        crate::core::portable::verify_file(
            &runtime.join("msedgewebview2.exe"),
            "7d46f9e074d3630f52cca1ee8de6840d8edf18700901348540c2690b79d7fd9d",
        )?;
    }
    // Startup calls this before creating the async runtime or any WebView threads.
    unsafe {
        std::env::set_var("TEMP", data.join("temp"));
        std::env::set_var("TMP", data.join("temp"));
        std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", data.join("webview"));
        std::env::remove_var("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER");
        if runtime.exists() {
            std::env::set_var("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER", &runtime);
        }
        if !cfg!(debug_assertions) {
            std::env::remove_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS");
        }
    }
    let version = tauri::webview_version()
        .context("WebView2 is missing; use the full ChainDesk package or install WebView2 157.0.4341.46 or newer")?;
    let parts = version
        .split('.')
        .map(str::parse::<u32>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if parts.as_slice() < [157, 0, 4341, 46].as_slice() {
        bail!(
            "WebView2 {version} is too old; use the full ChainDesk package or update WebView2 to 157.0.4341.46 or newer"
        );
    }
    PATHS
        .set(PortablePaths { root, data })
        .map_err(|_| anyhow::anyhow!("portable paths are already initialized"))?;
    Ok(())
}
