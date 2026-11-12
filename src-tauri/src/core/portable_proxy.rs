use anyhow::{Context as _, Result, bail};
use parking_lot::Mutex;
use std::{
    io,
    sync::{Arc, LazyLock},
};
use sysproxy::{Autoproxy, Sysproxy};
use windows_sys::Win32::Networking::WinInet::{
    INTERNET_OPTION_PER_CONNECTION_OPTION, INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED,
    INTERNET_PER_CONN_AUTOCONFIG_URL, INTERNET_PER_CONN_FLAGS, INTERNET_PER_CONN_OPTION_LISTW,
    INTERNET_PER_CONN_OPTIONW, INTERNET_PER_CONN_OPTIONW_0, INTERNET_PER_CONN_PROXY_BYPASS,
    INTERNET_PER_CONN_PROXY_SERVER, InternetQueryOptionW, InternetSetOptionW, PROXY_TYPE_AUTO_PROXY_URL,
    PROXY_TYPE_DIRECT, PROXY_TYPE_PROXY,
};
use winreg::{
    RegKey, RegValue,
    enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE},
};

const SETTINGS: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";
const VALUES: [&str; 4] = ["ProxyEnable", "ProxyServer", "ProxyOverride", "AutoConfigURL"];

struct Snapshot {
    values: Vec<Option<RegValue<'static>>>,
    flags: u32,
}

fn lan_flags() -> Result<u32> {
    let mut option = INTERNET_PER_CONN_OPTIONW {
        dwOption: INTERNET_PER_CONN_FLAGS,
        Value: INTERNET_PER_CONN_OPTIONW_0 { dwValue: 0 },
    };
    let mut list = INTERNET_PER_CONN_OPTION_LISTW {
        dwSize: std::mem::size_of::<INTERNET_PER_CONN_OPTION_LISTW>() as u32,
        pszConnection: std::ptr::null_mut(),
        dwOptionCount: 1,
        dwOptionError: 0,
        pOptions: &mut option,
    };
    let mut size = list.dwSize;
    if unsafe {
        InternetQueryOptionW(
            std::ptr::null_mut(),
            INTERNET_OPTION_PER_CONNECTION_OPTION,
            (&mut list as *mut INTERNET_PER_CONN_OPTION_LISTW).cast(),
            &mut size,
        )
    } == 0
    {
        return Err(io::Error::last_os_error()).context("could not read WinINET LAN proxy flags");
    }
    Ok(unsafe { option.Value.dwValue })
}

fn write_lan(flags: u32, target: Option<(&str, &str, &str)>) -> Result<()> {
    let values: [&str; 3] = target.unwrap_or(("", "", "")).into();
    let mut strings: Vec<Vec<u16>> = values
        .iter()
        .map(|value| value.encode_utf16().chain(Some(0)).collect())
        .collect();
    let mut options = vec![INTERNET_PER_CONN_OPTIONW {
        dwOption: INTERNET_PER_CONN_FLAGS,
        Value: INTERNET_PER_CONN_OPTIONW_0 { dwValue: flags },
    }];
    if target.is_some() {
        for (kind, string) in [
            INTERNET_PER_CONN_PROXY_SERVER,
            INTERNET_PER_CONN_PROXY_BYPASS,
            INTERNET_PER_CONN_AUTOCONFIG_URL,
        ]
        .into_iter()
        .zip(&mut strings)
        {
            options.push(INTERNET_PER_CONN_OPTIONW {
                dwOption: kind,
                Value: INTERNET_PER_CONN_OPTIONW_0 {
                    pszValue: string.as_mut_ptr(),
                },
            });
        }
    }
    // The upstream sysproxy setter also rewrites every RAS connection; portable mode owns only LAN settings.
    let mut list = INTERNET_PER_CONN_OPTION_LISTW {
        dwSize: std::mem::size_of::<INTERNET_PER_CONN_OPTION_LISTW>() as u32,
        pszConnection: std::ptr::null_mut(),
        dwOptionCount: options.len() as u32,
        dwOptionError: 0,
        pOptions: options.as_mut_ptr(),
    };
    if unsafe {
        InternetSetOptionW(
            std::ptr::null_mut(),
            INTERNET_OPTION_PER_CONNECTION_OPTION,
            (&mut list as *mut INTERNET_PER_CONN_OPTION_LISTW).cast(),
            list.dwSize,
        )
    } == 0
    {
        return Err(io::Error::last_os_error()).context("could not apply WinINET LAN proxy settings");
    }
    for option in [INTERNET_OPTION_SETTINGS_CHANGED, INTERNET_OPTION_REFRESH] {
        if unsafe { InternetSetOptionW(std::ptr::null_mut(), option, std::ptr::null_mut(), 0) } == 0 {
            return Err(io::Error::last_os_error()).context("could not notify WinINET of the proxy change");
        }
    }
    Ok(())
}

impl Snapshot {
    fn read() -> Result<Self> {
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(SETTINGS, KEY_QUERY_VALUE)?;
        let values = VALUES
            .iter()
            .map(|name| match key.get_raw_value(name) {
                Ok(value) => Ok(Some(value)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            })
            .collect::<io::Result<_>>()?;
        Ok(Self {
            values,
            flags: lan_flags()?,
        })
    }

    fn matches(&self, other: &Self) -> bool {
        self.flags == other.flags
            && self
                .values
                .iter()
                .zip(&other.values)
                .all(|(left, right)| match (left, right) {
                    (Some(left), Some(right)) => left.vtype == right.vtype && left.bytes == right.bytes,
                    (None, None) => true,
                    _ => false,
                })
    }

    fn restore_inactive_values(&self) -> Result<()> {
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(SETTINGS, KEY_SET_VALUE)?;
        for (name, value) in VALUES.iter().zip(&self.values) {
            match value {
                Some(value) => key.set_raw_value(name, value)?,
                None => match key.delete_value(name) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                },
            }
        }
        Ok(())
    }
}

struct Ownership {
    original: Arc<Snapshot>,
    applied: Arc<Snapshot>,
}

static OWNERSHIP: LazyLock<Mutex<Option<Ownership>>> = LazyLock::new(|| Mutex::new(None));

fn conflict() -> anyhow::Error {
    anyhow::anyhow!("Another application owns the system proxy. Disable its system proxy before enabling ChainDesk.")
}

fn restore_owned(ownership: &mut Option<Ownership>) -> Result<()> {
    let Some(owned) = ownership.as_ref() else {
        return Ok(());
    };
    if !owned.applied.matches(&Snapshot::read()?) {
        ownership.take();
        return Ok(());
    }
    // Ownership is acquired only from a state where both proxy types were disabled.
    write_lan(owned.original.flags, None)?;
    owned.original.restore_inactive_values()?;
    ownership.take();
    Ok(())
}

pub(super) async fn apply(system: Sysproxy, automatic: Autoproxy) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let mut ownership = OWNERSHIP.lock();
        if !system.enable && !automatic.enable {
            return restore_owned(&mut ownership);
        }
        if system.host != "127.0.0.1" || (automatic.enable && !automatic.url.starts_with("http://127.0.0.1:")) {
            bail!("ChainDesk system proxy targets must use the product loopback listener");
        }
        let current = Arc::new(Snapshot::read()?);
        if let Some(owned) = ownership.as_ref() {
            if !owned.applied.matches(&current) {
                ownership.take();
                return Err(conflict());
            }
        } else {
            if current.flags & (PROXY_TYPE_PROXY | PROXY_TYPE_AUTO_PROXY_URL) != 0 {
                return Err(conflict());
            }
            *ownership = Some(Ownership {
                original: Arc::clone(&current),
                applied: current,
            });
        }
        let latest = Snapshot::read()?;
        if ownership.as_ref().is_some_and(|owned| !owned.applied.matches(&latest)) {
            ownership.take();
            return Err(conflict());
        }
        let server = format!("{}:{}", system.host, system.port);
        let result = if automatic.enable {
            write_lan(
                PROXY_TYPE_DIRECT | PROXY_TYPE_AUTO_PROXY_URL,
                Some(("", &system.bypass, &automatic.url)),
            )
        } else {
            write_lan(
                PROXY_TYPE_DIRECT | PROXY_TYPE_PROXY,
                Some((&server, &system.bypass, "")),
            )
        };
        let actual = Snapshot::read()?;
        let target_matches = if automatic.enable {
            actual.flags == (PROXY_TYPE_DIRECT | PROXY_TYPE_AUTO_PROXY_URL)
                && Autoproxy::get_auto_proxy()?.url == automatic.url
        } else {
            actual.flags == (PROXY_TYPE_DIRECT | PROXY_TYPE_PROXY) && Sysproxy::get_system_proxy()? == system
        };
        // A WinINET notification can fail after the settings were already applied.
        if target_matches && let Some(owned) = ownership.as_mut() {
            owned.applied = Arc::new(actual);
        }
        drop(ownership);
        result.context("failed to apply ChainDesk system proxy")
    })
    .await
    .context("system proxy ownership task failed")?
}

pub(super) async fn restore() -> Result<()> {
    tokio::task::spawn_blocking(|| restore_owned(&mut OWNERSHIP.lock()))
        .await
        .context("system proxy restore task failed")?
}
