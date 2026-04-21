use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use std::{mem, ptr};

use serde::{Deserialize, Serialize};
use tauri::{
    image::Image,
    menu::{IconMenuItemBuilder, MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, Wry,
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_autostart::ManagerExt;
use windows::core::GUID;
use windows::Win32::Foundation::HLOCAL;
use windows::Win32::System::Power::{
    GetSystemPowerStatus, PowerGetActiveScheme, PowerSetActiveScheme, SYSTEM_POWER_STATUS,
};

// ---------------------------------------------------------------------------
// Power scheme GUIDs (well-known)
// ---------------------------------------------------------------------------

const GUID_BALANCED: GUID = GUID::from_u128(0x381b4222_f694_41f0_9685_ff5bb260df2e);
const GUID_HIGH_PERF: GUID = GUID::from_u128(0x8c5e7fda_e8bf_4a96_9a85_a6e23a8c635c);
const GUID_POWER_SAVER: GUID = GUID::from_u128(0xa1841308_3541_4fab_bc81_f71556f20b4a);

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerScheme {
    Balanced,
    HighPerformance,
    PowerSaver,
}

impl PowerScheme {
    const ALL: [Self; 3] = [Self::Balanced, Self::HighPerformance, Self::PowerSaver];

    fn guid(&self) -> &'static GUID {
        match self {
            Self::Balanced => &GUID_BALANCED,
            Self::HighPerformance => &GUID_HIGH_PERF,
            Self::PowerSaver => &GUID_POWER_SAVER,
        }
    }

    fn label(&self) -> &str {
        match self {
            Self::Balanced => "Balanced",
            Self::HighPerformance => "High Performance",
            Self::PowerSaver => "Power Saver",
        }
    }

    fn id(&self) -> &str {
        match self {
            Self::Balanced => "balanced",
            Self::HighPerformance => "high_performance",
            Self::PowerSaver => "power_saver",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        match id {
            "balanced" => Some(Self::Balanced),
            "high_performance" => Some(Self::HighPerformance),
            "power_saver" => Some(Self::PowerSaver),
            _ => None,
        }
    }

    fn from_guid(guid: &GUID) -> Option<Self> {
        Self::ALL.iter().find(|s| s.guid() == guid).copied()
    }

    fn icon(&self) -> Image<'static> {
        let bytes: &[u8] = match self {
            Self::Balanced => include_bytes!("../icons/tray_balanced.png"),
            Self::HighPerformance => include_bytes!("../icons/tray_high_performance.png"),
            Self::PowerSaver => include_bytes!("../icons/tray_power_saver.png"),
        };
        Image::from_bytes(bytes).expect("embedded icon is valid PNG")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub auto_switch: bool,
    pub on_ac: PowerScheme,
    pub on_battery: PowerScheme,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auto_switch: false,
            on_ac: PowerScheme::Balanced,
            on_battery: PowerScheme::PowerSaver,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub active_scheme: PowerScheme,
    pub power_source: String,
    pub battery_percent: u8,
}

struct AppState {
    config: Mutex<Config>,
    config_path: PathBuf,
}

// ---------------------------------------------------------------------------
// Windows API wrappers
// ---------------------------------------------------------------------------

fn get_active_scheme() -> PowerScheme {
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

fn set_active_scheme(scheme: PowerScheme) -> bool {
    unsafe { PowerSetActiveScheme(None, Some(scheme.guid())).0 == 0 }
}

fn query_power_status() -> Option<SYSTEM_POWER_STATUS> {
    unsafe {
        let mut status: SYSTEM_POWER_STATUS = mem::zeroed();
        // GetSystemPowerStatus returns Result<()> in windows 0.61
        match GetSystemPowerStatus(&mut status) {
            Ok(()) => Some(status),
            Err(_) => None,
        }
    }
}

fn power_source_str(status: &Option<SYSTEM_POWER_STATUS>) -> String {
    match status.as_ref().map(|s| s.ACLineStatus) {
        Some(0) => "battery".into(),
        Some(1) => "ac".into(),
        _ => "unknown".into(),
    }
}

fn battery_percent(status: &Option<SYSTEM_POWER_STATUS>) -> u8 {
    match status.as_ref().map(|s| s.BatteryLifePercent) {
        Some(p) if p <= 100 => p,
        _ => 255, // unknown / no battery
    }
}

fn current_status() -> Status {
    let ps = query_power_status();
    Status {
        active_scheme: get_active_scheme(),
        power_source: power_source_str(&ps),
        battery_percent: battery_percent(&ps),
    }
}

// ---------------------------------------------------------------------------
// Config persistence
// ---------------------------------------------------------------------------

fn load_config(path: &PathBuf) -> Config {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn persist_config(path: &PathBuf, config: &Config) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn cmd_get_config(state: State<'_, AppState>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn cmd_save_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: Config,
) -> Result<(), String> {
    let mut current = state.config.lock().map_err(|e| e.to_string())?;
    *current = config.clone();
    persist_config(&state.config_path, &config)?;

    // If auto-switch just got enabled, apply immediately for current source
    if config.auto_switch {
        let source = power_source_str(&query_power_status());
        let scheme = match source.as_str() {
            "ac" => Some(config.on_ac),
            "battery" => Some(config.on_battery),
            _ => None,
        };
        if let Some(s) = scheme {
            set_active_scheme(s);
        }
    }

    update_tray(&app);
    Ok(())
}

#[tauri::command]
fn cmd_get_status() -> Status {
    current_status()
}

#[tauri::command]
fn cmd_set_scheme(app: AppHandle, scheme: PowerScheme) -> Result<(), String> {
    if set_active_scheme(scheme) {
        update_tray(&app);
        let _ = app.emit("status-changed", current_status());
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

// ---------------------------------------------------------------------------
// System tray
// ---------------------------------------------------------------------------

fn build_tray_menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<Wry>> {
    let active = get_active_scheme();
    let ps = query_power_status();
    let mut builder = MenuBuilder::new(app);

    // Power source info (disabled, informational)
    let source_text = match ps.as_ref().map(|s| s.ACLineStatus) {
        Some(0) => {
            let pct = battery_percent(&ps);
            if pct <= 100 {
                format!("Battery ({}%)", pct)
            } else {
                "Battery".into()
            }
        }
        Some(1) => "AC Power".into(),
        _ => "Unknown".into(),
    };
    let info = MenuItemBuilder::with_id("_source", &format!("⚡ {}", source_text))
        .enabled(false)
        .build(app)?;
    builder = builder.item(&info);
    builder = builder.separator();

    // Power schemes (icon per mode, bullet prefix for active)
    for scheme in &PowerScheme::ALL {
        let label = if *scheme == active {
            format!("● {}", scheme.label())
        } else {
            format!("  {}", scheme.label())
        };
        let item = IconMenuItemBuilder::new(&label)
            .id(scheme.id())
            .icon(scheme.icon())
            .build(app)?;
        builder = builder.item(&item);
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

            let active = get_active_scheme();
            // Tray icon reflects active power mode
            let _ = tray.set_icon(Some(active.icon()));

            let source = power_source_str(&query_power_status());
            let label = match source.as_str() {
                "ac" => "AC Power",
                "battery" => "Battery",
                _ => "Unknown",
            };
            let _ = tray.set_tooltip(Some(&format!("{} — {}", active.label(), label)));
        }
    }
}

fn open_settings(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.set_focus();
    } else {
        let _ = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
            .title("Power Control")
            .inner_size(460.0, 500.0)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .decorations(false)
            .transparent(true)
            .center()
            .build();
    }
}

// ---------------------------------------------------------------------------
// Power source monitor (background thread)
// ---------------------------------------------------------------------------

fn start_power_monitor(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_source = power_source_str(&query_power_status());

        loop {
            std::thread::sleep(Duration::from_secs(3));

            let ps = query_power_status();
            let source = power_source_str(&ps);

            if source != last_source {
                last_source = source.clone();

                // Auto-switch if enabled
                let config = {
                    let state = app.state::<AppState>();
                    let cfg = state.config.lock().unwrap().clone();
                    cfg
                };

                if config.auto_switch {
                    let scheme = match source.as_str() {
                        "ac" => Some(config.on_ac),
                        "battery" => Some(config.on_battery),
                        _ => None,
                    };
                    if let Some(s) = scheme {
                        set_active_scheme(s);
                    }
                }

                update_tray(&app);
                let _ = app.emit("status-changed", current_status());
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Single-instance MUST be first plugin — on duplicate launch, open settings
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            open_settings(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            // Config
            let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
            let config_path = config_dir.join("config.json");
            let config = load_config(&config_path);

            // Apply auto-switch on startup
            if config.auto_switch {
                let source = power_source_str(&query_power_status());
                let scheme = match source.as_str() {
                    "ac" => Some(config.on_ac),
                    "battery" => Some(config.on_battery),
                    _ => None,
                };
                if let Some(s) = scheme {
                    set_active_scheme(s);
                }
            }

            app.manage(AppState {
                config: Mutex::new(config),
                config_path,
            });

            // Tray
            let handle = app.handle().clone();
            let menu = build_tray_menu(&handle)?;

            let active = get_active_scheme();
            let _tray = TrayIconBuilder::with_id("power-tray")
                .icon(active.icon())
                .tooltip(&format!("{} — Power Control", active.label()))
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| {
                    let id = event.id().as_ref();
                    match id {
                        "quit" => app.exit(0),
                        "settings" => open_settings(app),
                        "_source" => {}
                        _ => {
                            if let Some(scheme) = PowerScheme::from_id(id) {
                                if set_active_scheme(scheme) {
                                    update_tray(app);
                                    let _ = app.emit("status-changed", current_status());
                                }
                            }
                        }
                    }
                })
                .build(app)?;

            // Power monitor
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
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                api.prevent_exit();
            }
        });
}
