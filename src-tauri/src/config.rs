use crate::dell::{BatteryChargeMode, BatteryChargeSetting, ThermalProfile};
use crate::power::PowerScheme;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Power sub-config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PowerConfig {
    pub auto_switch: bool,
    pub on_ac: PowerScheme,
    pub on_battery: PowerScheme,
}

impl Default for PowerConfig {
    fn default() -> Self {
        Self {
            auto_switch: false,
            on_ac: PowerScheme::Balanced,
            on_battery: PowerScheme::PowerSaver,
        }
    }
}

// ---------------------------------------------------------------------------
// Thermal sub-config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThermalConfig {
    pub auto_switch: bool,
    pub on_ac: ThermalProfile,
    pub on_battery: ThermalProfile,
}

impl Default for ThermalConfig {
    fn default() -> Self {
        Self {
            auto_switch: false,
            on_ac: ThermalProfile::Optimized,
            on_battery: ThermalProfile::Quiet,
        }
    }
}

// ---------------------------------------------------------------------------
// Battery charge sub-config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatteryConfig {
    pub auto_switch: bool,
    pub on_ac: BatteryChargeSetting,
    pub on_battery: BatteryChargeSetting,
}

impl Default for BatteryConfig {
    fn default() -> Self {
        Self {
            auto_switch: false,
            on_ac: BatteryChargeSetting {
                mode: BatteryChargeMode::PrimarilyAc,
                custom_start: 50,
                custom_stop: 85,
            },
            on_battery: BatteryChargeSetting {
                mode: BatteryChargeMode::Adaptive,
                custom_start: 50,
                custom_stop: 85,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub power: PowerConfig,
    #[serde(default)]
    pub thermal: Option<ThermalConfig>,
    #[serde(default)]
    pub battery: Option<BatteryConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            power: PowerConfig::default(),
            thermal: None,
            battery: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Loading with legacy migration
// ---------------------------------------------------------------------------

/// Loads config from disk, handling both the new nested format and the old flat
/// format (`{auto_switch, on_ac, on_battery}` without a `power` wrapper).
///
/// Migration order: we probe the raw JSON for a `"power"` key first. Without
/// this check, old flat JSON would successfully deserialize as `Config` (because
/// serde's `#[serde(default)]` on `power` would silently fill in defaults),
/// losing the user's actual settings.
pub fn load_config(path: &PathBuf) -> Config {
    let json_str = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return Config::default(),
    };

    parse_config_json(&json_str)
}

/// Pure parsing logic, separated for unit testing without filesystem.
fn parse_config_json(json_str: &str) -> Config {
    let raw: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return Config::default(),
    };

    // If the JSON has a "power" key, it's the new format.
    if raw.get("power").is_some() {
        if let Ok(config) = serde_json::from_value::<Config>(raw) {
            return config;
        }
        return Config::default();
    }

    // Otherwise, try old flat format: {auto_switch, on_ac, on_battery}
    #[derive(Deserialize)]
    struct OldConfig {
        auto_switch: bool,
        on_ac: PowerScheme,
        on_battery: PowerScheme,
    }

    if let Ok(old) = serde_json::from_value::<OldConfig>(raw) {
        return Config {
            power: PowerConfig {
                auto_switch: old.auto_switch,
                on_ac: old.on_ac,
                on_battery: old.on_battery,
            },
            thermal: None,
            battery: None,
        };
    }

    Config::default()
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

pub fn persist_config(path: &PathBuf, config: &Config) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_new_format() {
        let json = r#"{
            "power": {
                "auto_switch": true,
                "on_ac": "high_performance",
                "on_battery": "balanced"
            },
            "thermal": {
                "auto_switch": false,
                "on_ac": "optimized",
                "on_battery": "quiet"
            }
        }"#;
        let config = parse_config_json(json);
        assert!(config.power.auto_switch);
        assert_eq!(config.power.on_ac, PowerScheme::HighPerformance);
        assert_eq!(config.power.on_battery, PowerScheme::Balanced);
        let thermal = config.thermal.as_ref().unwrap();
        assert!(!thermal.auto_switch);
        assert_eq!(thermal.on_ac, ThermalProfile::Optimized);
        assert_eq!(thermal.on_battery, ThermalProfile::Quiet);
    }

    #[test]
    fn parse_new_format_without_thermal() {
        let json = r#"{
            "power": {
                "auto_switch": false,
                "on_ac": "balanced",
                "on_battery": "power_saver"
            }
        }"#;
        let config = parse_config_json(json);
        assert!(!config.power.auto_switch);
        assert!(config.thermal.is_none());
    }

    #[test]
    fn parse_old_flat_format_migrates_correctly() {
        let json = r#"{
            "auto_switch": true,
            "on_ac": "high_performance",
            "on_battery": "power_saver"
        }"#;
        let config = parse_config_json(json);
        assert!(config.power.auto_switch);
        assert_eq!(config.power.on_ac, PowerScheme::HighPerformance);
        assert_eq!(config.power.on_battery, PowerScheme::PowerSaver);
        assert!(config.thermal.is_none());
    }

    #[test]
    fn parse_old_flat_format_does_not_lose_settings_to_defaults() {
        // Critical regression: without the "power" key probe, old flat JSON
        // would deserialize via Config's default on `power`, losing settings.
        let json = r#"{
            "auto_switch": true,
            "on_ac": "power_saver",
            "on_battery": "high_performance"
        }"#;
        let config = parse_config_json(json);
        assert_ne!(config.power, PowerConfig::default());
        assert_eq!(config.power.on_ac, PowerScheme::PowerSaver);
        assert_eq!(config.power.on_battery, PowerScheme::HighPerformance);
    }

    #[test]
    fn parse_invalid_json_returns_default() {
        let config = parse_config_json("not json at all");
        assert_eq!(config, Config::default());
    }

    #[test]
    fn parse_empty_object_returns_default() {
        let config = parse_config_json("{}");
        assert_eq!(config, Config::default());
    }

    #[test]
    fn config_serde_roundtrip() {
        let config = Config {
            power: PowerConfig {
                auto_switch: true,
                on_ac: PowerScheme::HighPerformance,
                on_battery: PowerScheme::PowerSaver,
            },
            thermal: Some(ThermalConfig {
                auto_switch: true,
                on_ac: ThermalProfile::UltraPerformance,
                on_battery: ThermalProfile::Cool,
            }),
            battery: Some(BatteryConfig::default()),
        };
        let json = serde_json::to_string_pretty(&config).unwrap();
        let parsed = parse_config_json(&json);
        assert_eq!(config, parsed);
    }
}
