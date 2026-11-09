//! Runtime elevation for release builds.
//!
//! gpui-component forces GPUI's own Windows manifest (DPI awareness) into the
//! executable, and a binary may contain only one manifest, so the old
//! `requireAdministrator` manifest cannot be embedded. Instead, an unelevated
//! release launch asks UAC to relaunch itself elevated, matching the previous
//! behavior for manual launches. The login task already runs elevated.
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

/// Quotes one argument for the Windows command line (CommandLineToArgvW rules).
fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_owned();
    }
    let mut out = String::from('"');
    let mut backslashes = 0;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(ch);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// Returns true when an elevated copy was started and this process should exit.
/// Returns false to continue unelevated (UAC declined or relaunch failed): the
/// power plan still works, and Dell operations report their own errors.
pub fn relaunch_elevated(args: &[String]) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let params = args
        .iter()
        .map(|arg| quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let (verb, file, params) = (
        HSTRING::from("runas"),
        HSTRING::from(exe.as_os_str()),
        HSTRING::from(params),
    );
    // ShellExecuteW returns a value greater than 32 on success.
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR(params.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    result.0 as usize > 32
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn arguments_round_trip_through_windows_quoting() {
        assert_eq!(quote("--autostart"), "--autostart");
        assert_eq!(quote(""), "\"\"");
        assert_eq!(quote("C:\\Program Files\\x"), "\"C:\\Program Files\\x\"");
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote("dir with space\\"), "\"dir with space\\\\\"");
    }
}
