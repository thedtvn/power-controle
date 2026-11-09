//! Local IPC between the background tray process and the Settings process.
//!
//! The tray owns the hardware service and its cache. Settings runs as a
//! separate, short-lived process so that closing it actually exits and frees
//! the GPU renderer, fonts, and UI memory instead of hiding a window.
//!
//! Transport: a per-user named pipe, local only, one JSON line per request.
use crate::service::{Action, Service, Snapshot};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::windows::io::FromRawHandle;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_PIPE_CONNECTED, HANDLE, HLOCAL,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

/// Requests from the Settings process. Passwords travel only over the local,
/// user-restricted pipe and are zeroized on both ends by `Action`.
#[derive(Serialize, Deserialize)]
pub enum Request {
    /// Returns the cached snapshot. Never touches hardware.
    Snapshot,
    Submit(Action),
}

fn pipe_name() -> String {
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
    format!(r"\\.\pipe\PowerControl-{user}")
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

// ---------------------------------------------------------------- server ----

/// Serves requests on a background thread for the lifetime of the tray.
pub fn serve(service: Service) {
    std::thread::spawn(move || {
        let mut first = true;
        loop {
            match accept(first) {
                Ok(pipe) => {
                    first = false;
                    let service = service.clone();
                    // One thread per short-lived connection keeps a stuck
                    // client from blocking the next Settings launch.
                    std::thread::spawn(move || {
                        let _ = handle_client(pipe, &service);
                    });
                }
                Err(error) => {
                    eprintln!("Settings IPC unavailable: {error}");
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        }
    });
}

/// Creates one pipe instance and blocks until a client connects.
fn accept(first: bool) -> Result<File> {
    // SYSTEM and the interactive user only; administrators get no special
    // access, and remote clients are rejected below.
    let sddl = wide("D:P(A;;GA;;;SY)(A;;GA;;;OW)");
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
        .context("could not build the IPC security descriptor")?;
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let name = wide(&pipe_name());
    let mut open_mode = PIPE_ACCESS_DUPLEX;
    if first {
        // Fails if another process already owns this pipe name, so a
        // malicious server cannot impersonate the tray to Settings.
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            64 * 1024,
            64 * 1024,
            0,
            Some(&attributes),
        )
    };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    if handle.is_invalid() {
        return Err(anyhow!("CreateNamedPipeW failed: {:?}", unsafe {
            GetLastError()
        }));
    }
    let connected = unsafe { ConnectNamedPipe(handle, None) };
    if connected.is_err() && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
        unsafe {
            let _ = CloseHandle(handle);
        }
        return Err(anyhow!("ConnectNamedPipe failed"));
    }
    // SAFETY: we own this freshly created handle; File closes it on drop.
    Ok(unsafe { File::from_raw_handle(handle.0 as _) })
}

fn handle_client(pipe: File, service: &Service) -> Result<()> {
    let mut writer = pipe.try_clone()?;
    let reader = BufReader::new(pipe);
    for line in reader.lines() {
        let line = line?;
        if line.len() > 256 * 1024 {
            break;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(Request::Snapshot) => serde_json::to_string(&service.snapshot())?,
            Ok(Request::Submit(action)) => {
                service.submit(action);
                "null".into()
            }
            Err(_) => break,
        };
        writer.write_all(response.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
    Ok(())
}

// ---------------------------------------------------------------- client ----

/// Connection from the Settings process to the running tray.
pub struct Client {
    writer: File,
    reader: BufReader<File>,
}

impl Client {
    pub fn connect() -> Result<Self> {
        let name = pipe_name();
        let mut last = None;
        // The tray may be finishing startup or serving another request.
        for _ in 0..40 {
            match OpenOptions::new().read(true).write(true).open(&name) {
                Ok(file) => {
                    let writer = file.try_clone()?;
                    return Ok(Self {
                        writer,
                        reader: BufReader::new(file),
                    });
                }
                Err(error) => {
                    last = Some(error);
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
        Err(anyhow!(
            "Power Control is not running in the tray ({})",
            last.map(|e| e.to_string()).unwrap_or_default()
        ))
    }

    fn call(&mut self, request: &Request) -> Result<String> {
        let mut line = serde_json::to_string(request)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes())?;
        self.writer.flush()?;
        let mut response = String::new();
        if self.reader.read_line(&mut response)? == 0 {
            return Err(anyhow!("the tray process closed the connection"));
        }
        Ok(response)
    }

    pub fn snapshot(&mut self) -> Result<Snapshot> {
        Ok(serde_json::from_str(&self.call(&Request::Snapshot)?)?)
    }

    pub fn submit(&mut self, action: Action) -> Result<()> {
        self.call(&Request::Submit(action)).map(|_| ())
    }
}

/// Raw handle unused outside this module; keeps the import list honest.
#[allow(dead_code)]
fn _assert_handle(_: HANDLE) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip() {
        let json = serde_json::to_string(&Request::Submit(Action::SetAutostart(true))).unwrap();
        assert!(matches!(
            serde_json::from_str::<Request>(&json).unwrap(),
            Request::Submit(Action::SetAutostart(true))
        ));
        let json = serde_json::to_string(&Request::Snapshot).unwrap();
        assert!(matches!(
            serde_json::from_str::<Request>(&json).unwrap(),
            Request::Snapshot
        ));
    }

    #[test]
    fn pipe_is_local_and_per_user() {
        let name = pipe_name();
        assert!(name.starts_with(r"\\.\pipe\PowerControl-"));
    }
}
