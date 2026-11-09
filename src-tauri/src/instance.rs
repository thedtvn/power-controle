//! Same-session single instance for the tray, and for the Settings window.
use anyhow::{Context, Result};
use std::sync::OnceLock;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0,
};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject,
};

fn object_name(suffix: &str) -> Vec<u16> {
    // Local namespaces isolate interactive sessions; the username further
    // scopes objects on shared machines.
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
    format!("Local\\PowerControl-{user}-{suffix}")
        .encode_utf16()
        .chain(Some(0))
        .collect()
}

/// A named mutex plus an auto-reset "come to the front" event.
struct Named {
    mutex: HANDLE,
    event: HANDLE,
}

impl Named {
    /// Returns `None` (after signalling the owner when `notify`) if another
    /// process already holds the mutex.
    fn acquire(mutex_suffix: &str, event_suffix: &str, notify: bool) -> Result<Option<Self>> {
        let mutex_name = object_name(mutex_suffix);
        let event_name = object_name(event_suffix);
        unsafe {
            let mutex = CreateMutexW(None, false, PCWSTR(mutex_name.as_ptr()))
                .context("Could not create the instance mutex")?;
            let duplicate = GetLastError() == ERROR_ALREADY_EXISTS;
            let event = match CreateEventW(None, false, false, PCWSTR(event_name.as_ptr())) {
                Ok(event) => event,
                Err(error) => {
                    let _ = CloseHandle(mutex);
                    return Err(error.into());
                }
            };
            if duplicate {
                if notify {
                    let _ = SetEvent(event);
                }
                let _ = CloseHandle(event);
                let _ = CloseHandle(mutex);
                return Ok(None);
            }
            Ok(Some(Self { mutex, event }))
        }
    }

    fn take_signal(&self) -> bool {
        unsafe { WaitForSingleObject(self.event, 0) == WAIT_OBJECT_0 }
    }
}

// SAFETY: kernel mutex/event handles are process-wide and thread-safe to use
// from any thread; `Named` only waits on and closes them.
unsafe impl Send for Named {}
unsafe impl Sync for Named {}

impl Drop for Named {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.event);
            let _ = CloseHandle(self.mutex);
        }
    }
}

/// The background tray process.
pub struct Instance(Named);

impl Instance {
    /// A duplicate manual launch asks the running tray to open Settings;
    /// a duplicate scheduled (`--autostart`) launch stays silent.
    pub fn acquire(autostart: bool) -> Result<Option<Self>> {
        Ok(Named::acquire("instance", "activate", !autostart)?.map(Self))
    }

    pub fn take_activation(&self) -> bool {
        self.0.take_signal()
    }
}

/// The Settings window process: at most one, focused on re-launch.
pub struct SettingsGuard;

static SETTINGS: OnceLock<Named> = OnceLock::new();

/// Shareable view of the Settings focus event, usable from async tasks.
#[derive(Clone, Copy)]
pub struct FocusRequests;

impl FocusRequests {
    pub fn take(&self) -> bool {
        SETTINGS.get().is_some_and(Named::take_signal)
    }
}

impl SettingsGuard {
    pub fn acquire() -> Result<Option<Self>> {
        match Named::acquire("settings", "settings-focus", true)? {
            Some(named) => {
                let _ = SETTINGS.set(named);
                Ok(Some(Self))
            }
            None => Ok(None),
        }
    }

    pub fn focus_requests() -> FocusRequests {
        FocusRequests
    }
}
