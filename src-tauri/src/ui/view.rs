use std::time::Duration;
use gpui::{
    div, prelude::FluentBuilder as _, px, App, AppContext as _, Bounds, Context, Entity,
    InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowOptions,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    input::{Input, InputState},
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, Root,
};
use zeroize::Zeroizing;

use crate::{
    bios_pwd::BiosPasswordKind,
    config::Config,
    dell::{BatteryChargeMode, ThermalProfile},
    power::PowerScheme,
    service::{Action, Snapshot},
    settings_client::Service,
};

use super::{components, theme};

pub struct SettingsView {
    service: Service,
    snapshot: Snapshot,
    last_revision: u64,
    dirty: bool,
    saving: bool,
    submitted_config: Option<Config>,
    submitted_autostart: Option<bool>,
    draft_config: Config,
    draft_autostart: bool,
    validation_error: Option<String>,
    status_notice: Option<(&'static str, String)>,
    password_input: Entity<InputState>,
    show_password_card: bool,
    active_password_kind: BiosPasswordKind,
}

impl SettingsView {
    pub fn new(service: Service, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let snapshot = service.snapshot();
        let last_revision = snapshot.revision;
        let draft_config = snapshot.config.clone();
        let draft_autostart = snapshot.autostart;
        let active_password_kind = snapshot
            .password_prompt
            .unwrap_or(BiosPasswordKind::Setup);
        let show_password_card = snapshot.password_prompt.is_some();

        let password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Enter BIOS password\u{2026}")
        });

        // Spawn timer loop to poll revision every 250ms using WeakEntity to prevent leaks
        cx.spawn(async move |this, cx| {
            loop {
                smol::Timer::after(Duration::from_millis(250)).await;
                let Some(view_entity) = this.upgrade() else {
                    break;
                };
                let res = view_entity.update(cx, |view, cx| {
                    let snap = view.service.snapshot();
                    if snap.revision != view.last_revision {
                        view.sync_snapshot(&snap);
                        cx.notify();
                    }
                });
                if res.is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            service,
            snapshot,
            last_revision,
            dirty: false,
            saving: false,
            submitted_config: None,
            submitted_autostart: None,
            draft_config,
            draft_autostart,
            validation_error: None,
            status_notice: None,
            password_input,
            show_password_card,
            active_password_kind,
        }
    }

    /// Background snapshot sync preserves dirty draft and tracks pending saves
    fn sync_snapshot(&mut self, snap: &Snapshot) {
        self.last_revision = snap.revision;
        self.snapshot = snap.clone();

        if self.saving {
            // Check if submitted config was applied
            let config_matched = self
                .submitted_config
                .as_ref()
                .map_or(true, |target| &snap.config == target);

            let autostart_matched = self
                .submitted_autostart
                .as_ref()
                .map_or(true, |target| snap.autostart == *target);

            if config_matched && autostart_matched && !snap.busy {
                self.saving = false;
                self.dirty = false;
                self.submitted_config = None;
                self.submitted_autostart = None;
                self.status_notice = Some(("success", "Configuration applied successfully.".into()));
            } else if !snap.busy && (snap.warning.is_some() || snap.message.as_ref().map_or(false, |m| m.contains("fail") || m.contains("err") || m.contains("Error"))) {
                // Operation completed with diagnostic error: stop saving spinner but retain draft
                self.saving = false;
                self.submitted_config = None;
                self.submitted_autostart = None;
                let msg = snap.message.clone().unwrap_or_else(|| "Failed to apply hardware setting.".into());
                self.status_notice = Some(("error", msg));
            }
        } else if !self.dirty {
            self.draft_config = snap.config.clone();
            self.draft_autostart = snap.autostart;
        }

        // If a password prompt appeared from background, show card
        if let Some(kind) = snap.password_prompt {
            self.active_password_kind = kind;
            self.show_password_card = true;
        }
    }

    fn mark_dirty(&mut self, cx: &mut Context<Self>) {
        self.dirty = true;
        self.status_notice = None;
        self.validate();
        cx.notify();
    }

    fn validate(&mut self) -> bool {
        self.validation_error = None;

        if let Some(ref bat) = self.draft_config.battery {
            if bat.on_ac.mode == BatteryChargeMode::Custom {
                if bat.on_ac.custom_start < 50 || bat.on_ac.custom_start > 95 {
                    self.validation_error = Some("AC Custom start must be between 50% and 95%.".into());
                    return false;
                }
                if bat.on_ac.custom_stop < 55 || bat.on_ac.custom_stop > 100 {
                    self.validation_error = Some("AC Custom stop must be between 55% and 100%.".into());
                    return false;
                }
                if bat.on_ac.custom_stop <= bat.on_ac.custom_start
                    || bat.on_ac.custom_stop - bat.on_ac.custom_start < 5
                {
                    self.validation_error =
                        Some("AC Custom stop threshold must be at least 5% higher than start.".into());
                    return false;
                }
            }

            if bat.on_battery.mode == BatteryChargeMode::Custom {
                if bat.on_battery.custom_start < 50 || bat.on_battery.custom_start > 95 {
                    self.validation_error =
                        Some("Battery Custom start must be between 50% and 95%.".into());
                    return false;
                }
                if bat.on_battery.custom_stop < 55 || bat.on_battery.custom_stop > 100 {
                    self.validation_error =
                        Some("Battery Custom stop must be between 55% and 100%.".into());
                    return false;
                }
                if bat.on_battery.custom_stop <= bat.on_battery.custom_start
                    || bat.on_battery.custom_stop - bat.on_battery.custom_start < 5
                {
                    self.validation_error =
                        Some("Battery Custom stop threshold must be at least 5% higher than start.".into());
                    return false;
                }
            }
        }

        true
    }

    fn save_changes(&mut self, cx: &mut Context<Self>) {
        if !self.validate() {
            cx.notify();
            return;
        }

        self.saving = true;
        self.status_notice = None;
        self.submitted_config = Some(self.draft_config.clone());

        // Save autostart if modified
        if self.draft_autostart != self.snapshot.autostart {
            self.submitted_autostart = Some(self.draft_autostart);
            self.service.submit(Action::SetAutostart(self.draft_autostart));
        }

        // Submit SaveConfig
        self.service
            .submit(Action::SaveConfig(self.draft_config.clone()));
        cx.notify();
    }

    fn discard_changes(&mut self, cx: &mut Context<Self>) {
        self.draft_config = self.snapshot.config.clone();
        self.draft_autostart = self.snapshot.autostart;
        self.dirty = false;
        self.saving = false;
        self.submitted_config = None;
        self.submitted_autostart = None;
        self.validation_error = None;
        self.status_notice = None;
        cx.notify();
    }

    fn submit_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.password_input.read(cx).text().to_string();
        if !raw.is_empty() {
            let zeroized = Zeroizing::new(raw);
            self.service
                .submit(Action::SavePassword(self.active_password_kind, zeroized));
            self.password_input.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
        }
        self.show_password_card = false;
        self.service.submit(Action::DismissPasswordPrompt);
        cx.notify();
    }

    fn clear_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.service.submit(Action::ClearPassword);
        self.password_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.show_password_card = false;
        cx.notify();
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_busy = self.snapshot.busy || self.saving;
        let snap = &self.snapshot;

        let mut root = div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::canvas_bg(cx))
            .text_color(cx.theme().foreground);

        // Warning banner if present from Dell CCTK
        if let Some(ref warn_msg) = snap.warning {
            let warn_text = warn_msg.clone();
            let service = self.service.clone();
            root = root.child(
                div().p(px(16.0)).pb(px(0.0)).child(components::status_banner(
                    "warn",
                    format!("Dell CCTK Diagnostic: {warn_text}"),
                    Some("Dismiss"),
                    Some(Box::new(move |_win, _cx| {
                        service.submit(Action::DismissWarning);
                    })),
                    cx,
                )),
            );
        }

        // Operational notice banner (success or error)
        if let Some((kind, ref notice_text)) = self.status_notice {
            root = root.child(
                div().p(px(16.0)).pb(px(0.0)).child(components::status_banner(
                    kind,
                    notice_text.clone(),
                    None,
                    None,
                    cx,
                )),
            );
        }

        // Validation error banner if present
        if let Some(ref err_msg) = self.validation_error {
            root = root.child(
                div().p(px(16.0)).pb(px(0.0)).child(components::status_banner(
                    "error",
                    format!("Validation Error: {err_msg}"),
                    None,
                    None,
                    cx,
                )),
            );
        }

        // Scrollable content area
        let mut content = div()
            .id("settings_content")
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .p(px(20.0))
            .overflow_y_scroll();

        // ---------------------------------------------------------------------
        // 1. Current Status Card
        // ---------------------------------------------------------------------
        let active_plan_label = snap.status.active_scheme.label();
        let power_src_label = if snap.status.power_source == "ac" {
            "AC Power"
        } else {
            "Battery"
        };
        let thermal_label = snap
            .dell
            .current_profile
            .map(|p| p.label())
            .unwrap_or("\u{2014}");

        let status_card = components::card("Current Status", cx)
            .child(
                div()
                    .flex()
                    .gap(px(10.0))
                    .w_full()
                    .child(components::metric_tile("POWER PLAN", active_plan_label, None, cx))
                    .child(components::metric_tile("POWER SOURCE", power_src_label, None, cx))
                    .child(components::metric_tile("THERMAL PROFILE", thermal_label, None, cx))
                    .child(components::metric_tile(
                        "BATTERY LEVEL",
                        format!("{}%", snap.status.battery_percent),
                        None,
                        cx,
                    )),
            );
        content = content.child(status_card);

        // ---------------------------------------------------------------------
        // 2. Battery Overview Card
        // ---------------------------------------------------------------------
        if snap.battery.available {
            let state_str = snap
                .battery
                .state
                .as_ref()
                .map(|s| format!("{s:?}"))
                .unwrap_or_else(|| "\u{2014}".into());
            let rate_str = snap
                .battery
                .charge_rate_mw
                .map(|mw| format!("{:.1} W", (mw as f32) / 1000.0))
                .unwrap_or_else(|| "\u{2014}".into());
            let health_str = snap
                .battery
                .health_percent
                .map(|h| format!("{h}%"))
                .unwrap_or_else(|| "\u{2014}".into());
            let cycles_str = snap
                .battery
                .cycle_count
                .map(|c| c.to_string())
                .unwrap_or_else(|| "\u{2014}".into());
            let voltage_str = snap
                .battery
                .voltage_mv
                .map(|v| format!("{v} mV"))
                .unwrap_or_else(|| "\u{2014}".into());
            let model_str = snap
                .battery
                .model
                .clone()
                .unwrap_or_else(|| "\u{2014}".into());

            let battery_card = components::card("Battery Overview", cx)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex()
                                .gap(px(10.0))
                                .child(components::metric_tile("STATE", state_str, None, cx))
                                .child(components::metric_tile("CHARGE RATE", rate_str, None, cx))
                                .child(components::metric_tile("HEALTH", health_str, None, cx)),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(10.0))
                                .child(components::metric_tile("CYCLE COUNT", cycles_str, None, cx))
                                .child(components::metric_tile("VOLTAGE", voltage_str, None, cx))
                                .child(components::metric_tile("MODEL", model_str, None, cx)),
                        ),
                );
            content = content.child(battery_card);
        }

        // ---------------------------------------------------------------------
        // 3. General Settings Card
        // ---------------------------------------------------------------------
        let draft_autostart = self.draft_autostart;
        let general_card = components::card("General", cx)
            .child(
                components::switch_row(
                    "autostart_switch",
                    "Start with Windows",
                    Some("Automatically launch Power Control on Windows login"),
                    draft_autostart,
                    is_busy,
                    cx.listener(|this, val: &bool, _win, cx| {
                        this.draft_autostart = *val;
                        this.mark_dirty(cx);
                    }),
                    cx,
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .py(px(4.0))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child("Appearance"),
                    )
                    .child(
                        Button::new("theme_toggle")
                            .label(if theme::is_dark(cx) {
                                "\u{2600} Light Mode"
                            } else {
                                "\u{263E} Dark Mode"
                            })
                            .small()
                            .on_click(cx.listener(|_this, _ev, _win, cx| {
                                theme::toggle_theme(cx);
                            })),
                    ),
            );
        content = content.child(general_card);

        // ---------------------------------------------------------------------
        // 4. Power Auto-Switch Card
        // ---------------------------------------------------------------------
        let power_auto = self.draft_config.power.auto_switch;
        let power_on_ac = self.draft_config.power.on_ac;
        let power_on_bat = self.draft_config.power.on_battery;

        let power_options = vec![
            components::PickerOption::new("balanced", "Balanced"),
            components::PickerOption::new("high_performance", "High Performance"),
            components::PickerOption::new("power_saver", "Power Saver"),
        ];

        let power_card = components::card("Power Auto-Switch", cx)
            .child(
                components::switch_row(
                    "power_auto_switch",
                    "Enable automatic power scheme switching",
                    Some("Switch power scheme automatically when AC power is connected or disconnected"),
                    power_auto,
                    is_busy,
                    cx.listener(|this, val: &bool, _win, cx| {
                        this.draft_config.power.auto_switch = *val;
                        this.mark_dirty(cx);
                    }),
                    cx,
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(cx.theme().muted_foreground)
                            .child("WHEN CONNECTED TO AC POWER:"),
                    )
                    .child(components::segmented_picker(
                        "power_ac",
                        power_options.clone(),
                        power_on_ac.id(),
                        is_busy || !power_auto,
                        cx.listener(|this, selected_id: &str, _win, cx| {
                            if let Some(s) = PowerScheme::from_id(selected_id) {
                                this.draft_config.power.on_ac = s;
                                this.mark_dirty(cx);
                            }
                        }),
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(cx.theme().muted_foreground)
                            .child("WHEN ON BATTERY POWER:"),
                    )
                    .child(components::segmented_picker(
                        "power_battery",
                        power_options,
                        power_on_bat.id(),
                        is_busy || !power_auto,
                        cx.listener(|this, selected_id: &str, _win, cx| {
                            if let Some(s) = PowerScheme::from_id(selected_id) {
                                this.draft_config.power.on_battery = s;
                                this.mark_dirty(cx);
                            }
                        }),
                        cx,
                    )),
            );
        content = content.child(power_card);

        // ---------------------------------------------------------------------
        // 5. Dell Command | Configure Section
        // ---------------------------------------------------------------------
        if !snap.dell.available {
            let service = self.service.clone();
            let cctk_missing_card = components::card("Dell Hardware Controls", cx)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("Dell Command | Configure (CCTK) is required to manage thermal profiles and battery charging thresholds."),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(8.0))
                                .child(
                                    Button::new("install_cctk_btn")
                                        .label("Install Dell Command | Configure")
                                        .primary()
                                        .disabled(is_busy)
                                        .on_click(move |_ev, _win, _cx| {
                                            service.submit(Action::InstallCctk);
                                        }),
                                )
                                .child(
                                    Button::new("refresh_cctk_btn")
                                        .label("Detect CCTK")
                                        .ghost()
                                        .disabled(is_busy)
                                        .on_click(cx.listener(|this, _ev, _win, _cx| {
                                            this.service.submit(Action::Refresh);
                                        })),
                                ),
                        ),
                );
            content = content.child(cctk_missing_card);
        } else {
            // Dell Thermal Management
            let thermal_cfg = self
                .draft_config
                .thermal
                .clone()
                .unwrap_or_default();
            let thermal_auto = thermal_cfg.auto_switch;

            let supported_profiles = &snap.dell.supported_profiles;
            let thermal_options: Vec<components::PickerOption> = ThermalProfile::ALL
                .iter()
                .map(|p| {
                    let allowed = supported_profiles.is_empty() || supported_profiles.contains(p);
                    components::PickerOption::new(p.id(), p.label()).with_disabled(!allowed)
                })
                .collect();

            let thermal_card = components::card("Thermal Management", cx)
                .child(
                    components::switch_row(
                        "thermal_auto_switch",
                        "Enable thermal auto-switch",
                        Some("Automatically adjust Dell fan and cooling profile based on power source"),
                        thermal_auto,
                        is_busy,
                        cx.listener(|this, val: &bool, _win, cx| {
                            let mut t = this.draft_config.thermal.clone().unwrap_or_default();
                            t.auto_switch = *val;
                            this.draft_config.thermal = Some(t);
                            this.mark_dirty(cx);
                        }),
                        cx,
                    ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(cx.theme().muted_foreground)
                                .child("ON AC POWER:"),
                        )
                        .child(components::segmented_picker(
                            "thermal_ac",
                            thermal_options.clone(),
                            thermal_cfg.on_ac.id(),
                            is_busy || !thermal_auto,
                            cx.listener(|this, selected_id: &str, _win, cx| {
                                if let Some(p) = ThermalProfile::from_id(selected_id) {
                                    let mut t = this.draft_config.thermal.clone().unwrap_or_default();
                                    t.on_ac = p;
                                    this.draft_config.thermal = Some(t);
                                    this.mark_dirty(cx);
                                }
                            }),
                            cx,
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(cx.theme().muted_foreground)
                                .child("ON BATTERY POWER:"),
                        )
                        .child(components::segmented_picker(
                            "thermal_battery",
                            thermal_options,
                            thermal_cfg.on_battery.id(),
                            is_busy || !thermal_auto,
                            cx.listener(|this, selected_id: &str, _win, cx| {
                                if let Some(p) = ThermalProfile::from_id(selected_id) {
                                    let mut t = this.draft_config.thermal.clone().unwrap_or_default();
                                    t.on_battery = p;
                                    this.draft_config.thermal = Some(t);
                                    this.mark_dirty(cx);
                                }
                            }),
                            cx,
                        )),
                );
            content = content.child(thermal_card);

            // Dell Battery Management & Custom Thresholds
            let battery_cfg = self
                .draft_config
                .battery
                .clone()
                .unwrap_or_default();
            let battery_auto = battery_cfg.auto_switch;

            let all_battery_modes = [
                BatteryChargeMode::Standard,
                BatteryChargeMode::Express,
                BatteryChargeMode::PrimarilyAc,
                BatteryChargeMode::Adaptive,
                BatteryChargeMode::Custom,
            ];
            let supported_battery_modes = &snap.dell.supported_battery_modes;
            let battery_options: Vec<components::PickerOption> = all_battery_modes
                .iter()
                .map(|m| {
                    let allowed = supported_battery_modes.is_empty()
                        || supported_battery_modes.contains(m);
                    components::PickerOption::new(m.id(), m.label()).with_disabled(!allowed)
                })
                .collect();

            let mut battery_card = components::card("Battery Charging Management", cx)
                .child(
                    components::switch_row(
                        "battery_auto_switch",
                        "Enable battery charging auto-switch",
                        Some("Automatically set Dell battery charging algorithm and stop thresholds"),
                        battery_auto,
                        is_busy,
                        cx.listener(|this, val: &bool, _win, cx| {
                            let mut b = this.draft_config.battery.clone().unwrap_or_default();
                            b.auto_switch = *val;
                            this.draft_config.battery = Some(b);
                            this.mark_dirty(cx);
                        }),
                        cx,
                    ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(cx.theme().muted_foreground)
                                .child("ON AC POWER:"),
                        )
                        .child(components::segmented_picker(
                            "battery_ac",
                            battery_options.clone(),
                            battery_cfg.on_ac.mode.id(),
                            is_busy || !battery_auto,
                            cx.listener(|this, selected_id: &str, _win, cx| {
                                if let Some(m) = BatteryChargeMode::from_id(selected_id) {
                                    let mut b = this.draft_config.battery.clone().unwrap_or_default();
                                    b.on_ac.mode = m;
                                    this.draft_config.battery = Some(b);
                                    this.mark_dirty(cx);
                                }
                            }),
                            cx,
                        )),
                );

            if battery_cfg.on_ac.mode == BatteryChargeMode::Custom {
                battery_card = battery_card.child(
                    div()
                        .flex()
                        .gap(px(10.0))
                        .child(components::threshold_stepper(
                            "ac_custom_start",
                            "Start Charging At",
                            battery_cfg.on_ac.custom_start,
                            50,
                            95,
                            is_busy || !battery_auto,
                            cx.listener(|this, val: &u8, _win, cx| {
                                let mut b = this.draft_config.battery.clone().unwrap_or_default();
                                b.on_ac.custom_start = *val;
                                this.draft_config.battery = Some(b);
                                this.mark_dirty(cx);
                            }),
                            cx,
                        ))
                        .child(components::threshold_stepper(
                            "ac_custom_stop",
                            "Stop Charging At",
                            battery_cfg.on_ac.custom_stop,
                            55,
                            100,
                            is_busy || !battery_auto,
                            cx.listener(|this, val: &u8, _win, cx| {
                                let mut b = this.draft_config.battery.clone().unwrap_or_default();
                                b.on_ac.custom_stop = *val;
                                this.draft_config.battery = Some(b);
                                this.mark_dirty(cx);
                            }),
                            cx,
                        )),
                );
            }

            battery_card = battery_card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(cx.theme().muted_foreground)
                            .child("ON BATTERY POWER:"),
                    )
                    .child(components::segmented_picker(
                        "battery_battery",
                        battery_options,
                        battery_cfg.on_battery.mode.id(),
                        is_busy || !battery_auto,
                        cx.listener(|this, selected_id: &str, _win, cx| {
                            if let Some(m) = BatteryChargeMode::from_id(selected_id) {
                                let mut b = this.draft_config.battery.clone().unwrap_or_default();
                                b.on_battery.mode = m;
                                this.draft_config.battery = Some(b);
                                this.mark_dirty(cx);
                            }
                        }),
                        cx,
                    )),
            );

            if battery_cfg.on_battery.mode == BatteryChargeMode::Custom {
                battery_card = battery_card.child(
                    div()
                        .flex()
                        .gap(px(10.0))
                        .child(components::threshold_stepper(
                            "bat_custom_start",
                            "Start Charging At",
                            battery_cfg.on_battery.custom_start,
                            50,
                            95,
                            is_busy || !battery_auto,
                            cx.listener(|this, val: &u8, _win, cx| {
                                let mut b = this.draft_config.battery.clone().unwrap_or_default();
                                b.on_battery.custom_start = *val;
                                this.draft_config.battery = Some(b);
                                this.mark_dirty(cx);
                            }),
                            cx,
                        ))
                        .child(components::threshold_stepper(
                            "bat_custom_stop",
                            "Stop Charging At",
                            battery_cfg.on_battery.custom_stop,
                            55,
                            100,
                            is_busy || !battery_auto,
                            cx.listener(|this, val: &u8, _win, cx| {
                                let mut b = this.draft_config.battery.clone().unwrap_or_default();
                                b.on_battery.custom_stop = *val;
                                this.draft_config.battery = Some(b);
                                this.mark_dirty(cx);
                            }),
                            cx,
                        )),
                );
            }

            content = content.child(battery_card);
        }

        // ---------------------------------------------------------------------
        // 6. BIOS Password Security Card
        // ---------------------------------------------------------------------
        let pwd_status = &snap.password;
        let pwd_configured_label = if pwd_status.configured {
            let kind_label = match pwd_status.kind {
                Some(BiosPasswordKind::System) => "System Password Saved",
                Some(BiosPasswordKind::Setup) => "Setup Password Saved",
                None => "Password Configured",
            };
            format!("\u{2713} {kind_label}")
        } else {
            "No Password Configured".to_string()
        };

        let mut bios_card = components::card("BIOS Security", cx)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child("Dell BIOS Password"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(if pwd_status.configured {
                                        theme::STATUS_OK
                                    } else {
                                        cx.theme().muted_foreground
                                    })
                                    .child(pwd_configured_label),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.0))
                            .child(
                                Button::new("toggle_pwd_card")
                                    .label(if self.show_password_card {
                                        "Hide Input"
                                    } else {
                                        "Enter Password"
                                    })
                                    .small()
                                    .on_click(cx.listener(|this, _ev, _win, cx| {
                                        this.show_password_card = !this.show_password_card;
                                        cx.notify();
                                    })),
                            )
                            .when(pwd_status.configured, |el| {
                                el.child(
                                    Button::new("clear_pwd_btn")
                                        .label("Clear Saved")
                                        .danger()
                                        .small()
                                        .disabled(is_busy)
                                        .on_click(cx.listener(|this, _ev, win, cx| {
                                            this.clear_password(win, cx);
                                        })),
                                )
                            }),
                    ),
            );

        if self.show_password_card {
            let kind_name = match self.active_password_kind {
                BiosPasswordKind::System => "System",
                BiosPasswordKind::Setup => "Setup",
            };

            bios_card = bios_card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .p(px(12.0))
                    .bg(theme::tile_bg(cx))
                    .border_1()
                    .border_color(theme::card_border(cx))
                    .rounded(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(cx.theme().foreground)
                                    .child(format!("Enter BIOS {kind_name} Password:")),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap(px(4.0))
                                    .child(
                                        Button::new("kind_setup")
                                            .label("Setup")
                                            .small()
                                            .selected(self.active_password_kind == BiosPasswordKind::Setup)
                                            .on_click(cx.listener(|this, _ev, _win, cx| {
                                                this.active_password_kind = BiosPasswordKind::Setup;
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new("kind_system")
                                            .label("System")
                                            .small()
                                            .selected(self.active_password_kind == BiosPasswordKind::System)
                                            .on_click(cx.listener(|this, _ev, _win, cx| {
                                                this.active_password_kind = BiosPasswordKind::System;
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                    .child(Input::new(&self.password_input))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.0))
                            .child(
                                Button::new("dismiss_pwd_btn")
                                    .label("Cancel")
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(|this, _ev, win, cx| {
                                        this.password_input.update(cx, |input, cx| {
                                            input.set_value("", win, cx);
                                        });
                                        this.show_password_card = false;
                                        this.service.submit(Action::DismissPasswordPrompt);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("save_pwd_btn")
                                    .label("Save Password")
                                    .primary()
                                    .small()
                                    .disabled(is_busy)
                                    .on_click(cx.listener(|this, _ev, win, cx| {
                                        this.submit_password(win, cx);
                                    })),
                            ),
                    ),
            );
        }

        content = content.child(bios_card);
        root = root.child(content);

        // ---------------------------------------------------------------------
        // 7. Bottom Sticky Action Bar
        // ---------------------------------------------------------------------
        let mut status_msg = snap.message.clone();
        if is_busy && status_msg.is_none() {
            status_msg = Some("Applying hardware settings\u{2026}".into());
        }

        let bottom_bar = div()
            .w_full()
            .h(px(56.0))
            .bg(theme::card_bg(cx))
            .border_t_1()
            .border_color(theme::card_border(cx))
            .px(px(20.0))
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        Button::new("refresh_btn")
                            .label("\u{21BB} Refresh")
                            .ghost()
                            .small()
                            .disabled(is_busy)
                            .on_click(cx.listener(|this, _ev, _win, _cx| {
                                this.service.submit(Action::Refresh);
                            })),
                    )
                    .when_some(status_msg, |el, msg| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(theme::ACCENT_BLUE)
                                .child(msg),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .when(self.dirty, |el| {
                        el.child(
                            Button::new("discard_btn")
                                .label("Discard")
                                .ghost()
                                .small()
                                .disabled(is_busy)
                                .on_click(cx.listener(|this, _ev, _win, cx| {
                                    this.discard_changes(cx);
                                })),
                        )
                    })
                    .child(
                        Button::new("save_btn")
                            .label(if self.dirty {
                                "Save Changes \u{2022}"
                            } else {
                                "Save"
                            })
                            .primary()
                            .small()
                            .disabled(is_busy || !self.dirty || self.validation_error.is_some())
                            .on_click(cx.listener(|this, _ev, _win, cx| {
                                this.save_changes(cx);
                            })),
                    ),
            );

        root = root.child(bottom_bar);
        root
    }
}

pub fn open_settings(
    service: Service,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<Root>> {
    let bounds = Bounds::centered(None, gpui::size(px(660.0), px(720.0)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("Power Control Settings".into()),
            ..Default::default()
        }),
        window_min_size: Some(gpui::size(px(520.0), px(440.0))),
        ..Default::default()
    };

    let handle = cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| SettingsView::new(service, window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    })?;

    Ok(handle)
}
