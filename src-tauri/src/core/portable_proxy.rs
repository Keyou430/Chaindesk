use super::process_identity::{Identity, Process};
use crate::{
    config::chain::{failure, protection},
    utils::{help, portable::paths},
};
use anyhow::{Context as _, Result, bail};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{io, sync::LazyLock};
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
    enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_DWORD, REG_SZ, RegType},
};

const SETTINGS: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";
const VALUES: [&str; 4] = ["ProxyEnable", "ProxyServer", "ProxyOverride", "AutoConfigURL"];
const MAX_JOURNAL_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
struct RawValue {
    kind: u32,
    bytes: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Snapshot {
    values: Vec<Option<RawValue>>,
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
                Ok(value) if value.bytes.len() <= 65536 => Ok(Some(RawValue {
                    kind: value.vtype as u32,
                    bytes: value.bytes.into_owned(),
                })),
                Ok(_) => Err(io::Error::other("proxy registry value is too large to journal")),
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
                    (Some(left), Some(right)) => left.kind == right.kind && left.bytes == right.bytes,
                    (None, None) => true,
                    _ => false,
                })
    }

    fn restore_inactive_values(&self) -> Result<()> {
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(SETTINGS, KEY_SET_VALUE)?;
        for (name, value) in VALUES.iter().zip(&self.values) {
            match value {
                Some(value) => {
                    let kind = registry_type(value.kind)?;
                    key.set_raw_value(
                        name,
                        &RegValue {
                            vtype: kind,
                            bytes: std::borrow::Cow::Borrowed(&value.bytes),
                        },
                    )?;
                }
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

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum JournalPhase {
    Prepared,
    Applied,
    Restoring,
    Conflict,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Journal {
    schema_version: u32,
    session: String,
    parent: Identity,
    phase: JournalPhase,
    original: Snapshot,
    applied: Snapshot,
    #[serde(default)]
    previous_applied: Option<Snapshot>,
}

static OPERATION: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static CONFLICT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn has_conflict() -> bool {
    CONFLICT.load(std::sync::atomic::Ordering::Acquire)
}

fn registry_type(kind: u32) -> Result<RegType> {
    use winreg::enums::{
        REG_BINARY, REG_DWORD_BIG_ENDIAN, REG_EXPAND_SZ, REG_FULL_RESOURCE_DESCRIPTOR, REG_LINK, REG_MULTI_SZ,
        REG_NONE, REG_QWORD, REG_RESOURCE_LIST, REG_RESOURCE_REQUIREMENTS_LIST,
    };
    [
        REG_NONE,
        REG_SZ,
        REG_EXPAND_SZ,
        REG_BINARY,
        REG_DWORD,
        REG_DWORD_BIG_ENDIAN,
        REG_LINK,
        REG_MULTI_SZ,
        REG_RESOURCE_LIST,
        REG_FULL_RESOURCE_DESCRIPTOR,
        REG_RESOURCE_REQUIREMENTS_LIST,
        REG_QWORD,
    ]
    .into_iter()
    .find(|value| value.clone() as u32 == kind)
    .context("the proxy journal contains an unsupported registry type")
}

fn journal_path() -> Result<std::path::PathBuf> {
    Ok(paths()?.data.join("proxy-session.bin"))
}

fn lock_journal() -> Result<std::fs::File> {
    let file = super::owner_identity::open_or_create_current_user_lock(&paths()?.data.join("proxy-session.lock"))?;
    file.lock()
        .context("could not lock the private proxy recovery journal")?;
    Ok(file)
}

fn read_journal() -> Result<Option<Journal>> {
    let path = journal_path()?;
    if !path.try_exists()? {
        return Ok(None);
    }
    let journal: Journal = serde_json::from_slice(&protection::unprotect(&help::read_private_bytes(
        &path,
        MAX_JOURNAL_BYTES,
    )?)?)?;
    if journal.schema_version != 1
        || journal.original.values.len() != 4
        || journal.applied.values.len() != 4
        || journal
            .original
            .values
            .iter()
            .chain(&journal.applied.values)
            .flatten()
            .any(|value| value.bytes.len() > 65536)
        || journal.session.len() != 64
        || !journal.session.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("the proxy recovery journal is invalid; preserve it for recovery");
    }
    Ok(Some(journal))
}

fn write_journal(journal: &Journal) -> Result<()> {
    if journal
        .original
        .values
        .iter()
        .chain(&journal.applied.values)
        .chain(journal.previous_applied.iter().flat_map(|snapshot| &snapshot.values))
        .flatten()
        .any(|value| value.bytes.len() > 65536)
    {
        bail!("proxy values are too large to recover safely; the proxy write was refused");
    }
    let encoded = protection::protect(&serde_json::to_vec(journal)?)?;
    if encoded.len() > MAX_JOURNAL_BYTES {
        bail!("the proxy recovery journal is too large; the proxy write was refused");
    }
    help::save_private_bytes(&journal_path()?, &encoded)
        .context("could not persist the proxy recovery journal; the proxy write was refused")
}

fn clear_journal() -> Result<()> {
    match std::fs::remove_file(journal_path()?) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn conflict() -> anyhow::Error {
    CONFLICT.store(true, std::sync::atomic::Ordering::Release);
    failure(
        "RESTORE_CONFLICT",
        "Another application changed the system proxy. Its current settings were preserved. Review recovery and keep the current settings before reconnecting.",
    )
}

fn target_snapshot(system: &Sysproxy, automatic: &Autoproxy) -> Snapshot {
    let string = |value: &str| {
        // WinINET deletes an option's registry value when its string is empty.
        (!value.is_empty()).then(|| RawValue {
            kind: REG_SZ as u32,
            bytes: value.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect(),
        })
    };
    Snapshot {
        flags: PROXY_TYPE_DIRECT
            | if automatic.enable {
                PROXY_TYPE_AUTO_PROXY_URL
            } else {
                PROXY_TYPE_PROXY
            },
        values: vec![
            Some(RawValue {
                kind: REG_DWORD as u32,
                bytes: u32::from(!automatic.enable).to_le_bytes().to_vec(),
            }),
            string(&if automatic.enable {
                String::new()
            } else {
                format!("{}:{}", system.host, system.port)
            }),
            string(&system.bypass),
            string(if automatic.enable { &automatic.url } else { "" }),
        ],
    }
}

fn permitted_restore_progress(journal: &Journal, current: &Snapshot) -> bool {
    (current.flags == journal.original.flags || current.flags == journal.applied.flags)
        && current
            .values
            .iter()
            .zip(journal.original.values.iter().zip(&journal.applied.values))
            .all(|(current, (original, applied))| raw_matches(current, original) || raw_matches(current, applied))
}

fn raw_matches(left: &Option<RawValue>, right: &Option<RawValue>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.kind == right.kind && left.bytes == right.bytes,
        (None, None) => true,
        _ => false,
    }
}

fn restore_journal(mut journal: Journal) -> Result<()> {
    let current = Snapshot::read()?;
    if journal.original.matches(&current) {
        return clear_journal();
    }
    let previous_matches = journal.phase == JournalPhase::Prepared
        && journal
            .previous_applied
            .as_ref()
            .is_some_and(|previous| previous.matches(&current));
    if !journal.applied.matches(&current)
        && !previous_matches
        && !(journal.phase == JournalPhase::Restoring && permitted_restore_progress(&journal, &current))
    {
        journal.phase = JournalPhase::Conflict;
        write_journal(&journal)?;
        return Err(conflict());
    }
    if previous_matches {
        journal.applied = current;
    }
    journal.phase = JournalPhase::Restoring;
    write_journal(&journal)?;
    write_lan(journal.original.flags, None)?;
    journal.original.restore_inactive_values()?;
    if !journal.original.matches(&Snapshot::read()?) {
        bail!("proxy restoration did not match the saved original values");
    }
    clear_journal()
}

pub(crate) fn recover_orphaned() -> Result<()> {
    let _operation = OPERATION.lock();
    let _file = lock_journal()?;
    let Some(journal) = read_journal()? else {
        return Ok(());
    };
    if Process::matching(&journal.parent)?
        .map(|parent| parent.exited(0))
        .transpose()?
        == Some(false)
    {
        return Err(failure(
            "RECOVERY_OWNER_ACTIVE",
            "The verified owning application is still running. Disconnect it before independent recovery.",
        ));
    }
    restore_journal(journal)
}

pub(crate) fn keep_current() -> Result<()> {
    let _operation = OPERATION.lock();
    let _file = lock_journal()?;
    if let Some(journal) = read_journal()? {
        if journal.phase != JournalPhase::Conflict {
            bail!("only a recorded recovery conflict can be acknowledged");
        }
        if journal.parent != Process::open(std::process::id())?.identity
            && Process::matching(&journal.parent)?
                .map(|parent| parent.exited(0))
                .transpose()?
                == Some(false)
        {
            bail!("the proxy journal belongs to a running application");
        }
        clear_journal()?;
    }
    CONFLICT.store(false, std::sync::atomic::Ordering::Release);
    Ok(())
}

pub(super) async fn apply(system: Sysproxy, automatic: Autoproxy) -> Result<()> {
    if !system.enable && !automatic.enable {
        return restore().await;
    }
    let watchdog = super::proxy_watchdog::ensure().await?;
    tokio::task::spawn_blocking(move || {
        let _operation = OPERATION.lock();
        let _file = lock_journal()?;
        watchdog.verify_alive()?;
        if system.host != "127.0.0.1" || (automatic.enable && !automatic.url.starts_with("http://127.0.0.1:")) {
            bail!("ChainDesk system proxy targets must use the product loopback listener");
        }
        let current = Snapshot::read()?;
        let mut journal = if let Some(mut journal) = read_journal()? {
            if journal.parent != watchdog.parent
                || journal.phase == JournalPhase::Conflict
                || (!journal.applied.matches(&current)
                    && !(journal.phase == JournalPhase::Prepared
                        && journal
                            .previous_applied
                            .as_ref()
                            .is_some_and(|previous| previous.matches(&current))))
            {
                journal.phase = JournalPhase::Conflict;
                write_journal(&journal)?;
                return Err(conflict());
            }
            journal
        } else {
            if current.flags & (PROXY_TYPE_PROXY | PROXY_TYPE_AUTO_PROXY_URL) != 0 {
                return Err(conflict());
            }
            Journal {
                schema_version: 1,
                session: watchdog.session.clone(),
                parent: watchdog.parent.clone(),
                phase: JournalPhase::Prepared,
                original: current.clone(),
                applied: current.clone(),
                previous_applied: None,
            }
        };
        journal.session.clone_from(&watchdog.session);
        journal.phase = JournalPhase::Prepared;
        journal.previous_applied = Some(current.clone());
        journal.applied = target_snapshot(&system, &automatic);
        write_journal(&journal)?;
        if !current.matches(&Snapshot::read()?) {
            journal.phase = JournalPhase::Conflict;
            write_journal(&journal)?;
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
        if journal.applied.matches(&actual) {
            journal.phase = JournalPhase::Applied;
            journal.applied = actual;
            journal.previous_applied = None;
            write_journal(&journal)?;
        } else {
            return Err(failure(
                "RECOVERY_REQUIRED",
                "The proxy write did not match its durable intent. Recovery information was retained.",
            ));
        }
        result.context("failed to apply ChainDesk system proxy")
    })
    .await
    .context("system proxy ownership task failed")?
}

pub(super) async fn restore() -> Result<()> {
    tokio::task::spawn_blocking(|| {
        let _operation = OPERATION.lock();
        let _file = lock_journal()?;
        if let Some(journal) = read_journal()? {
            if journal.parent != Process::open(std::process::id())?.identity {
                bail!("the recovery journal belongs to another application instance");
            }
            restore_journal(journal)?;
        }
        Ok(())
    })
    .await
    .context("system proxy restore task failed")?
}

#[cfg(test)]
mod tests {
    use super::{
        Autoproxy, Journal, JournalPhase, PROXY_TYPE_DIRECT, Process, REG_SZ, RawValue, Result, Snapshot, Sysproxy,
        permitted_restore_progress, registry_type, target_snapshot,
    };

    #[test]
    fn restore_progress_accepts_only_original_or_owned_bytes_and_flags() -> Result<()> {
        let original = Snapshot {
            flags: PROXY_TYPE_DIRECT,
            values: vec![None, None, None, None],
        };
        let applied = target_snapshot(
            &Sysproxy {
                enable: true,
                host: "127.0.0.1".into(),
                port: 7897,
                bypass: "<local>".into(),
            },
            &Autoproxy {
                enable: false,
                url: String::new(),
            },
        );
        assert!(applied.values[3].is_none());
        let mut unexpected = applied.clone();
        unexpected.values[3] = Some(RawValue {
            kind: REG_SZ as u32,
            bytes: vec![0, 0],
        });
        assert!(!applied.matches(&unexpected));
        let journal = Journal {
            schema_version: 1,
            session: "a".repeat(64),
            parent: Process::open(std::process::id())?.identity,
            phase: JournalPhase::Restoring,
            original: original.clone(),
            applied: applied.clone(),
            previous_applied: None,
        };
        let mut partial = applied;
        partial.flags = original.flags;
        partial.values[0] = None;
        assert!(permitted_restore_progress(&journal, &partial));
        partial.values[1] = Some(RawValue {
            kind: REG_SZ as u32,
            bytes: b"foreign-proxy".to_vec(),
        });
        assert!(!permitted_restore_progress(&journal, &partial));
        assert!(registry_type(0xffffffff).is_err());
        Ok(())
    }
}
