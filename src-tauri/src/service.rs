//! Serialized, UI-independent system operations with a persistent state cache.
//!
//! The tray process owns this service. All hardware reads happen once on
//! startup (and again only after a change or power event); every reader —
//! tray menu and Settings window — reuses the cached snapshot. The last good
//! hardware state is persisted so the next launch shows real data instantly
//! while the background refresh runs.
use crate::{battery_info, bios_pwd, cctk_install, config, dell, monitor, power, startup};
use bios_pwd::{BiosPasswordKind, BiosPasswordStatus};
use config::Config;
use dell::{BatteryChargeSetting, DellState, ThermalProfile};
use power::PowerScheme;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// Everything a UI needs. Cloning is cheap; reading never launches a process.
#[derive(Clone, Serialize, Deserialize)]
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
    /// False until the first full hardware refresh of this session finishes;
    /// cached values from the previous run are shown until then.
    #[serde(default)]
    pub loaded: bool,
}

// Deliberately not Debug: this queue can contain a BIOS password.
#[derive(Serialize, Deserialize)]
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

/// Hardware facts persisted between launches. Contains no secrets.
#[derive(Serialize, Deserialize)]
struct CachedState {
    dell: DellState,
    battery: battery_info::BatteryOverview,
    autostart: bool,
    password: BiosPasswordStatus,
    cctk_path: Option<PathBuf>,
}

struct Shared {
    snapshot: Snapshot,
    pending: usize,
    /// CCTK location discovered once and reused for every Dell command.
    cctk_path: Option<PathBuf>,
    battery_checked: Option<Instant>,
}
struct Inner {
    shared: Mutex<Shared>,
    sender: mpsc::Sender<Action>,
    receiver: Mutex<Option<mpsc::Receiver<Action>>>,
    config_path: PathBuf,
    cache_path: PathBuf,
}
#[derive(Clone)]
pub struct Service {
    inner: Arc<Inner>,
}

/// Battery telemetry (a PowerShell/WMI call) is the slowest read; reuse it
/// for this long unless a power event invalidates it.
const BATTERY_TTL: Duration = Duration::from_secs(60);

pub fn data_dir() -> Result<PathBuf, String> {
    let base = std::env::var_os("APPDATA").ok_or("Windows APPDATA is unavailable")?;
    Ok(PathBuf::from(base).join("com.duong.power-controle"))
}

fn load_cache(path: &Path) -> Option<CachedState> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

impl Service {
    pub fn new() -> Result<Self, String> {
        let dir = data_dir()?;
        let config_path = dir.join("config.json");
        let cache_path = dir.join("state-cache.json");
        let (sender, receiver) = mpsc::channel();
        let cached = load_cache(&cache_path);
        let cctk_path = cached
            .as_ref()
            .and_then(|c| c.cctk_path.clone())
            .filter(|p| p.exists());
        let snapshot = Snapshot {
            config: config::load_config(&config_path),
            status: power::current_status(),
            battery: cached
                .as_ref()
                .map(|c| c.battery.clone())
                .unwrap_or_default(),
            dell: cached
                .as_ref()
                .map(|c| c.dell.clone())
                .unwrap_or_else(DellState::unavailable),
            autostart: cached.as_ref().is_some_and(|c| c.autostart),
            password: cached
                .as_ref()
                .map(|c| c.password.clone())
                .unwrap_or(BiosPasswordStatus {
                    configured: false,
                    kind: None,
                }),
            busy: false,
            message: None,
            warning: None,
            password_prompt: None,
            revision: 0,
            loaded: false,
        };
        Ok(Self {
            inner: Arc::new(Inner {
                shared: Mutex::new(Shared {
                    snapshot,
                    pending: 0,
                    cctk_path,
                    battery_checked: None,
                }),
                sender,
                receiver: Mutex::new(Some(receiver)),
                config_path,
                cache_path,
            }),
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        self.lock().snapshot.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.inner.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn update(&self, f: impl FnOnce(&mut Snapshot)) {
        let mut shared = self.lock();
        f(&mut shared.snapshot);
        shared.snapshot.revision = shared.snapshot.revision.wrapping_add(1);
    }

    fn save_cache(&self) {
        let shared = self.lock();
        let cache = CachedState {
            dell: shared.snapshot.dell.clone(),
            battery: shared.snapshot.battery.clone(),
            autostart: shared.snapshot.autostart,
            password: shared.snapshot.password.clone(),
            cctk_path: shared.cctk_path.clone(),
        };
        drop(shared);
        if let Ok(json) = serde_json::to_vec(&cache) {
            let tmp = self.inner.cache_path.with_extension("tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &self.inner.cache_path);
            }
        }
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
            let mut shared = self.lock();
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
            // Load everything once up front; afterwards readers use the cache.
            service.submit(Action::ApplySource);
            service.submit(Action::Refresh);
            if let Err(error) = startup::migrate_legacy() {
                service.update(|s| s.message = Some(error));
            }
            for action in receiver {
                if let Err(error) = service.perform(action) {
                    service.update(|s| s.message = Some(error));
                }
                let mut shared = service.lock();
                shared.pending = shared.pending.saturating_sub(1);
                shared.snapshot.busy = shared.pending != 0;
                shared.snapshot.revision = shared.snapshot.revision.wrapping_add(1);
            }
        });
        let service = self.clone();
        std::thread::spawn(move || {
            for event in monitor::start_power_event_monitor() {
                // Power events change battery telemetry, so drop its TTL.
                service.lock().battery_checked = None;
                // UPS/short-term notifications are not an AC/battery policy
                // transition; keep the old no-hardware-write behavior.
                if !matches!(event, monitor::PowerEvent::ShortTermPower) {
                    service.submit(Action::ApplySource);
                }
                service.submit(Action::Refresh);
            }
        });
    }

    /// Cached CCTK location; filesystem probing only when unknown or moved.
    fn cctk_path(&self) -> Option<PathBuf> {
        let cached = self.lock().cctk_path.clone();
        if let Some(path) = cached.filter(|p| p.exists()) {
            return Some(path);
        }
        let found = dell::find_cctk();
        self.lock().cctk_path = found.clone();
        found
    }

    fn cctk(&self) -> Result<PathBuf, String> {
        self.cctk_path()
            .ok_or("Dell Command Configure is not installed".into())
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
            .map_err(|e| self.dell_error(e))?;
        // A successful write tells us the new value; no re-query needed.
        self.update(|s| s.dell.current_profile = Some(profile));
        Ok(())
    }

    fn set_battery(&self, setting: &BatteryChargeSetting) -> Result<(), String> {
        let setting =
            BatteryChargeSetting::new(setting.mode, setting.custom_start, setting.custom_stop)
                .map_err(|e| e.to_string())?;
        let path = self.cctk()?;
        let credential = bios_pwd::get_bios_password().map_err(|e| e.to_string())?;
        dell::set_battery_charge_setting(&path, &setting, credential.as_ref())
            .map_err(|e| self.dell_error(e))?;
        self.update(|s| s.dell.current_battery_mode = Some(setting));
        Ok(())
    }

    /// Full Dell query: supported lists plus current modes.
    fn refresh_dell_full(&self) {
        let state = dell::query_dell_state(self.cctk_path().as_ref());
        self.update(|s| s.dell = state);
    }

    /// Cheap Dell query: only the current modes. Supported profile/mode lists
    /// are hardware facts that do not change at runtime, so they stay cached.
    fn refresh_dell_current(&self) {
        let have_lists = self.lock().snapshot.dell.available;
        let Some(path) = self.cctk_path() else {
            self.update(|s| s.dell = DellState::unavailable());
            return;
        };
        if !have_lists {
            return self.refresh_dell_full();
        }
        if let Ok((thermal, battery)) = dell::get_current_modes(&path) {
            self.update(|s| {
                s.dell.current_profile = Some(thermal);
                s.dell.current_battery_mode = Some(battery);
            });
        }
    }

    fn refresh_battery(&self, force: bool) {
        let fresh = self
            .lock()
            .battery_checked
            .is_some_and(|at| at.elapsed() < BATTERY_TTL);
        if fresh && !force {
            return;
        }
        let battery = battery_info::get_battery_overview();
        let mut shared = self.lock();
        shared.snapshot.battery = battery;
        shared.battery_checked = Some(Instant::now());
        shared.snapshot.revision = shared.snapshot.revision.wrapping_add(1);
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
            let scheme = if ac {
                cfg.power.on_ac
            } else {
                cfg.power.on_battery
            };
            outcomes.push(if power::set_active_scheme(scheme) {
                Ok(())
            } else {
                Err("Failed to switch the Windows power plan".into())
            });
        }
        let thermal =
            cfg.thermal
                .filter(|t| t.auto_switch)
                .map(|t| if ac { t.on_ac } else { t.on_battery });
        let battery =
            cfg.battery
                .filter(|b| b.auto_switch)
                .map(|b| if ac { b.on_ac } else { b.on_battery });
        if thermal.is_some() || battery.is_some() {
            outcomes.push(self.apply_dell(thermal, battery));
        }
        combine_outcomes(outcomes)
    }

    /// Applies thermal and battery policy in a single CCTK invocation (one
    /// process spawn and BIOS round-trip instead of two). Falls back to
    /// separate calls if the combined write fails for a non-password reason,
    /// so one unsupported setting cannot block the other.
    fn apply_dell(
        &self,
        thermal: Option<ThermalProfile>,
        battery: Option<BatteryChargeSetting>,
    ) -> Result<(), String> {
        let battery = match battery {
            Some(b) => Some(
                BatteryChargeSetting::new(b.mode, b.custom_start, b.custom_stop)
                    .map_err(|e| e.to_string())?,
            ),
            None => None,
        };
        let path = self.cctk()?;
        let credential = bios_pwd::get_bios_password().map_err(|e| e.to_string())?;
        match dell::set_power_source_modes(&path, thermal, battery.as_ref(), credential.as_ref()) {
            Ok((applied_thermal, applied_battery)) => {
                self.update(|s| {
                    if applied_thermal.is_some() {
                        s.dell.current_profile = applied_thermal;
                    }
                    if applied_battery.is_some() {
                        s.dell.current_battery_mode = applied_battery;
                    }
                });
                Ok(())
            }
            Err(
                error @ (dell::DellError::SetupPasswordError(_)
                | dell::DellError::SystemPasswordError(_)),
            ) => Err(self.dell_error(error)),
            Err(_) if thermal.is_some() && battery.is_some() => {
                let outcomes = vec![
                    self.set_thermal(thermal.unwrap()),
                    self.set_battery(battery.as_ref().unwrap()),
                ];
                combine_outcomes(outcomes)
            }
            Err(error) => Err(self.dell_error(error)),
        }
    }

    fn perform(&self, action: Action) -> Result<(), String> {
        match action {
            Action::Refresh => {
                let first = !self.lock().snapshot.loaded;
                let status = power::current_status();
                let password = bios_pwd::get_status().map_err(|e| e.to_string());
                let autostart = startup::is_enabled();
                // Lists are only re-read on the first refresh of a session or
                // when CCTK was missing; later refreshes read current modes.
                if first {
                    self.refresh_dell_full();
                } else {
                    self.refresh_dell_current();
                }
                self.refresh_battery(first);
                self.update(|s| {
                    s.status = status;
                    if let Ok(p) = &password {
                        s.password = p.clone();
                    }
                    if let Ok(enabled) = &autostart {
                        s.autostart = *enabled;
                    }
                    s.loaded = true;
                });
                self.save_cache();
                password?;
                autostart?;
                return Ok(());
            }
            Action::SaveConfig(mut cfg) => {
                validate_config(&mut cfg)?;
                config::persist_config(&self.inner.config_path, &cfg)?;
                self.update(|s| s.config = cfg);
                let outcome = self.apply_source();
                self.update(|s| s.status = power::current_status());
                outcome?;
            }
            Action::ApplySource => {
                let outcome = self.apply_source();
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
            Action::SetThermal(profile) => self.set_thermal(profile)?,
            Action::SetBattery(setting) => self.set_battery(&setting)?,
            Action::SetAutostart(enabled) => {
                startup::set_enabled(enabled)?;
                self.update(|s| s.autostart = enabled);
                self.save_cache();
            }
            Action::SavePassword(kind, mut password) => {
                bios_pwd::set_bios_password(kind, password.as_mut_str())
                    .map_err(|e| e.to_string())?;
                let status = bios_pwd::get_status().map_err(|e| e.to_string())?;
                self.update(|s| {
                    s.password = status;
                    s.password_prompt = None;
                });
                self.save_cache();
            }
            Action::ClearPassword => {
                bios_pwd::clear_bios_password().map_err(|e| e.to_string())?;
                self.update(|s| {
                    s.password = BiosPasswordStatus {
                        configured: false,
                        kind: None,
                    }
                });
                self.save_cache();
            }
            Action::InstallCctk => {
                let result = cctk_install::install_cctk().map_err(|e| e.to_string())?;
                self.lock().cctk_path = None;
                self.refresh_dell_full();
                self.save_cache();
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
        self.save_cache();
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

    #[test]
    fn cache_round_trips_without_secrets() {
        let cache = CachedState {
            dell: DellState::unavailable(),
            battery: Default::default(),
            autostart: true,
            password: BiosPasswordStatus {
                configured: true,
                kind: Some(BiosPasswordKind::Setup),
            },
            cctk_path: Some(PathBuf::from(r"C:\Program Files\Dell\cctk.exe")),
        };
        let json = serde_json::to_string(&cache).unwrap();
        assert!(
            !json.to_lowercase().contains("password\":\""),
            "cache must not hold secrets"
        );
        let back: CachedState = serde_json::from_str(&json).unwrap();
        assert!(back.autostart && back.password.configured);
        assert!(load_cache(Path::new(r"Z:\does\not\exist.json")).is_none());
    }

    #[test]
    fn cache_is_written_by_service_and_reused_on_next_start() {
        let dir = std::env::temp_dir().join(format!("pc-svc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (sender, receiver) = mpsc::channel();
        let make = |sender, receiver| Service {
            inner: Arc::new(Inner {
                shared: Mutex::new(Shared {
                    snapshot: Snapshot {
                        config: Config::default(),
                        status: power::current_status(),
                        battery: Default::default(),
                        dell: DellState::unavailable(),
                        autostart: false,
                        password: BiosPasswordStatus { configured: false, kind: None },
                        busy: false,
                        message: None,
                        warning: None,
                        password_prompt: None,
                        revision: 0,
                        loaded: false,
                    },
                    pending: 0,
                    cctk_path: None,
                    battery_checked: None,
                }),
                sender,
                receiver: Mutex::new(Some(receiver)),
                config_path: dir.join("config.json"),
                cache_path: dir.join("state-cache.json"),
            }),
        };
        let service = make(sender, receiver);
        service.update(|s| {
            s.autostart = true;
            s.dell.available = true;
            s.dell.current_profile = Some(ThermalProfile::Quiet);
        });
        service.save_cache();
        let cached = load_cache(&dir.join("state-cache.json")).expect("cache written");
        assert!(cached.autostart);
        assert_eq!(cached.dell.current_profile, Some(ThermalProfile::Quiet));
        assert!(!dir.join("state-cache.tmp").exists(), "atomic rename leaves no temp file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_cache_is_ignored() {
        let dir = std::env::temp_dir().join(format!("pc-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state-cache.json");
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(load_cache(&path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
