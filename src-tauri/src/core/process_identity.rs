use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    os::windows::{
        ffi::OsStringExt as _,
        io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle},
    },
    path::PathBuf,
};
use windows_sys::Win32::{
    Foundation::{FILETIME, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        QueryFullProcessImageNameW, WaitForSingleObject,
    },
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Identity {
    pub pid: u32,
    pub created_at: u64,
    pub sid: String,
    pub executable: PathBuf,
}

pub(crate) struct Process {
    handle: OwnedHandle,
    pub identity: Identity,
}

#[derive(Debug)]
struct ProcessGone;

impl std::fmt::Display for ProcessGone {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the owned process no longer exists")
    }
}

impl std::error::Error for ProcessGone {}

impl Process {
    pub fn open(pid: u32) -> Result<Self> {
        let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, 0, pid) };
        if raw.is_null() {
            if std::io::Error::last_os_error().raw_os_error() == Some(87) {
                return Err(ProcessGone.into());
            }
            return Err(std::io::Error::last_os_error()).context("could not inspect the owned process");
        }
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        let identity = (|| -> Result<Identity> {
            let mut created = FILETIME::default();
            let mut exited = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            if unsafe { GetProcessTimes(raw, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
                return Err(std::io::Error::last_os_error()).context("could not inspect process creation time");
            }
            let mut image = vec![0_u16; 32768];
            let mut length = image.len() as u32;
            if unsafe { QueryFullProcessImageNameW(raw, 0, image.as_mut_ptr(), &mut length) } == 0 {
                return Err(std::io::Error::last_os_error()).context("could not inspect process executable");
            }
            image.truncate(length as usize);
            Ok(Identity {
                pid,
                created_at: (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
                sid: super::owner_identity::process_sid(raw)?,
                executable: PathBuf::from(std::ffi::OsString::from_wide(&image)).canonicalize()?,
            })
        })();
        match identity {
            Ok(identity) => Ok(Self { handle, identity }),
            Err(_) if unsafe { WaitForSingleObject(raw, 0) } == WAIT_OBJECT_0 => Err(ProcessGone.into()),
            Err(error) => Err(error),
        }
    }

    pub fn verify(expected: &Identity) -> Result<Self> {
        let process = Self::open(expected.pid)?;
        if &process.identity != expected {
            bail!("process identity changed; ownership was not verified");
        }
        Ok(process)
    }

    pub fn matching(expected: &Identity) -> Result<Option<Self>> {
        match Self::open(expected.pid) {
            Ok(process) if &process.identity == expected => Ok(Some(process)),
            Ok(_) => Ok(None),
            Err(error) if error.is::<ProcessGone>() => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub fn exited(&self, milliseconds: u32) -> Result<bool> {
        match unsafe { WaitForSingleObject(self.handle.as_raw_handle(), milliseconds) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(std::io::Error::last_os_error()).context("could not wait for the owned process"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Process;
    use anyhow::Context as _;
    use std::{io::Write as _, os::windows::process::CommandExt as _, process::Stdio};

    #[test]
    fn recognizes_an_exited_process_while_its_identity_handle_is_retained() -> anyhow::Result<()> {
        let mut child = std::process::Command::new(std::env::var_os("ComSpec").context("ComSpec is missing")?)
            .args(["/d", "/c", "set /p CHAIN_DESK_PROBE="])
            .creation_flags(0x08000000)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let process = Process::open(child.id())?;
        child
            .stdin
            .take()
            .context("owned probe stdin is missing")?
            .write_all(b"done\r\n")?;
        child.wait()?;
        assert!(process.exited(0)?);
        assert!(
            Process::matching(&process.identity)?
                .map(|process| process.exited(0))
                .transpose()?
                .unwrap_or(true)
        );
        Ok(())
    }

    #[test]
    fn rejects_a_reused_pid_identity_without_signalling_the_process() -> anyhow::Result<()> {
        let process = Process::open(std::process::id())?;
        assert!(!process.exited(0)?);
        assert_eq!(Process::verify(&process.identity)?.identity, process.identity);
        let mut stale = process.identity.clone();
        stale.created_at += 1;
        assert!(Process::verify(&stale).is_err());
        stale = process.identity;
        stale.sid.push('0');
        assert!(Process::verify(&stale).is_err());
        Ok(())
    }
}
