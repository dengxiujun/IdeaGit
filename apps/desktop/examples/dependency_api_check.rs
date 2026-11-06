//! Compile-only API probe for INFRA-002; does not initialize a platform or window.

use gpui_kit::{App, Application, Render, WindowOptions};

fn requires_render<T: Render>() {}

fn main() {
    let _: fn() -> Application = gpui_kit::application;
    let _: fn(&mut App) = gpui_kit::init;
    let _ = WindowOptions::default();

    requires_render::<gpui_kit::base::dock::DockArea>();
    requires_render::<gpui_kit::base::input::EditorState>();
    let _ = std::mem::size_of::<gpui_kit::base::VirtualList>();
    let _ = std::mem::size_of::<gpui_kit::component::input::Editor>();
    let _ = std::mem::size_of::<gpui_kit::component::button::Button>();
}
