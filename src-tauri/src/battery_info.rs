use serde::{Deserialize, Serialize};
use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BatteryState {
    Unknown,
    Charging,
    Discharging,
    PluggedNotCharging,
    FullyCharged,
    OnBattery,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BatteryOverview {
    pub available: bool,
    pub state: Option<BatteryState>,
    pub charge_rate_mw: Option<i32>,
    pub health_percent: Option<u8>,
    pub cycle_count: Option<u32>,
    pub model: Option<String>,
    pub vendor: Option<String>,
    pub chemistry: Option<String>,
    pub designed_capacity_mwh: Option<u32>,
    pub full_capacity_mwh: Option<u32>,
    pub current_capacity_mwh: Option<u32>,
    pub voltage_mv: Option<u32>,
    pub serial: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct PsBatteryStatus {
    power_online: Option<bool>,
    charging: Option<bool>,
    discharging: Option<bool>,
    charge_rate: Option<u32>,
    discharge_rate: Option<u32>,
    remaining_capacity: Option<u32>,
    voltage: Option<u32>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct PsBatteryStaticData {
    designed_capacity: Option<u32>,
    device_name: Option<String>,
    manufacture_name: Option<String>,
    serial_number: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct PsBatteryFull {
    full_charged_capacity: Option<u32>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct PsBatteryCycle {
    cycle_count: Option<u32>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct PsWin32Battery {
    chemistry: Option<u16>,
    design_capacity: Option<u32>,
    full_charge_capacity: Option<u32>,
    name: Option<String>,
}

#[derive(Deserialize, Debug)]
struct PsOutput {
    status: Option<PsBatteryStatus>,
    static_data: Option<PsBatteryStaticData>,
    full: Option<PsBatteryFull>,
    cycle: Option<PsBatteryCycle>,
    win32: Option<PsWin32Battery>,
}

pub fn get_battery_overview() -> BatteryOverview {
    let script = r#"
$status = Get-CimInstance -Namespace root\wmi -ClassName BatteryStatus -ErrorAction SilentlyContinue
$static = Get-CimInstance -Namespace root\wmi -ClassName BatteryStaticData -ErrorAction SilentlyContinue
$full = Get-CimInstance -Namespace root\wmi -ClassName BatteryFullChargedCapacity -ErrorAction SilentlyContinue
$cycle = Get-CimInstance -Namespace root\wmi -ClassName BatteryCycleCount -ErrorAction SilentlyContinue
$win32 = Get-CimInstance -Namespace root\cimv2 -ClassName Win32_Battery -ErrorAction SilentlyContinue
@{
  status = if ($status) { $status | Select-Object PowerOnline, Charging, Discharging, ChargeRate, DischargeRate, RemainingCapacity, Voltage } else { $null }
  static_data = if ($static) { $static | Select-Object DesignedCapacity, DeviceName, ManufactureName, SerialNumber, Chemistry } else { $null }
  full = if ($full) { $full | Select-Object FullChargedCapacity } else { $null }
  cycle = if ($cycle) { $cycle | Select-Object CycleCount } else { $null }
  win32 = if ($win32) { $win32 | Select-Object Chemistry, DesignCapacity, FullChargeCapacity, Name } else { $null }
} | ConvertTo-Json -Depth 3 -Compress
"#;

    let output = match Command::new("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            return BatteryOverview {
                available: false,
                ..Default::default()
            }
        }
    };

    if !output.status.success() {
        return BatteryOverview {
            available: false,
            ..Default::default()
        };
    }

    let stdout = match String::from_utf8(output.stdout) {
        Ok(s) => s,
        Err(_) => {
            return BatteryOverview {
                available: false,
                ..Default::default()
            }
        }
    };

    parse_battery_overview(&stdout)
}

fn parse_battery_overview(json: &str) -> BatteryOverview {
    let data: PsOutput = match serde_json::from_str(json) {
        Ok(d) => d,
        Err(_) => {
            return BatteryOverview {
                available: false,
                ..Default::default()
            }
        }
    };

    let mut overview = BatteryOverview {
        available: true,
        ..Default::default()
    };

    let mut has_data = false;

    if let Some(status) = data.status {
        has_data = true;
        let online = status.power_online.unwrap_or(false);
        let charging = status.charging.unwrap_or(false);
        let discharging = status.discharging.unwrap_or(false);

        let state = if charging {
            BatteryState::Charging
        } else if discharging {
            if online {
                BatteryState::OnBattery
            } else {
                BatteryState::Discharging
            }
        } else if online {
            // Not charging, not discharging, but plugged in
            BatteryState::PluggedNotCharging
        } else {
            BatteryState::Unknown
        };
        overview.state = Some(state);

        let charge_rate = status.charge_rate.unwrap_or(0) as i32;
        let discharge_rate = status.discharge_rate.unwrap_or(0) as i32;

        if charging && charge_rate > 0 {
            overview.charge_rate_mw = Some(charge_rate);
        } else if discharging && discharge_rate > 0 {
            overview.charge_rate_mw = Some(-discharge_rate);
        } else if charge_rate > 0 {
            overview.charge_rate_mw = Some(charge_rate);
        } else if discharge_rate > 0 {
            overview.charge_rate_mw = Some(-discharge_rate);
        } else {
            overview.charge_rate_mw = Some(0);
        }

        overview.current_capacity_mwh = status.remaining_capacity;
        overview.voltage_mv = status.voltage;
    }

    if let Some(win32) = data.win32 {
        has_data = true;
        if overview.model.is_none() {
            overview.model = win32.name.clone();
        }
        if overview.designed_capacity_mwh.is_none() {
            overview.designed_capacity_mwh = win32.design_capacity;
        }
        if overview.full_capacity_mwh.is_none() {
            overview.full_capacity_mwh = win32.full_charge_capacity;
        }
        if overview.chemistry.is_none() && win32.chemistry.is_some() {
            overview.chemistry = map_chemistry(win32.chemistry.unwrap());
        }
    }

    if let Some(stat) = data.static_data {
        has_data = true;
        if overview.model.is_none() {
            overview.model = stat.device_name;
        }
        overview.vendor = stat.manufacture_name;
        overview.serial = stat.serial_number;
        if overview.designed_capacity_mwh.is_none() {
            overview.designed_capacity_mwh = stat.designed_capacity;
        }
    }

    if let Some(full) = data.full {
        has_data = true;
        overview.full_capacity_mwh = full.full_charged_capacity;
    }

    if let Some(cycle) = data.cycle {
        overview.cycle_count = cycle.cycle_count;
    }

    // Attempt to compute health
    if let (Some(full), Some(design)) = (overview.full_capacity_mwh, overview.designed_capacity_mwh)
    {
        if design > 0 {
            let mut pct = (full as f32 / design as f32 * 100.0).round() as u32;
            if pct > 100 {
                pct = 100;
            }
            overview.health_percent = Some(pct as u8);
        }
    }

    // Refine state
    if overview.state == Some(BatteryState::PluggedNotCharging) {
        if let (Some(cur), Some(full)) = (overview.current_capacity_mwh, overview.full_capacity_mwh)
        {
            // Assume fully charged if within 1%
            if full > 0 && cur >= (full * 99 / 100) {
                overview.state = Some(BatteryState::FullyCharged);
            }
        }
    }

    if !has_data {
        overview.available = false;
    }

    overview
}

fn map_chemistry(val: u16) -> Option<String> {
    match val {
        1 => Some("Other".into()),
        3 => Some("Lead Acid".into()),
        4 => Some("Nickel Cadmium".into()),
        5 => Some("Nickel Metal Hydride".into()),
        6 => Some("Lithium-ion".into()),
        7 => Some("Zinc air".into()),
        8 => Some("Lithium Polymer".into()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_battery_overview() {
        let json = r#"
        {
          "status": {
            "PowerOnline": true,
            "Charging": false,
            "Discharging": false,
            "ChargeRate": 0,
            "DischargeRate": 0,
            "RemainingCapacity": 35265,
            "Voltage": 16116
          },
          "static_data": null,
          "full": {
            "FullChargedCapacity": 35265
          },
          "cycle": {
            "CycleCount": 5
          },
          "win32": {
            "Chemistry": 2,
            "DesignCapacity": 40000,
            "FullChargeCapacity": 35265,
            "Name": "DELL XDY9K1C"
          }
        }"#;

        let overview = parse_battery_overview(json);
        assert!(overview.available);
        assert_eq!(overview.state, Some(BatteryState::FullyCharged));
        assert_eq!(overview.charge_rate_mw, Some(0));
        assert_eq!(overview.current_capacity_mwh, Some(35265));
        assert_eq!(overview.voltage_mv, Some(16116));
        assert_eq!(overview.full_capacity_mwh, Some(35265));
        assert_eq!(overview.designed_capacity_mwh, Some(40000));
        assert_eq!(overview.model, Some("DELL XDY9K1C".into()));
        assert_eq!(overview.cycle_count, Some(5));
        assert_eq!(overview.health_percent, Some(88)); // 35265 / 40000 = 88.16% -> 88
    }

    #[test]
    fn test_parse_charging() {
        let json = r#"
        {
          "status": {
            "PowerOnline": true,
            "Charging": true,
            "Discharging": false,
            "ChargeRate": 15000,
            "DischargeRate": 0,
            "RemainingCapacity": 10000
          }
        }"#;
        let overview = parse_battery_overview(json);
        assert_eq!(overview.state, Some(BatteryState::Charging));
        assert_eq!(overview.charge_rate_mw, Some(15000));
    }

    #[test]
    fn test_parse_discharging() {
        let json = r#"
        {
          "status": {
            "PowerOnline": false,
            "Charging": false,
            "Discharging": true,
            "ChargeRate": 0,
            "DischargeRate": 12000,
            "RemainingCapacity": 10000
          }
        }"#;
        let overview = parse_battery_overview(json);
        assert_eq!(overview.state, Some(BatteryState::Discharging));
        assert_eq!(overview.charge_rate_mw, Some(-12000));
    }
}
