pub mod components;
pub mod theme;
pub mod view;

use gpui::{App, WindowHandle};
use gpui_component::Root;
use crate::service::Service;

pub fn open_settings(
    service: Service,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<Root>> {
    view::open_settings(service, cx)
}
