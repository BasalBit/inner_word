//! The zoom of the text in the workspace: what the commands do, the item
//! of the status bar and its menu, where the choice is kept.
//!
//! The zoom is the editor's ([`writer_editor::Zoom`], one for flow view
//! and one for page view); [`crate::zoom`] has the commands, their keys
//! and the stored form. This file connects the two:
//!
//! - the commands run on the open document's view and are only there
//!   while a document is open ([`Workspace::zoom_actions`]), so the menu
//!   bar greys them out without one;
//! - the status bar shows the zoom of the view ("125 %"); a click or a
//!   right-click on it opens a menu with the same commands
//!   ([`MenuKind::Zoom`]);
//! - a change is written to the library's settings a moment after it was
//!   made (the wheel with `cmd` makes many in a row), and at the latest
//!   when the app quits or the window closes;
//! - a pinch on the trackpad arrives here from AppKit
//!   ([`Workspace::pinch`]).
//!
//! Nothing of this marks the document as changed: a zoom is how the text
//! is looked at.

use std::time::Duration;

use gpui::{
    App, ClickEvent, Context, Entity, Pixels, Point, Stateful, Subscription, Task, Window,
    prelude::*,
};
use writer_editor::{Editor, EditorEvent, Zoom};
use writer_library::Library;
use writer_ui::{ContextMenuItem, KeyHint, Toast, Tooltip};

use super::{MenuKind, Workspace};
use crate::zoom::{
    self, ActualSize, FLOW_ZOOM_KEY, FitPage, FitWidth, LifeSize, MENU_STEPS, PAGE_ZOOM_KEY,
    ZoomIn, ZoomOut, percent_label,
};

/// How long after the last change of zoom it is written to the library.
pub const ZOOM_STORE_DELAY: Duration = Duration::from_millis(300);

/// What the workspace keeps about the zoom.
pub(crate) struct ZoomState {
    /// What the library has for flow view and for page view; `None` where
    /// the writer never chose a zoom.
    stored: [Option<Zoom>; 2],
    /// The views whose zoom the writer changed since it was last written.
    changed: [bool; 2],
    /// Life size on the display the window is on, in percent.
    pub(crate) life_size: Option<u16>,
    store_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// The index of a view in [`ZoomState::stored`].
fn slot(page_view: bool) -> usize {
    usize::from(page_view)
}

fn key_of(page_view: bool) -> &'static str {
    if page_view {
        PAGE_ZOOM_KEY
    } else {
        FLOW_ZOOM_KEY
    }
}

impl ZoomState {
    /// Reads the zooms the library has and gives them to `editor`. Page
    /// view without one gets the zoom a page starts with
    /// ([`zoom::first_page_zoom`]).
    pub(crate) fn new(
        editor: &Entity<Editor>,
        library: &Entity<Library>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Self {
        let read = |key: &str| {
            library
                .read(cx)
                .setting::<String>(key)
                .ok()
                .flatten()
                .and_then(|text| zoom::from_stored(&text))
        };
        let stored = [read(FLOW_ZOOM_KEY), read(PAGE_ZOOM_KEY)];
        let life_size = zoom::life_size_percent(window, cx);
        let (flow, page) = (
            stored[0].unwrap_or_default(),
            stored[1].unwrap_or_else(|| zoom::first_page_zoom(life_size)),
        );
        editor.update(cx, |editor, cx| editor.set_view_zooms(flow, page, cx));
        let subscriptions = vec![
            cx.subscribe_in(editor, window, Workspace::on_zoom_event),
            // The window may have gone to a display of another density.
            cx.observe_window_bounds(window, |this, window, cx| {
                this.follow_the_display(window, cx)
            }),
            // A change that is still waiting to be written (the window
            // that closes writes it too: `Workspace::new`).
            cx.on_app_quit(|this, cx| {
                this.store_zoom_now(cx);
                async {}
            }),
        ];
        Self {
            stored,
            changed: [false; 2],
            life_size,
            store_task: None,
            _subscriptions: subscriptions,
        }
    }
}

impl Workspace {
    // ----- what the zoom is -----------------------------------------------------------

    /// The zoom of the view that is showing.
    pub fn zoom(&self, cx: &App) -> Zoom {
        self.editor.read(cx).zoom()
    }

    /// The zoom the view is drawn at, in percent.
    pub fn zoom_percent(&self, cx: &App) -> f32 {
        self.editor.read(cx).zoom_percent()
    }

    /// What the status bar shows for the zoom: "125 %". Empty without a
    /// document.
    pub fn zoom_label(&self, cx: &App) -> String {
        if self.session.is_none() {
            return String::new();
        }
        percent_label(self.zoom_percent(cx))
    }

    /// Life size on the display the window is on, in percent, where the
    /// display says how large it is.
    pub fn life_size(&self) -> Option<u16> {
        self.zoom.life_size
    }

    /// Zooms the view that is showing. The line the caret is on stays
    /// where it is.
    pub fn set_zoom(&mut self, zoom: Zoom, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        self.editor
            .update(cx, |editor, cx| editor.set_zoom(zoom, cx));
    }

    /// One line about the zoom for scripted runs: the view, the zoom it
    /// is drawn at, what it follows when it follows the window, and how
    /// far a sheet that is wider than its room is scrolled sideways.
    pub fn describe_zoom(&self, cx: &App) -> String {
        let editor = self.editor.read(cx);
        let view = if editor.page_view() { "page" } else { "flow" };
        let mut text = format!("{view} {}%", editor.zoom_percent().round() as i32);
        match editor.zoom() {
            Zoom::Percent(_) => {}
            Zoom::FitWidth => text.push_str(" fit-width"),
            Zoom::FitPage => text.push_str(" fit-page"),
            Zoom::FitWidthUpTo(limit) => text.push_str(&format!(" fit-width-up-to-{limit}")),
        }
        let max = f32::from(editor.max_horizontal_scroll());
        if max > 0.5 {
            text.push_str(&format!(
                " sideways={}/{}",
                f32::from(editor.horizontal_scroll_offset()).round() as i32,
                max.round() as i32
            ));
        }
        if let Some(life) = self.zoom.life_size {
            text.push_str(&format!(" life={life}%"));
        }
        text
    }

    // ----- the commands ---------------------------------------------------------------

    /// The handlers of the zoom commands, on the workspace root. Only
    /// while a document is open: the menu bar greys the items out without
    /// one, and the keys do nothing.
    pub(crate) fn zoom_actions(
        &self,
        root: Stateful<gpui::Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<gpui::Div> {
        if self.session.is_none() {
            return root;
        }
        let mut root = root
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| {
                this.editor.update(cx, |editor, cx| editor.zoom_in(cx));
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| {
                this.editor.update(cx, |editor, cx| editor.zoom_out(cx));
            }))
            .on_action(cx.listener(|this, _: &ActualSize, _, cx| {
                this.set_zoom(Zoom::Percent(100), cx);
            }))
            .on_action(cx.listener(|this, _: &FitWidth, _, cx| {
                this.set_zoom(Zoom::FitWidth, cx);
            }))
            .on_action(cx.listener(|this, _: &FitPage, _, cx| this.zoom_to_fit_page(cx)))
            .on_action(
                cx.listener(|this, _: &LifeSize, window, cx| this.zoom_to_life_size(window, cx)),
            );
        for percent in MENU_STEPS {
            let Some(action) = zoom::step_action(percent) else {
                continue;
            };
            if percent == 100 {
                // "Actual size", above.
                continue;
            }
            root = root.on_boxed_action(
                action.as_ref(),
                cx.listener(move |this, _: &dyn gpui::Action, _, cx| {
                    this.set_zoom(Zoom::Percent(percent), cx);
                }),
            );
        }
        root
    }

    /// One whole page in the window. Flow view has no pages: it says so.
    pub fn zoom_to_fit_page(&mut self, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        if !self.editor.read(cx).page_view() {
            self.show_toast(
                Toast::new("Fit Page is for page view. In one column, Fit Width fills the window.")
                    .duration(Duration::from_secs(4)),
                cx,
            );
            return;
        }
        self.set_zoom(Zoom::FitPage, cx);
    }

    /// The page as large as the paper it stands for, on the display the
    /// window is on now.
    pub fn zoom_to_life_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        self.follow_the_display(window, cx);
        match self.zoom.life_size {
            Some(percent) => self.set_zoom(Zoom::Percent(percent), cx),
            None => self.show_toast(
                Toast::new("This display does not say how large it is, so life size is not known.")
                    .duration(Duration::from_secs(4)),
                cx,
            ),
        }
    }

    /// Two fingers pinched on the trackpad over the window: zooms the text
    /// by `factor` (above 1: in) around `point`, when the point is on the
    /// text and nothing lies over it.
    pub fn pinch(&mut self, factor: f32, point: Point<Pixels>, cx: &mut Context<Self>) {
        if self.session.is_none() || self.modal_layer.read(cx).has_modal() {
            return;
        }
        let on_text = self
            .editor
            .read(cx)
            .viewport_bounds()
            .is_some_and(|viewport| viewport.contains(&point));
        if on_text {
            self.editor
                .update(cx, |editor, cx| editor.zoom_by(factor, point, cx));
        }
    }

    // ----- the status bar ---------------------------------------------------------------

    /// The zoom in the status bar: the percentage, with the menu of the
    /// zoom on a click or a right-click. `None` without a document.
    pub(crate) fn render_zoom_item(
        &mut self,
        tips: bool,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<gpui::Div>> {
        self.session.as_ref()?;
        let label = self.zoom_label(cx);
        let hint = KeyHint::for_action(&ZoomIn, cx);
        let item = self
            .status_item(MenuKind::Zoom, "status-zoom", None, cx)
            .debug_selector(|| MenuKind::Zoom.selector().into())
            .when(tips, |el| {
                el.tooltip(
                    Tooltip::new("Zoom. Click for the sizes")
                        .key_hint(hint)
                        .build(),
                )
            })
            .child(label)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.open_menu(MenuKind::Zoom, None, window, cx);
            }));
        Some(item)
    }

    /// The menu of the zoom: in and out, the sizes that mean something
    /// (actual size, the width of the window, a whole page, the size of
    /// the paper), then the steps. The one in force is checked.
    pub(crate) fn zoom_menu_items(&self, cx: &App) -> Vec<ContextMenuItem> {
        let editor = self.editor.read(cx);
        let current = editor.zoom();
        let page_view = editor.page_view();
        let mut items = vec![
            ContextMenuItem::action("Zoom In", ZoomIn),
            ContextMenuItem::action("Zoom Out", ZoomOut),
            ContextMenuItem::separator(),
            ContextMenuItem::action("Actual Size", ActualSize)
                .checked(current == Zoom::Percent(100)),
            ContextMenuItem::action("Fit Width", FitWidth).checked(current == Zoom::FitWidth),
        ];
        if page_view {
            items.push(
                ContextMenuItem::action("Fit Page", FitPage).checked(current == Zoom::FitPage),
            );
        }
        if let Some(life) = self.zoom.life_size {
            items.push(
                ContextMenuItem::action(
                    format!("Life Size ({})", percent_label(f32::from(life))),
                    LifeSize,
                )
                .checked(current == Zoom::Percent(life)),
            );
        }
        items.push(ContextMenuItem::separator());
        for percent in MENU_STEPS {
            // "Actual Size" is the 100 % of the list.
            if percent == 100 {
                continue;
            }
            if let Some(action) = zoom::step_action(percent) {
                items.push(
                    ContextMenuItem::action_boxed(percent_label(f32::from(percent)), action)
                        .checked(current == Zoom::Percent(percent)),
                );
            }
        }
        items
    }

    // ----- keeping the choice -----------------------------------------------------------

    fn on_zoom_event(
        &mut self,
        _: &Entity<Editor>,
        event: &EditorEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if *event != EditorEvent::ZoomChanged {
            return;
        }
        // The zoom of the view that is showing is the one that changed. A
        // zoom that follows the window says so each time it comes out
        // differently: that is no new choice.
        let (page_view, zoom) = {
            let editor = self.editor.read(cx);
            (editor.page_view(), editor.zoom())
        };
        if zoom::stored_form(zoom).is_some() && self.zoom.stored[slot(page_view)] != Some(zoom) {
            self.zoom.changed[slot(page_view)] = true;
            self.zoom.store_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(ZOOM_STORE_DELAY).await;
                this.update(cx, |this, cx| this.store_zoom_now(cx)).ok();
            }));
        }
        cx.notify();
    }

    /// The zooms of the two views as the editor has them now.
    fn view_zooms(&self, cx: &App) -> [Zoom; 2] {
        let editor = self.editor.read(cx);
        [editor.zoom_of(false), editor.zoom_of(true)]
    }

    /// Writes the zoom the writer chose for a view to the library, now
    /// (it is written by itself a moment after the last change, and when
    /// the window closes or the app quits). A zoom nobody chose, what a
    /// view starts with, is not written.
    pub fn store_zoom_now(&mut self, cx: &mut Context<Self>) {
        self.zoom.store_task = None;
        for page_view in [false, true] {
            if !std::mem::take(&mut self.zoom.changed[slot(page_view)]) {
                continue;
            }
            let zoom = self.editor.read(cx).zoom_of(page_view);
            let Some(text) = zoom::stored_form(zoom) else {
                continue;
            };
            if self.zoom.stored[slot(page_view)] == Some(zoom) {
                continue;
            }
            let written = self.library.update(cx, |library, cx| {
                library.set_setting(key_of(page_view), &text, cx)
            });
            if written.is_ok() {
                self.zoom.stored[slot(page_view)] = Some(zoom);
            }
        }
    }

    /// Life size is a property of the display: read it again, and let a
    /// page that still has the zoom it started with follow.
    fn follow_the_display(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let life_size = zoom::life_size_percent(window, cx);
        if life_size == self.zoom.life_size {
            return;
        }
        self.zoom.life_size = life_size;
        if self.zoom.stored[slot(true)].is_none() {
            let [flow, page] = self.view_zooms(cx);
            if matches!(page, Zoom::FitWidthUpTo(_)) {
                let page = zoom::first_page_zoom(life_size);
                self.editor
                    .update(cx, |editor, cx| editor.set_view_zooms(flow, page, cx));
            }
        }
        cx.notify();
    }

    // ----- scripted runs ----------------------------------------------------------------

    /// The `zoom:` step of a scripted run: `in`, `out`, `actual`, `fit`
    /// (the width), `page`, `life`, a percentage (`125`), `by:<factor>`
    /// (as a pinch at the middle of the text does) and `sideways:<pixels>`
    /// (scrolls a sheet that is wider than its room).
    pub fn zoom_step(
        &mut self,
        what: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        if self.session.is_none() {
            anyhow::bail!("no document is open");
        }
        let what = what.trim();
        let number = |text: &str| -> anyhow::Result<f32> {
            text.trim()
                .trim_end_matches('%')
                .trim()
                .parse::<f32>()
                .map_err(|_| anyhow::anyhow!("expected a number, got {text:?}"))
        };
        match what {
            "in" => self.editor.update(cx, |editor, cx| editor.zoom_in(cx)),
            "out" => self.editor.update(cx, |editor, cx| editor.zoom_out(cx)),
            "actual" => self.set_zoom(Zoom::Percent(100), cx),
            "fit" | "width" | "fit-width" => self.set_zoom(Zoom::FitWidth, cx),
            "page" | "fit-page" => self.zoom_to_fit_page(cx),
            "life" => self.zoom_to_life_size(window, cx),
            other => {
                if let Some(factor) = other.strip_prefix("by:") {
                    let factor = number(factor)?;
                    let Some(viewport) = self.editor.read(cx).viewport_bounds() else {
                        anyhow::bail!("the text was not drawn yet");
                    };
                    self.pinch(factor, viewport.center(), cx);
                } else if let Some(pixels) = other.strip_prefix("sideways:") {
                    let pixels = number(pixels)?;
                    self.editor.update(cx, |editor, cx| {
                        editor.scroll_sideways_by_pixels(gpui::px(pixels), cx)
                    });
                } else {
                    let percent = number(other)?;
                    self.set_zoom(Zoom::percent(percent), cx);
                }
            }
        }
        Ok(())
    }
}
