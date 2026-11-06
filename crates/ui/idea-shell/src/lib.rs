//! Desktop shell, window layout, menus, focus, and overlays; no concrete feature imports.
//!
//! UI-001 intentionally exposes only a minimal startup window. Workbench, Dock, theme tokens,
//! feature panels, and Git behavior belong to later tracked tasks.

use gpui_kit::{
    AnyWindowHandle, App, Context, IntoElement, Render, TitlebarOptions, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};

const APP_NAME: &str = "IdeaGit";
const APP_ID: &str = "com.ideagit.desktop";

/// Opens the minimal IdeaGit application window for the UI-001 startup spike.
///
/// The shell does not initialize GPUI-Kit itself; the desktop composition root must call
/// [`gpui_kit::init`] before this function.
pub fn open_main_window(cx: &mut App) -> gpui_kit::Result<AnyWindowHandle> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(900.0), px(600.0)), cx)),
        titlebar: Some(TitlebarOptions {
            title: Some(APP_NAME.into()),
            ..Default::default()
        }),
        app_id: Some(APP_ID.to_owned()),
        window_min_size: Some(size(px(640.0), px(420.0))),
        ..Default::default()
    };

    gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| StartupWindow)).map(|(window, _)| window)
}

/// Minimal render root used only to prove GPUI-Kit native window startup.
struct StartupWindow;

impl Render for StartupWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(rgb(0x2b2d30))
            .text_color(rgb(0xdfe1e5))
            .child(div().text_xl().child(APP_NAME))
            .child(div().child("GPUI-Kit startup spike"))
    }
}
