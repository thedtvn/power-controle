mod battery_info;
mod bios_pwd;
mod cctk_install;
mod config;
mod dell;
mod monitor;
mod power;

use std::path::PathBuf;
use std::sync::{Mutex, RwLock};

use tauri::{
    menu::{IconMenuItemBuilder, MenuBuilder, MenuItemBuilder, SubmenuBuilder},
    tray::{TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, Wry,
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_autostart::ManagerExt;

use config::{BatteryConfig, Config, PowerConfig};
use dell::{BatteryChargeMode, BatteryChargeSetting, DellState, ThermalProfile};
use monitor::PowerEvent;
use power::PowerScheme;

// ---------------------------------------------------------------------------
// Shared application state
// ---------------------------------------------------------------------------

struct AppState {
    config: Mutex<Config>,
    config_path: PathBuf,
    cctk_path: RwLock<Option<PathBuf>>,
    dell_state: RwLock<DellState>,
    tray_actions: Mutex<Vec<TrayAction>>,
    cctk_warning: Mutex<Option<String>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TrayAction {
    PowerScheme,
    ThermalProfile,
    BatteryCharge,
}

impl TrayAction {
    fn from_menu_id(id: &str) -> Option<Self> {
        if PowerScheme::from_id(id).is_some() {
            Some(Self::PowerScheme)
        } else if ThermalProfile::from_id(id).is_some() {
            Some(Self::ThermalProfile)
        } else if BatteryChargeMode::from_id(id).is_some() {
            Some(Self::BatteryCharge)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn cmd_get_config(state: State<'_, AppState>) -> Result<Config, String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?;
    Ok(cfg.clone())
}

/// Accepts the new nested format from the frontend.
///
/// For backward compatibility with the existing frontend (which sends flat
/// `{auto_switch, on_ac, on_battery}`), the frontend JS should wrap its
/// payload inside `{power: {...}}`. The backend always expects `Config`.
#[tauri::command]
async fn cmd_save_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: Config,
) -> Result<(), String> {
    {
        let mut current = state.config.lock().map_err(|e| e.to_string())?;
        *current = config.clone();
    }
    config::persist_config(&state.config_path, &config)?;
    let cctk_path = state.cctk_path.read().map_err(|e| e.to_string())?.clone();
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if config.power.auto_switch {
            apply_power_for_current_source(&config.power);
        }
        if let Some(ref thermal) = config.thermal {
            if thermal.auto_switch {
                apply_thermal_for_current_source(thermal, cctk_path.as_ref())?;
            }
        }
        if let Some(ref battery) = config.battery {
            if battery.auto_switch {
                apply_battery_for_current_source(battery, cctk_path.as_ref())?;
            }
        }
        let state = worker_app.state::<AppState>();
        let dell_state = refresh_dell_state(&state);
        let _ = worker_app.emit("thermal-changed", dell_state.clone());
        let _ = worker_app.emit("battery-setting-changed", dell_state);
        update_tray(&worker_app);
        Ok::<(), String>(())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn cmd_get_status() -> power::Status {
    power::current_status()
}

#[tauri::command]
fn cmd_set_scheme(app: AppHandle, scheme: PowerScheme) -> Result<(), String> {
    if power::set_active_scheme(scheme) {
        update_tray(&app);
        let _ = app.emit("status-changed", power::current_status());
        Ok(())
    } else {
        Err("Failed to set power scheme".into())
    }
}

#[tauri::command]
fn cmd_get_autostart(app: AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
fn cmd_set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mgr = app.autolaunch();
    if enabled {
        mgr.enable().map_err(|e| e.to_string())
    } else {
        mgr.disable().map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn cmd_get_dell_state(state: State<'_, AppState>) -> DellState {
    state
        .dell_state
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

#[tauri::command]
fn cmd_refresh_dell_state(state: State<'_, AppState>) -> DellState {
    refresh_dell_state(&state)
}

#[tauri::command]
fn cmd_set_thermal_profile(
    app: AppHandle,
    state: State<'_, AppState>,
    profile: ThermalProfile,
) -> Result<(), String> {
    let cctk_guard = state.cctk_path.read().map_err(|e| e.to_string())?;
    let cctk = cctk_guard
        .as_ref()
        .ok_or_else(|| "Dell CCTK not available".to_string())?;

    let cred = bios_pwd::get_bios_password().map_err(|e| e.to_string())?;
    dell::set_thermal_profile(cctk, profile, cred.as_ref()).map_err(|e| e.to_string())?;

    drop(cctk_guard);
    let dell_state = refresh_dell_state(&state);
    update_tray(&app);
    let _ = app.emit("thermal-changed", dell_state);
    Ok(())
}

#[tauri::command]
fn cmd_set_battery_charge_setting(
    app: AppHandle,
    state: State<'_, AppState>,
    setting: BatteryChargeSetting,
) -> Result<(), String> {
    let cctk_guard = state.cctk_path.read().map_err(|e| e.to_string())?;
    let cctk = cctk_guard
        .as_ref()
        .ok_or_else(|| "Dell CCTK not available".to_string())?;
    let validated =
        BatteryChargeSetting::new(setting.mode, setting.custom_start, setting.custom_stop)
            .map_err(|e| e.to_string())?;

    let cred = bios_pwd::get_bios_password().map_err(|e| e.to_string())?;
    dell::set_battery_charge_setting(cctk, &validated, cred.as_ref()).map_err(|e| e.to_string())?;

    drop(cctk_guard);
    let dell_state = refresh_dell_state(&state);
    update_tray(&app);
    let _ = app.emit("battery-setting-changed", dell_state);
    Ok(())
}

use bios_pwd::{BiosPasswordKind, BiosPasswordStatus};

#[tauri::command]
fn cmd_get_bios_password_status() -> Result<BiosPasswordStatus, String> {
    bios_pwd::get_status().map_err(|e| e.to_string())
}

#[tauri::command]
fn cmd_set_bios_password(kind: BiosPasswordKind, password: &str) -> Result<(), String> {
    bios_pwd::set_bios_password(kind, password).map_err(|e| e.to_string())
}

#[tauri::command]
fn cmd_clear_bios_password() -> Result<(), String> {
    bios_pwd::clear_bios_password().map_err(|e| e.to_string())
}

#[tauri::command]
fn cmd_open_bios_password_prompt(app: AppHandle, kind: BiosPasswordKind) -> Result<(), String> {
    open_bios_password_prompt(&app, kind)
}

#[tauri::command]
fn cmd_get_cctk_warning(state: State<'_, AppState>) -> Result<Option<String>, String> {
    state
        .cctk_warning
        .lock()
        .map(|warning| warning.clone())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn cmd_close_settings(app: AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("settings")
        .ok_or_else(|| "Settings window is not open".to_string())?;
    window.close().map_err(|e| e.to_string())
}

#[tauri::command]
fn cmd_get_battery_overview() -> battery_info::BatteryOverview {
    battery_info::get_battery_overview()
}

#[tauri::command]
async fn cmd_install_cctk(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<cctk_install::InstallResult, String> {
    let result = cctk_install::install_cctk().map_err(|e| e.to_string())?;

    let new_path = dell::find_cctk();

    if new_path.is_some() {
        if let Ok(mut guard) = state.cctk_path.write() {
            *guard = new_path;
        }

        if let Ok(cfg) = state.config.lock() {
            let cctk_guard = state.cctk_path.read().unwrap_or_else(|e| e.into_inner());

            if let Some(ref thermal) = cfg.thermal {
                if thermal.auto_switch {
                    let _ = apply_thermal_for_current_source(thermal, cctk_guard.as_ref());
                }
            }
            if let Some(ref battery) = cfg.battery {
                if battery.auto_switch {
                    let _ = apply_battery_for_current_source(battery, cctk_guard.as_ref());
                }
            }
        }
    }

    let dell_state = refresh_dell_state(&state);
    update_tray(&app);
    let _ = app.emit("thermal-changed", dell_state.clone());
    let _ = app.emit("battery-setting-changed", dell_state);

    Ok(result)
}

fn refresh_dell_state(state: &AppState) -> DellState {
    let cctk_path = state
        .cctk_path
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let dell_state = dell::query_dell_state(cctk_path.as_ref());
    *state.dell_state.write().unwrap_or_else(|e| e.into_inner()) = dell_state.clone();
    dell_state
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

fn apply_power_for_current_source(power_cfg: &PowerConfig) {
    let source = power::power_source_str(&power::query_power_status());
    let scheme = match source.as_str() {
        "ac" => Some(power_cfg.on_ac),
        "battery" => Some(power_cfg.on_battery),
        _ => None,
    };
    if let Some(s) = scheme {
        power::set_active_scheme(s);
    }
}

fn apply_battery_for_current_source(
    battery_cfg: &BatteryConfig,
    cctk_path: Option<&PathBuf>,
) -> Result<(), String> {
    let cctk = cctk_path.ok_or_else(|| "Dell CCTK not available".to_string())?;
    let source = power::power_source_str(&power::query_power_status());
    let setting = match source.as_str() {
        "ac" => Some(&battery_cfg.on_ac),
        "battery" => Some(&battery_cfg.on_battery),
        _ => None,
    };
    if let Some(s) = setting {
        let validated = BatteryChargeSetting::new(s.mode, s.custom_start, s.custom_stop)
            .map_err(|e| e.to_string())?;

        let cred = bios_pwd::get_bios_password().map_err(|e| e.to_string())?;
        dell::set_battery_charge_setting(cctk, &validated, cred.as_ref())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn apply_thermal_for_current_source(
    thermal_cfg: &config::ThermalConfig,
    cctk_path: Option<&PathBuf>,
) -> Result<(), String> {
    let Some(cctk) = cctk_path else {
        return Err("Dell CCTK not available".into());
    };
    let source = power::power_source_str(&power::query_power_status());
    let profile = match source.as_str() {
        "ac" => Some(thermal_cfg.on_ac),
        "battery" => Some(thermal_cfg.on_battery),
        _ => None,
    };
    if let Some(p) = profile {
        let cred = bios_pwd::get_bios_password().map_err(|e| e.to_string())?;
        dell::set_thermal_profile(cctk, p, cred.as_ref()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// System tray
// ---------------------------------------------------------------------------

fn build_tray_menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<Wry>> {
    let active = power::get_active_scheme();
    let ps = power::query_power_status();
    let mut builder = MenuBuilder::new(app);
    let active_actions = app
        .state::<AppState>()
        .tray_actions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();

    let source_text = match ps.as_ref().map(|s| s.ACLineStatus) {
        Some(0) => {
            let pct = power::battery_percent(&ps);
            if pct <= 100 {
                format!("Battery ({}%)", pct)
            } else {
                "Battery".into()
            }
        }
        Some(1) => "AC Power".into(),
        _ => "Unknown".into(),
    };
    let info = MenuItemBuilder::with_id("_source", &format!("\u{26A1} {}", source_text))
        .enabled(false)
        .build(app)?;
    builder = builder.item(&info);
    builder = builder.separator();

    let power_label = format!("Power Plan: {}", active.label());
    let mut power_submenu = SubmenuBuilder::new(app, power_label);
    for scheme in &PowerScheme::ALL {
        let label = if *scheme == active {
            format!("\u{25CF} {}", scheme.label())
        } else {
            format!("  {}", scheme.label())
        };
        let item = IconMenuItemBuilder::new(&label)
            .id(scheme.id())
            .icon(scheme.icon())
            .enabled(!active_actions.contains(&TrayAction::PowerScheme))
            .build(app)?;
        power_submenu = power_submenu.item(&item);
    }
    builder = builder.item(&power_submenu.build()?);

    let dell_state = app
        .state::<AppState>()
        .dell_state
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    if dell_state.available {
        let thermal_label = match dell_state.current_profile {
            Some(p) => format!("\u{1F321}\u{FE0F} Thermal: {}", p.label()),
            None => "\u{1F321}\u{FE0F} Thermal: Unknown".into(),
        };
        let mut thermal_submenu = SubmenuBuilder::new(app, thermal_label);

        for profile in &dell_state.supported_profiles {
            let prefix = if dell_state.current_profile == Some(*profile) {
                "\u{25CF} "
            } else {
                "  "
            };
            let item =
                MenuItemBuilder::with_id(profile.id(), &format!("{}{}", prefix, profile.label()))
                    .enabled(!active_actions.contains(&TrayAction::ThermalProfile))
                    .build(app)?;
            thermal_submenu = thermal_submenu.item(&item);
        }
        builder = builder.item(&thermal_submenu.build()?);

        if !dell_state.supported_battery_modes.is_empty() {
            let battery_label = match &dell_state.current_battery_mode {
                Some(setting) => format!("Battery Charge: {}", setting.mode.label()),
                None => "Battery Charge: Unknown".into(),
            };
            let mut battery_submenu = SubmenuBuilder::new(app, battery_label);

            for mode in &dell_state.supported_battery_modes {
                if *mode == BatteryChargeMode::Custom {
                    continue;
                }
                let current_mode = dell_state
                    .current_battery_mode
                    .as_ref()
                    .map(|setting| setting.mode);
                let prefix = if current_mode == Some(*mode) {
                    "\u{25CF} "
                } else {
                    "  "
                };
                let item =
                    MenuItemBuilder::with_id(mode.id(), &format!("{}{}", prefix, mode.label()))
                        .enabled(!active_actions.contains(&TrayAction::BatteryCharge))
                        .build(app)?;
                battery_submenu = battery_submenu.item(&item);
            }
            builder = builder.item(&battery_submenu.build()?);
        }
    }

    builder = builder.separator();
    builder = builder.item(&MenuItemBuilder::with_id("settings", "Settings").build(app)?);
    builder = builder.item(&MenuItemBuilder::with_id("quit", "Quit").build(app)?);
    builder.build()
}

fn update_tray(app: &AppHandle) {
    if let Ok(menu) = build_tray_menu(app) {
        if let Some(tray) = app.tray_by_id("power-tray") {
            let _ = tray.set_menu(Some(menu));

            let active = power::get_active_scheme();
            let _ = tray.set_icon(Some(active.icon()));

            let source = power::power_source_str(&power::query_power_status());
            let label = match source.as_str() {
                "ac" => "AC Power",
                "battery" => "Battery",
                _ => "Unknown",
            };
            let _ = tray.set_tooltip(Some(&format!("{} \u{2014} {}", active.label(), label)));
        }
    }
}

fn handle_tray_selection(app: AppHandle, id: &str) {
    if let Some(scheme) = PowerScheme::from_id(id) {
        if power::set_active_scheme(scheme) {
            update_tray(&app);
            let _ = app.emit("status-changed", power::current_status());
        }
        return;
    }

    let state = app.state::<AppState>();
    let cctk_path = state
        .cctk_path
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let Some(cctk) = cctk_path else { return };
    let credential = match bios_pwd::get_bios_password() {
        Ok(credential) => credential,
        Err(error) => {
            let _ = app.emit("cctk-action-failed", error.to_string());
            return;
        }
    };

    if let Some(profile) = ThermalProfile::from_id(id) {
        match dell::set_thermal_profile(&cctk, profile, credential.as_ref()) {
            Ok(()) => {
                let mut dell_state = state
                    .dell_state
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                match dell::get_current_modes(&cctk) {
                    Ok((thermal, battery)) => {
                        dell_state.current_profile = Some(thermal);
                        dell_state.current_battery_mode = Some(battery);
                    }
                    Err(_) => dell_state.current_profile = Some(profile),
                }
                *state.dell_state.write().unwrap_or_else(|e| e.into_inner()) = dell_state.clone();
                let _ = app.emit("thermal-changed", dell_state);
            }
            Err(error) => {
                open_password_prompt_for_error(&app, &error);
                let _ = app.emit("cctk-action-failed", error.to_string());
            }
        }
        return;
    }

    if let Some(mode) = BatteryChargeMode::from_id(id) {
        let Ok(setting) = BatteryChargeSetting::new(mode, 50, 85) else {
            return;
        };
        match dell::set_battery_charge_setting(&cctk, &setting, credential.as_ref()) {
            Ok(()) => {
                let mut dell_state = state
                    .dell_state
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                match dell::get_current_modes(&cctk) {
                    Ok((thermal, battery)) => {
                        dell_state.current_profile = Some(thermal);
                        dell_state.current_battery_mode = Some(battery);
                    }
                    Err(_) => dell_state.current_battery_mode = Some(setting.clone()),
                }
                *state.dell_state.write().unwrap_or_else(|e| e.into_inner()) = dell_state.clone();
                let _ = app.emit("battery-setting-changed", dell_state);
            }
            Err(error) => {
                open_password_prompt_for_error(&app, &error);
                let _ = app.emit("cctk-action-failed", error.to_string());
            }
        }
    }
}

fn open_password_prompt_for_error(app: &AppHandle, error: &dell::DellError) {
    let kind = match error {
        dell::DellError::SetupPasswordError(_) => Some(BiosPasswordKind::Setup),
        dell::DellError::SystemPasswordError(_) => Some(BiosPasswordKind::System),
        dell::DellError::CommandFailed(_) | dell::DellError::ParseError(_) => None,
    };
    if let Some(kind) = kind {
        let _ = open_bios_password_prompt(app, kind);
    } else if let dell::DellError::CommandFailed(message) = error {
        open_cctk_warning(app, message.clone());
    }
}

fn open_cctk_warning(app: &AppHandle, message: String) {
    *app.state::<AppState>()
        .cctk_warning
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(message);

    if let Some(window) = app.get_webview_window("cctk-warning") {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }

    let _ = WebviewWindowBuilder::new(
        app,
        "cctk-warning",
        WebviewUrl::App("index.html?view=cctk-warning".into()),
    )
    .title("Dell Command Configure Warning")
    .inner_size(460.0, 300.0)
    .min_inner_size(340.0, 240.0)
    .resizable(true)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .transparent(false)
    .center()
    .build();
}

fn open_settings(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.set_focus();
    } else {
        let _ = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
            .title("Power Control")
            .inner_size(520.0, 720.0)
            .min_inner_size(380.0, 480.0)
            .resizable(true)
            .maximizable(false)
            .minimizable(false)
            .decorations(false)
            .transparent(false)
            .center()
            .build();
    }
}

fn open_bios_password_prompt(app: &AppHandle, kind: BiosPasswordKind) -> Result<(), String> {
    let kind = match kind {
        BiosPasswordKind::Setup => "setup",
        BiosPasswordKind::System => "system",
    };

    if let Some(window) = app.get_webview_window("bios-password") {
        let _ = app.emit("bios-password-requested", kind);
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    WebviewWindowBuilder::new(
        app,
        "bios-password",
        WebviewUrl::App(format!("index.html?view=bios-password&kind={kind}").into()),
    )
    .title("BIOS Password Required")
    .inner_size(400.0, 320.0)
    .min_inner_size(320.0, 260.0)
    .resizable(true)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .transparent(false)
    .center()
    .build()
    .map_err(|e| e.to_string())?;
    let _ = app.emit("bios-password-requested", kind);
    Ok(())
}

// ---------------------------------------------------------------------------
// Event-driven power monitor
// ---------------------------------------------------------------------------

fn start_power_monitor(app: AppHandle) {
    let rx = monitor::start_power_event_monitor();

    std::thread::spawn(move || {
        while let Ok(event) = rx.recv() {
            let state = app.state::<AppState>();
            let cfg = match state.config.lock() {
                Ok(c) => c.clone(),
                Err(_) => continue,
            };

            if cfg.power.auto_switch {
                let scheme = match event {
                    PowerEvent::AcConnected => Some(cfg.power.on_ac),
                    PowerEvent::BatteryActive => Some(cfg.power.on_battery),
                    PowerEvent::ShortTermPower => None,
                };
                if let Some(s) = scheme {
                    power::set_active_scheme(s);
                }
            }

            let cctk_snapshot = state
                .cctk_path
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone();

            if let Some(ref battery) = cfg.battery {
                if battery.auto_switch {
                    let setting = match event {
                        PowerEvent::AcConnected => Some(&battery.on_ac),
                        PowerEvent::BatteryActive => Some(&battery.on_battery),
                        PowerEvent::ShortTermPower => None,
                    };
                    if let (Some(s), Some(ref cctk)) = (setting, &cctk_snapshot) {
                        if let Ok(validated) =
                            BatteryChargeSetting::new(s.mode, s.custom_start, s.custom_stop)
                        {
                            if let Ok(cred) = bios_pwd::get_bios_password() {
                                if dell::set_battery_charge_setting(cctk, &validated, cred.as_ref())
                                    .is_ok()
                                {
                                    let dell_state = refresh_dell_state(&state);
                                    let _ = app.emit("battery-setting-changed", dell_state);
                                }
                            }
                        }
                    }
                }
            }

            if let Some(ref thermal) = cfg.thermal {
                if thermal.auto_switch {
                    let profile = match event {
                        PowerEvent::AcConnected => Some(thermal.on_ac),
                        PowerEvent::BatteryActive => Some(thermal.on_battery),
                        PowerEvent::ShortTermPower => None,
                    };
                    if let Some(p) = profile {
                        if let Some(ref cctk) = cctk_snapshot {
                            match bios_pwd::get_bios_password() {
                                Ok(cred) => {
                                    if let Err(e) =
                                        dell::set_thermal_profile(cctk, p, cred.as_ref())
                                    {
                                        eprintln!("Failed to apply thermal profile on event: {e}");
                                    }
                                }
                                Err(e) => eprintln!("Failed to retrieve BIOS credential: {e}"),
                            }
                        }
                    }
                }
            }

            let dell_state = if cctk_snapshot.is_some() {
                refresh_dell_state(&state)
            } else {
                state
                    .dell_state
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
            };
            update_tray(&app);
            let _ = app.emit("status-changed", power::current_status());
            let _ = app.emit("thermal-changed", dell_state);
            let _ = app.emit(
                "battery-overview-changed",
                battery_info::get_battery_overview(),
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            open_settings(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            TrayIconBuilder::with_id("power-tray")
                .icon(power::loading_icon())
                .tooltip("Power Control — Loading…")
                .show_menu_on_left_click(true)
                .build(app)?;

            let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
            let config_path = config_dir.join("config.json");
            let cfg = config::load_config(&config_path);

            if cfg.power.auto_switch {
                apply_power_for_current_source(&cfg.power);
            }

            let cctk_path = dell::find_cctk();

            if let Some(ref thermal) = cfg.thermal {
                if thermal.auto_switch {
                    if let Err(error) =
                        apply_thermal_for_current_source(thermal, cctk_path.as_ref())
                    {
                        eprintln!("Failed to apply startup thermal profile: {error}");
                    }
                }
            }

            if let Some(ref battery) = cfg.battery {
                if battery.auto_switch {
                    if let Err(error) =
                        apply_battery_for_current_source(battery, cctk_path.as_ref())
                    {
                        eprintln!("Failed to apply startup battery charge setting: {error}");
                    }
                }
            }

            let dell_state = dell::query_dell_state(cctk_path.as_ref());
            app.manage(AppState {
                config: Mutex::new(cfg),
                config_path,
                cctk_path: RwLock::new(cctk_path),
                dell_state: RwLock::new(dell_state),
                tray_actions: Mutex::new(Vec::new()),
                cctk_warning: Mutex::new(None),
            });

            let handle = app.handle().clone();
            let menu = build_tray_menu(&handle)?;

            let active = power::get_active_scheme();
            let tray = app
                .tray_by_id("power-tray")
                .ok_or_else(|| "loading tray was not created".to_string())?;
            tray.set_icon(Some(active.icon()))?;
            tray.set_tooltip(Some(&format!("{} \u{2014} Power Control", active.label())))?;
            tray.set_menu(Some(menu))?;
            tray.on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::DoubleClick { .. } = event {
                    open_settings(tray.app_handle());
                }
            });
            tray.on_menu_event(|app, event| {
                let id = event.id().as_ref();
                match id {
                    "quit" => std::process::exit(0),
                    "settings" => open_settings(app),
                    "_source" | "_thermal" | "_battery_charge" => {}
                    _ => {
                        let state = app.state::<AppState>();
                        let Some(action) = TrayAction::from_menu_id(id) else {
                            return;
                        };
                        let mut running =
                            state.tray_actions.lock().unwrap_or_else(|e| e.into_inner());
                        if running.contains(&action) {
                            return;
                        }
                        running.push(action);
                        drop(running);
                        update_tray(app);
                        let app = app.clone();
                        let id = id.to_owned();
                        std::thread::spawn(move || {
                            handle_tray_selection(app.clone(), &id);
                            app.state::<AppState>()
                                .tray_actions
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .retain(|running| *running != action);
                            update_tray(&app);
                        });
                    }
                }
            });

            start_power_monitor(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            cmd_get_config,
            cmd_save_config,
            cmd_get_status,
            cmd_set_scheme,
            cmd_get_autostart,
            cmd_set_autostart,
            cmd_get_dell_state,
            cmd_refresh_dell_state,
            cmd_install_cctk,
            cmd_set_thermal_profile,
            cmd_set_battery_charge_setting,
            cmd_get_bios_password_status,
            cmd_set_bios_password,
            cmd_clear_bios_password,
            cmd_open_bios_password_prompt,
            cmd_get_cctk_warning,
            cmd_close_settings,
            cmd_get_battery_overview,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| match event {
            tauri::RunEvent::ExitRequested { api, .. } => {
                api.prevent_exit();
            }
            _ => {}
        });
}
