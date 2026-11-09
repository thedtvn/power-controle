use serde::{Deserialize, Serialize};
use std::{mem, ptr};
use windows::core::GUID;
use windows::Win32::Foundation::HLOCAL;
use windows::Win32::System::Power::{
    GetSystemPowerStatus, PowerGetActiveScheme, PowerSetActiveScheme, SYSTEM_POWER_STATUS,
};

// ---------------------------------------------------------------------------
// Power scheme GUIDs (well-known Windows power plan identifiers)
// ---------------------------------------------------------------------------

const GUID_BALANCED: GUID = GUID::from_u128(0x381b4222_f694_41f0_9685_ff5bb260df2e);
const GUID_HIGH_PERF: GUID = GUID::from_u128(0x8c5e7fda_e8bf_4a96_9a85_a6e23a8c635c);
const GUID_POWER_SAVER: GUID = GUID::from_u128(0xa1841308_3541_4fab_bc81_f71556f20b4a);

// ---------------------------------------------------------------------------
// PowerScheme enum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerScheme {
    Balanced,
    HighPerformance,
    PowerSaver,
}

impl PowerScheme {
    pub const ALL: [Self; 3] = [Self::Balanced, Self::HighPerformance, Self::PowerSaver];

    pub fn guid(&self) -> &'static GUID {
        match self {
            Self::Balanced => &GUID_BALANCED,
            Self::HighPerformance => &GUID_HIGH_PERF,
            Self::PowerSaver => &GUID_POWER_SAVER,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Balanced => "Balanced",
            Self::HighPerformance => "High Performance",
            Self::PowerSaver => "Power Saver",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::HighPerformance => "high_performance",
            Self::PowerSaver => "power_saver",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "balanced" => Some(Self::Balanced),
            "high_performance" => Some(Self::HighPerformance),
            "power_saver" => Some(Self::PowerSaver),
            _ => None,
        }
    }

    pub fn from_guid(guid: &GUID) -> Option<Self> {
        Self::ALL.iter().find(|s| s.guid() == guid).copied()
    }

    /// Embedded PNG data; the native tray adapter owns image decoding.
    pub fn icon(&self) -> &'static [u8] {
        match self {
            Self::Balanced => include_bytes!("../icons/tray_balanced.png"),
            Self::HighPerformance => include_bytes!("../icons/tray_high_performance.png"),
            Self::PowerSaver => include_bytes!("../icons/tray_power_saver.png"),
        }
    }
}

pub fn loading_icon() -> &'static [u8] {
    include_bytes!("../icons/tray_loading.png")
}

// ---------------------------------------------------------------------------
// Status (exposed to frontend)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub active_scheme: PowerScheme,
    pub power_source: String,
    pub battery_percent: u8,
}

// ---------------------------------------------------------------------------
// Windows API wrappers
// ---------------------------------------------------------------------------

/// Returns the currently active Windows power scheme.
/// Falls back to `Balanced` if the API call fails or returns an unknown GUID.
pub fn get_active_scheme() -> PowerScheme {
    // SAFETY: `PowerGetActiveScheme` writes a heap-allocated GUID pointer
    // that must be freed with `LocalFree`. We read the GUID and free
    // immediately. The pointer is only dereferenced after a null check.
    unsafe {
        let mut guid_ptr: *mut GUID = ptr::null_mut();
        let result = PowerGetActiveScheme(None, &mut guid_ptr);
        if result.0 == 0 && !guid_ptr.is_null() {
            let guid = *guid_ptr;
            let _ = windows::Win32::Foundation::LocalFree(Some(HLOCAL(guid_ptr as *mut _)));
            PowerScheme::from_guid(&guid).unwrap_or(PowerScheme::Balanced)
        } else {
            PowerScheme::Balanced
        }
    }
}

/// Sets the active Windows power scheme. Returns `true` on success.
pub fn set_active_scheme(scheme: PowerScheme) -> bool {
    // SAFETY: `PowerSetActiveScheme` is a safe Win32 call that reads
    // the GUID from the provided pointer. The GUID is a static reference.
    unsafe { PowerSetActiveScheme(None, Some(scheme.guid())).0 == 0 }
}

/// Queries the system power status (AC/battery, charge level).
pub fn query_power_status() -> Option<SYSTEM_POWER_STATUS> {
    // SAFETY: `GetSystemPowerStatus` writes to a caller-provided
    // stack-allocated struct. The struct is zero-initialized first.
    unsafe {
        let mut status: SYSTEM_POWER_STATUS = mem::zeroed();
        match GetSystemPowerStatus(&mut status) {
            Ok(()) => Some(status),
            Err(_) => None,
        }
    }
}

pub fn power_source_str(status: &Option<SYSTEM_POWER_STATUS>) -> String {
    match status.as_ref().map(|s| s.ACLineStatus) {
        Some(0) => "battery".into(),
        Some(1) => "ac".into(),
        _ => "unknown".into(),
    }
}

pub fn battery_percent(status: &Option<SYSTEM_POWER_STATUS>) -> u8 {
    match status.as_ref().map(|s| s.BatteryLifePercent) {
        Some(p) if p <= 100 => p,
        _ => 255, // unknown / no battery
    }
}

pub fn current_status() -> Status {
    let ps = query_power_status();
    Status {
        active_scheme: get_active_scheme(),
        power_source: power_source_str(&ps),
        battery_percent: battery_percent(&ps),
    }
}
