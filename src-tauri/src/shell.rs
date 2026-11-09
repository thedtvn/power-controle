use crate::dell::{BatteryChargeMode, BatteryChargeSetting, ThermalProfile};
use crate::instance::Instance;
use crate::power::PowerScheme;
use crate::service::{Action, Service, Snapshot};
use anyhow::Result;
use gpui::{
    div, px, size, App, AppContext as _, Application, Bounds, Context, IntoElement, Render, Window,
    WindowBounds, WindowHandle, WindowKind, WindowOptions,
};
use gpui_component::Root;
use std::time::Duration;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// Invisible keep-alive window. GPUI 0.2.2 on Windows posts WM_QUIT when its
/// last window closes, which would kill this tray app whenever Settings is
/// closed. This never-shown tool window (no taskbar button) keeps it alive.
struct KeepAlive;
impl Render for KeepAlive {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn open_keep_alive(cx: &mut App) -> Result<()> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            gpui::point(px(-32000.), px(-32000.)),
            size(px(1.), px(1.)),
        ))),
        titlebar: None,
        focus: false,
        show: false,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    cx.open_window(options, |_, cx| cx.new(|_| KeepAlive))?;
    Ok(())
}

fn icon(bytes: &[u8]) -> Result<Icon> {
    let image = image::load_from_memory(bytes)?.into_rgba8();
    let (width, height) = image.dimensions();
    Ok(Icon::from_rgba(image.into_raw(), width, height)?)
}

fn menu(snapshot: &Snapshot) -> Result<Menu> {
    let menu = Menu::new();
    let title = MenuItem::new(
        format!(
            "{} · {}% · {}",
            snapshot.status.active_scheme.label(),
            snapshot.status.battery_percent,
            snapshot.status.power_source
        ),
        false,
        None,
    );
    menu.append(&title)?;
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

struct Shell {
    service: Service,
    instance: Instance,
    tray: TrayIcon,
    settings: Option<WindowHandle<Root>>,
    revision: u64,
    prompt_visible: bool,
}
impl Shell {
    fn open_settings(&mut self, cx: &mut App) {
        if let Some(handle) = self.settings {
            if handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
            {
                cx.activate(true);
                return;
            }
        }
        match crate::ui::open_settings(self.service.clone(), cx) {
            Ok(handle) => {
                self.settings = Some(handle);
                cx.activate(true);
            }
            Err(error) => eprintln!("Could not open Settings: {error}"),
        }
    }
    fn tick(&mut self, cx: &mut App) {
        if self.instance.take_activation() {
            self.open_settings(cx);
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let id = event.id.as_ref();
            match id {
                "settings" => self.open_settings(cx),
                "quit" => {
                    cx.quit();
                    return;
                }
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
        let snapshot = self.service.snapshot();
        if snapshot.revision != self.revision {
            self.revision = snapshot.revision;
            if let Ok(menu) = menu(&snapshot) {
                self.tray.set_menu(Some(Box::new(menu)));
            }
            if let Ok(icon) = icon(snapshot.status.active_scheme.icon()) {
                let _ = self.tray.set_icon(Some(icon));
            }
            let _ = self.tray.set_tooltip(Some(format!(
                "Power Control — {} · {}%",
                snapshot.status.active_scheme.label(),
                snapshot.status.battery_percent
            )));
        }
        let prompt = snapshot.password_prompt.is_some() || snapshot.warning.is_some();
        if prompt && !self.prompt_visible {
            self.open_settings(cx);
        }
        self.prompt_visible = prompt;
    }
}

pub fn run() -> Result<()> {
    let autostart = std::env::args().any(|arg| arg == "--autostart");
    // Safe native launch check: no worker, power writes, credential changes, or
    // startup migration. The temporary window closes automatically.
    let smoke_test = std::env::args().any(|arg| arg == "--smoke-test");
    // Release builds need administrator rights for Dell CCTK, but GPUI owns the
    // executable manifest, so elevate at runtime. This runs before taking the
    // single-instance mutex so the elevated copy can acquire it.
    if !cfg!(debug_assertions) && !smoke_test && !crate::elevation::is_elevated() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if crate::elevation::relaunch_elevated(&args) {
            return Ok(());
        }
    }
    let Some(instance) = Instance::acquire(autostart || smoke_test)? else {
        return Ok(());
    };
    let service = Service::new().map_err(anyhow::Error::msg)?;
    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx| {
            gpui_component::init(cx);
            let snapshot = service.snapshot();
            let tray = icon(crate::power::loading_icon()).and_then(|icon| {
                Ok(TrayIconBuilder::new()
                    .with_icon(icon)
                    .with_tooltip("Power Control — Loading…")
                    .with_menu(Box::new(menu(&snapshot)?))
                    .with_menu_on_left_click(true)
                    .build()?)
            });
            let tray = match tray {
                Ok(tray) => tray,
                Err(error) => {
                    eprintln!("Could not create the tray: {error}");
                    cx.quit();
                    return;
                }
            };
            let mut shell = Shell {
                service: service.clone(),
                instance,
                tray,
                settings: None,
                revision: u64::MAX,
                prompt_visible: false,
            };
            // Tray-first: keep the platform alive when Settings closes.
            if let Err(error) = open_keep_alive(cx) {
                eprintln!("Could not create the background window: {error}");
                cx.quit();
                return;
            }
            if smoke_test {
                shell.open_settings(cx);
                if shell.settings.is_none() {
                    panic!("native settings window failed to open during smoke test");
                }
            } else {
                // Scheduled login stays tray-only; a deliberate manual launch
                // immediately exposes the native settings interface.
                if !autostart {
                    shell.open_settings(cx);
                }
                service.start();
            }
            cx.spawn(async move |cx| {
                let started = std::time::Instant::now();
                let mut smoke_closed = false;
                let mut smoke_reopened = false;
                loop {
                    smol::Timer::after(Duration::from_millis(200)).await;
                    if smoke_test && !smoke_closed && started.elapsed() > Duration::from_secs(2) {
                        cx.update(|cx| {
                            shell
                                .settings
                                .unwrap()
                                .update(cx, |_, window, _| window.remove_window())
                                .expect("smoke window close failed");
                        })
                        .expect("app quit after closing Settings instead of staying in tray");
                        smoke_closed = true;
                    }
                    if smoke_test && !smoke_reopened && started.elapsed() > Duration::from_secs(3) {
                        cx.update(|cx| {
                            shell.open_settings(cx);
                            shell
                                .settings
                                .unwrap()
                                .update(cx, |_, _, _| ())
                                .expect("smoke Settings reopen failed");
                        })
                        .expect("tray app did not survive closing Settings");
                        smoke_reopened = true;
                    }
                    if smoke_test && started.elapsed() > Duration::from_secs(5) {
                        let _ = cx.update(|cx| cx.quit());
                        break;
                    }
                    if cx.update(|cx| shell.tick(cx)).is_err() {
                        break;
                    }
                }
            })
            .detach();
        });
    Ok(())
}
