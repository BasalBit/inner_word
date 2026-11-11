//! The floating format bar: the small row of buttons that comes up by
//! itself over a selection made with the mouse.
//!
//! It is for whoever selects with the mouse and looks for a button, so it
//! follows the mouse only:
//!
//! - **It appears** [`FORMAT_BAR_DELAY`] after the left button is released
//!   on a selection that the press made or changed: the end of a drag, a
//!   double or triple click, a `shift`-click. A selection made with the
//!   keyboard never brings it up (`cmd-.` is the keyboard's format menu),
//!   nor does one in a compact or read-only editor.
//! - **It sits** over the first row of the selection, centred on that row,
//!   inside the editor's viewport; under the selection when there is no
//!   room above. It is placed in the frame's prepaint, after the editor's,
//!   so it follows scrolling without lagging a frame; with the selection
//!   scrolled out of sight it is not drawn (and is back when the selection
//!   is). Only a selection that fills the viewport from top to bottom has
//!   the bar on top of selected text: at the top edge.
//! - **It never takes the keyboard.** Its buttons do not move the focus, so
//!   typing still replaces the selection and every key still works.
//! - **A button** does what its key does, through the same code as the
//!   format menu and the text context menu (one undo step), keeps the
//!   selection and keeps the bar. The toggles show the state of the
//!   selection (on, off, a dash for "in part of it") and follow every
//!   change at once. Highlight, text colour and "turn into" drop a
//!   `writer_ui::ContextMenu` from their button; link and "more" open the
//!   link popover and the format menu, which take the bar's place.
//! - **It goes away** when the selection collapses or is changed from the
//!   keyboard, when text is typed, on `escape`, on a press anywhere else,
//!   when a popup tool or a context menu opens, when the keyboard leaves
//!   the text, when the window is deactivated and when another document is
//!   loaded. The next mouse selection brings it back.
//!
//! How a mouse selection is told from a keyboard one: the layer this module
//! puts over the editor listens to the window's mouse events. A left press
//! in the text opens a "press"; a `SelectionChanged` that arrives while it
//! is open was made by the pointer; the release offers the bar when there
//! was one. Every other `SelectionChanged` closes the bar unless the
//! selection is still the one the bar belongs to (a button of the bar
//! records the selection it leaves behind).

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gpui::{
    Action, AnyElement, App, AvailableSpace, Bounds, Context, DismissEvent, DispatchPhase, Entity,
    Focusable, FontWeight, HitboxBehavior, Hsla, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, Rgba, SharedString, Size, Subscription, Task, Window, canvas, div,
    point, prelude::*, px,
};
use writer_core::{BlockKind, Choice, Highlight, Rgb, Selection, Toggle};
use writer_editor::actions as editor;
use writer_ui::{
    ContextMenu, ContextMenuItem, Icon, IconName, KeyHint, MenuAnchor,
    theme::{ActiveTheme, HighlightColor, RADIUS_LG, RADIUS_SM, UI_TEXT_BASE},
};

use crate::{
    actions::{OpenFormatMenu, OpenLink},
    format_menu::{Effect, Section, TEXT_COLORS, format_items},
    menu::{Mark, selected_row_fill},
    tip::tip,
    tools::EditorTools,
};

/// How long after the mouse button is released the format bar appears. A
/// double click on the way to a triple click, or a selection that is
/// dragged again at once, does not flash it.
pub const FORMAT_BAR_DELAY: Duration = Duration::from_millis(150);

/// The buttons of the format bar, in order: their names for
/// [`EditorTools::press_format_bar_button`] and [`FormatBarItem::id`], and
/// their debug selectors. The host's buttons follow "format-bar-turn-into"
/// as "format-bar-extra-0", "format-bar-extra-1", and so on.
pub const FORMAT_BAR_BUTTONS: [&str; 10] = [
    "format-bar-bold",
    "format-bar-italic",
    "format-bar-underline",
    "format-bar-strikethrough",
    "format-bar-code",
    "format-bar-highlight",
    "format-bar-color",
    "format-bar-link",
    "format-bar-turn-into",
    "format-bar-more",
];

/// Space between the bar and the selection.
const GAP: Pixels = px(8.);
/// Space kept to the edges of the editor's viewport.
const MARGIN: Pixels = px(8.);
/// A viewport narrower than this gets the short names of the block kinds.
const NARROW: Pixels = px(520.);
/// The bar is drawn over the text like the editor's own popup.
const DRAW_PRIORITY: usize = 1;

// ---------------------------------------------------------------------------
// The host's buttons
// ---------------------------------------------------------------------------

type Handler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone)]
enum Does {
    Action(Rc<dyn Action>),
    Handler(Handler),
}

/// A button the host adds to the format bar
/// ([`EditorTools::set_format_bar_extension`]).
///
/// ```ignore
/// FormatBarButton::action("Add Comment", comments::AddComment).icon(IconName::Comment)
/// FormatBarButton::new("Define", move |window, cx| dictionary.update(cx, |d, cx| d.look_up(window, cx)))
/// ```
#[derive(Clone)]
pub struct FormatBarButton {
    label: SharedString,
    icon: Option<IconName>,
    key_hint: Option<KeyHint>,
    does: Does,
    disabled: bool,
    active: bool,
}

impl FormatBarButton {
    fn with(label: impl Into<SharedString>, does: Does) -> Self {
        Self {
            label: label.into(),
            icon: None,
            key_hint: None,
            does,
            disabled: false,
            active: false,
        }
    }

    /// A button that dispatches `action` from the text outwards, exactly as
    /// the action's key binding does while the keyboard is in the text.
    /// Its tooltip shows `label` and the key bound to the action.
    pub fn action(label: impl Into<SharedString>, action: impl Action) -> Self {
        Self::with(label, Does::Action(Rc::new(action)))
    }

    /// Like [`FormatBarButton::action`] for an action that is already
    /// boxed.
    pub fn action_boxed(label: impl Into<SharedString>, action: Box<dyn Action>) -> Self {
        Self::with(label, Does::Action(Rc::from(action)))
    }

    /// A button that runs `handler`. It runs outside any update of the
    /// tools view, with the selection and the keyboard as they were.
    pub fn new(
        label: impl Into<SharedString>,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self::with(label, Does::Handler(Rc::new(handler)))
    }

    /// Shows this icon instead of the label; the label stays in the
    /// tooltip. Use an icon only where it cannot be misread.
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The key shown in the tooltip (an action button looks its own up).
    /// Accepts an `Option` so `KeyHint::for_action(..)` can be passed
    /// directly.
    pub fn key_hint(mut self, hint: impl Into<Option<KeyHint>>) -> Self {
        self.key_hint = hint.into();
        self
    }

    /// Shows the button dimmed; it cannot be pressed.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Shows the button in its "on" state, like a toggle that holds for the
    /// whole selection.
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// The button's name: its face without an icon, and its tooltip.
    pub fn label(&self) -> &SharedString {
        &self.label
    }

    fn hint(&self, cx: &App) -> Option<KeyHint> {
        self.key_hint.clone().or_else(|| match &self.does {
            Does::Action(action) => KeyHint::for_action(action.as_ref(), cx),
            Does::Handler(_) => None,
        })
    }
}

impl std::fmt::Debug for FormatBarButton {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FormatBarButton")
            .field("label", &self.label)
            .field("icon", &self.icon)
            .field("disabled", &self.disabled)
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

/// What the host adds to the bar.
type Extension = Rc<dyn Fn(&mut Window, &mut App) -> Vec<FormatBarButton>>;

/// One button of the showing format bar, for tests, automation and a
/// status line ([`EditorTools::format_bar_items`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatBarItem {
    /// The button's name and debug selector: one of
    /// [`FORMAT_BAR_BUTTONS`], or "format-bar-extra-N" for the host's.
    pub id: String,
    /// What the tooltip calls it ("Bold", "Turn into").
    pub label: String,
    /// The key in the tooltip, as glyphs ("⌘B").
    pub key_hint: Option<String>,
    /// What the button shows besides its state: the block kind on "turn
    /// into" ("Heading 2", "Mixed"), the colour on highlight and text
    /// colour when the whole selection has one ("Yellow").
    pub detail: Option<String>,
    /// The state of the selection: [`Mark::Check`] when on for all of it,
    /// [`Mark::Mixed`] when on for a part of it, [`Mark::None`] when off
    /// (and always for a button that is not a toggle).
    pub state: Mark,
    /// `false` for a button that is shown dimmed and cannot be pressed.
    pub enabled: bool,
    /// The button drops a menu instead of acting at once.
    pub menu: bool,
    /// The button was added by the host.
    pub host: bool,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// The buttons of this crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Core {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Code,
    Highlight,
    Color,
    Link,
    TurnInto,
    More,
}

impl Core {
    const ALL: [Core; 10] = [
        Core::Bold,
        Core::Italic,
        Core::Underline,
        Core::Strikethrough,
        Core::Code,
        Core::Highlight,
        Core::Color,
        Core::Link,
        Core::TurnInto,
        Core::More,
    ];

    fn id(self) -> &'static str {
        FORMAT_BAR_BUTTONS[self as usize]
    }

    fn label(self) -> &'static str {
        match self {
            Core::Bold => "Bold",
            Core::Italic => "Italic",
            Core::Underline => "Underline",
            Core::Strikethrough => "Strikethrough",
            Core::Code => "Inline code",
            Core::Highlight => "Highlight",
            Core::Color => "Text colour",
            Core::Link => "Link",
            Core::TurnInto => "Turn into",
            Core::More => "More formatting",
        }
    }

    /// The buttons between hairlines.
    fn group(self) -> u8 {
        match self {
            Core::Bold | Core::Italic | Core::Underline | Core::Strikethrough | Core::Code => 0,
            Core::Highlight | Core::Color => 1,
            Core::Link => 2,
            Core::TurnInto => 3,
            Core::More => 5,
        }
    }

    /// The menu the button drops, if it drops one.
    fn menu(self) -> Option<Section> {
        match self {
            Core::Highlight => Some(Section::Highlight),
            Core::Color => Some(Section::Color),
            Core::TurnInto => Some(Section::TurnInto),
            _ => None,
        }
    }

    fn hint(self, cx: &App) -> Option<KeyHint> {
        match self {
            Core::Bold => KeyHint::for_action(&editor::ToggleBold, cx),
            Core::Italic => KeyHint::for_action(&editor::ToggleItalic, cx),
            Core::Underline => KeyHint::for_action(&editor::ToggleUnderline, cx),
            Core::Strikethrough => KeyHint::for_action(&editor::ToggleStrikethrough, cx),
            Core::Code => KeyHint::for_action(&editor::ToggleInlineCode, cx),
            Core::Highlight => KeyHint::for_action(&editor::ToggleHighlight, cx),
            Core::Link => KeyHint::for_action(&OpenLink, cx),
            Core::More => KeyHint::for_action(&OpenFormatMenu, cx),
            Core::Color | Core::TurnInto => None,
        }
    }
}

/// The group of the host's buttons: between "turn into" and "more".
const HOST_GROUP: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Which {
    Core(Core),
    Host(usize),
}

/// A button as it is drawn and as it is reported.
struct Spec {
    which: Which,
    id: SharedString,
    label: SharedString,
    detail: Option<SharedString>,
    hint: Option<KeyHint>,
    state: Mark,
    enabled: bool,
    group: u8,
}

/// What the bar shows of the selection. Kept until the document or the
/// selection changes, so a frame that only scrolls reads nothing.
#[derive(Clone)]
struct Snapshot {
    revision: u64,
    selection: Selection,
    /// Bold, italic, underline, strikethrough, inline code.
    toggles: [Mark; 5],
    highlight: Choice<Option<Highlight>>,
    color: Choice<Option<Rgb>>,
    link: Mark,
    /// `None` for blocks of different kinds.
    kind: Option<BlockKind>,
    /// Inline formatting can apply (the selection is not all code block).
    inline: bool,
}

/// A left press in the text that has not been released yet.
struct Press {
    /// The selection changed while the button was down.
    changed: bool,
}

/// The bar, from the release of the mouse button until it goes away.
struct OpenBar {
    /// The selection the bar belongs to.
    selection: Selection,
    /// `false` while [`FORMAT_BAR_DELAY`] runs.
    visible: bool,
    /// The host's buttons.
    extra: Vec<FormatBarButton>,
    _appear: Option<Task<()>>,
}

/// The menu a button of the bar has dropped.
struct OpenMenu {
    menu: Entity<ContextMenu>,
    _subscription: Subscription,
}

type SharedBounds = Rc<Cell<Option<Bounds<Pixels>>>>;

/// The format bar's part of [`EditorTools`].
pub(crate) struct FormatBar {
    enabled: bool,
    open: Option<OpenBar>,
    press: Option<Press>,
    menu: Option<OpenMenu>,
    extension: Option<Extension>,
    snapshot: RefCell<Option<Snapshot>>,
    /// Where the bar was drawn in the last frame.
    bounds: SharedBounds,
    /// Where the buttons that drop a menu were drawn: highlight, text
    /// colour, turn into.
    anchors: [SharedBounds; 3],
}

impl Default for FormatBar {
    fn default() -> Self {
        Self {
            enabled: true,
            open: None,
            press: None,
            menu: None,
            extension: None,
            snapshot: RefCell::new(None),
            bounds: SharedBounds::default(),
            anchors: Default::default(),
        }
    }
}

impl FormatBar {
    fn visible(&self) -> bool {
        self.open.as_ref().is_some_and(|open| open.visible)
    }

    fn anchor(&self, core: Core) -> Option<&SharedBounds> {
        match core {
            Core::Highlight => Some(&self.anchors[0]),
            Core::Color => Some(&self.anchors[1]),
            Core::TurnInto => Some(&self.anchors[2]),
            _ => None,
        }
    }

    /// The theme changed: nothing of the snapshot is a colour of the
    /// theme, but the next frame should not trust that.
    pub(crate) fn forget_snapshot(&self) {
        self.snapshot.borrow_mut().take();
    }
}

fn toggle_mark(toggle: Toggle) -> Mark {
    match toggle {
        Toggle::On => Mark::Check,
        Toggle::Mixed => Mark::Mixed,
        Toggle::Off => Mark::None,
    }
}

/// On when the whole selection has a value, mixed when a part of it has.
fn value_mark<T>(choice: &Choice<Option<T>>) -> Mark {
    match choice {
        Choice::Same(Some(_)) => Mark::Check,
        Choice::Same(None) => Mark::None,
        Choice::Mixed => Mark::Mixed,
    }
}

/// A block kind as the "turn into" button names it: in full, and short
/// for a narrow editor.
fn kind_names(kind: Option<&BlockKind>) -> (&'static str, &'static str) {
    const HEADINGS: [(&str, &str); 6] = [
        ("Heading 1", "H1"),
        ("Heading 2", "H2"),
        ("Heading 3", "H3"),
        ("Heading 4", "H4"),
        ("Heading 5", "H5"),
        ("Heading 6", "H6"),
    ];
    match kind {
        None => ("Mixed", "Mixed"),
        Some(BlockKind::Paragraph) => ("Paragraph", "Text"),
        Some(BlockKind::Heading { level }) => HEADINGS[usize::from((*level).clamp(1, 6)) - 1],
        Some(BlockKind::BulletItem) => ("Bullet list", "Bullets"),
        Some(BlockKind::NumberedItem) => ("Numbered list", "Numbers"),
        Some(BlockKind::TodoItem { .. }) => ("To-do list", "To-do"),
        Some(BlockKind::Quote) => ("Quote", "Quote"),
        Some(BlockKind::CodeBlock { .. }) => ("Code block", "Code"),
        Some(BlockKind::Divider | BlockKind::PageBreak) => ("Turn into", "Turn into"),
    }
}

fn highlight_name(highlight: Highlight) -> &'static str {
    match highlight {
        Highlight::Yellow => "Yellow",
        Highlight::Green => "Green",
        Highlight::Blue => "Blue",
        Highlight::Pink => "Pink",
        Highlight::Orange => "Orange",
        Highlight::Purple => "Purple",
    }
}

fn rgb_color(rgb: Rgb) -> Hsla {
    Rgba {
        r: f32::from(rgb.r) / 255.,
        g: f32::from(rgb.g) / 255.,
        b: f32::from(rgb.b) / 255.,
        a: 1.,
    }
    .into()
}

/// Where the bar goes: over the first row of the selection and centred on
/// it, under the selection when there is no room above, at the top of the
/// viewport when the selection leaves no room on either side, and nowhere
/// (`None`) while the selection is out of sight. Always inside `viewport`,
/// with [`MARGIN`] to its edges where the bar is narrower than it.
///
/// `first` is the first row of the selection, `all` the rectangle around
/// the whole of it; all in window coordinates.
pub(crate) fn place_format_bar(
    first: Bounds<Pixels>,
    all: Bounds<Pixels>,
    size: Size<Pixels>,
    viewport: Bounds<Pixels>,
) -> Option<Point<Pixels>> {
    if all.bottom() <= viewport.top() || all.top() >= viewport.bottom() {
        return None;
    }
    let left = viewport.left() + MARGIN;
    let right = (viewport.right() - MARGIN - size.width).max(left);
    let x = (first.center().x - size.width / 2.).clamp(left, right);

    let top = viewport.top() + MARGIN;
    let bottom = viewport.bottom() - MARGIN - size.height;
    let above = first.top() - GAP - size.height;
    let below = all.bottom() + GAP;
    let y = if above >= top {
        above
    } else if below <= bottom {
        below
    } else {
        top
    };
    Some(point(x.round(), y.round()))
}

// ---------------------------------------------------------------------------
// The public surface
// ---------------------------------------------------------------------------

impl EditorTools {
    /// Turns the format bar on or off. It is on by default. Turning it off
    /// closes a showing bar; while it is off no mouse selection brings it
    /// up.
    ///
    /// ```ignore
    /// // the host's setting, at startup and whenever it changes
    /// tools.update(cx, |tools, cx| tools.set_format_bar_enabled(settings.format_bar, cx));
    /// ```
    pub fn set_format_bar_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.format_bar.enabled == enabled {
            return;
        }
        self.format_bar.enabled = enabled;
        if !enabled {
            self.format_bar.press = None;
            self.dismiss_format_bar(cx);
        }
        cx.notify();
    }

    /// Whether a mouse selection brings up the format bar.
    pub fn format_bar_enabled(&self) -> bool {
        self.format_bar.enabled
    }

    /// Lets the host add its own buttons to the format bar: they go
    /// between "turn into" and "more", behind a hairline.
    ///
    /// `extension` is called each time the bar is about to appear, and
    /// again after one of the host's buttons was used (so a button can
    /// change its state). By then the selection is what the bar acts on
    /// and the editor has the keyboard; both are still so when a button
    /// runs, so an action button arrives exactly as its key would. An
    /// empty list adds nothing.
    ///
    /// The extension runs outside any update of this view: it may read
    /// and update whatever it likes, this view included.
    ///
    /// ```ignore
    /// let panel = comments.downgrade();
    /// tools.update(cx, |tools, _| {
    ///     tools.set_format_bar_extension(move |_window, cx| {
    ///         let Some(panel) = panel.upgrade() else {
    ///             return Vec::new();
    ///         };
    ///         vec![
    ///             FormatBarButton::action("Add Comment", comments::AddComment)
    ///                 .icon(IconName::Comment)
    ///                 .disabled(!panel.read(cx).can_add_comment(cx)),
    ///         ]
    ///     })
    /// });
    /// ```
    pub fn set_format_bar_extension(
        &mut self,
        extension: impl Fn(&mut Window, &mut App) -> Vec<FormatBarButton> + 'static,
    ) {
        self.format_bar.extension = Some(Rc::new(extension));
    }

    /// Whether the format bar is showing: it has appeared for the current
    /// selection and has not gone away. (It is not drawn while the
    /// selection is scrolled out of sight; see
    /// [`EditorTools::format_bar_bounds`].)
    pub fn format_bar_visible(&self) -> bool {
        self.format_bar.visible()
    }

    /// Where the format bar was drawn in the last frame, in window
    /// coordinates. `None` when it is not showing or the selection is out
    /// of sight.
    pub fn format_bar_bounds(&self) -> Option<Bounds<Pixels>> {
        self.format_bar
            .visible()
            .then(|| self.format_bar.bounds.get())
            .flatten()
    }

    /// The buttons of the showing format bar, in order, with the state
    /// they show. Empty when the bar is not showing.
    pub fn format_bar_items(&self, cx: &App) -> Vec<FormatBarItem> {
        if !self.format_bar.visible() {
            return Vec::new();
        }
        self.format_bar_specs(cx)
            .into_iter()
            .map(|spec| FormatBarItem {
                id: spec.id.to_string(),
                label: spec.label.to_string(),
                key_hint: spec.hint.as_ref().map(KeyHint::text),
                detail: spec.detail.as_ref().map(SharedString::to_string),
                state: spec.state,
                enabled: spec.enabled,
                menu: matches!(spec.which, Which::Core(core) if core.menu().is_some()),
                host: matches!(spec.which, Which::Host(_)),
            })
            .collect()
    }

    /// The menu a button of the format bar has dropped (highlight, text
    /// colour or "turn into"), while it is open.
    pub fn format_bar_menu(&self) -> Option<&Entity<ContextMenu>> {
        self.format_bar.menu.as_ref().map(|open| &open.menu)
    }

    /// Shows the format bar for the current selection at once, as a mouse
    /// selection does after [`FORMAT_BAR_DELAY`]. Returns whether it is
    /// showing: not without a selection, in a compact or read-only editor,
    /// while it is turned off, or under an open popup tool or text menu.
    pub fn show_format_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.format_bar_allowed(cx) {
            return false;
        }
        let selection = self.editor.read(cx).selection();
        let extra = match self.format_bar.open.take() {
            Some(open) if open.selection == selection => open.extra,
            _ => Vec::new(),
        };
        self.format_bar.open = Some(OpenBar {
            selection,
            visible: true,
            extra,
            _appear: None,
        });
        // The host's buttons are asked for outside this update.
        let tools = self.this.clone();
        window.defer(cx, move |window, cx| {
            if let Some(tools) = tools.upgrade() {
                Self::load_format_bar_extension(&tools, window, cx);
            }
        });
        cx.notify();
        true
    }

    /// Closes the format bar, if it is showing or about to. The selection
    /// and the keyboard stay as they are; the next mouse selection brings
    /// the bar back.
    pub fn dismiss_format_bar(&mut self, cx: &mut Context<Self>) {
        if self.format_bar.open.take().is_some() {
            self.format_bar.bounds.set(None);
            cx.notify();
        }
    }

    /// Presses a button of the showing format bar, as a click on it does.
    /// `id` is one of [`FORMAT_BAR_BUTTONS`] or "format-bar-extra-N".
    /// Returns `false` when the bar is not showing or has no such button
    /// that can be pressed. A host's button runs right after the current
    /// update.
    pub fn press_format_bar_button(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.format_bar.visible() {
            return false;
        }
        let Some(which) = self
            .format_bar_specs(cx)
            .into_iter()
            .find(|spec| spec.id.as_ref() == id && spec.enabled)
            .map(|spec| spec.which)
        else {
            return false;
        };
        match which {
            Which::Core(core) => self.press_format_bar_core(core, window, cx),
            Which::Host(index) => {
                let Some(button) = self
                    .format_bar
                    .open
                    .as_ref()
                    .and_then(|open| open.extra.get(index))
                    .cloned()
                else {
                    return false;
                };
                let focus = self.editor.read(cx).focus_handle(cx);
                let tools = self.this.clone();
                window.defer(cx, move |window, cx| {
                    match &button.does {
                        // The path of the action's key binding.
                        Does::Action(action) => focus.dispatch_action(action.as_ref(), window, cx),
                        Does::Handler(handler) => handler(window, cx),
                    }
                    if let Some(tools) = tools.upgrade() {
                        Self::load_format_bar_extension(&tools, window, cx);
                    }
                });
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// Appearing and going away
// ---------------------------------------------------------------------------

impl EditorTools {
    /// The bar is showing, or waiting out its delay.
    pub(crate) fn format_bar_pending_or_visible(&self) -> bool {
        self.format_bar.open.is_some()
    }

    /// Whether the bar may show for the selection as it is.
    fn format_bar_allowed(&self, cx: &App) -> bool {
        let editor = self.editor.read(cx);
        self.format_bar.enabled
            && !editor.is_read_only()
            && !editor.config().mode.is_compact()
            && !editor.selection().is_collapsed()
            && !editor.has_popup()
            && self.popup.is_none()
            && self.text_menu.is_none()
    }

    /// A mouse button went down somewhere in the window (seen before
    /// anything acts on it). `in_text`: the pointer is on the editor, with
    /// nothing over it.
    fn format_bar_pointer_down(
        &mut self,
        event: &MouseDownEvent,
        in_text: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let bar = &self.format_bar;
        // On the bar itself: the button under the pointer deals with it.
        if bar.visible()
            && bar
                .bounds
                .get()
                .is_some_and(|bounds| bounds.contains(&event.position))
        {
            return;
        }
        // While the bar's own menu is open a press belongs to that menu:
        // it chooses a row, or only closes the menu.
        if let Some(open) = &bar.menu
            && ContextMenu::current(window, cx).is_some_and(|current| current == open.menu)
        {
            return;
        }
        if event.button == MouseButton::Left && in_text && !event.modifiers.control {
            // The bar stays until the press turns out to change the
            // selection (a press on the scroll bar or a checkbox does not).
            self.format_bar.press = Some(Press { changed: false });
        } else {
            self.format_bar.press = None;
            self.dismiss_format_bar(cx);
        }
    }

    /// The left button was released.
    fn format_bar_pointer_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(press) = self.format_bar.press.take() else {
            return;
        };
        if press.changed {
            self.offer_format_bar(window, cx);
        }
    }

    /// The editor's selection changed.
    pub(crate) fn format_bar_selection_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(press) = &mut self.format_bar.press {
            // The pointer is selecting: no bar in the way of a drag.
            press.changed = true;
            self.dismiss_format_bar(cx);
            return;
        }
        let selection = self.editor.read(cx).selection();
        if self
            .format_bar
            .open
            .as_ref()
            .is_some_and(|open| open.selection != selection)
        {
            self.dismiss_format_bar(cx);
        }
    }

    /// Another document was loaded: a press that was under way belongs to
    /// the old one.
    pub(crate) fn format_bar_document_replaced(&mut self, cx: &mut Context<Self>) {
        self.format_bar.press = None;
        self.format_bar.forget_snapshot();
        self.dismiss_format_bar(cx);
    }

    /// The mouse made a selection: the bar appears after a moment.
    fn offer_format_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.format_bar_allowed(cx) {
            return;
        }
        let selection = self.editor.read(cx).selection();
        let appear = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(FORMAT_BAR_DELAY).await;
            if let Some(tools) = this.upgrade() {
                cx.update(|window, cx| Self::load_format_bar_extension(&tools, window, cx))
                    .ok();
            }
        });
        self.format_bar.open = Some(OpenBar {
            selection,
            visible: false,
            extra: Vec::new(),
            _appear: Some(appear),
        });
    }

    /// Asks the host for its buttons and shows the bar with them. Runs
    /// outside any update of the tools view (the extension may use it).
    fn load_format_bar_extension(tools: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let this = tools.read(cx);
        if this.format_bar.open.is_none() {
            return;
        }
        let extension = this.format_bar.extension.clone();
        let extra = extension
            .map(|extension| extension(window, cx))
            .unwrap_or_default();
        tools.update(cx, |this, cx| {
            if let Some(open) = &mut this.format_bar.open {
                open.extra = extra;
                open.visible = true;
                cx.notify();
            }
        });
    }
}

// ---------------------------------------------------------------------------
// The buttons
// ---------------------------------------------------------------------------

impl EditorTools {
    fn format_bar_snapshot(&self, cx: &App) -> Snapshot {
        let editor = self.editor.read(cx);
        let state = editor.state();
        let (revision, selection) = (state.revision(), editor.selection());
        if let Some(snapshot) = self.format_bar.snapshot.borrow().as_ref()
            && snapshot.revision == revision
            && snapshot.selection == selection
        {
            return snapshot.clone();
        }
        let format = state.format_state();
        let kind = match &format.kind {
            Choice::Same(kind) => Some(kind.clone()),
            Choice::Mixed => None,
        };
        let snapshot = Snapshot {
            revision,
            selection,
            toggles: [
                toggle_mark(format.bold),
                toggle_mark(format.italic),
                toggle_mark(format.underline),
                toggle_mark(format.strikethrough),
                toggle_mark(format.code),
            ],
            link: value_mark(&format.link),
            // A code block's text carries no inline formatting.
            inline: !kind.as_ref().is_some_and(BlockKind::is_code),
            highlight: format.highlight,
            color: format.color,
            kind,
        };
        *self.format_bar.snapshot.borrow_mut() = Some(snapshot.clone());
        snapshot
    }

    /// The buttons of the bar, in order, with what they show right now.
    fn format_bar_specs(&self, cx: &App) -> Vec<Spec> {
        let snapshot = self.format_bar_snapshot(cx);
        let mut specs: Vec<Spec> = Core::ALL
            .into_iter()
            .map(|core| {
                let (state, detail, enabled) = match core {
                    Core::Bold
                    | Core::Italic
                    | Core::Underline
                    | Core::Strikethrough
                    | Core::Code => (snapshot.toggles[core as usize], None, snapshot.inline),
                    Core::Highlight => (
                        value_mark(&snapshot.highlight),
                        match snapshot.highlight {
                            Choice::Same(Some(highlight)) => Some(highlight_name(highlight)),
                            _ => None,
                        },
                        snapshot.inline,
                    ),
                    Core::Color => (
                        value_mark(&snapshot.color),
                        match snapshot.color {
                            Choice::Same(Some(rgb)) => TEXT_COLORS
                                .iter()
                                .find(|(_, color)| *color == rgb)
                                .map(|(name, _)| *name),
                            _ => None,
                        },
                        snapshot.inline,
                    ),
                    Core::Link => (snapshot.link, None, snapshot.inline),
                    Core::TurnInto => {
                        (Mark::None, Some(kind_names(snapshot.kind.as_ref()).0), true)
                    }
                    Core::More => (Mark::None, None, true),
                };
                Spec {
                    which: Which::Core(core),
                    id: core.id().into(),
                    label: core.label().into(),
                    detail: detail.map(SharedString::from),
                    hint: core.hint(cx),
                    state,
                    enabled,
                    group: core.group(),
                }
            })
            .collect();
        let extra = self
            .format_bar
            .open
            .as_ref()
            .map(|open| open.extra.as_slice())
            .unwrap_or_default();
        let host = extra.iter().enumerate().map(|(index, button)| Spec {
            which: Which::Host(index),
            id: format!("format-bar-extra-{index}").into(),
            label: button.label.clone(),
            detail: None,
            hint: button.hint(cx),
            state: if button.active {
                Mark::Check
            } else {
                Mark::None
            },
            enabled: !button.disabled,
            group: HOST_GROUP,
        });
        // Before "more", which closes the row.
        let more = specs.len() - 1;
        specs.splice(more..more, host);
        specs
    }

    fn press_format_bar_core(&mut self, core: Core, window: &mut Window, cx: &mut Context<Self>) {
        let action: Rc<dyn Action> = match core {
            Core::Bold => Rc::new(editor::ToggleBold),
            Core::Italic => Rc::new(editor::ToggleItalic),
            Core::Underline => Rc::new(editor::ToggleUnderline),
            Core::Strikethrough => Rc::new(editor::ToggleStrikethrough),
            Core::Code => Rc::new(editor::ToggleInlineCode),
            Core::Highlight | Core::Color | Core::TurnInto => {
                return self.open_format_bar_menu(core, window, cx);
            }
            // Both are popup tools with a field: they take the bar's
            // place.
            Core::Link => return self.open_link(window, cx),
            Core::More => return self.open_format_menu(window, cx),
        };
        self.apply_from_format_bar(Effect::Editor(action), window, cx);
    }

    /// Runs a row of the format menu for the bar: the selection it leaves
    /// behind is the bar's from now on, so the bar stays.
    fn apply_from_format_bar(
        &mut self,
        effect: Effect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.format_bar.open.is_none() || self.read_only(cx) {
            return;
        }
        self.apply_format_effect(effect, window, cx);
        let selection = self.editor.read(cx).selection();
        match &mut self.format_bar.open {
            Some(open) if !selection.is_collapsed() => {
                open.selection = selection;
                cx.notify();
            }
            Some(_) => self.dismiss_format_bar(cx),
            // The effect opened a popup tool.
            None => {}
        }
    }

    /// Drops the menu of a button under it: the colours, or the block
    /// kinds. The menu is the app's one menu widget, which never takes the
    /// keyboard; its rows are the rows of the format menu.
    fn open_format_bar_menu(&mut self, core: Core, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(section), Some(anchor)) = (
            core.menu(),
            self.format_bar.anchor(core).and_then(|anchor| anchor.get()),
        ) else {
            return;
        };
        let editor = self.editor.read(cx);
        let state = editor.state();
        let tools = self.this.clone();
        let row = |label: &'static str, effect: Effect| {
            let tools = tools.clone();
            ContextMenuItem::new(label, move |window, cx| {
                tools
                    .update(cx, |tools, cx| {
                        tools.apply_from_format_bar(effect.clone(), window, cx)
                    })
                    .ok();
            })
        };
        let mut items: Vec<ContextMenuItem> = format_items(state, cx)
            .iter()
            .filter(|item| item.section == section)
            .map(|item| {
                let row = row(item.short, item.effect.clone())
                    .key_hint(item.key_hint.clone())
                    .checked(item.mark == Mark::Check);
                match item.swatch {
                    Some(color) => row.swatch(color),
                    None => row,
                }
            })
            .collect();
        if section == Section::Highlight {
            let none = matches!(state.format_state().highlight, Choice::Same(None));
            items.push(ContextMenuItem::separator());
            items.push(row("None", Effect::NoHighlight).checked(none));
        }
        let focus = editor.focus_handle(cx);
        let menu = ContextMenu::open_for(&focus, MenuAnchor::Below(anchor), items, window, cx);
        let subscription = cx.subscribe(&menu, |this, menu, _: &DismissEvent, cx| {
            if this
                .format_bar
                .menu
                .as_ref()
                .is_some_and(|open| open.menu == menu)
            {
                this.format_bar.menu = None;
                cx.notify();
            }
        });
        self.format_bar.menu = Some(OpenMenu {
            menu,
            _subscription: subscription,
        });
        cx.notify();
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// What a frame does about the bar.
enum Verdict {
    /// Nothing is showing.
    Hidden,
    /// The bar's own menu is still open over a bar that is gone.
    CloseMenu,
    /// Something opened that takes the bar's place, or the selection is
    /// not the bar's any more.
    Dismiss,
    Draw,
}

impl EditorTools {
    fn format_bar_verdict(&self, window: &Window, cx: &App) -> Verdict {
        let bar = &self.format_bar;
        let menu = ContextMenu::current(window, cx);
        let own_menu = bar.menu.as_ref().map(|open| &open.menu);
        let Some(open) = &bar.open else {
            return match (menu, own_menu) {
                (Some(menu), Some(own)) if menu == *own => Verdict::CloseMenu,
                _ => Verdict::Hidden,
            };
        };
        let foreign_menu = menu.is_some_and(|menu| own_menu != Some(&menu));
        if foreign_menu
            || !self.format_bar_allowed(cx)
            || self.editor.read(cx).selection() != open.selection
        {
            return Verdict::Dismiss;
        }
        if open.visible {
            Verdict::Draw
        } else {
            Verdict::Hidden
        }
    }

    /// The layer over the editor that watches the mouse and draws the
    /// bar. A child of the editor's container, after the editor: its
    /// prepaint runs after the editor's, with the geometry of this frame.
    pub(crate) fn render_format_bar_layer(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.format_bar.enabled || self.editor.read(cx).config().mode.is_compact() {
            return None;
        }
        let tools = cx.entity();
        let watcher = cx.entity();
        Some(
            canvas(
                move |bounds, window, cx| {
                    prepaint_format_bar(&tools, window, cx);
                    // The editor's area, to tell a press in the text from a
                    // press on something that lies over it.
                    window.insert_hitbox(bounds, HitboxBehavior::Normal)
                },
                move |_, hitbox, window, _| {
                    let tools = watcher.clone();
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture {
                            let in_text = hitbox.is_hovered(window);
                            tools.update(cx, |tools, cx| {
                                tools.format_bar_pointer_down(event, in_text, window, cx)
                            });
                        }
                    });
                    let tools = watcher.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                            tools.update(cx, |tools, cx| tools.format_bar_pointer_up(window, cx));
                        }
                    });
                    // A release that never arrived (let go outside the
                    // window) shows in the next move without the button.
                    let tools = watcher;
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture
                            && event.pressed_button != Some(MouseButton::Left)
                            && tools.read(cx).format_bar.press.is_some()
                        {
                            tools.update(cx, |tools, cx| tools.format_bar_pointer_up(window, cx));
                        }
                    });
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .into_any_element(),
        )
    }

    /// The bar as an element. `narrow`: the editor is narrow, names are
    /// short.
    fn render_format_bar(&self, narrow: bool, cx: &mut Context<Self>) -> AnyElement {
        let snapshot = self.format_bar_snapshot(cx);
        let specs = self.format_bar_specs(cx);
        let theme = cx.theme();
        let colors = theme.colors;
        let on_fill = selected_row_fill(&colors);
        let extra = self
            .format_bar
            .open
            .as_ref()
            .map(|open| open.extra.as_slice())
            .unwrap_or_default();

        let mut row = div()
            .id("format-bar")
            .debug_selector(|| "format-bar".into())
            // Over the text: a press must not reach the text under the
            // bar, and must not take the keyboard from it.
            .occlude()
            .on_any_mouse_down(|_, window, _| window.prevent_default())
            .flex()
            .items_center()
            .gap(px(1.))
            .p(px(3.))
            .bg(colors.elevated)
            .border_1()
            .border_color(colors.hairline)
            .rounded(RADIUS_LG)
            .shadow(theme.shadow_popover())
            .font_family(theme.fonts.ui.clone())
            .text_size(UI_TEXT_BASE)
            .line_height(px(18.))
            .text_color(colors.ink)
            .whitespace_nowrap();

        let mut group = None;
        for spec in specs {
            if group.is_some_and(|group| group != spec.group) {
                row = row.child(
                    div()
                        .flex_none()
                        .w(px(1.))
                        .h(px(16.))
                        .mx(px(3.))
                        .bg(colors.hairline),
                );
            }
            group = Some(spec.group);

            let on = spec.state == Mark::Check;
            let ink = if !spec.enabled {
                colors.faint
            } else if on {
                colors.accent
            } else {
                colors.ink
            };
            let chevron = || {
                Icon::new(IconName::ChevronDown)
                    .size(px(10.))
                    .color(if spec.enabled {
                        colors.muted
                    } else {
                        colors.faint
                    })
            };
            let letter = |letter: &'static str| {
                div()
                    .font_family(theme.fonts.document.clone())
                    .text_size(px(15.))
                    .child(letter)
            };
            let face: Vec<AnyElement> = match spec.which {
                Which::Core(Core::Bold) => {
                    vec![letter("B").font_weight(FontWeight::BOLD).into_any_element()]
                }
                Which::Core(Core::Italic) => vec![letter("I").italic().into_any_element()],
                Which::Core(Core::Underline) => vec![letter("U").underline().into_any_element()],
                Which::Core(Core::Strikethrough) => {
                    vec![letter("S").line_through().into_any_element()]
                }
                Which::Core(Core::Code) => vec![
                    div()
                        .font_family(theme.fonts.mono.clone())
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .child("<>")
                        .into_any_element(),
                ],
                Which::Core(Core::Highlight) => {
                    // The letter on the marker's colour: the one the
                    // selection has, else the one the key gives.
                    let highlight = match snapshot.highlight {
                        Choice::Same(Some(highlight)) => highlight,
                        _ => Highlight::Yellow,
                    };
                    let chip = HighlightColor::parse(highlight.name())
                        .map(|color| colors.highlight.get(color))
                        .unwrap_or(colors.hover);
                    vec![
                        div()
                            .flex_none()
                            .size(px(18.))
                            .rounded(px(4.))
                            .bg(chip)
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(colors.ink)
                            .when(!spec.enabled, |el| el.opacity(0.45))
                            .child("A")
                            .into_any_element(),
                        chevron().into_any_element(),
                    ]
                }
                Which::Core(Core::Color) => {
                    // The letter over a stroke of the text's colour.
                    let stroke = match snapshot.color {
                        Choice::Same(Some(rgb)) => rgb_color(rgb),
                        _ => colors.ink,
                    };
                    vec![
                        div()
                            .flex_none()
                            .w(px(18.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .line_height(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("A"),
                            )
                            .child(
                                div()
                                    .w(px(12.))
                                    .h(px(3.))
                                    .rounded(px(1.5))
                                    .bg(stroke)
                                    .when(!spec.enabled, |el| el.opacity(0.45)),
                            )
                            .into_any_element(),
                        chevron().into_any_element(),
                    ]
                }
                Which::Core(Core::Link) => {
                    vec![Icon::new(IconName::Link).color(ink).into_any_element()]
                }
                Which::Core(Core::TurnInto) => {
                    let (full, short) = kind_names(snapshot.kind.as_ref());
                    vec![
                        div()
                            .px(px(2.))
                            .child(if narrow { short } else { full })
                            .into_any_element(),
                        chevron().into_any_element(),
                    ]
                }
                Which::Core(Core::More) => {
                    vec![Icon::new(IconName::More).color(ink).into_any_element()]
                }
                Which::Host(index) => match extra.get(index).and_then(|button| button.icon) {
                    Some(icon) => vec![Icon::new(icon).color(ink).into_any_element()],
                    None => vec![
                        div()
                            .px(px(2.))
                            .child(spec.label.clone())
                            .into_any_element(),
                    ],
                },
            };

            let id = spec.id.clone();
            let selector = spec.id.to_string();
            let hover = colors.hover;
            let mut button = div()
                .id(spec.id.clone())
                .debug_selector(move || selector)
                .relative()
                .flex_none()
                .h(px(26.))
                .min_w(px(26.))
                .px(px(4.))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(2.))
                .rounded(RADIUS_SM)
                .text_color(ink)
                .when(on, |el| el.bg(on_fill))
                .tooltip(tip(spec.label.clone(), spec.hint.clone()))
                .children(face);
            if spec.state == Mark::Mixed {
                // "In part of the selection": a short stroke under the
                // face.
                button = button.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom(px(2.))
                        .flex()
                        .justify_center()
                        .child(div().w(px(8.)).h(px(2.)).rounded(px(1.)).bg(colors.accent)),
                );
            }
            if let Which::Core(core) = spec.which
                && let Some(anchor) = self.format_bar.anchor(core)
            {
                // Where the button is, for the menu that drops from it.
                let anchor = anchor.clone();
                button = button.child(
                    canvas(move |at, _, _| anchor.set(Some(at)), |_, _, _, _| {})
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                );
            }
            if spec.enabled {
                button = button
                    .cursor_pointer()
                    .when(!on, |el| el.hover(move |style| style.bg(hover)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.press_format_bar_button(&id, window, cx);
                    }));
            }
            row = row.child(button);
        }
        row.into_any_element()
    }
}

/// The bar's part of a frame's prepaint: decides whether it shows, builds
/// it, places it against the editor's geometry of this frame and hands it
/// to the window to be drawn on top.
fn prepaint_format_bar(tools: &Entity<EditorTools>, window: &mut Window, cx: &mut App) {
    let this = tools.read(cx);
    let drawn = this.format_bar.bounds.clone();
    match this.format_bar_verdict(window, cx) {
        Verdict::Draw => {}
        Verdict::Hidden => return drawn.set(None),
        Verdict::Dismiss => {
            drawn.set(None);
            // State changes wait for the end of the frame.
            let tools = tools.downgrade();
            return window.defer(cx, move |_, cx| {
                tools
                    .update(cx, |tools, cx| tools.dismiss_format_bar(cx))
                    .ok();
            });
        }
        Verdict::CloseMenu => {
            drawn.set(None);
            let tools = tools.downgrade();
            return window.defer(cx, move |window, cx| {
                let own = tools
                    .read_with(cx, |tools, _| tools.format_bar_menu().cloned())
                    .ok()
                    .flatten();
                if own.is_some() && ContextMenu::current(window, cx) == own {
                    ContextMenu::dismiss(window, cx);
                }
            });
        }
    }
    let editor = this.editor.read(cx);
    let geometry = editor
        .range_bounds(editor.selection().range())
        .and_then(|rows| {
            let first = *rows.first()?;
            let all = rows.iter().fold(first, |all, row| all.union(row));
            Some((first, all, editor.viewport_bounds()?))
        });
    let Some((first, all, viewport)) = geometry else {
        return drawn.set(None);
    };
    let narrow = viewport.size.width < NARROW;
    let mut bar = tools.update(cx, |tools, cx| tools.render_format_bar(narrow, cx));
    let size = bar.layout_as_root(AvailableSpace::min_size(), window, cx);
    match place_format_bar(first, all, size, viewport) {
        Some(origin) => {
            drawn.set(Some(Bounds::new(origin, size)));
            window.defer_draw(bar, origin, DRAW_PRIORITY);
        }
        None => drawn.set(None),
    }
}
