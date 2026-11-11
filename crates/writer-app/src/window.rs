//! The windows of the app: the main window (options, bounds restored
//! between runs) and the message window shown when the library cannot be
//! opened.

use gpui::{
    App, AppContext, Bounds, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement,
    Pixels, Render, Styled, TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions,
    div, point, prelude::*, px, size,
};
use serde::{Deserialize, Serialize};
use writer_library::Library;
use writer_ui::{
    Button, Icon, IconName, KeyHint,
    theme::{self, ActiveTheme},
};

use crate::{actions::Quit, settings::keys, workspace::Workspace};

/// Default content size of a new main window.
pub const DEFAULT_WINDOW_SIZE: (f32, f32) = (1200., 800.);
/// The window cannot be made smaller than this.
pub const MIN_WINDOW_SIZE: (f32, f32) = (640., 480.);

/// The main window's bounds as stored in the settings
/// ([`keys::WINDOW_BOUNDS`]): screen points, origin at the top left of the
/// main display.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredBounds {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
    /// `"windowed"`, `"maximized"` or `"fullscreen"`; the rectangle is then
    /// the one to restore to.
    #[serde(default)]
    pub mode: StoredWindowMode,
}

/// How the window was shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StoredWindowMode {
    /// A normal window.
    #[default]
    Windowed,
    /// Zoomed.
    Maximized,
    /// Full screen.
    Fullscreen,
}

impl StoredBounds {
    /// Converts GPUI's window bounds.
    pub fn from_window_bounds(bounds: WindowBounds) -> Self {
        let (mode, rect) = match bounds {
            WindowBounds::Windowed(rect) => (StoredWindowMode::Windowed, rect),
            WindowBounds::Maximized(rect) => (StoredWindowMode::Maximized, rect),
            WindowBounds::Fullscreen(rect) => (StoredWindowMode::Fullscreen, rect),
        };
        Self {
            x: rect.origin.x.into(),
            y: rect.origin.y.into(),
            width: rect.size.width.into(),
            height: rect.size.height.into(),
            mode,
        }
    }

    /// The rectangle.
    pub fn rect(&self) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(self.x), px(self.y)),
            size: size(px(self.width), px(self.height)),
        }
    }

    /// Back to GPUI's window bounds.
    pub fn to_window_bounds(&self) -> WindowBounds {
        match self.mode {
            StoredWindowMode::Windowed => WindowBounds::Windowed(self.rect()),
            StoredWindowMode::Maximized => WindowBounds::Maximized(self.rect()),
            StoredWindowMode::Fullscreen => WindowBounds::Fullscreen(self.rect()),
        }
    }

    /// Whether these bounds can be used: finite numbers, at least the
    /// minimum size, and a good part of the titlebar on one of `displays`
    /// (a display may have been unplugged since the bounds were stored).
    pub fn is_usable(&self, displays: &[Bounds<Pixels>]) -> bool {
        let finite = [self.x, self.y, self.width, self.height]
            .iter()
            .all(|value| value.is_finite());
        if !finite || self.width < MIN_WINDOW_SIZE.0 || self.height < MIN_WINDOW_SIZE.1 {
            return false;
        }
        // The strip one can grab to move the window.
        let grab = Bounds {
            origin: point(px(self.x + 80.), px(self.y)),
            size: size(px((self.width - 160.).max(80.)), px(30.)),
        };
        displays.is_empty()
            || displays.iter().any(|display| {
                let overlap = display.intersect(&grab);
                f32::from(overlap.size.width) >= 80. && f32::from(overlap.size.height) >= 20.
            })
    }
}

/// The stored bounds of the main window, when there are usable ones.
pub fn stored_window_bounds(library: &Library, cx: &App) -> Option<WindowBounds> {
    let stored: StoredBounds = library.setting(keys::WINDOW_BOUNDS).ok().flatten()?;
    let displays: Vec<Bounds<Pixels>> = cx
        .displays()
        .iter()
        .map(|display| display.bounds())
        .collect();
    stored
        .is_usable(&displays)
        .then(|| stored.to_window_bounds())
}

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
            traffic_light_position: Some(point(theme::TRAFFIC_LIGHT_X, theme::TRAFFIC_LIGHT_Y)),
        }),
        window_min_size: Some(size(px(min_width), px(min_height))),
        focus: true,
        show: true,
        ..Default::default()
    }
}

/// Opens the main window over `library`, at the bounds it had last time.
pub fn open_main_window(
    library: Entity<Library>,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<Workspace>> {
    let bounds = stored_window_bounds(library.read(cx), cx);
    let options = main_window_options(bounds, cx);
    cx.open_window(options, |window, cx| {
        theme::observe_system_appearance(window, cx).detach();
        cx.new(|cx| Workspace::new(library, window, cx))
    })
}

/// The view of the message window: what went wrong, and a way out.
pub struct ErrorView {
    title: String,
    message: String,
    detail: String,
    focus_handle: FocusHandle,
}

impl ErrorView {
    /// A message view.
    pub fn new(
        title: impl Into<String>,
        message: impl Into<String>,
        detail: impl Into<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            detail: detail.into(),
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Focusable for ErrorView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ErrorView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        div()
            .id("error-window")
            .key_context("ErrorWindow")
            .track_focus(&self.focus_handle)
            .on_action(|_: &crate::actions::CloseWindow, _, cx| cx.quit())
            .on_action(|_: &writer_ui::button::Press, _, cx| cx.quit())
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.canvas)
            .text_color(colors.ink)
            .font_family(theme.fonts.ui.clone())
            .text_size(theme::UI_TEXT_BASE)
            .line_height(px(19.))
            .child(div().h(theme::TITLEBAR_HEIGHT).flex_none())
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px(px(40.))
                    .pb(px(28.))
                    .flex()
                    .flex_col()
                    .gap(theme::SPACE_3)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(theme::SPACE_2)
                            .child(Icon::new(IconName::Alert).color(colors.danger))
                            .child(
                                div()
                                    .font_family(theme.fonts.document.clone())
                                    .text_size(px(22.))
                                    .line_height(px(28.))
                                    .child(self.title.clone()),
                            ),
                    )
                    .child(div().child(self.message.clone()))
                    .child(
                        div()
                            .id("error-detail")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .p(theme::SPACE_3)
                            .rounded(theme::RADIUS_MD)
                            .border_1()
                            .border_color(colors.hairline)
                            .bg(colors.sidebar)
                            .font_family(theme.fonts.mono.clone())
                            .text_size(theme::UI_TEXT_SM)
                            .text_color(colors.muted)
                            .child(self.detail.clone()),
                    )
                    .child(
                        div().flex().justify_end().child(
                            Button::new("quit", "Quit The Writer")
                                .primary()
                                .key_hint(KeyHint::for_action(&Quit, cx))
                                .on_click(|_, _, cx| cx.quit()),
                        ),
                    ),
            )
    }
}

/// What the message window says when the library cannot be opened.
pub fn library_error_text(error: &writer_library::StoreError) -> (String, String, String) {
    let path = writer_library::database_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "the library file".to_string());
    (
        "The library could not be opened".to_string(),
        "The Writer keeps every document in one library file. It could not open that file, so \
         nothing was changed. Your documents are still in it, and daily backups are in the \
         “backups” folder beside it."
            .to_string(),
        format!("{error}\n\nLibrary file: {path}"),
    )
}

/// Opens a small window that explains an error and offers to quit. Used
/// instead of the main window when the library cannot be opened.
pub fn open_error_window(
    title: impl Into<String>,
    message: impl Into<String>,
    detail: impl Into<String>,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<ErrorView>> {
    let (title, message, detail) = (title.into(), message.into(), detail.into());
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(560.), px(360.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("The Writer".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(theme::TRAFFIC_LIGHT_X, theme::TRAFFIC_LIGHT_Y)),
        }),
        window_min_size: Some(size(px(420.), px(280.))),
        is_resizable: true,
        focus: true,
        show: true,
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        theme::observe_system_appearance(window, cx).detach();
        cx.new(|cx| {
            let view = ErrorView::new(title, message, detail, cx);
            window.focus(&view.focus_handle);
            view
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display() -> Vec<Bounds<Pixels>> {
        vec![Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(1512.), px(982.)),
        }]
    }

    #[test]
    fn bounds_round_trip_through_json() {
        let stored = StoredBounds {
            x: 120.,
            y: 80.,
            width: 1100.,
            height: 760.,
            mode: StoredWindowMode::Maximized,
        };
        let json = serde_json::to_string(&stored).unwrap();
        assert!(json.contains("\"maximized\""));
        let back: StoredBounds = serde_json::from_str(&json).unwrap();
        assert_eq!(back, stored);
        assert_eq!(
            StoredBounds::from_window_bounds(back.to_window_bounds()),
            stored
        );
        // An older value without the mode is a plain window.
        let old: StoredBounds =
            serde_json::from_str(r#"{"x":1,"y":2,"width":800,"height":600}"#).unwrap();
        assert_eq!(old.mode, StoredWindowMode::Windowed);
    }

    #[test]
    fn unusable_bounds_are_rejected() {
        let good = StoredBounds {
            x: 100.,
            y: 60.,
            width: 1000.,
            height: 700.,
            mode: StoredWindowMode::Windowed,
        };
        assert!(good.is_usable(&display()));
        // No display information: trust the numbers.
        assert!(good.is_usable(&[]));
        // Too small.
        assert!(!StoredBounds { width: 300., ..good }.is_usable(&display()));
        // On a display that is no longer there.
        assert!(!StoredBounds { x: 4000., ..good }.is_usable(&display()));
        assert!(!StoredBounds { y: -900., ..good }.is_usable(&display()));
        // Garbage.
        assert!(!StoredBounds { x: f32::NAN, ..good }.is_usable(&display()));
        // Partly off the left edge but still grabbable.
        assert!(StoredBounds { x: -300., ..good }.is_usable(&display()));
    }
}
