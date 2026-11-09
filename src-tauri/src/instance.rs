//! Same-session single instance and manual-launch activation without a webview.
use anyhow::{Context, Result};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0,
};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject,
};

pub struct Instance {
    mutex: HANDLE,
    activation: HANDLE,
}
impl Instance {
    pub fn acquire(autostart: bool) -> Result<Option<Self>> {
        // Local namespaces isolate interactive sessions; username further scopes
        // the objects on shared machines without touching machine-wide state.
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
        let name = |suffix: &str| {
            format!("Local\\PowerControl-{user}-{suffix}")
                .encode_utf16()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        let mutex_name = name("instance");
        let event_name = name("activate");
        unsafe {
            let mutex = CreateMutexW(None, false, PCWSTR(mutex_name.as_ptr()))
                .context("Could not create the instance mutex")?;
            let duplicate = GetLastError() == ERROR_ALREADY_EXISTS;
            let activation = match CreateEventW(None, false, false, PCWSTR(event_name.as_ptr())) {
                Ok(event) => event,
                Err(error) => {
                    let _ = CloseHandle(mutex);
                    return Err(error.into());
                }
            };
            if duplicate {
                if !autostart {
                    let _ = SetEvent(activation);
                }
                let _ = CloseHandle(activation);
                let _ = CloseHandle(mutex);
                return Ok(None);
            }
            Ok(Some(Self { mutex, activation }))
        }
    }
    pub fn take_activation(&self) -> bool {
        unsafe { WaitForSingleObject(self.activation, 0) == WAIT_OBJECT_0 }
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.activation);
            let _ = CloseHandle(self.mutex);
        }
    }
}
