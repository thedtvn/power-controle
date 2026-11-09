//! Per-user, elevated interactive login startup. Never run the tray as SYSTEM.
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

const SCRIPT: &str = include_str!("startup.ps1");
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
// Serialize migration and settings changes so disabling cannot race migration.
static STARTUP_LOCK: Mutex<()> = Mutex::new(());

fn run(operation: &str) -> Result<String, String> {
    let _guard = STARTUP_LOCK
        .lock()
        .map_err(|_| "Startup configuration lock is unavailable")?;
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    // Use the system copy instead of resolving an executable from PATH/CWD.
    let powershell =
        PathBuf::from(std::env::var_os("SystemRoot").ok_or("SystemRoot is unavailable")?)
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let output = Command::new(powershell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
            &encode_script(SCRIPT),
        ])
        .env("POWER_CONTROL_EXE", exe)
        .env("POWER_CONTROL_OPERATION", operation)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| format!("Could not start Windows startup configuration: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{} (exit {})",
            String::from_utf8_lossy(&output.stderr).trim(),
            output.status
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

// PowerShell's -Command reparses embedded quotes on Windows. EncodedCommand
// preserves the script exactly, including quotes, and expects UTF-16LE base64.
fn encode_script(script: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let a = chunk[0] as usize;
        let b = chunk.get(1).copied().unwrap_or(0) as usize;
        let c = chunk.get(2).copied().unwrap_or(0) as usize;
        result.push(ALPHABET[a >> 2] as char);
        result.push(ALPHABET[((a & 3) << 4) | (b >> 4)] as char);
        result.push(if chunk.len() > 1 {
            ALPHABET[((b & 15) << 2) | (c >> 6)] as char
        } else {
            '='
        });
        result.push(if chunk.len() > 2 {
            ALPHABET[c & 63] as char
        } else {
            '='
        });
    }
    result
}

fn parse_enabled(output: &str) -> Result<bool, String> {
    match output.trim() {
        "enabled" => Ok(true),
        "disabled" => Ok(false),
        other => Err(format!("Unexpected Windows startup response: {other}")),
    }
}

pub fn is_enabled() -> Result<bool, String> {
    parse_enabled(&run("query")?)
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    run(if enabled { "enable" } else { "disable" }).and_then(expect_ok)
}

pub fn migrate_legacy() -> Result<(), String> {
    run("migrate").and_then(expect_ok)
}

fn expect_ok(response: String) -> Result<(), String> {
    if response == "ok" {
        Ok(())
    } else {
        Err(format!("Unexpected Windows startup response: {response}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_encoding_preserves_quotes_and_unicode() {
        assert_eq!(encode_script(""), "");
        assert_eq!(encode_script("a"), "YQA=");
        assert_eq!(encode_script("abc"), "YQBiAGMA");
        assert_eq!(encode_script("\"é\""), "IgDpACIA");
    }

    #[test]
    fn powershell_query_crosses_the_real_process_boundary_read_only() {
        // Query does not register/unregister tasks or modify the registry.
        // This catches Windows argument quoting and PowerShell runtime errors.
        let response = run("query").expect("read-only Windows startup query failed");
        assert!(parse_enabled(&response).is_ok());
    }

    #[test]
    fn startup_response_is_strict() {
        assert_eq!(parse_enabled("enabled\r\n"), Ok(true));
        assert_eq!(parse_enabled("disabled"), Ok(false));
        assert!(parse_enabled("").is_err());
        assert!(parse_enabled("warning\nenabled").is_err());
        assert!(expect_ok("disabled".into()).is_err());
    }

    #[test]
    fn task_is_in_the_interactive_user_session() {
        for setting in [
            "-AtLogOn -User $sid",
            "PT15S",
            "-LogonType Interactive -RunLevel Highest",
            "-Argument '--autostart'",
            "-WorkingDirectory",
            "-MultipleInstances IgnoreNew",
            "-ExecutionTimeLimit ([TimeSpan]::Zero)",
            "-AllowStartIfOnBatteries",
            "-DontStopIfGoingOnBatteries",
            "PowerControl-$sid",
        ] {
            assert!(SCRIPT.contains(setting), "Missing task setting: {setting}");
        }
        assert!(!SCRIPT.contains("-AtStartup"));
        assert!(!SCRIPT.contains("-LogonType ServiceAccount"));
    }

    #[test]
    fn executable_is_passed_as_data_and_legacy_cleanup_is_ordered() {
        assert!(SCRIPT.contains("$exe = $env:POWER_CONTROL_EXE"));
        assert!(SCRIPT.contains("$valueName = 'power-controle'"));
        let enable = SCRIPT
            .split("'enable' {")
            .nth(1)
            .unwrap()
            .split("'disable'")
            .next()
            .unwrap();
        assert!(
            enable.find("Register-StartupTask").unwrap()
                < enable.find("Remove-LegacyEntries").unwrap()
        );
        let query = SCRIPT
            .split("'query' {")
            .nth(1)
            .unwrap()
            .split("'enable'")
            .next()
            .unwrap();
        assert!(!query.contains("Register-StartupTask"));
        assert!(!query.contains("Remove-LegacyEntries"));
    }
}
