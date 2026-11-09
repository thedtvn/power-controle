use std::rc::Rc;
use gpui::{
    div, hsla, px, App, Div, ParentElement as _, SharedString, Styled as _, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    switch::Switch,
    ActiveTheme as _, Disableable as _, Sizable as _,
};

use super::theme;

/// A clean, elevated card container
pub fn card(title: &'static str, cx: &App) -> Div {
    div()
        .w_full()
        .bg(theme::card_bg(cx))
        .border_1()
        .border_color(theme::card_border(cx))
        .rounded(px(10.0))
        .p(px(16.0))
        .flex()
        .flex_col()
        .gap(px(12.0))
        .shadow_sm()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(cx.theme().muted_foreground)
                        .child(title.to_uppercase()),
                ),
        )
}

/// A compact metric tile showing a label and a prominent value
pub fn metric_tile(
    label: &'static str,
    value: impl Into<SharedString>,
    subtext: Option<&'static str>,
    cx: &App,
) -> Div {
    let mut base = div()
        .flex_1()
        .bg(theme::tile_bg(cx))
        .border_1()
        .border_color(theme::card_border(cx))
        .rounded(px(8.0))
        .p(px(10.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(cx.theme().foreground)
                .child(value.into()),
        );

    if let Some(sub) = subtext {
        base = base.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(sub),
        );
    }
    base
}

/// An option item for segmented picker
#[derive(Clone, Debug)]
pub struct PickerOption {
    pub id: String,
    pub label: String,
    pub disabled: bool,
}

impl PickerOption {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            disabled: false,
        }
    }

    pub fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// An interactive segmented pill picker
pub fn segmented_picker<F>(
    id_prefix: &'static str,
    options: Vec<PickerOption>,
    selected_id: &str,
    disabled: bool,
    on_select: F,
    cx: &App,
) -> Div
where
    F: Fn(&str, &mut Window, &mut App) + 'static,
{
    let on_select = Rc::new(on_select);
    let mut row = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.0))
        .p(px(3.0))
        .bg(theme::pill_bg(cx))
        .rounded(px(8.0));

    for opt in options {
        let is_active = opt.id == selected_id;
        let is_btn_disabled = disabled || opt.disabled;
        let opt_id = opt.id.clone();
        let on_select = on_select.clone();
        let btn_id: SharedString = format!("{}_{}", id_prefix, opt.id).into();

        let mut btn = Button::new(btn_id)
            .label(opt.label)
            .small()
            .disabled(is_btn_disabled);

        if is_active {
            btn = btn.primary();
        } else {
            btn = btn.ghost();
        }

        btn = btn.on_click(move |_ev, window, cx| {
            on_select(&opt_id, window, cx);
        });

        row = row.child(btn);
    }

    row
}

/// Row with title, optional description, and a Switch
pub fn switch_row<F>(
    id: &'static str,
    title: &'static str,
    description: Option<&'static str>,
    checked: bool,
    disabled: bool,
    on_toggle: F,
    _cx: &App,
) -> Div
where
    F: Fn(&bool, &mut Window, &mut App) + 'static,
{
    let switch = Switch::new(id)
        .checked(checked)
        .disabled(disabled)
        .on_click(on_toggle);

    let mut left = div().flex().flex_col().gap(px(2.0)).child(
        div()
            .text_sm()
            .font_weight(gpui::FontWeight::MEDIUM)
            .child(title),
    );

    if let Some(desc) = description {
        left = left.child(
            div()
                .text_xs()
                .child(desc),
        );
    }

    div()
        .flex()
        .items_center()
        .justify_between()
        .w_full()
        .py(px(4.0))
        .child(left)
        .child(switch)
}

/// Interactive number stepper for custom charge thresholds (start / stop %)
pub fn threshold_stepper<F>(
    id_prefix: &'static str,
    label: &'static str,
    value: u8,
    min: u8,
    max: u8,
    disabled: bool,
    on_change: F,
    cx: &App,
) -> Div
where
    F: Fn(&u8, &mut Window, &mut App) + 'static,
{
    let on_change = Rc::new(on_change);
    let minus_on_change = on_change.clone();
    let plus_on_change = on_change;

    let can_dec = !disabled && value > min;
    let can_inc = !disabled && value < max;

    let minus_id: SharedString = format!("{id_prefix}_minus").into();
    let plus_id: SharedString = format!("{id_prefix}_plus").into();

    div()
        .flex()
        .items_center()
        .justify_between()
        .bg(theme::tile_bg(cx))
        .border_1()
        .border_color(theme::card_border(cx))
        .rounded(px(6.0))
        .px(px(10.0))
        .py(px(6.0))
        .child(
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(cx.theme().muted_foreground)
                        .child(label),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{min}% \u{2014} {max}%")),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    Button::new(minus_id)
                        .label("\u{2212}")
                        .small()
                        .disabled(!can_dec)
                        .on_click(move |_ev, window, cx| {
                            if value > min {
                                minus_on_change(&(value - 1), window, cx);
                            }
                        }),
                )
                .child(
                    div()
                        .min_w(px(42.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(cx.theme().foreground)
                        .child(format!("{value}%")),
                )
                .child(
                    Button::new(plus_id)
                        .label("+")
                        .small()
                        .disabled(!can_inc)
                        .on_click(move |_ev, window, cx| {
                            if value < max {
                                plus_on_change(&(value + 1), window, cx);
                            }
                        }),
                ),
        )
}

/// Status / Notification banner
pub fn status_banner(
    variant: &'static str, // "info", "warn", "error", "success"
    message: impl Into<SharedString>,
    action_label: Option<&'static str>,
    on_action: Option<Box<dyn Fn(&mut Window, &mut App)>>,
    _cx: &App,
) -> Div {
    let (bg, border, fg) = match variant {
        "warn" => (
            hsla(38.0 / 360.0, 0.90, 0.50, 0.15),
            theme::STATUS_WARN,
            theme::STATUS_WARN,
        ),
        "error" => (
            hsla(0.0, 0.85, 0.50, 0.15),
            theme::STATUS_ERR,
            theme::STATUS_ERR,
        ),
        "success" => (
            hsla(142.0 / 360.0, 0.70, 0.40, 0.15),
            theme::STATUS_OK,
            theme::STATUS_OK,
        ),
        _ => (
            hsla(207.0 / 360.0, 0.80, 0.50, 0.15),
            theme::ACCENT_BLUE,
            theme::ACCENT_BLUE,
        ),
    };

    let mut row = div()
        .w_full()
        .bg(bg)
        .border_1()
        .border_color(border)
        .rounded(px(8.0))
        .p(px(12.0))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(fg)
                        .child(message.into()),
                ),
        );

    if let (Some(label), Some(action)) = (action_label, on_action) {
        let rc_action = Rc::new(action);
        row = row.child(
            Button::new("banner_action")
                .label(label)
                .small()
                .on_click(move |_ev, window, cx| {
                    rc_action(window, cx);
                }),
        );
    }

    row
}
