use crate::bios_pwd::{BiosPassword, BiosPasswordKind};
use serde::{Deserialize, Serialize};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::{Mutex, OnceLock};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
static CCTK_PROCESS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn cctk_command(path: &PathBuf) -> Command {
    let mut command = Command::new(path);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

fn run_cctk(path: &PathBuf, args: &[&str]) -> Result<Output, DellError> {
    let lock = CCTK_PROCESS_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock
        .lock()
        .map_err(|_| DellError::CommandFailed("CCTK process lock is unavailable".into()))?;
    cctk_command(path)
        .args(args)
        .output()
        .map_err(|error| DellError::CommandFailed(error.to_string()))
}

fn decode_cctk_output(bytes: &[u8]) -> String {
    let utf16 =
        bytes.starts_with(&[0xff, 0xfe]) || bytes.chunks_exact(2).take(16).any(|pair| pair[1] == 0);
    if utf16 {
        let offset = usize::from(bytes.starts_with(&[0xff, 0xfe])) * 2;
        let units = bytes[offset..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

#[cfg(debug_assertions)]
fn debug_cctk(event: &str, detail: impl std::fmt::Display) {
    eprintln!("[cctk] {event}: {detail}");
}

#[cfg(not(debug_assertions))]
fn debug_cctk(_: &str, _: impl std::fmt::Display) {}

// ---------------------------------------------------------------------------
// Thermal profile enum (maps to Dell CCTK --ThermalManagement values)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThermalProfile {
    Optimized,
    Cool,
    Quiet,
    UltraPerformance,
}

impl ThermalProfile {
    pub const ALL: [Self; 4] = [
        Self::Optimized,
        Self::Cool,
        Self::Quiet,
        Self::UltraPerformance,
    ];

    pub fn cctk_value(&self) -> &'static str {
        match self {
            Self::Optimized => "Optimized",
            Self::Cool => "Cool",
            Self::Quiet => "Quiet",
            Self::UltraPerformance => "UltraPerformance",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Optimized => "Optimized",
            Self::Cool => "Cool",
            Self::Quiet => "Quiet",
            Self::UltraPerformance => "Ultra Performance",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Optimized => "thermal_optimized",
            Self::Cool => "thermal_cool",
            Self::Quiet => "thermal_quiet",
            Self::UltraPerformance => "thermal_ultra_performance",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "thermal_optimized" => Some(Self::Optimized),
            "thermal_cool" => Some(Self::Cool),
            "thermal_quiet" => Some(Self::Quiet),
            "thermal_ultra_performance" => Some(Self::UltraPerformance),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Battery Charge Setting
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatteryChargeMode {
    Standard,
    Express,
    PrimarilyAc,
    Adaptive,
    Custom,
}

impl BatteryChargeMode {
    pub fn cctk_value(&self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Express => "Express",
            Self::PrimarilyAc => "PrimAcUse",
            Self::Adaptive => "Adaptive",
            Self::Custom => "Custom",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Express => "Express",
            Self::PrimarilyAc => "Primarily AC",
            Self::Adaptive => "Adaptive",
            Self::Custom => "Custom",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Standard => "battery_standard",
            Self::Express => "battery_express",
            Self::PrimarilyAc => "battery_primarily_ac",
            Self::Adaptive => "battery_adaptive",
            Self::Custom => "battery_custom",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "battery_standard" => Some(Self::Standard),
            "battery_express" => Some(Self::Express),
            "battery_primarily_ac" => Some(Self::PrimarilyAc),
            "battery_adaptive" => Some(Self::Adaptive),
            "battery_custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatteryChargeSetting {
    pub mode: BatteryChargeMode,
    #[serde(default = "default_custom_start")]
    pub custom_start: u8,
    #[serde(default = "default_custom_stop")]
    pub custom_stop: u8,
}

fn default_custom_start() -> u8 {
    50
}
fn default_custom_stop() -> u8 {
    85
}

impl Default for BatteryChargeSetting {
    fn default() -> Self {
        Self {
            mode: BatteryChargeMode::Adaptive,
            custom_start: 50,
            custom_stop: 85,
        }
    }
}

impl BatteryChargeSetting {
    pub fn new(
        mode: BatteryChargeMode,
        custom_start: u8,
        custom_stop: u8,
    ) -> Result<Self, DellError> {
        if mode == BatteryChargeMode::Custom {
            if !(50..=95).contains(&custom_start) {
                return Err(DellError::ParseError(format!(
                    "start {} out of range 50..=95",
                    custom_start
                )));
            }
            if !(55..=100).contains(&custom_stop) {
                return Err(DellError::ParseError(format!(
                    "stop {} out of range 55..=100",
                    custom_stop
                )));
            }
            if custom_stop < custom_start || custom_stop - custom_start < 5 {
                return Err(DellError::ParseError(format!(
                    "stop-start must be >= 5 ({} to {})",
                    custom_start, custom_stop
                )));
            }
        }
        Ok(Self {
            mode,
            custom_start,
            custom_stop,
        })
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum DellError {
    CommandFailed(String),
    ParseError(String),
    SetupPasswordError(String),
    SystemPasswordError(String),
}

impl std::fmt::Display for DellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CommandFailed(msg) => write!(f, "CCTK command failed: {msg}"),
            Self::ParseError(msg) => write!(f, "Failed to parse CCTK output: {msg}"),
            Self::SetupPasswordError(msg) => write!(f, "Setup password error: {msg}"),
            Self::SystemPasswordError(msg) => write!(f, "System password error: {msg}"),
        }
    }
}

impl std::error::Error for DellError {}

// ---------------------------------------------------------------------------
// State exposed to frontend
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DellState {
    pub available: bool,
    pub current_profile: Option<ThermalProfile>,
    pub supported_profiles: Vec<ThermalProfile>,
    pub current_battery_mode: Option<BatteryChargeSetting>,
    pub supported_battery_modes: Vec<BatteryChargeMode>,
}

impl DellState {
    pub fn unavailable() -> Self {
        Self {
            available: false,
            current_profile: None,
            supported_profiles: Vec::new(),
            current_battery_mode: None,
            supported_battery_modes: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// CCTK path discovery
// ---------------------------------------------------------------------------

/// Searches well-known Dell Command Configure installation directories.
/// Returns `None` if cctk.exe is not installed.
pub fn find_cctk() -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = ["ProgramFiles(x86)", "ProgramFiles"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .map(|base| {
            PathBuf::from(base)
                .join("Dell")
                .join("Command Configure")
                .join("X86_64")
                .join("cctk.exe")
        })
        .collect();

    candidates.into_iter().find(|p| p.exists())
}

// ---------------------------------------------------------------------------
// CCTK output parsing (pure functions — unit-testable)
// ---------------------------------------------------------------------------

/// Parses the current thermal profile from `cctk --ThermalManagement` output.
///
/// Expected output line: `ThermalManagement=Optimized` (or Cool, Quiet, UltraPerformance)
pub fn parse_thermal_output(output: &str) -> Result<ThermalProfile, DellError> {
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("ThermalManagement=") {
            return parse_profile_value(value.trim());
        }
    }
    Err(DellError::ParseError(format!(
        "no ThermalManagement= line in output: {output:?}"
    )))
}

/// Parses supported profiles from `cctk -H --ThermalManagement` help output.
///
/// Only profiles suffixed with `+` on the `Arguments:` line are considered
/// supported.  Example line:
///
/// ```text
/// Arguments: Optimized+|Quiet+|Cool|UltraPerformance+
/// ```
/// Here `Cool` (no `+`) is excluded.
pub fn parse_thermal_list(output: &str) -> Vec<ThermalProfile> {
    // Locate the "Arguments:" token and collect the value that follows it,
    // which may be on the same line or on a continuation line separated by `|`.
    let args_value = output.lines().find_map(|line| {
        line.find("Arguments:")
            .map(|index| line[index + "Arguments:".len()..].trim().to_owned())
    });

    let Some(args) = args_value else {
        return Vec::new();
    };

    let mut profiles = Vec::new();
    for token in args.split('|') {
        let token = token.trim();
        if let Some(name) = token.strip_suffix('+').map(str::trim) {
            if let Ok(profile) = parse_profile_value(name) {
                profiles.push(profile);
            }
        }
    }
    profiles
}

fn parse_profile_value(value: &str) -> Result<ThermalProfile, DellError> {
    let name = value.split(':').next().unwrap_or(value);
    match name {
        "Optimized" => Ok(ThermalProfile::Optimized),
        "Cool" => Ok(ThermalProfile::Cool),
        "Quiet" => Ok(ThermalProfile::Quiet),
        "UltraPerformance" => Ok(ThermalProfile::UltraPerformance),
        other => Err(DellError::ParseError(format!(
            "unknown thermal profile: {other:?}"
        ))),
    }
}

// ---------------------------------------------------------------------------
// CCTK Battery Parsing
// ---------------------------------------------------------------------------

pub fn parse_battery_output(output: &str) -> Result<BatteryChargeSetting, DellError> {
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("PrimaryBattChargeCfg=") {
            return parse_battery_value(value.trim());
        }
    }
    Err(DellError::ParseError(format!(
        "no PrimaryBattChargeCfg= line in output: {output:?}"
    )))
}

pub fn parse_battery_list(output: &str) -> Vec<BatteryChargeMode> {
    let args_value = output.lines().find_map(|line| {
        line.find("Arguments:")
            .map(|index| line[index + "Arguments:".len()..].trim().to_owned())
    });

    let Some(args) = args_value else {
        return Vec::new();
    };

    let mut modes = Vec::new();
    for token in args.split('|') {
        let token = token.trim();
        if let Some(name) = token.strip_suffix('+').map(str::trim) {
            let base_name = name.split(':').next().unwrap_or(name).trim();
            if let Ok(setting) = parse_battery_value_no_thresholds(base_name) {
                if !modes.contains(&setting.mode) {
                    modes.push(setting.mode);
                }
            }
        }
    }
    modes
}

fn parse_battery_value_no_thresholds(value: &str) -> Result<BatteryChargeSetting, DellError> {
    match value {
        "Standard" => Ok(BatteryChargeSetting::new(
            BatteryChargeMode::Standard,
            50,
            85,
        )?),
        "Express" => Ok(BatteryChargeSetting::new(
            BatteryChargeMode::Express,
            50,
            85,
        )?),
        "PrimAcUse" => Ok(BatteryChargeSetting::new(
            BatteryChargeMode::PrimarilyAc,
            50,
            85,
        )?),
        "Adaptive" => Ok(BatteryChargeSetting::new(
            BatteryChargeMode::Adaptive,
            50,
            85,
        )?),
        "Custom" => Ok(BatteryChargeSetting::new(
            BatteryChargeMode::Custom,
            50,
            85,
        )?),
        other => Err(DellError::ParseError(format!(
            "unknown battery mode: {other:?}"
        ))),
    }
}

fn parse_battery_value(value: &str) -> Result<BatteryChargeSetting, DellError> {
    let split: Vec<&str> = value.split(':').collect();
    let name = split[0];

    match name {
        "Standard" | "Express" | "PrimAcUse" | "Adaptive" => {
            parse_battery_value_no_thresholds(name)
        }
        "Custom" => {
            let mut start = 50;
            let mut stop = 85;
            if split.len() >= 2 {
                let thresholds: Vec<&str> = split[1].split('-').collect();
                if thresholds.len() == 2 {
                    if let (Ok(st), Ok(sp)) = (thresholds[0].parse(), thresholds[1].parse()) {
                        start = st;
                        stop = sp;
                    }
                }
            }
            BatteryChargeSetting::new(BatteryChargeMode::Custom, start, stop)
        }
        other => Err(DellError::ParseError(format!(
            "unknown battery mode: {other:?}"
        ))),
    }
}

// ---------------------------------------------------------------------------
// CCTK command execution
// ---------------------------------------------------------------------------

/// Queries the current thermal management profile via CCTK.
pub fn get_thermal_profile(cctk_path: &PathBuf) -> Result<ThermalProfile, DellError> {
    let output = run_cctk(cctk_path, &["--ThermalManagement"])?;

    let stdout = decode_cctk_output(&output.stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DellError::CommandFailed(format!(
            "exit {}: {stderr}",
            output.status
        )));
    }
    parse_thermal_output(&stdout)
}

pub fn get_current_modes(
    cctk_path: &PathBuf,
) -> Result<(ThermalProfile, BatteryChargeSetting), DellError> {
    let output = run_cctk(
        cctk_path,
        &["--ThermalManagement", "--PrimaryBattChargeCfg"],
    )?;
    debug_cctk("query.current_modes.exit", output.status);
    if !output.status.success() {
        debug_cctk(
            "query.current_modes.output",
            format_args!(
                "stdout={:?} stderr={:?}",
                decode_cctk_output(&output.stdout).trim(),
                decode_cctk_output(&output.stderr).trim()
            ),
        );
        return Err(DellError::CommandFailed(format!(
            "exit {}",
            output.status.code().unwrap_or(-1)
        )));
    }
    let stdout = decode_cctk_output(&output.stdout);
    let stderr = decode_cctk_output(&output.stderr);
    let text = format!("{stdout}\n{stderr}");
    let thermal = parse_thermal_output(&text)?;
    let battery = parse_battery_output(&text)?;
    debug_cctk("query.current_modes.thermal", thermal.cctk_value());
    debug_cctk("query.current_modes.battery", battery.mode.cctk_value());
    Ok((thermal, battery))
}

fn execute_cctk_set(
    cctk_path: &PathBuf,
    arg: &str,
    cred: Option<&BiosPassword>,
) -> Result<(), DellError> {
    let mut cmd = cctk_command(cctk_path);
    cmd.arg(arg);

    if let Some(c) = cred {
        match c.kind {
            BiosPasswordKind::Setup => {
                cmd.arg(format!("--ValSetupPwd={}", c.password.as_str()));
            }
            BiosPasswordKind::System => {
                cmd.arg(format!("--ValSysPwd={}", c.password.as_str()));
            }
        }
    }

    let lock = CCTK_PROCESS_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock
        .lock()
        .map_err(|_| DellError::CommandFailed("CCTK process lock is unavailable".into()))?;
    let output = cmd
        .output()
        .map_err(|e| DellError::CommandFailed(e.to_string()))?;
    debug_cctk("set.exit", output.status);

    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        let stderr = decode_cctk_output(&output.stderr);
        let stdout = decode_cctk_output(&output.stdout);
        let message = format!("{stderr}\n{stdout}");
        let normalized = message.to_ascii_lowercase();

        if matches!(code, 58 | 65)
            || normalized.contains("setup password is required")
            || normalized.contains("--valsetuppwd")
        {
            return Err(DellError::SetupPasswordError(format!("exit {code}")));
        }
        if matches!(code, 66 | 67)
            || normalized.contains("system password is required")
            || normalized.contains("--valsyspwd")
        {
            return Err(DellError::SystemPasswordError(format!("exit {code}")));
        }

        return Err(DellError::CommandFailed(format!("exit {code}")));
    }
    Ok(())
}

pub fn set_power_source_modes(
    cctk_path: &PathBuf,
    thermal: Option<ThermalProfile>,
    battery: Option<&BatteryChargeSetting>,
    credential: Option<&BiosPassword>,
) -> Result<(Option<ThermalProfile>, Option<BatteryChargeSetting>), DellError> {
    let mut arguments = Vec::with_capacity(3);
    if let Some(profile) = thermal {
        arguments.push(format!("--ThermalManagement={}", profile.cctk_value()));
    }
    if let Some(setting) = battery {
        let value = if setting.mode == BatteryChargeMode::Custom {
            format!("Custom:{}-{}", setting.custom_start, setting.custom_stop)
        } else {
            setting.mode.cctk_value().to_string()
        };
        arguments.push(format!("--PrimaryBattChargeCfg={value}"));
    }
    if let Some(password) = credential {
        arguments.push(match password.kind {
            BiosPasswordKind::Setup => format!("--ValSetupPwd={}", password.password.as_str()),
            BiosPasswordKind::System => format!("--ValSysPwd={}", password.password.as_str()),
        });
    }
    let refs = arguments.iter().map(String::as_str).collect::<Vec<_>>();
    let output = run_cctk(cctk_path, &refs)?;
    debug_cctk("set.power_source.exit", output.status);
    if !output.status.success() {
        return classify_set_failure(&output);
    }

    let response = format!(
        "{}\n{}",
        decode_cctk_output(&output.stdout),
        decode_cctk_output(&output.stderr)
    );
    let returned_thermal = thermal.and_then(|_| parse_thermal_output(&response).ok());
    let returned_battery = battery.and_then(|_| parse_battery_output(&response).ok());
    debug_cctk(
        "set.power_source.response",
        format_args!(
            "thermal={returned_thermal:?} battery={:?}",
            returned_battery.as_ref().map(|setting| setting.mode)
        ),
    );
    Ok((
        returned_thermal.or(thermal),
        returned_battery.or_else(|| battery.cloned()),
    ))
}

fn classify_set_failure<T>(output: &Output) -> Result<T, DellError> {
    let code = output.status.code().unwrap_or(-1);
    let message = format!(
        "{}\n{}",
        decode_cctk_output(&output.stderr),
        decode_cctk_output(&output.stdout)
    );
    let normalized = message.to_ascii_lowercase();
    if matches!(code, 58 | 65)
        || normalized.contains("setup password is required")
        || normalized.contains("--valsetuppwd")
    {
        Err(DellError::SetupPasswordError(format!("exit {code}")))
    } else if matches!(code, 66 | 67)
        || normalized.contains("system password is required")
        || normalized.contains("--valsyspwd")
    {
        Err(DellError::SystemPasswordError(format!("exit {code}")))
    } else {
        Err(DellError::CommandFailed(format!("exit {code}")))
    }
}

/// Sets the thermal management profile via CCTK.
pub fn set_thermal_profile(
    cctk_path: &PathBuf,
    profile: ThermalProfile,
    cred: Option<&BiosPassword>,
) -> Result<(), DellError> {
    let arg = format!("--ThermalManagement={}", profile.cctk_value());
    execute_cctk_set(cctk_path, &arg, cred)
}

/// Lists supported thermal profiles by parsing CCTK help output.
pub fn list_thermal_profiles(cctk_path: &PathBuf) -> Result<Vec<ThermalProfile>, DellError> {
    let output = run_cctk(cctk_path, &["-H", "--ThermalManagement"])?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DellError::CommandFailed(format!(
            "exit {}: {stderr}",
            output.status
        )));
    }

    let stdout = decode_cctk_output(&output.stdout);
    Ok(parse_thermal_list(&stdout))
}

pub fn get_battery_charge_setting(cctk_path: &PathBuf) -> Result<BatteryChargeSetting, DellError> {
    let output = run_cctk(cctk_path, &["--PrimaryBattChargeCfg"])?;

    let stdout = decode_cctk_output(&output.stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DellError::CommandFailed(format!(
            "exit {}: {stderr}",
            output.status
        )));
    }
    parse_battery_output(&stdout)
}

pub fn set_battery_charge_setting(
    cctk_path: &PathBuf,
    setting: &BatteryChargeSetting,
    cred: Option<&BiosPassword>,
) -> Result<(), DellError> {
    let arg_val = if setting.mode == BatteryChargeMode::Custom {
        format!("Custom:{}-{}", setting.custom_start, setting.custom_stop)
    } else {
        setting.mode.cctk_value().to_string()
    };

    let arg = format!("--PrimaryBattChargeCfg={}", arg_val);
    execute_cctk_set(cctk_path, &arg, cred)
}

pub fn list_battery_charge_modes(cctk_path: &PathBuf) -> Result<Vec<BatteryChargeMode>, DellError> {
    let output = run_cctk(cctk_path, &["-H", "--PrimaryBattChargeCfg"])?;

    let stdout = decode_cctk_output(&output.stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DellError::CommandFailed(format!(
            "exit {}: {stderr}",
            output.status
        )));
    }
    Ok(parse_battery_list(&stdout))
}

/// Queries full Dell state: availability, current profile, supported profiles.
/// Returns `DellState::unavailable()` if CCTK is not found.
pub fn query_dell_state(cctk_path: Option<&PathBuf>) -> DellState {
    let Some(path) = cctk_path else {
        return DellState::unavailable();
    };

    let (current_profile, current_battery_mode) = match get_current_modes(path) {
        Ok((thermal, battery)) => (Some(thermal), Some(battery)),
        Err(_) => (
            get_thermal_profile(path).ok(),
            get_battery_charge_setting(path).ok(),
        ),
    };
    let mut supported_profiles = list_thermal_profiles(path).unwrap_or_default();
    if supported_profiles.is_empty() {
        supported_profiles = ThermalProfile::ALL.to_vec();
    }

    let mut supported_battery_modes = list_battery_charge_modes(path).unwrap_or_default();
    if supported_battery_modes.is_empty() {
        supported_battery_modes = vec![
            BatteryChargeMode::Standard,
            BatteryChargeMode::Express,
            BatteryChargeMode::PrimarilyAc,
            BatteryChargeMode::Adaptive,
            BatteryChargeMode::Custom,
        ];
    }

    let state = DellState {
        available: true,
        current_profile,
        supported_profiles,
        current_battery_mode,
        supported_battery_modes,
    };
    debug_cctk(
        "state.current",
        format_args!(
            "thermal={:?} battery={:?} thermal_supported={} battery_supported={}",
            state.current_profile,
            state
                .current_battery_mode
                .as_ref()
                .map(|setting| setting.mode),
            state.supported_profiles.len(),
            state.supported_battery_modes.len()
        ),
    );
    state
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_formatting() {
        let err1 = DellError::SetupPasswordError("exit 58: error text".into());
        let err2 = DellError::SystemPasswordError("exit 67: other error".into());
        assert_eq!(
            err1.to_string(),
            "Setup password error: exit 58: error text"
        );
        assert_eq!(
            err2.to_string(),
            "System password error: exit 67: other error"
        );
    }

    #[test]
    fn parse_thermal_output_optimized() {
        let output = "ThermalManagement=Optimized\n";
        assert_eq!(
            parse_thermal_output(output).unwrap(),
            ThermalProfile::Optimized
        );
    }

    #[test]
    fn parse_thermal_output_ultra_performance() {
        let output = "Some header\nThermalManagement=UltraPerformance\nSome footer\n";
        assert_eq!(
            parse_thermal_output(output).unwrap(),
            ThermalProfile::UltraPerformance
        );
    }

    #[test]
    fn parse_thermal_output_quiet() {
        let output = "ThermalManagement=Quiet";
        assert_eq!(parse_thermal_output(output).unwrap(), ThermalProfile::Quiet);
    }

    #[test]
    fn parse_thermal_output_cool() {
        let output = "  ThermalManagement=Cool  \n";
        assert_eq!(parse_thermal_output(output).unwrap(), ThermalProfile::Cool);
    }

    #[test]
    fn parse_thermal_output_missing_returns_error() {
        let output = "SomeOtherSetting=Value\n";
        assert!(parse_thermal_output(output).is_err());
    }

    #[test]
    fn parse_thermal_output_unknown_value_returns_error() {
        let output = "ThermalManagement=Turbo\n";
        assert!(parse_thermal_output(output).is_err());
    }

    #[test]
    fn parse_thermal_output_strips_extra_suffix() {
        let output = "ThermalManagement=Optimized:extra\n";
        assert_eq!(
            parse_thermal_output(output).unwrap(),
            ThermalProfile::Optimized
        );
    }

    #[test]
    fn parse_thermal_list_all_profiles() {
        let help = "Some help text\nArguments: Optimized+|Cool+|Quiet+|UltraPerformance+\n";
        let profiles = parse_thermal_list(help);
        assert_eq!(profiles, ThermalProfile::ALL.to_vec());
    }

    #[test]
    fn parse_thermal_list_subset() {
        let help = "Options: Arguments: Optimized+|Quiet+|Cool|UltraPerformance\n";
        let profiles = parse_thermal_list(help);
        assert_eq!(
            profiles,
            vec![ThermalProfile::Optimized, ThermalProfile::Quiet]
        );
    }

    #[test]
    fn parse_thermal_list_accepts_cctk_spacing() {
        let help = "Arguments: Optimized+ | Cool+ | Quiet+ | UltraPerformance+\n";
        assert_eq!(parse_thermal_list(help), ThermalProfile::ALL.to_vec());
    }

    #[test]
    fn parse_thermal_list_empty() {
        let help = "No thermal options available. Arguments: Cool\n";
        let profiles = parse_thermal_list(help);
        assert!(profiles.is_empty());
    }

    #[test]
    fn thermal_profile_serde_roundtrip() {
        for profile in &ThermalProfile::ALL {
            let json = serde_json::to_string(profile).unwrap();
            let parsed: ThermalProfile = serde_json::from_str(&json).unwrap();
            assert_eq!(*profile, parsed);
        }
    }

    #[test]
    fn thermal_profile_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&ThermalProfile::UltraPerformance).unwrap(),
            "\"ultra_performance\""
        );
        assert_eq!(
            serde_json::to_string(&ThermalProfile::Optimized).unwrap(),
            "\"optimized\""
        );
    }

    #[test]
    fn thermal_profile_id_roundtrip() {
        for profile in &ThermalProfile::ALL {
            assert_eq!(ThermalProfile::from_id(profile.id()), Some(*profile));
        }
    }

    #[test]
    fn thermal_profile_from_id_unknown() {
        assert_eq!(ThermalProfile::from_id("thermal_unknown"), None);
        assert_eq!(ThermalProfile::from_id("optimized"), None); // no prefix
    }

    #[test]
    fn dell_state_unavailable() {
        let state = DellState::unavailable();
        assert!(!state.available);
        assert!(state.current_profile.is_none());
        assert!(state.supported_profiles.is_empty());
        assert!(state.current_battery_mode.is_none());
        assert!(state.supported_battery_modes.is_empty());
    }

    #[test]
    fn parse_battery_output_standard() {
        let output = "PrimaryBattChargeCfg=Standard\n";
        assert_eq!(
            parse_battery_output(output).unwrap().mode,
            BatteryChargeMode::Standard
        );
    }

    #[test]
    fn parse_battery_output_custom() {
        let output = "PrimaryBattChargeCfg=Custom:60-80\n";
        let cfg = parse_battery_output(output).unwrap();
        assert_eq!(cfg.mode, BatteryChargeMode::Custom);
        assert_eq!(cfg.custom_start, 60);
        assert_eq!(cfg.custom_stop, 80);
    }

    #[test]
    fn parse_battery_list_modes() {
        let help = "Arguments: Standard+|Express+|PrimAcUse+|Adaptive+|Custom:abc-def+\n";
        let modes = parse_battery_list(help);
        assert_eq!(
            modes,
            vec![
                BatteryChargeMode::Standard,
                BatteryChargeMode::Express,
                BatteryChargeMode::PrimarilyAc,
                BatteryChargeMode::Adaptive,
                BatteryChargeMode::Custom,
            ]
        );
    }

    #[test]
    fn parse_battery_list_accepts_cctk_spacing() {
        let help = "Arguments: Standard+ | Express+ | PrimAcUse+ | Adaptive+ | Custom+\n";
        assert_eq!(
            parse_battery_list(help),
            vec![
                BatteryChargeMode::Standard,
                BatteryChargeMode::Express,
                BatteryChargeMode::PrimarilyAc,
                BatteryChargeMode::Adaptive,
                BatteryChargeMode::Custom,
            ]
        );
    }

    #[test]
    fn decode_cctk_utf16_output() {
        let text = "ThermalManagement=UltraPerformance\r\nPrimaryBattChargeCfg=PrimAcUse\r\n";
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        let decoded = decode_cctk_output(&bytes);
        assert_eq!(
            parse_thermal_output(&decoded).ok(),
            Some(ThermalProfile::UltraPerformance)
        );
        assert_eq!(
            parse_battery_output(&decoded)
                .ok()
                .map(|setting| setting.mode),
            Some(BatteryChargeMode::PrimarilyAc)
        );
    }
}
