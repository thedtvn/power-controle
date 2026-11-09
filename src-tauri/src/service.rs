//! Serialized, UI-independent system operations. Snapshot reads never launch a process.
use crate::{battery_info, bios_pwd, cctk_install, config, dell, monitor, power, startup};
use bios_pwd::{BiosPasswordKind, BiosPasswordStatus};
use config::Config;
use dell::{BatteryChargeSetting, DellState, ThermalProfile};
use power::PowerScheme;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct Snapshot {
    pub config: Config,
    pub status: power::Status,
    pub battery: battery_info::BatteryOverview,
    pub dell: DellState,
    pub autostart: bool,
    pub password: BiosPasswordStatus,
    pub busy: bool,
    pub message: Option<String>,
    pub warning: Option<String>,
    pub password_prompt: Option<BiosPasswordKind>,
    pub revision: u64,
}

// Deliberately not Debug/Serialize: this queue can contain a BIOS password.
pub enum Action {
    Refresh,
    SaveConfig(Config),
    SetScheme(PowerScheme),
    SetThermal(ThermalProfile),
    SetBattery(BatteryChargeSetting),
    SetAutostart(bool),
    SavePassword(BiosPasswordKind, Zeroizing<String>),
    ClearPassword,
    InstallCctk,
    DismissWarning,
    DismissPasswordPrompt,
    ApplySource,
}

struct Shared {
    snapshot: Snapshot,
    pending: usize,
}
struct Inner {
    shared: Mutex<Shared>,
    sender: mpsc::Sender<Action>,
    receiver: Mutex<Option<mpsc::Receiver<Action>>>,
    config_path: PathBuf,
}
#[derive(Clone)]
pub struct Service {
    inner: Arc<Inner>,
}

impl Service {
    pub fn new() -> Result<Self, String> {
        let base = std::env::var_os("APPDATA").ok_or("Windows APPDATA is unavailable")?;
        let config_path = PathBuf::from(base).join("com.duong.power-controle/config.json");
        let (sender, receiver) = mpsc::channel();
        let snapshot = Snapshot {
            config: config::load_config(&config_path),
            status: power::current_status(),
            battery: Default::default(),
            dell: DellState::unavailable(),
            autostart: false,
            password: BiosPasswordStatus {
                configured: false,
                kind: None,
            },
            busy: false,
            message: None,
            warning: None,
            password_prompt: None,
            revision: 0,
        };
        Ok(Self {
            inner: Arc::new(Inner {
                shared: Mutex::new(Shared {
                    snapshot,
                    pending: 0,
                }),
                sender,
                receiver: Mutex::new(Some(receiver)),
                config_path,
            }),
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        self.inner
            .shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot
            .clone()
    }

    fn update(&self, f: impl FnOnce(&mut Snapshot)) {
        let mut shared = self.inner.shared.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut shared.snapshot);
        shared.snapshot.revision = shared.snapshot.revision.wrapping_add(1);
    }

    pub fn submit(&self, action: Action) {
        // Dismissing a prompt should not wait behind a hardware subprocess.
        match action {
            Action::DismissWarning => {
                self.update(|s| s.warning = None);
                return;
            }
            Action::DismissPasswordPrompt => {
                self.update(|s| s.password_prompt = None);
                return;
            }
            _ => {}
        }
        {
            let mut shared = self.inner.shared.lock().unwrap_or_else(|e| e.into_inner());
            shared.pending += 1;
            shared.snapshot.busy = true;
            shared.snapshot.revision = shared.snapshot.revision.wrapping_add(1);
        }
        if self.inner.sender.send(action).is_err() {
            self.update(|s| {
                s.busy = false;
                s.message = Some("System worker is unavailable. Restart Power Control.".into());
            });
        }
    }

    pub fn start(&self) {
        let Some(receiver) = self
            .inner
            .receiver
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        else {
            return;
        };
        let service = self.clone();
        std::thread::spawn(move || {
            if let Err(error) = startup::migrate_legacy() {
                service.update(|s| s.message = Some(error));
            }
            service.submit(Action::ApplySource);
            service.submit(Action::Refresh);
            for action in receiver {
                if let Err(error) = service.perform(action) {
                    service.update(|s| s.message = Some(error));
                }
                let mut shared = service
                    .inner
                    .shared
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                shared.pending = shared.pending.saturating_sub(1);
                shared.snapshot.busy = shared.pending != 0;
                shared.snapshot.revision = shared.snapshot.revision.wrapping_add(1);
            }
        });
        let service = self.clone();
        std::thread::spawn(move || {
            for event in monitor::start_power_event_monitor() {
                // UPS/short-term source notifications are not an AC/battery
                // policy transition; retain the old no-hardware-write behavior.
                if !matches!(event, monitor::PowerEvent::ShortTermPower) {
                    service.submit(Action::ApplySource);
                }
                service.submit(Action::Refresh);
            }
        });
    }

    fn cctk(&self) -> Result<PathBuf, String> {
        dell::find_cctk().ok_or("Dell Command Configure is not installed".into())
    }

    fn dell_error(&self, error: dell::DellError) -> String {
        let text = error.to_string();
        self.update(|s| match error {
            dell::DellError::SetupPasswordError(_) => {
                s.password_prompt = Some(BiosPasswordKind::Setup)
            }
            dell::DellError::SystemPasswordError(_) => {
                s.password_prompt = Some(BiosPasswordKind::System)
            }
            dell::DellError::CommandFailed(_) => s.warning = Some(text.clone()),
            _ => {}
        });
        text
    }

    fn set_thermal(&self, profile: ThermalProfile) -> Result<(), String> {
        let path = self.cctk()?;
        let credential = bios_pwd::get_bios_password().map_err(|e| e.to_string())?;
        dell::set_thermal_profile(&path, profile, credential.as_ref())
            .map_err(|e| self.dell_error(e))
    }
    fn set_battery(&self, setting: &BatteryChargeSetting) -> Result<(), String> {
        let setting =
            BatteryChargeSetting::new(setting.mode, setting.custom_start, setting.custom_stop)
                .map_err(|e| e.to_string())?;
        let path = self.cctk()?;
        let credential = bios_pwd::get_bios_password().map_err(|e| e.to_string())?;
        dell::set_battery_charge_setting(&path, &setting, credential.as_ref())
            .map_err(|e| self.dell_error(e))
    }
    fn refresh_dell(&self) {
        let state = dell::query_dell_state(dell::find_cctk().as_ref());
        self.update(|s| s.dell = state);
    }
    fn apply_source(&self) -> Result<(), String> {
        let cfg = self.snapshot().config;
        let source = power::power_source_str(&power::query_power_status());
        if source != "ac" && source != "battery" {
            return Ok(());
        }
        let ac = source == "ac";
        // Each domain is independent: an unsupported power plan must not
        // suppress the configured thermal or battery policy.
        let mut outcomes = Vec::new();
        if cfg.power.auto_switch {
            outcomes.push(
                if power::set_active_scheme(if ac {
                    cfg.power.on_ac
                } else {
                    cfg.power.on_battery
                }) {
                    Ok(())
                } else {
                    Err("Failed to switch the Windows power plan".into())
                },
            );
        }
        if let Some(thermal) = cfg.thermal.filter(|t| t.auto_switch) {
            outcomes.push(self.set_thermal(if ac {
                thermal.on_ac
            } else {
                thermal.on_battery
            }));
        }
        if let Some(battery) = cfg.battery.filter(|b| b.auto_switch) {
            outcomes.push(self.set_battery(if ac {
                &battery.on_ac
            } else {
                &battery.on_battery
            }));
        }
        combine_outcomes(outcomes)
    }

    fn perform(&self, action: Action) -> Result<(), String> {
        match action {
            Action::Refresh => {
                let status = power::current_status();
                let battery = battery_info::get_battery_overview();
                let password = bios_pwd::get_status().map_err(|e| e.to_string())?;
                let autostart = startup::is_enabled();
                self.refresh_dell();
                self.update(|s| {
                    s.status = status;
                    s.battery = battery;
                    s.password = password;
                    if let Ok(enabled) = &autostart {
                        s.autostart = *enabled;
                    }
                });
                autostart?;
                return Ok(());
            }
            Action::SaveConfig(mut cfg) => {
                validate_config(&mut cfg)?;
                config::persist_config(&self.inner.config_path, &cfg)?;
                self.update(|s| s.config = cfg);
                let outcome = self.apply_source();
                self.refresh_dell();
                self.update(|s| s.status = power::current_status());
                outcome?;
            }
            Action::ApplySource => {
                let outcome = self.apply_source();
                self.refresh_dell();
                self.update(|s| s.status = power::current_status());
                outcome?;
                // Automatic power events must not erase an actionable error.
                return Ok(());
            }
            Action::SetScheme(scheme) => {
                if !power::set_active_scheme(scheme) {
                    return Err("Failed to set the Windows power plan".into());
                }
            }
            Action::SetThermal(profile) => {
                self.set_thermal(profile)?;
                self.refresh_dell();
            }
            Action::SetBattery(setting) => {
                self.set_battery(&setting)?;
                self.refresh_dell();
            }
            Action::SetAutostart(enabled) => {
                startup::set_enabled(enabled)?;
                self.update(|s| s.autostart = enabled);
            }
            Action::SavePassword(kind, mut password) => {
                bios_pwd::set_bios_password(kind, password.as_mut_str())
                    .map_err(|e| e.to_string())?;
                let status = bios_pwd::get_status().map_err(|e| e.to_string())?;
                self.update(|s| {
                    s.password = status;
                    s.password_prompt = None;
                });
            }
            Action::ClearPassword => {
                bios_pwd::clear_bios_password().map_err(|e| e.to_string())?;
                self.update(|s| {
                    s.password = BiosPasswordStatus {
                        configured: false,
                        kind: None,
                    }
                });
            }
            Action::InstallCctk => {
                let result = cctk_install::install_cctk().map_err(|e| e.to_string())?;
                self.refresh_dell();
                self.update(|s| {
                    s.message = Some(
                        if result.reboot_recommended {
                            "Dell Command Configure installed. Restart Windows to finish setup."
                        } else {
                            "Dell Command Configure installed."
                        }
                        .into(),
                    )
                });
                return Ok(());
            }
            Action::DismissWarning | Action::DismissPasswordPrompt => return Ok(()),
        }
        let status = power::current_status();
        self.update(|s| {
            s.status = status;
            s.message = Some("Changes applied.".into());
        });
        Ok(())
    }
}

fn combine_outcomes(outcomes: Vec<Result<(), String>>) -> Result<(), String> {
    let errors: Vec<_> = outcomes.into_iter().filter_map(Result::err).collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn validate_config(cfg: &mut Config) -> Result<(), String> {
    cfg.thermal.get_or_insert_with(Default::default);
    let battery = cfg.battery.get_or_insert_with(Default::default);
    battery.on_ac = BatteryChargeSetting::new(
        battery.on_ac.mode,
        battery.on_ac.custom_start,
        battery.on_ac.custom_stop,
    )
    .map_err(|e| e.to_string())?;
    battery.on_battery = BatteryChargeSetting::new(
        battery.on_battery.mode,
        battery.on_battery.custom_start,
        battery.on_battery.custom_stop,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_policy_failures_are_all_reported() {
        let mut attempted = Vec::new();
        let results = (0..3)
            .map(|domain| {
                attempted.push(domain);
                if domain != 1 {
                    Err(format!("domain {domain}"))
                } else {
                    Ok(())
                }
            })
            .collect();
        assert_eq!(combine_outcomes(results), Err("domain 0; domain 2".into()));
        assert_eq!(attempted, vec![0, 1, 2]);
        assert!(combine_outcomes(vec![Ok(()), Ok(())]).is_ok());
    }

    #[test]
    fn validation_preserves_defaults_and_rejects_invalid_custom_thresholds() {
        let mut cfg = Config::default();
        validate_config(&mut cfg).unwrap();
        cfg.battery.as_mut().unwrap().on_ac = BatteryChargeSetting {
            mode: dell::BatteryChargeMode::Custom,
            custom_start: 90,
            custom_stop: 50,
        };
        assert!(validate_config(&mut cfg).is_err());
    }
}
