use serde::Serialize;
use std::env;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const CCTK_SHA256: &str = "bac829a34b53eaa98afa10c033646932db9f9775d285799180d5bec30fe90225";
const CCTK_FILENAME: &str = "Dell-Command-Configure-Application_F2V9N_WIN64_5.2.2.292_A00.EXE";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Serialize)]
pub struct InstallResult {
    pub installed: bool,
    pub reboot_recommended: bool,
}

#[derive(Debug, Serialize)]
pub enum InstallError {
    DownloadFailed(String),
    HashMismatch(String),
    InstallExecutionFailed(String),
    VerificationFailed(String),
    Other(String),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::DownloadFailed(m) => write!(f, "Download failed: {}", m),
            InstallError::HashMismatch(m) => write!(f, "Hash mismatch: {}", m),
            InstallError::InstallExecutionFailed(m) => write!(f, "Install execution failed: {}", m),
            InstallError::VerificationFailed(m) => write!(f, "Verification failed: {}", m),
            InstallError::Other(m) => write!(f, "Error: {}", m),
        }
    }
}

pub fn install_cctk() -> Result<InstallResult, InstallError> {
    if crate::dell::find_cctk().is_some() {
        return Ok(InstallResult {
            installed: true,
            reboot_recommended: false,
        });
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| InstallError::Other("System clock is unavailable".into()))?
        .as_nanos();
    let temp_dir = env::temp_dir().join(format!("cctk_install_{}_{}", std::process::id(), nonce));
    if let Err(e) = std::fs::create_dir_all(&temp_dir) {
        return Err(InstallError::Other(format!(
            "Failed to create temp dir: {}",
            e
        )));
    }

    let exe_path = temp_dir.join(CCTK_FILENAME);

    let download_output = Command::new("powershell.exe")
        .creation_flags(CREATE_NO_WINDOW)
        .env("CCTK_INSTALLER_PATH", &exe_path)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Invoke-WebRequest -Uri 'https://dl.dell.com/FOLDER14333137M/1/Dell-Command-Configure-Application_F2V9N_WIN64_5.2.2.292_A00.EXE' -OutFile $env:CCTK_INSTALLER_PATH",
        ])
        .output()
        .map_err(|e| InstallError::DownloadFailed(e.to_string()))?;

    if !download_output.status.success() {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return Err(InstallError::DownloadFailed(
            "PowerShell Invoke-WebRequest failed".into(),
        ));
    }

    let hash_output = Command::new("powershell.exe")
        .creation_flags(CREATE_NO_WINDOW)
        .env("CCTK_INSTALLER_PATH", &exe_path)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-FileHash -LiteralPath $env:CCTK_INSTALLER_PATH -Algorithm SHA256).Hash",
        ])
        .output()
        .map_err(|e| InstallError::Other(e.to_string()))?;

    let computed_hash = String::from_utf8_lossy(&hash_output.stdout)
        .trim()
        .to_lowercase();

    if computed_hash != CCTK_SHA256.to_lowercase() {
        return Err(InstallError::HashMismatch(format!(
            "Expected {}, got {}",
            CCTK_SHA256, computed_hash
        )));
    }

    // Run installer
    let install_output = Command::new(&exe_path)
        .creation_flags(CREATE_NO_WINDOW)
        .arg("/s")
        .output()
        .map_err(|e| InstallError::InstallExecutionFailed(e.to_string()))?;

    // Wait for the installer process is handled by Command::output()
    // It returns exit status
    if !install_output.status.success() {
        return Err(InstallError::InstallExecutionFailed(format!(
            "Installer exited with {}",
            install_output.status
        )));
    }

    // Verify find_cctk
    let cctk_path = crate::dell::find_cctk();
    if cctk_path.is_none() {
        return Err(InstallError::VerificationFailed(
            "cctk.exe not found after install".into(),
        ));
    }

    // Clean up
    let _ = std::fs::remove_dir_all(&temp_dir);

    // Dell updates often recommend a reboot
    Ok(InstallResult {
        installed: true,
        reboot_recommended: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_install_result_behavior() {
        let res = InstallResult {
            installed: true,
            reboot_recommended: true,
        };
        assert!(res.installed);
        assert!(res.reboot_recommended);
    }
}
