use anyhow::{Context as _, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::{
    NetworkManagement::IpHelper::{
        CancelMibChangeNotify2, MIB_IPINTERFACE_ROW, MIB_NOTIFICATION_TYPE, NotifyIpInterfaceChange,
    },
    Networking::WinSock::AF_UNSPEC,
};

pub(crate) struct NetworkWatch {
    handle: usize,
    changed: Option<Box<AtomicBool>>,
}

unsafe extern "system" fn on_change(
    context: *const std::ffi::c_void,
    _: *const MIB_IPINTERFACE_ROW,
    _: MIB_NOTIFICATION_TYPE,
) {
    if let Some(changed) = unsafe { context.cast::<AtomicBool>().as_ref() } {
        changed.store(true, Ordering::Release);
    }
}

impl NetworkWatch {
    pub fn start() -> Result<Self> {
        let changed = Box::new(AtomicBool::new(false));
        let mut handle = std::ptr::null_mut();
        let status = unsafe {
            NotifyIpInterfaceChange(
                AF_UNSPEC,
                Some(on_change),
                (changed.as_ref() as *const AtomicBool).cast(),
                false,
                &mut handle,
            )
        };
        if status != 0 {
            return Err(std::io::Error::from_raw_os_error(status as i32))
                .context("could not observe network interface changes");
        }
        Ok(Self {
            handle: handle as usize,
            changed: Some(changed),
        })
    }

    pub fn take_changed(&self) -> bool {
        self.changed
            .as_ref()
            .is_some_and(|changed| changed.swap(false, Ordering::AcqRel))
    }
}

impl Drop for NetworkWatch {
    fn drop(&mut self) {
        // Successful cancellation waits for callbacks before their context can be freed.
        if unsafe { CancelMibChangeNotify2(self.handle as *mut std::ffi::c_void) } != 0
            && let Some(context) = self.changed.take()
        {
            let _ = Box::leak(context);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn registers_and_cancels_a_read_only_current_process_notification() -> anyhow::Result<()> {
        let watch = super::NetworkWatch::start()?;
        watch.take_changed();
        drop(watch);
        Ok(())
    }
}
