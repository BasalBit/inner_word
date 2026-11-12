//! The panel slot at the right of the text, and the two panels that share
//! it: the comments and the assistant's side chat.
//!
//! # How they share it
//!
//! One at a time. The slot is as wide as one panel, and a list of comment
//! threads squeezed above a conversation would serve neither.
//!
//! - A panel is *there* when it has something to show: the comments while
//!   the document has a thread or a comment is being written (as before),
//!   the side chat while the user has it open.
//! - With one panel there, the slot shows it. With both there, it shows the
//!   one that was used last ([`RightPanel`], remembered across launches),
//!   under a strip of two tabs, "Comments" and "Assistant": the tab of the
//!   panel in front is marked, a click on the other brings it forward, and
//!   each says its key in a tooltip.
//! - What brings a panel forward: its key (`cmd-2` goes to the comments,
//!   `cmd-3` to the assistant), its commands (adding a comment, stepping to
//!   the next one, asking in the side chat), its tab, and the keyboard
//!   arriving in it.
//!
//! # Width
//!
//! Each panel has a width of its own, changed by dragging the left edge of
//! the slot and remembered (`comments.panel_width`,
//! `assistant.panel_width`); a double click on the edge brings the default
//! back. In a narrow window the slot is drawn narrower, see
//! [`super::panel_layout`].

use gpui::{
    AnyElement, AnyView, App, ClickEvent, Context, CursorStyle, Entity, MouseButton,
    MouseMoveEvent, MouseUpEvent, Window, div, prelude::*, px,
};
use writer_assistant_ui::AssistantPanel;
use writer_comments as comments;
use writer_ui::{
    KeyHint, Tooltip,
    theme::{ActiveTheme, SPACE_2, TITLEBAR_HEIGHT, UI_TEXT_SM},
};

use super::{DEFAULT_RIGHT_PANEL_WIDTH, MIN_RIGHT_PANEL_WIDTH, STATUS_BAR_HEIGHT, Workspace};
use crate::{actions::FocusAssistant, settings::keys};

/// The widest the panel at the right of the text can be made.
pub const MAX_RIGHT_PANEL_WIDTH: f32 = 560.;
/// The width the side chat starts with: a conversation wants a little more
/// room than a list of comments.
pub const DEFAULT_ASSISTANT_PANEL_WIDTH: f32 = 380.;
/// The height of the strip of tabs above the panel, while both panels are
/// there.
pub const RIGHT_PANEL_TABS_HEIGHT: f32 = 32.;
/// Width of the strip at the left edge of the slot that takes the resize
/// drag.
const RESIZE_HANDLE_WIDTH: f32 = 7.;

/// One of the two panels that share the slot at the right of the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RightPanel {
    /// The comment threads of the open document.
    Comments,
    /// The assistant's side chat.
    Assistant,
}

impl RightPanel {
    /// Both panels, in the order of their tabs.
    pub const ALL: [RightPanel; 2] = [RightPanel::Comments, RightPanel::Assistant];

    /// The stored name.
    pub fn as_str(self) -> &'static str {
        match self {
            RightPanel::Comments => "comments",
            RightPanel::Assistant => "assistant",
        }
    }

    /// Parses a stored name; anything unknown is the comments.
    pub fn parse(name: &str) -> RightPanel {
        match name {
            "assistant" => RightPanel::Assistant,
            _ => RightPanel::Comments,
        }
    }

    /// What the tab says.
    pub fn label(self) -> &'static str {
        match self {
            RightPanel::Comments => "Comments",
            RightPanel::Assistant => "Assistant",
        }
    }

    /// The debug selector of the panel's tab (tests click it).
    pub fn tab_selector(self) -> &'static str {
        match self {
            RightPanel::Comments => "right-tab-comments",
            RightPanel::Assistant => "right-tab-assistant",
        }
    }

    /// The width the panel starts with.
    pub fn default_width(self) -> f32 {
        match self {
            RightPanel::Comments => DEFAULT_RIGHT_PANEL_WIDTH,
            RightPanel::Assistant => DEFAULT_ASSISTANT_PANEL_WIDTH,
        }
    }

    fn width_key(self) -> &'static str {
        match self {
            RightPanel::Comments => keys::COMMENTS_PANEL_WIDTH,
            RightPanel::Assistant => keys::ASSISTANT_PANEL_WIDTH,
        }
    }

    fn index(self) -> usize {
        match self {
            RightPanel::Comments => 0,
            RightPanel::Assistant => 1,
        }
    }
}

/// What the workspace keeps about the slot at the right of the text.
pub(crate) struct RightSlot {
    /// Which of the two panels is in front when both are there.
    pub choice: RightPanel,
    /// Which of them is in the slot now.
    pub shown: Option<RightPanel>,
    /// Whether both are there (the strip of tabs is drawn).
    pub tabs: bool,
    /// The side chat, while the assistant is shown.
    pub assistant: Option<Entity<AssistantPanel>>,
    /// Whether the user has the side chat open.
    pub assistant_open: bool,
    /// The widths of the two panels.
    widths: [f32; 2],
    /// Whether the left edge of the slot is being dragged.
    pub dragging: bool,
}

impl RightSlot {
    /// Reads what was remembered in `library`.
    pub fn load(library: &writer_library::Library) -> Self {
        let choice: String = library.setting_or(keys::RIGHT_PANEL, String::new());
        let width = |panel: RightPanel| {
            library
                .setting_or(panel.width_key(), panel.default_width())
                .clamp(MIN_RIGHT_PANEL_WIDTH, MAX_RIGHT_PANEL_WIDTH)
        };
        Self {
            choice: RightPanel::parse(&choice),
            shown: None,
            tabs: false,
            assistant: None,
            assistant_open: library.setting_or(keys::ASSISTANT_PANEL_OPEN, false),
            widths: [width(RightPanel::Comments), width(RightPanel::Assistant)],
            dragging: false,
        }
    }

    /// The width of `panel`.
    pub fn width(&self, panel: RightPanel) -> f32 {
        self.widths[panel.index()]
    }
}

impl Workspace {
    /// Which of the two panels is in the slot at the right of the text;
    /// `None` when neither is (or another view was put there with
    /// [`Workspace::set_right_panel`]).
    pub fn right_panel_shown(&self) -> Option<RightPanel> {
        self.right.shown
    }

    /// Which panel is in front when both have something to show: the one
    /// that was used last.
    pub fn right_panel_choice(&self) -> RightPanel {
        self.right.choice
    }

    /// Whether both panels are there, so that the slot has its strip of
    /// tabs.
    pub fn right_panel_has_tabs(&self) -> bool {
        self.right.tabs
    }

    /// The width `panel` has been given.
    pub fn right_panel_width_of(&self, panel: RightPanel) -> f32 {
        self.right.width(panel)
    }

    /// Whether the user has the side chat open. It is drawn while this is
    /// so, a document is open and the assistant is available; see
    /// [`Workspace::right_panel_shown`] for what is on screen.
    pub fn assistant_panel_open(&self) -> bool {
        self.right.assistant_open
    }

    /// Whether the side chat has something to show: the assistant is
    /// available, the user has the panel open and a document is open.
    fn assistant_panel_there(&self) -> bool {
        self.right.assistant.is_some() && self.right.assistant_open && self.session.is_some()
    }

    /// Fills the slot at the right of the text with the panel that belongs
    /// there now (the rules are at the top of this file), or empties it.
    pub(crate) fn sync_right_panel(&mut self, cx: &mut Context<Self>) {
        let comments = self.comments.read(cx).is_open();
        let assistant = self.assistant_panel_there();
        let shown = match (comments, assistant) {
            (true, true) => Some(self.right.choice),
            (true, false) => Some(RightPanel::Comments),
            (false, true) => Some(RightPanel::Assistant),
            (false, false) => None,
        };
        let tabs = comments && assistant;
        if shown == self.right.shown && tabs == self.right.tabs {
            return;
        }
        self.right.shown = shown;
        self.right.tabs = tabs;
        let view: Option<AnyView> = match shown {
            Some(RightPanel::Comments) => Some(self.comments.clone().into()),
            Some(RightPanel::Assistant) => self
                .right
                .assistant
                .as_ref()
                .map(|panel| panel.clone().into()),
            None => None,
        };
        if let Some(panel) = shown {
            self.right_panel_width = px(self.right.width(panel));
        }
        self.set_right_panel(view, cx);
        self.assistant_panel_shown_changed(cx);
    }

    /// Brings `panel` to the front of the slot and remembers the choice.
    /// It only shows when the panel has something to show.
    pub fn choose_right_panel(&mut self, panel: RightPanel, cx: &mut Context<Self>) {
        // The slot is the panel beside the text that was used last: it
        // keeps its place in a window too narrow for the library too.
        self.last_side_panel = super::SidePanel::Right;
        if self.right.choice != panel {
            self.right.choice = panel;
            self.library
                .update(cx, |library, cx| {
                    library.set_setting(keys::RIGHT_PANEL, panel.as_str(), cx)
                })
                .ok();
        }
        self.sync_right_panel(cx);
    }

    /// Opens or closes the side chat and remembers it. Closing it while it
    /// has the keyboard returns the keyboard to the text.
    pub fn set_assistant_panel_open(
        &mut self,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !open && self.assistant_panel_has_focus(window, cx) {
            self.focus_editor(window, cx);
        }
        if self.right.assistant_open != open {
            self.right.assistant_open = open;
            self.library
                .update(cx, |library, cx| {
                    library.set_setting(keys::ASSISTANT_PANEL_OPEN, &open, cx)
                })
                .ok();
        }
        if open {
            self.choose_right_panel(RightPanel::Assistant, cx);
        } else {
            self.sync_right_panel(cx);
        }
    }

    /// Whether the keyboard is in the side chat.
    pub fn assistant_panel_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.right
            .assistant
            .as_ref()
            .is_some_and(|panel| panel.read(cx).contains_focus(window, cx))
    }

    /// Notes which of the two panels the keyboard is in: that one is the
    /// one in front from now on. Called while a frame is built.
    pub(crate) fn note_right_panel_focus(&mut self, window: &Window, cx: &mut Context<Self>) {
        let focused = if self.comments.read(cx).has_focus(window, cx) {
            Some(RightPanel::Comments)
        } else if self.assistant_panel_has_focus(window, cx) {
            Some(RightPanel::Assistant)
        } else {
            None
        };
        if let Some(panel) = focused
            && self.right.choice != panel
            && self.right.shown == Some(panel)
        {
            self.right.choice = panel;
            self.library
                .update(cx, |library, cx| {
                    library.set_setting(keys::RIGHT_PANEL, panel.as_str(), cx)
                })
                .ok();
        }
    }

    /// Sets the width of the panel that is in the slot (clamped), without
    /// storing it; used while dragging.
    fn set_right_width_live(&mut self, width: f32, cx: &mut Context<Self>) {
        let Some(panel) = self.right.shown else {
            return;
        };
        let width = width.clamp(MIN_RIGHT_PANEL_WIDTH, MAX_RIGHT_PANEL_WIDTH);
        if self.right.widths[panel.index()] != width {
            self.right.widths[panel.index()] = width;
            self.right_panel_width = px(width);
            cx.notify();
        }
    }

    /// Gives the panel that is in the slot the width `width` (clamped to
    /// [`MIN_RIGHT_PANEL_WIDTH`]..=[`MAX_RIGHT_PANEL_WIDTH`]) and remembers
    /// it for that panel.
    pub fn resize_right_panel(&mut self, width: f32, cx: &mut Context<Self>) {
        self.set_right_width_live(width, cx);
        self.store_right_width(cx);
    }

    fn store_right_width(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.right.shown else {
            return;
        };
        let width = self.right.width(panel);
        self.library
            .update(cx, |library, cx| {
                library.set_setting(panel.width_key(), &width, cx)
            })
            .ok();
    }

    /// The pointer moved while the edge of the slot may be held: the edge
    /// follows it, or the drag ends when the button is no longer down.
    pub(crate) fn drag_right_panel(
        &mut self,
        event: &MouseMoveEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self.right.dragging {
            if event.dragging() {
                let viewport = f32::from(window.viewport_size().width);
                self.set_right_width_live(viewport - f32::from(event.position.x), cx);
            } else {
                self.end_right_panel_drag(cx);
            }
        }
    }

    pub(crate) fn end_right_panel_drag(&mut self, cx: &mut Context<Self>) {
        if self.right.dragging {
            self.right.dragging = false;
            self.store_right_width(cx);
            cx.notify();
        }
    }

    /// The strip of two tabs above the panel, drawn while both panels are
    /// there.
    fn render_right_tabs(&self, tips: bool, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors;
        let shown = self.right.shown;
        let tab = |panel: RightPanel, cx: &mut Context<Self>| {
            let active = shown == Some(panel);
            let hint = match panel {
                RightPanel::Comments => KeyHint::for_action(&comments::FocusComments, cx),
                RightPanel::Assistant => KeyHint::for_action(&FocusAssistant, cx),
            };
            let tooltip = match panel {
                RightPanel::Comments => "Show the comments",
                RightPanel::Assistant => "Show the assistant",
            };
            div()
                .id(panel.tab_selector())
                .debug_selector(move || panel.tab_selector().into())
                .h(px(22.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(writer_ui::theme::RADIUS_SM)
                .text_size(UI_TEXT_SM)
                .whitespace_nowrap()
                .cursor_pointer()
                .map(|el| {
                    if active {
                        el.bg(colors.hover)
                            .text_color(colors.ink)
                            .font_weight(gpui::FontWeight::MEDIUM)
                    } else {
                        el.text_color(colors.muted)
                            .hover(|style| style.text_color(colors.ink))
                    }
                })
                .when(tips, |el| {
                    el.tooltip(Tooltip::new(tooltip).key_hint(hint).build())
                })
                // A press on a tab is not a press on the text: the
                // keyboard stays where it is.
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.show_right_panel_tab(panel, window, cx);
                }))
                .child(panel.label())
        };
        div()
            .id("right-panel-tabs")
            .debug_selector(|| "right-panel-tabs".into())
            .flex_none()
            .h(px(RIGHT_PANEL_TABS_HEIGHT))
            .px(SPACE_2)
            .flex()
            .items_center()
            .gap(px(2.))
            .border_b_1()
            .border_color(colors.hairline)
            .child(tab(RightPanel::Comments, cx))
            .child(tab(RightPanel::Assistant, cx))
            .into_any_element()
    }

    /// A tab was chosen: its panel comes forward. The keyboard follows
    /// when it was in the panel that steps back (it would otherwise be in
    /// something that is no longer drawn).
    pub fn show_right_panel_tab(
        &mut self,
        panel: RightPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keyboard_in_slot = self.comments.read(cx).has_focus(window, cx)
            || self.assistant_panel_has_focus(window, cx);
        self.choose_right_panel(panel, cx);
        if keyboard_in_slot && self.right.shown == Some(panel) {
            match panel {
                RightPanel::Comments => {
                    let comments = self.comments.clone();
                    comments.update(cx, |comments, cx| comments.focus_panel(window, cx));
                }
                RightPanel::Assistant => {
                    if let Some(panel) = self.right.assistant.clone() {
                        panel.update(cx, |panel, cx| panel.focus(window, cx));
                    }
                }
            }
        }
    }

    /// The slot at the right of the text, `width` wide: the strip of tabs
    /// while both panels are there, and the panel.
    pub(crate) fn render_right_panel(
        &mut self,
        view: AnyView,
        width: f32,
        tips: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors;
        let tabs = self.right.tabs.then(|| self.render_right_tabs(tips, cx));
        div()
            .id("right-panel")
            .debug_selector(|| "right-panel".into())
            .w(px(width))
            .h_full()
            .flex_none()
            .overflow_hidden()
            .border_l_1()
            .border_color(colors.hairline)
            .bg(colors.sidebar)
            .flex()
            .flex_col()
            .children(tabs)
            .child(div().flex_1().min_h_0().overflow_hidden().child(view))
            .into_any_element()
    }

    /// The strip over the left edge of the slot that is dragged to change
    /// its width. Only for the two panels the workspace knows the widths
    /// of.
    pub(crate) fn render_right_resize_handle(
        &mut self,
        width: f32,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let panel = self.right.shown?;
        let colors = cx.theme().colors;
        let dragging = self.right.dragging;
        Some(
            div()
                .id("right-panel-resize")
                .debug_selector(|| "right-panel-resize".into())
                .absolute()
                .top(TITLEBAR_HEIGHT)
                .bottom(px(STATUS_BAR_HEIGHT))
                .right(px(width - RESIZE_HANDLE_WIDTH / 2.))
                .w(px(RESIZE_HANDLE_WIDTH))
                .cursor(CursorStyle::ResizeLeftRight)
                // Half of the handle lies over the text: a press on it must
                // not reach the editor underneath.
                .occlude()
                .flex()
                .justify_center()
                .child(
                    div()
                        .w(px(1.))
                        .h_full()
                        .when(dragging, |el| el.bg(colors.accent)),
                )
                .hover(|style| style.bg(colors.hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                        if event.click_count == 2 {
                            this.resize_right_panel(panel.default_width(), cx);
                        } else {
                            this.right.dragging = true;
                            cx.notify();
                        }
                    }),
                )
                .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                    this.drag_right_panel(event, window, cx);
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _, cx| this.end_right_panel_drag(cx)),
                )
                .into_any_element(),
        )
    }
}
