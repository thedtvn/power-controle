pub mod components;
pub mod theme;
pub mod view;

use crate::settings_client::Service;
use gpui::{App, WindowHandle};
use gpui_component::Root;

pub fn open_settings(service: Service, cx: &mut App) -> anyhow::Result<WindowHandle<Root>> {
    view::open_settings(service, cx)
}
