use gpui::{hsla, App, Hsla};
use gpui_component::{ActiveTheme as _, Theme, ThemeMode};

/// Accent blue matching Windows 11 Fluent accent
pub const ACCENT_BLUE: Hsla = Hsla {
    h: 207.0 / 360.0,
    s: 1.0,
    l: 0.45,
    a: 1.0,
};

/// Success green for battery / health
pub const STATUS_OK: Hsla = Hsla {
    h: 142.0 / 360.0,
    s: 0.72,
    l: 0.40,
    a: 1.0,
};

/// Warning amber for alerts
pub const STATUS_WARN: Hsla = Hsla {
    h: 38.0 / 360.0,
    s: 0.92,
    l: 0.50,
    a: 1.0,
};

/// Error red for failures
pub const STATUS_ERR: Hsla = Hsla {
    h: 0.0,
    s: 0.84,
    l: 0.60,
    a: 1.0,
};

pub fn is_dark(cx: &App) -> bool {
    cx.theme().mode.is_dark()
}

pub fn toggle_theme(cx: &mut App) {
    let new_mode = if is_dark(cx) {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    Theme::change(new_mode, None, cx);
}

#[allow(dead_code)]
pub fn set_theme_mode(dark: bool, cx: &mut App) {
    let mode = if dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    Theme::change(mode, None, cx);
}

/// Card background color adapted to current theme
pub fn card_bg(cx: &App) -> Hsla {
    if is_dark(cx) {
        hsla(220.0 / 360.0, 0.13, 0.14, 1.0)
    } else {
        hsla(0.0, 0.0, 1.0, 1.0)
    }
}

/// Card border color
pub fn card_border(cx: &App) -> Hsla {
    if is_dark(cx) {
        hsla(220.0 / 360.0, 0.13, 0.22, 1.0)
    } else {
        hsla(220.0 / 360.0, 0.14, 0.90, 1.0)
    }
}

/// Subtle tile background inside cards
pub fn tile_bg(cx: &App) -> Hsla {
    if is_dark(cx) {
        hsla(220.0 / 360.0, 0.13, 0.18, 0.8)
    } else {
        hsla(220.0 / 360.0, 0.14, 0.96, 1.0)
    }
}

/// Pill unselected background
pub fn pill_bg(cx: &App) -> Hsla {
    if is_dark(cx) {
        hsla(220.0 / 360.0, 0.13, 0.20, 1.0)
    } else {
        hsla(220.0 / 360.0, 0.14, 0.93, 1.0)
    }
}

/// Window canvas background
pub fn canvas_bg(cx: &App) -> Hsla {
    if is_dark(cx) {
        hsla(220.0 / 360.0, 0.14, 0.10, 1.0)
    } else {
        hsla(220.0 / 360.0, 0.14, 0.96, 1.0)
    }
}
