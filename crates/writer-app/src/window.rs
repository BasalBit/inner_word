//! The main window: options (macOS titlebar, minimum size) and a placeholder
//! root view. The real app shell replaces [`Root`].

use gpui::{
    App, AppContext, Bounds, Context, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    Styled, TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions, div,
    prelude::InteractiveElement, px, size,
};
use writer_ui::theme::{self, ActiveTheme};

use crate::CloseWindow;

/// Default content size of a new main window.
pub const DEFAULT_WINDOW_SIZE: (f32, f32) = (1200., 800.);
/// The window cannot be made smaller than this.
pub const MIN_WINDOW_SIZE: (f32, f32) = (640., 480.);

/// Window options for the main window: a transparent titlebar with the title
/// hidden (content draws under it), traffic lights inset to line up with the
/// app's own titlebar band, and a minimum size.
pub fn main_window_options(bounds: Option<WindowBounds>, cx: &App) -> WindowOptions {
    let (width, height) = DEFAULT_WINDOW_SIZE;
    let (min_width, min_height) = MIN_WINDOW_SIZE;
    WindowOptions {
        window_bounds: Some(bounds.unwrap_or_else(|| {
            WindowBounds::Windowed(Bounds::centered(None, size(px(width), px(height)), cx))
        })),
        titlebar: Some(TitlebarOptions {
            // Still used by Mission Control and the Window menu.
            title: Some("The Writer".into()),
            appears_transparent: true,
            traffic_light_position: Some(gpui::point(
                theme::TRAFFIC_LIGHT_X,
                theme::TRAFFIC_LIGHT_Y,
            )),
        }),
        window_min_size: Some(size(px(min_width), px(min_height))),
        focus: true,
        show: true,
        ..Default::default()
    }
}

/// Opens the main window with the placeholder root view.
pub fn open_main_window(cx: &mut App) -> anyhow::Result<WindowHandle<Root>> {
    let options = main_window_options(None, cx);
    cx.open_window(options, |window, cx| {
        theme::observe_system_appearance(window, cx).detach();
        cx.new(|cx| {
            let root = Root::new(cx);
            window.focus(&root.focus_handle);
            root
        })
    })
}

/// Placeholder root view until the app shell exists.
pub struct Root {
    focus_handle: FocusHandle,
}

impl Root {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Focusable for Root {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        div()
            .id("root")
            .key_context("Root")
            .track_focus(&self.focus_handle)
            .on_action(|_: &CloseWindow, window, _| window.remove_window())
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.canvas)
            .text_color(colors.ink)
            .font_family(theme.fonts.ui.clone())
            .text_size(theme::UI_TEXT_BASE)
            .child(
                // Titlebar band: content draws under the transparent titlebar.
                div()
                    .h(theme::TITLEBAR_HEIGHT)
                    .flex_none()
                    .border_b_1()
                    .border_color(colors.hairline)
                    .bg(colors.sidebar),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(theme::SPACE_2)
                    .child(
                        div()
                            .font_family(theme.fonts.document.clone())
                            .text_size(px(28.))
                            .child("The Writer"),
                    )
                    .child(
                        div()
                            .text_color(colors.muted)
                            .child("No document open"),
                    ),
            )
    }
}
