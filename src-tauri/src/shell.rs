//! Process roles.
//!
//! * **Tray** (default): lightweight background process. Owns the tray icon,
//!   the hardware service, and its cache. It runs a plain Win32 message loop
//!   with no GPU renderer and no windows — nothing transparent can appear.
//! * **Settings** (`--settings`): a separate GPUI process. It reads the tray's
//!   cached state over IPC, and fully exits when its window closes, releasing
//!   all UI memory instead of hiding.
use crate::dell::{BatteryChargeMode, BatteryChargeSetting, ThermalProfile};
use crate::instance::Instance;
use crate::power::PowerScheme;
use crate::service::{Action, Service, Snapshot};
use anyhow::Result;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};
use std::time::{Duration, Instant};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, PostQuitMessage, SetTimer, TranslateMessage, MSG,
    WM_TIMER,
};

const SETTINGS_ARG: &str = "--settings";

// ------------------------------------------------------------------ tray ----

fn icon(bytes: &[u8]) -> Result<Icon> {
    let image = image::load_from_memory(bytes)?.into_rgba8();
    let (width, height) = image.dimensions();
    Ok(Icon::from_rgba(image.into_raw(), width, height)?)
}

fn menu(snapshot: &Snapshot) -> Result<Menu> {
    let menu = Menu::new();
    let header = if snapshot.loaded {
        format!(
            "{} · {}% · {}",
            snapshot.status.active_scheme.label(),
            snapshot.status.battery_percent,
            snapshot.status.power_source
        )
    } else {
        format!("{} · loading…", snapshot.status.active_scheme.label())
    };
    menu.append(&MenuItem::new(header, false, None))?;
    menu.append(&PredefinedMenuItem::separator())?;
    for scheme in PowerScheme::ALL {
        menu.append(&CheckMenuItem::with_id(
            scheme.id(),
            scheme.label(),
            !snapshot.busy,
            scheme == snapshot.status.active_scheme,
            None,
        ))?;
    }
    if snapshot.dell.available {
        let thermal = Submenu::new("Thermal profile", true);
        for profile in &snapshot.dell.supported_profiles {
            thermal.append(&CheckMenuItem::with_id(
                profile.id(),
                profile.label(),
                !snapshot.busy,
                snapshot.dell.current_profile == Some(*profile),
                None,
            ))?;
        }
        menu.append(&thermal)?;
        let battery = Submenu::new("Battery charging", true);
        for mode in &snapshot.dell.supported_battery_modes {
            if *mode == BatteryChargeMode::Custom {
                continue;
            }
            battery.append(&CheckMenuItem::with_id(
                mode.id(),
                mode.label(),
                !snapshot.busy,
                snapshot
                    .dell
                    .current_battery_mode
                    .as_ref()
                    .is_some_and(|s| s.mode == *mode),
                None,
            ))?;
        }
        if !snapshot.dell.supported_battery_modes.is_empty() {
            menu.append(&battery)?;
        }
    }
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&MenuItem::with_id("settings", "Settings…", true, None))?;
    menu.append(&MenuItem::with_id("quit", "Quit", true, None))?;
    Ok(menu)
}

struct Tray {
    service: Service,
    instance: Instance,
    tray: TrayIcon,
    settings: Option<Child>,
    revision: u64,
    prompt_visible: bool,
    menu_key: Option<String>,
    icon_scheme: Option<PowerScheme>,
}

impl Tray {
    /// Launches Settings as its own process, or does nothing if it is open.
    /// A second Settings launch focuses the existing window itself.
    fn open_settings(&mut self) {
        if let Some(child) = &mut self.settings {
            if matches!(child.try_wait(), Ok(None)) {
                // Still running: ask it to come to the foreground by
                // launching a second copy, which hands off and exits.
                let _ = spawn_settings();
                return;
            }
        }
        self.settings = spawn_settings().ok();
    }

    fn tick(&mut self) -> bool {
        if self.instance.take_activation() {
            self.open_settings();
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } = event
            {
                self.open_settings();
            }
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let id = event.id.as_ref();
            match id {
                "settings" => self.open_settings(),
                "quit" => return false,
                _ => {
                    if let Some(scheme) = PowerScheme::from_id(id) {
                        self.service.submit(Action::SetScheme(scheme));
                    } else if let Some(profile) = ThermalProfile::from_id(id) {
                        self.service.submit(Action::SetThermal(profile));
                    } else if let Some(mode) = BatteryChargeMode::from_id(id) {
                        if let Ok(setting) = BatteryChargeSetting::new(mode, 50, 85) {
                            self.service.submit(Action::SetBattery(setting));
                        }
                    }
                }
            }
        }
        // Reap a closed Settings process so it never lingers as a zombie.
        if let Some(child) = &mut self.settings {
            if !matches!(child.try_wait(), Ok(None)) {
                self.settings = None;
            }
        }
        let snapshot = self.service.snapshot();
        if snapshot.revision != self.revision {
            self.revision = snapshot.revision;
            self.refresh_tray(&snapshot);
        }
        let prompt = snapshot.password_prompt.is_some() || snapshot.warning.is_some();
        if prompt && !self.prompt_visible {
            self.open_settings();
        }
        self.prompt_visible = prompt;
        true
    }

    /// Rebuilds the native menu only when its visible content changed; the
    /// snapshot revision also bumps for busy/message updates.
    fn refresh_tray(&mut self, snapshot: &Snapshot) {
        let key = format!(
            "{:?}|{}|{}|{}|{}|{:?}|{:?}|{:?}|{:?}",
            snapshot.status.active_scheme,
            snapshot.status.battery_percent,
            snapshot.status.power_source,
            snapshot.busy,
            snapshot.loaded,
            snapshot.dell.current_profile,
            snapshot.dell.current_battery_mode.as_ref().map(|s| s.mode),
            snapshot.dell.supported_profiles,
            snapshot.dell.supported_battery_modes,
        );
        if self.menu_key.as_deref() != Some(key.as_str()) {
            if let Ok(menu) = menu(snapshot) {
                self.tray.set_menu(Some(Box::new(menu)));
            }
            self.menu_key = Some(key);
        }
        if self.icon_scheme != Some(snapshot.status.active_scheme) {
            if let Ok(icon) = icon(snapshot.status.active_scheme.icon()) {
                let _ = self.tray.set_icon(Some(icon));
            }
            self.icon_scheme = Some(snapshot.status.active_scheme);
        }
        let _ = self.tray.set_tooltip(Some(format!(
            "Power Control — {} · {}%",
            snapshot.status.active_scheme.label(),
            snapshot.status.battery_percent
        )));
    }
}

fn spawn_settings() -> std::io::Result<Child> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new(std::env::current_exe()?)
        .arg(SETTINGS_ARG)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
}

fn run_tray(autostart: bool, smoke_test: bool) -> Result<()> {
    let Some(instance) = Instance::acquire(autostart || smoke_test)? else {
        return Ok(());
    };
    let service = Service::new().map_err(anyhow::Error::msg)?;
    // The pipe must be listening before Settings can be opened.
    crate::ipc::serve(service.clone());
    let snapshot = service.snapshot();
    let tray = TrayIconBuilder::new()
        .with_icon(icon(crate::power::loading_icon())?)
        .with_tooltip("Power Control — Loading…")
        .with_menu(Box::new(menu(&snapshot)?))
        .with_menu_on_left_click(true)
        .build()?;
    let mut state = Tray {
        service: service.clone(),
        instance,
        tray,
        settings: None,
        revision: u64::MAX,
        prompt_visible: false,
        menu_key: None,
        icon_scheme: None,
    };
    if !smoke_test {
        // Load all hardware state once, in the background; the tray already
        // shows the cached state from the last run.
        service.start();
    }
    // Scheduled login stays tray-only; a deliberate manual launch opens
    // Settings right away.
    if !autostart && !smoke_test {
        state.open_settings();
    }

    // Plain Win32 loop: tray-icon/muda deliver clicks through window messages,
    // and a 150 ms timer polls cached state. No GPU, no hidden windows.
    let started = Instant::now();
    unsafe {
        let timer = SetTimer(None, 0, 150, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_TIMER && msg.hwnd.is_invalid() {
                let keep = state.tick();
                if !keep || (smoke_test && started.elapsed() > Duration::from_secs(2)) {
                    PostQuitMessage(0);
                }
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = KillTimer(None, timer);
    }
    if let Some(mut child) = state.settings.take() {
        let _ = child.kill();
    }
    Ok(())
}

// -------------------------------------------------------------- settings ----

fn run_settings(smoke_test: bool) -> Result<()> {
    use gpui::Application;

    // Only one Settings window: a second launch focuses the first and exits.
    let Some(_guard) = crate::instance::SettingsGuard::acquire()? else {
        return Ok(());
    };
    // Snapshot comes from the tray's cache: no hardware queries here, so the
    // window opens with complete data immediately.
    let service = crate::settings_client::Service::connect()?;
    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx| {
            gpui_component::init(cx);
            let handle = match crate::ui::open_settings(service, cx) {
                Ok(handle) => handle,
                Err(error) => {
                    eprintln!("Could not open Settings: {error}");
                    cx.quit();
                    return;
                }
            };
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            cx.activate(true);
            // GPUI quits the process when its last window closes, which is
            // exactly the desired "close, don't hide" behavior here.
            let guard_events = crate::instance::SettingsGuard::focus_requests();
            cx.spawn(async move |cx| {
                let started = Instant::now();
                loop {
                    smol::Timer::after(Duration::from_millis(200)).await;
                    if smoke_test && started.elapsed() > Duration::from_secs(3) {
                        let _ = cx.update(|cx| cx.quit());
                        break;
                    }
                    if guard_events.take() {
                        let focused = cx.update(|cx| {
                            handle
                                .update(cx, |_, window, _| window.activate_window())
                                .is_ok()
                        });
                        if !matches!(focused, Ok(true)) {
                            break;
                        }
                    }
                }
            })
            .detach();
        });
    Ok(())
}

// ------------------------------------------------------------------ entry ----

pub fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let autostart = args.iter().any(|a| a == "--autostart");
    let smoke_test = args.iter().any(|a| a == "--smoke-test");
    let settings = args.iter().any(|a| a == SETTINGS_ARG);

    if settings {
        // Elevation is not needed: all privileged work happens in the tray.
        return run_settings(smoke_test);
    }
    // Release builds need administrator rights for Dell CCTK. GPUI owns the
    // executable manifest, so elevate at runtime before taking the mutex.
    if !cfg!(debug_assertions) && !smoke_test && !crate::elevation::is_elevated() {
        if crate::elevation::relaunch_elevated(&args) {
            return Ok(());
        }
    }
    run_tray(autostart, smoke_test)
}
