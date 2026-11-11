//! [`ContextMenu`]: the one menu widget of the app. Right-click menus, the
//! menus that drop down from a button, and the menu at the text caret
//! (`ctrl-enter`) are all this.
//!
//! # Hosting: nothing to mount in the app
//!
//! The open menu is drawn by a [`ContextMenuLayer`] element. **[`crate::ToastLayer`]
//! contains one**, so a window whose root view mounts the toast layer (the app
//! does) needs nothing else: one call opens a menu from anywhere. A window
//! without a toast layer (a test window of another crate, an example) mounts
//! the layer itself, once, anywhere in its root view:
//!
//! ```ignore
//! div().size_full().child(content).child(writer_ui::ContextMenuLayer)
//! ```
//!
//! Mounting it twice is harmless (the first layer drawn in a frame draws the
//! menu). Without any layer the menu is open (keys work, [`ContextMenu::is_open`]
//! is true, [`ContextMenu::choose`] works) but nothing is drawn.
//!
//! # Opening
//!
//! ```ignore
//! use gpui::{MouseButton, MouseDownEvent};
//! use writer_ui::{ContextMenu, ContextMenuItem, IconName};
//!
//! // a row with a right-click menu: an action item, a submenu, a closure item
//! div()
//!     .id(("row", ix))
//!     .child(name)
//!     .on_mouse_down(
//!         MouseButton::Right,
//!         cx.listener(move |this, event: &MouseDownEvent, window, cx| {
//!             // put the keyboard where the actions must arrive, select what was clicked
//!             window.focus(&this.focus_handle);
//!             this.select(ix, cx);
//!             let folders = this.folders.clone();
//!             let panel = cx.weak_entity();
//!             ContextMenu::open(
//!                 event.position,
//!                 vec![
//!                     // dispatches `Rename` to the focused element, shows its key from the keymap
//!                     ContextMenuItem::action("Rename", Rename).icon(IconName::Document),
//!                     // one line per existing command descriptor
//!                     ContextMenuItem::command(&duplicate_descriptor),
//!                     ContextMenuItem::submenu(
//!                         "Move to",
//!                         folders
//!                             .iter()
//!                             .map(|folder| {
//!                                 ContextMenuItem::action(folder.name.clone(), MoveTo { folder: folder.id })
//!                             })
//!                             .collect(),
//!                     ),
//!                     ContextMenuItem::separator(),
//!                     ContextMenuItem::new("Reveal in Finder", move |_window, cx| {
//!                         panel.update(cx, |panel, cx| panel.reveal(ix, cx)).ok();
//!                     }),
//!                     ContextMenuItem::action("Delete", Delete).danger(),
//!                 ],
//!                 window,
//!                 cx,
//!             );
//!         }),
//!     )
//! ```
//!
//! [`ContextMenu::open`] takes a window position (`event.position`) or the
//! bounds of what the menu belongs to (`Bounds<Pixels>`: the menu drops down
//! below a button or the caret, see [`MenuAnchor`]). The menu stays inside
//! the window: it flips to the other side of its anchor when it does not fit
//! and is clamped to the window edges; a menu taller than the window scrolls.
//! Opening a menu closes the one that is open: there is one menu per window.
//!
//! From the keyboard a menu opens on `ctrl-enter`. This crate defines the
//! action, [`OpenContextMenu`], and binds the key for its own text fields; a
//! crate with objects of its own binds `ctrl-enter` to the same action in its
//! key context and opens its menu anchored to the selected object:
//!
//! ```ignore
//! KeyBinding::new("ctrl-enter", writer_ui::context_menu::OpenContextMenu, Some("Library"));
//! .on_action(cx.listener(|this, _: &OpenContextMenu, window, cx| {
//!     let menu = ContextMenu::open(this.selected_row_bounds(), this.menu_items(cx), window, cx);
//!     menu.update(cx, |menu, cx| menu.select_first(cx));   // keyboard: start on the first item
//! }))
//! ```
//!
//! # The keyboard stays where it was
//!
//! The menu never takes the focus. While it is open it sees every key of
//! its window first (one `App::intercept_keystrokes` callback installed by
//! [`crate::init`]), so the element that had the keyboard keeps it: a rename
//! field does not commit, a popup that closes on blur stays open, the text
//! selection stays highlighted, and closing the menu needs no focus restore.
//! An action item is dispatched to the focused element exactly like its key
//! binding would be (or to an explicit target, [`ContextMenu::open_for`]).
//!
//! Keys while a menu is open: `up` / `down` (also `ctrl-p` / `ctrl-n`) move,
//! `home` / `end` (`pageup` / `pagedown`) jump, `enter` or `space` chooses,
//! `right` or `enter` opens a submenu, `left` or `escape` leaves it, `escape`
//! closes the menu, letters jump to the next item that starts with what was
//! typed. A `cmd` chord closes the menu and then does what it always does.
//! Every other key is swallowed.
//!
//! The mouse: moving over an item marks it, a click chooses it, a submenu
//! opens after a short rest on its row (or at once on a click). A click
//! outside closes the menu and does nothing else (as native menus do); a
//! right-click outside closes it and goes on to what was clicked, so that
//! place opens its own menu. Scrolling outside the menu and the window
//! losing its active state close it too.
//!
//! # Items
//!
//! | Constructor | What |
//! |---|---|
//! | [`ContextMenuItem::action`] | dispatches a GPUI action; shows the key bound to it |
//! | [`ContextMenuItem::command`] | the same from a [`CommandDescriptor`] |
//! | [`ContextMenuItem::new`] | runs a closure |
//! | [`ContextMenuItem::submenu`] | opens a second menu (one level) |
//! | [`ContextMenuItem::separator`] | a hairline |
//! | [`ContextMenuItem::section`] | a small heading above a group |
//!
//! Modifiers: [`icon`](ContextMenuItem::icon), [`swatch`](ContextMenuItem::swatch)
//! (a colour chip before the title), [`checked`](ContextMenuItem::checked)
//! (a check mark column), [`disabled`](ContextMenuItem::disabled) (shown
//! greyed, cannot be chosen), [`danger`](ContextMenuItem::danger) (destructive
//! actions), [`key_hint`](ContextMenuItem::key_hint) (for closure items).
//! Items are `Clone`, so a crate can build a list and let another crate add
//! to it before it is opened.
//!
//! # Tests and scripting
//!
//! [`ContextMenu::is_open`], [`ContextMenu::current`], [`ContextMenu::labels`],
//! [`ContextMenu::item_bounds`] (where to click) and [`ContextMenu::choose`]
//! (choose an item by its label, as a click would).

use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    Action, AnyElement, App, Bounds, Context, DismissEvent, DispatchPhase, Element, ElementId,
    Entity, EventEmitter, FocusHandle, GlobalElementId, Hsla, InspectorElementId, Keystroke,
    LayoutId, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point, Position, ScrollHandle,
    ScrollWheelEvent, SharedString, Size, Style, Subscription, Task, Window, WindowId, actions,
    deferred, div, point, prelude::*, px, size,
};

use crate::{
    command::CommandDescriptor,
    icon::{Icon, IconName},
    key_hint::KeyHint,
    theme::{ActiveTheme, RADIUS_LG, RADIUS_SM, UI_TEXT_BASE, UI_TEXT_XS},
};

actions!(
    context_menu,
    [
        /// Open the context menu of what has the keyboard (`ctrl-enter`).
        OpenContextMenu
    ]
);

/// Space kept between a menu and the window edges.
pub const WINDOW_MARGIN: Pixels = px(6.);
/// Height of an item row.
pub const ROW_HEIGHT: Pixels = px(24.);
/// Height of a separator.
pub const SEPARATOR_HEIGHT: Pixels = px(9.);
/// Height of a section heading.
pub const SECTION_HEIGHT: Pixels = px(22.);
/// How long the pointer rests on a submenu row before the submenu opens
/// (and on another row before an open submenu closes).
pub const SUBMENU_DELAY: Duration = Duration::from_millis(180);

const PANEL_PADDING: Pixels = px(4.);
const PANEL_BORDER: Pixels = px(1.);
const ANCHOR_GAP: Pixels = px(4.);
const SUBMENU_OVERLAP: Pixels = px(3.);
const TYPE_AHEAD_PAUSE: Duration = Duration::from_millis(900);
const DRAW_PRIORITY: usize = 10_000;

// ---------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------

type Handler = Rc<dyn Fn(&mut Window, &mut App)>;

enum ItemKind {
    Action(Box<dyn Action>),
    Handler(Handler),
    Submenu(Vec<ContextMenuItem>),
    Separator,
    Section,
}

impl Clone for ItemKind {
    fn clone(&self) -> Self {
        match self {
            ItemKind::Action(action) => ItemKind::Action(action.boxed_clone()),
            ItemKind::Handler(handler) => ItemKind::Handler(handler.clone()),
            ItemKind::Submenu(items) => ItemKind::Submenu(items.clone()),
            ItemKind::Separator => ItemKind::Separator,
            ItemKind::Section => ItemKind::Section,
        }
    }
}

/// One entry of a [`ContextMenu`]. See the [module docs](self).
#[derive(Clone)]
pub struct ContextMenuItem {
    kind: ItemKind,
    title: SharedString,
    icon: Option<IconName>,
    swatch: Option<Hsla>,
    checked: Option<bool>,
    disabled: bool,
    danger: bool,
    key_hint: Option<KeyHint>,
}

impl ContextMenuItem {
    fn with_kind(title: impl Into<SharedString>, kind: ItemKind) -> Self {
        Self {
            kind,
            title: title.into(),
            icon: None,
            swatch: None,
            checked: None,
            disabled: false,
            danger: false,
            key_hint: None,
        }
    }

    /// An item that dispatches `action` to the element that has the keyboard
    /// (or to the menu's target, see [`ContextMenu::open_for`]), exactly as
    /// the action's key binding does. The key bound to the action is shown
    /// at the right, looked up in the keymap.
    pub fn action(title: impl Into<SharedString>, action: impl Action) -> Self {
        Self::action_boxed(title, Box::new(action))
    }

    /// Like [`ContextMenuItem::action`] for an action that is already boxed.
    pub fn action_boxed(title: impl Into<SharedString>, action: Box<dyn Action>) -> Self {
        Self::with_kind(title, ItemKind::Action(action))
    }

    /// The item of an existing command: its title, its action, its key.
    ///
    /// ```ignore
    /// let items: Vec<_> = commands().iter().map(ContextMenuItem::command).collect();
    /// ```
    pub fn command(command: &CommandDescriptor) -> Self {
        Self::action_boxed(command.title.clone(), command.action())
    }

    /// An item that runs `handler` after the menu has closed.
    pub fn new(
        title: impl Into<SharedString>,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self::with_kind(title, ItemKind::Handler(Rc::new(handler)))
    }

    /// An item that opens `items` as a second menu beside it. One level:
    /// submenus inside `items` are shown as plain disabled rows.
    pub fn submenu(title: impl Into<SharedString>, items: Vec<ContextMenuItem>) -> Self {
        Self::with_kind(title, ItemKind::Submenu(items))
    }

    /// A hairline between groups. Separators at the start, at the end and
    /// next to each other are dropped when the menu opens, so a list can be
    /// put together from optional groups without bookkeeping.
    pub fn separator() -> Self {
        Self::with_kind("", ItemKind::Separator)
    }

    /// A small heading above a group of items.
    pub fn section(title: impl Into<SharedString>) -> Self {
        Self::with_kind(title, ItemKind::Section)
    }

    /// An icon in the leading column.
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// A small chip of `color` before the title: the item stands for that
    /// colour (a highlight, a text colour). It is drawn beside the leading
    /// column, so a colour item can be a checkable one too.
    pub fn swatch(mut self, color: Hsla) -> Self {
        self.swatch = Some(color);
        self
    }

    /// Makes the item a checkable one: a check mark in the leading column
    /// when `checked` (the column is kept free in the whole menu).
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    /// Shows the item greyed; it cannot be chosen and the keyboard skips it.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The style of a destructive action ("Delete forever").
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    /// Shows this key instead of the one looked up for the action (and gives
    /// a closure item a key to show). Accepts an `Option` so
    /// `KeyHint::for_action(..)` can be passed directly.
    pub fn key_hint(mut self, hint: impl Into<Option<KeyHint>>) -> Self {
        self.key_hint = hint.into();
        self
    }

    /// The text of the item (empty for a separator).
    pub fn title(&self) -> &SharedString {
        &self.title
    }

    pub fn is_separator(&self) -> bool {
        matches!(self.kind, ItemKind::Separator)
    }

    pub fn is_section(&self) -> bool {
        matches!(self.kind, ItemKind::Section)
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub fn is_danger(&self) -> bool {
        self.danger
    }

    /// The colour of the item's chip ([`ContextMenuItem::swatch`]).
    pub fn swatch_color(&self) -> Option<Hsla> {
        self.swatch
    }

    /// `Some(true)` / `Some(false)` for a checkable item.
    pub fn is_checked(&self) -> Option<bool> {
        self.checked
    }

    /// The registered name of the item's action (`"editor::ToggleBold"`).
    pub fn action_name(&self) -> Option<&'static str> {
        match &self.kind {
            ItemKind::Action(action) => Some(action.name()),
            _ => None,
        }
    }

    /// The item's action, borrowed.
    pub fn action_ref(&self) -> Option<&dyn Action> {
        match &self.kind {
            ItemKind::Action(action) => Some(action.as_ref()),
            _ => None,
        }
    }

    /// The items of a submenu.
    pub fn submenu_items(&self) -> Option<&[ContextMenuItem]> {
        match &self.kind {
            ItemKind::Submenu(items) => Some(items),
            _ => None,
        }
    }

    /// Whether the keyboard can land on the item and a click chooses it.
    fn selectable(&self) -> bool {
        !self.disabled
            && matches!(
                self.kind,
                ItemKind::Action(_) | ItemKind::Handler(_) | ItemKind::Submenu(_)
            )
    }

    fn height(&self) -> Pixels {
        match self.kind {
            ItemKind::Separator => SEPARATOR_HEIGHT,
            ItemKind::Section => SECTION_HEIGHT,
            _ => ROW_HEIGHT,
        }
    }
}

impl From<&CommandDescriptor> for ContextMenuItem {
    fn from(command: &CommandDescriptor) -> Self {
        Self::command(command)
    }
}

impl std::fmt::Debug for ContextMenuItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match &self.kind {
            ItemKind::Action(action) => action.name(),
            ItemKind::Handler(_) => "closure",
            ItemKind::Submenu(_) => "submenu",
            ItemKind::Separator => "separator",
            ItemKind::Section => "section",
        };
        f.debug_struct("ContextMenuItem")
            .field("title", &self.title)
            .field("kind", &kind)
            .field("disabled", &self.disabled)
            .field("checked", &self.checked)
            .finish()
    }
}

/// Drops separators that separate nothing and flattens submenus below the
/// first level.
fn normalize(items: Vec<ContextMenuItem>, nested: bool) -> Vec<ContextMenuItem> {
    let mut out: Vec<ContextMenuItem> = Vec::with_capacity(items.len());
    for mut item in items {
        match &mut item.kind {
            ItemKind::Separator => {
                if out.last().is_none_or(|last| last.is_separator()) {
                    continue;
                }
            }
            ItemKind::Submenu(children) => {
                if nested {
                    item.disabled = true;
                    children.clear();
                } else {
                    *children = normalize(std::mem::take(children), true);
                    if children.is_empty() {
                        item.disabled = true;
                    }
                }
            }
            _ => {}
        }
        out.push(item);
    }
    while out.last().is_some_and(|last| last.is_separator()) {
        out.pop();
    }
    out
}

// ---------------------------------------------------------------------------
// Placement
// ---------------------------------------------------------------------------

/// Where a menu opens. `Point<Pixels>` converts into [`MenuAnchor::Point`]
/// and `Bounds<Pixels>` into [`MenuAnchor::Below`], so [`ContextMenu::open`]
/// takes either directly. All coordinates are window coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuAnchor {
    /// The pointer: the menu's top left corner is here (its other corners
    /// when it does not fit to the right or below).
    Point(Point<Pixels>),
    /// A button, a field, the text caret: the menu drops down below these
    /// bounds, left edges aligned, and goes above them when there is no room
    /// below.
    Below(Bounds<Pixels>),
    /// A row: the menu opens to the right of these bounds, or to the left
    /// when there is no room (how a submenu sits beside its item).
    Beside(Bounds<Pixels>),
}

impl From<Point<Pixels>> for MenuAnchor {
    fn from(position: Point<Pixels>) -> Self {
        MenuAnchor::Point(position)
    }
}

impl From<Bounds<Pixels>> for MenuAnchor {
    fn from(bounds: Bounds<Pixels>) -> Self {
        MenuAnchor::Below(bounds)
    }
}

/// The top left corner of a menu of `menu` size opened at `anchor` in a
/// window of `viewport` size: at the anchor when it fits, flipped to the
/// anchor's other side when it does not, and in any case inside the window
/// with `margin` to its edges (a menu larger than the window starts at the
/// margin).
pub fn place(
    anchor: MenuAnchor,
    menu: Size<Pixels>,
    viewport: Size<Pixels>,
    margin: Pixels,
) -> Point<Pixels> {
    let max_x = viewport.width - margin - menu.width;
    let max_y = viewport.height - margin - menu.height;
    let clamp = |value: Pixels, max: Pixels| value.min(max).max(margin);
    let (x, y) = match anchor {
        MenuAnchor::Point(at) => {
            let x = if at.x > max_x && at.x - menu.width >= margin {
                at.x - menu.width
            } else {
                at.x
            };
            let y = if at.y > max_y && at.y - menu.height >= margin {
                at.y - menu.height
            } else {
                at.y
            };
            (x, y)
        }
        MenuAnchor::Below(bounds) => {
            let below = bounds.bottom() + ANCHOR_GAP;
            let above = bounds.top() - ANCHOR_GAP - menu.height;
            let y = if below > max_y && above >= margin {
                above
            } else {
                below
            };
            (bounds.left(), y)
        }
        MenuAnchor::Beside(bounds) => {
            let right = bounds.right() - SUBMENU_OVERLAP;
            let left = bounds.left() + SUBMENU_OVERLAP - menu.width;
            let x = if right > max_x && left >= margin {
                left
            } else {
                right
            };
            // the first row of the menu level with the row it belongs to
            (x, bounds.top() - PANEL_PADDING - PANEL_BORDER)
        }
    };
    point(clamp(x, max_x), clamp(y, max_y))
}

type SharedBounds = Rc<Cell<Option<Bounds<Pixels>>>>;

/// What a [`Placed`] panel is positioned against.
enum PlacedAt {
    Anchor(MenuAnchor),
    /// Beside the row of the root panel that starts `row_top` below the
    /// panel's top edge.
    RootRow {
        root: SharedBounds,
        row_top: Pixels,
    },
}

/// Lays its child out, then moves it to where [`place`] puts a menu of that
/// size, and records where that is. The root panel's element also installs
/// the window-wide listeners that close the menu.
struct Placed {
    at: PlacedAt,
    child: AnyElement,
    bounds: SharedBounds,
    /// The bounds of every open panel (for "outside"), when this element
    /// installs the closing listeners.
    watch: Option<Vec<SharedBounds>>,
}

impl IntoElement for Placed {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Placed {
    type RequestLayoutState = LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let child = self.child.request_layout(window, cx);
        let style = Style {
            position: Position::Absolute,
            ..Style::default()
        };
        (window.request_layout(style, [child], cx), child)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let laid_out = window.layout_bounds(*child);
        let anchor = match &self.at {
            PlacedAt::Anchor(anchor) => Some(*anchor),
            PlacedAt::RootRow { root, row_top } => root.get().map(|root| {
                MenuAnchor::Beside(Bounds::new(
                    point(root.left(), root.top() + *row_top),
                    size(root.size.width, ROW_HEIGHT),
                ))
            }),
        };
        let origin = match anchor {
            Some(anchor) => place(anchor, laid_out.size, window.viewport_size(), WINDOW_MARGIN),
            None => laid_out.origin,
        };
        let origin = point(origin.x.round(), origin.y.round());
        self.bounds.set(Some(Bounds::new(origin, laid_out.size)));
        window.with_element_offset(origin - laid_out.origin, |window| {
            self.child.prepaint(window, cx)
        });
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _child: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
        let Some(watch) = self.watch.take() else {
            return;
        };
        // Before the listeners below: a press first forgets a release that
        // never came, then may ask for its own to be eaten.
        install_mouse_up_swallow(window);
        let inside = move |position: Point<Pixels>| {
            watch
                .iter()
                .any(|bounds| bounds.get().is_some_and(|b| b.contains(&position)))
        };
        window.on_mouse_event({
            let inside = inside.clone();
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || inside(event.position) {
                    return;
                }
                ContextMenu::dismiss(window, cx);
                if event.button == MouseButton::Left {
                    // A click outside closes the menu and nothing else; the
                    // release that follows must not reach what is there.
                    swallow_next_mouse_up(window, cx);
                    cx.stop_propagation();
                }
            }
        });
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase == DispatchPhase::Capture && !inside(event.position) {
                ContextMenu::dismiss(window, cx);
            }
        });
    }
}

// ---------------------------------------------------------------------------
// The registry: one open menu per window
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Registry {
    open: HashMap<WindowId, Entity<ContextMenu>>,
    /// Windows in which the release of a click that closed a menu is still
    /// to come.
    swallow_mouse_up: HashSet<WindowId>,
    /// Windows whose menu a layer has drawn in the frame being built.
    drawn: HashSet<WindowId>,
    interceptor: bool,
}

impl gpui::Global for Registry {}

fn window_id(window: &Window) -> WindowId {
    window.window_handle().window_id()
}

fn swallow_next_mouse_up(window: &Window, cx: &mut App) {
    cx.default_global::<Registry>()
        .swallow_mouse_up
        .insert(window_id(window));
}

/// Registers, for this frame, the listener that eats the mouse release
/// following a click that closed a menu. A release that never arrives (the
/// button was let go outside the window) is forgotten at the next press.
fn install_mouse_up_swallow(window: &mut Window) {
    window.on_mouse_event(|_: &MouseDownEvent, phase, window, cx| {
        if phase == DispatchPhase::Capture
            && let Some(registry) = cx.try_global::<Registry>()
            && registry.swallow_mouse_up.contains(&window_id(window))
        {
            let id = window_id(window);
            cx.global_mut::<Registry>().swallow_mouse_up.remove(&id);
        }
    });
    window.on_mouse_event(|event: &MouseUpEvent, phase, window, cx| {
        if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
            return;
        }
        let id = window_id(window);
        if cx
            .try_global::<Registry>()
            .is_some_and(|registry| registry.swallow_mouse_up.contains(&id))
        {
            cx.global_mut::<Registry>().swallow_mouse_up.remove(&id);
            cx.stop_propagation();
        }
    });
}

/// Installs the key interception of open menus. [`crate::init`] calls it at
/// startup (before any other interceptor is registered, so an open menu is
/// asked first); opening a menu installs it too if that has not happened.
pub fn init(cx: &mut App) {
    if std::mem::replace(&mut cx.default_global::<Registry>().interceptor, true) {
        return;
    }
    cx.intercept_keystrokes(|event, window, cx| {
        let Some(menu) = ContextMenu::current(window, cx) else {
            return;
        };
        let keystroke = &event.keystroke;
        if is_modifier_only(keystroke) {
            return;
        }
        if keystroke.modifiers.platform {
            // A cmd chord closes the menu and goes on to do what it does.
            ContextMenu::dismiss(window, cx);
            return;
        }
        cx.stop_propagation();
        // Interceptors registered after this one still see the key, and one
        // of them may act on it when "its" element has the keyboard (the
        // editor's does). For the rest of this key the window has no focused
        // element; it is back before anything is drawn or dispatched, so
        // nothing observes a blur.
        let focused = window.focused(cx);
        if let Some(focused) = focused.clone() {
            window.blur();
            window.defer(cx, move |window, cx| {
                if window.focused(cx).is_none() {
                    window.focus(&focused);
                }
            });
        }
        menu.update(cx, |menu, cx| {
            menu.handle_keystroke(keystroke, focused, window, cx)
        });
    })
    .detach();
}

fn is_modifier_only(keystroke: &Keystroke) -> bool {
    matches!(
        keystroke.key.as_str(),
        "" | "shift" | "control" | "alt" | "platform" | "function"
    )
}

// ---------------------------------------------------------------------------
// The menu
// ---------------------------------------------------------------------------

struct OpenSubmenu {
    /// Index of the submenu item in the root list.
    parent: usize,
    selected: Option<usize>,
}

/// The open menu of a window. See the [module docs](self).
///
/// Emits [`DismissEvent`] when it closes, whether an item was chosen or not.
pub struct ContextMenu {
    items: Vec<ContextMenuItem>,
    anchor: MenuAnchor,
    target: Option<FocusHandle>,
    window: WindowId,
    selected: Option<usize>,
    submenu: Option<OpenSubmenu>,
    hover_task: Option<Task<()>>,
    type_ahead: String,
    type_ahead_at: Option<Instant>,
    scroll: ScrollHandle,
    bounds: SharedBounds,
    submenu_bounds: SharedBounds,
    _activation: Subscription,
}

impl EventEmitter<DismissEvent> for ContextMenu {}

impl ContextMenu {
    /// Opens a menu with `items` at `anchor` (a window position, or the
    /// bounds of what it belongs to) and returns it. A menu that is open in
    /// this window is closed first. Action items go to the element that has
    /// the keyboard when the item is chosen.
    pub fn open(
        anchor: impl Into<MenuAnchor>,
        items: Vec<ContextMenuItem>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<ContextMenu> {
        Self::open_with(anchor.into(), items, None, window, cx)
    }

    /// Like [`ContextMenu::open`], with action items dispatched to `target`
    /// (as if it had the keyboard) instead of the focused element. `target`
    /// must be drawn (`track_focus`) when the item is chosen.
    pub fn open_for(
        target: &FocusHandle,
        anchor: impl Into<MenuAnchor>,
        items: Vec<ContextMenuItem>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<ContextMenu> {
        Self::open_with(anchor.into(), items, Some(target.clone()), window, cx)
    }

    fn open_with(
        anchor: MenuAnchor,
        items: Vec<ContextMenuItem>,
        target: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<ContextMenu> {
        init(cx);
        Self::dismiss(window, cx);
        let id = window_id(window);
        let mut items = normalize(items, false);
        if items.is_empty() {
            items.push(ContextMenuItem::section("No actions here"));
        }
        let menu = cx.new(|cx| ContextMenu {
            items,
            anchor,
            target,
            window: id,
            selected: None,
            submenu: None,
            hover_task: None,
            type_ahead: String::new(),
            type_ahead_at: None,
            scroll: ScrollHandle::new(),
            bounds: SharedBounds::default(),
            submenu_bounds: SharedBounds::default(),
            _activation: cx.observe_window_activation(window, |this: &mut Self, window, cx| {
                if !window.is_window_active() {
                    this.close(window, cx);
                }
            }),
        });
        cx.default_global::<Registry>()
            .open
            .insert(id, menu.clone());
        window.refresh();
        menu
    }

    /// The menu that is open in `window`.
    pub fn current(window: &Window, cx: &App) -> Option<Entity<ContextMenu>> {
        cx.try_global::<Registry>()?
            .open
            .get(&window_id(window))
            .cloned()
    }

    /// Whether a menu is open in `window`.
    pub fn is_open(window: &Window, cx: &App) -> bool {
        Self::current(window, cx).is_some()
    }

    /// Closes the menu of `window`, if one is open. Returns whether one was.
    /// Not for use inside an update of the menu itself.
    pub fn dismiss(window: &mut Window, cx: &mut App) -> bool {
        let Some(menu) = Self::current(window, cx) else {
            return false;
        };
        menu.update(cx, |menu, cx| menu.close(window, cx));
        true
    }

    /// Chooses the item titled `title` of the open menu, as a click on it
    /// would: the root items are searched first, then the items of every
    /// submenu. Returns `false` when no menu is open or it has no such item
    /// that can be chosen. For tests and scripted runs.
    pub fn choose(title: &str, window: &mut Window, cx: &mut App) -> bool {
        let Some(menu) = Self::current(window, cx) else {
            return false;
        };
        let focused = window.focused(cx);
        menu.update(cx, |menu, cx| {
            let root = menu.items.iter();
            let nested = menu
                .items
                .iter()
                .filter(|item| !item.disabled)
                .filter_map(|item| item.submenu_items())
                .flatten();
            let found = root
                .chain(nested)
                .find(|item| {
                    item.title.as_ref() == title
                        && item.selectable()
                        && item.submenu_items().is_none()
                })
                .cloned();
            match found {
                Some(item) => {
                    menu.run(item, focused, window, cx);
                    true
                }
                None => false,
            }
        })
    }

    // -- reading ----------------------------------------------------------

    /// The items of the menu (after dropping needless separators).
    pub fn items(&self) -> &[ContextMenuItem] {
        &self.items
    }

    /// The titles of the root items, `"-"` for a separator. For tests.
    pub fn labels(&self) -> Vec<String> {
        self.items
            .iter()
            .map(|item| {
                if item.is_separator() {
                    "-".to_string()
                } else {
                    item.title.to_string()
                }
            })
            .collect()
    }

    /// Where the menu opened.
    pub fn anchor(&self) -> MenuAnchor {
        self.anchor
    }

    /// Index of the marked root item.
    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    /// The marked item of the level that has the keyboard (the submenu's
    /// when one is open).
    pub fn selected_item(&self) -> Option<&ContextMenuItem> {
        match &self.submenu {
            Some(submenu) => self.level_items(true).get(submenu.selected?),
            None => self.items.get(self.selected?),
        }
    }

    /// Index of the root item whose submenu is open.
    pub fn open_submenu(&self) -> Option<usize> {
        self.submenu.as_ref().map(|submenu| submenu.parent)
    }

    /// Index of the marked item in the open submenu.
    pub fn submenu_selected_index(&self) -> Option<usize> {
        self.submenu.as_ref()?.selected
    }

    /// Where the menu was drawn, in window coordinates (`None` before the
    /// first frame and without a [`ContextMenuLayer`]).
    pub fn bounds(&self) -> Option<Bounds<Pixels>> {
        self.bounds.get()
    }

    /// Where the open submenu was drawn.
    pub fn submenu_bounds(&self) -> Option<Bounds<Pixels>> {
        self.submenu.as_ref()?;
        self.submenu_bounds.get()
    }

    /// Where root item `ix` was drawn (click its centre to choose it).
    pub fn item_bounds(&self, ix: usize) -> Option<Bounds<Pixels>> {
        row_bounds(&self.items, ix, self.bounds.get()?, self.scroll.offset().y)
    }

    /// Where item `ix` of the open submenu was drawn.
    pub fn submenu_item_bounds(&self, ix: usize) -> Option<Bounds<Pixels>> {
        let panel = self.submenu_bounds()?;
        row_bounds(self.level_items(true), ix, panel, px(0.))
    }

    // -- changing ---------------------------------------------------------

    /// Marks the first item that can be chosen. Call it after opening a menu
    /// from the keyboard, so `enter` has something to choose.
    pub fn select_first(&mut self, cx: &mut Context<Self>) {
        let in_submenu = self.submenu.is_some();
        let first = first_selectable(self.level_items(in_submenu));
        self.set_selected(in_submenu, first, cx);
    }

    fn level_items(&self, submenu: bool) -> &[ContextMenuItem] {
        if submenu
            && let Some(open) = &self.submenu
            && let Some(items) = self.items.get(open.parent).and_then(|i| i.submenu_items())
        {
            return items;
        }
        &self.items
    }

    fn level_selected(&self, submenu: bool) -> Option<usize> {
        match (&self.submenu, submenu) {
            (Some(open), true) => open.selected,
            _ => self.selected,
        }
    }

    fn set_selected(&mut self, submenu: bool, ix: Option<usize>, cx: &mut Context<Self>) {
        match (&mut self.submenu, submenu) {
            (Some(open), true) => open.selected = ix,
            _ => {
                self.selected = ix;
                if let Some(ix) = ix {
                    self.scroll.scroll_to_item(ix);
                }
            }
        }
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity_id();
        let registry = cx.default_global::<Registry>();
        let is_current = registry
            .open
            .get(&self.window)
            .is_some_and(|menu| menu.entity_id() == this);
        if !is_current {
            return;
        }
        registry.open.remove(&self.window);
        self.hover_task = None;
        cx.emit(DismissEvent);
        // The registry held the menu; whoever subscribed without keeping it
        // still hears that it closed (an event of a dropped entity is not
        // delivered).
        let alive = cx.entity();
        cx.defer(move |_| drop(alive));
        window.refresh();
    }

    /// Closes the menu and runs `item`.
    fn run(
        &mut self,
        item: ContextMenuItem,
        focused: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.target.clone();
        self.close(window, cx);
        // Deferred: the menu is being updated here, and what runs may open
        // another menu or a modal.
        match item.kind {
            ItemKind::Action(action) => window.defer(cx, move |window, cx| match target {
                Some(target) => target.dispatch_action(action.as_ref(), window, cx),
                None => match focused {
                    // not `window.dispatch_action`: the focus may be set
                    // aside for the key that chose the item (see `init`)
                    Some(focused) if !focused.is_focused(window) => {
                        focused.dispatch_action(action.as_ref(), window, cx)
                    }
                    _ => window.dispatch_action(action, cx),
                },
            }),
            ItemKind::Handler(handler) => {
                window.defer(cx, move |window, cx| handler(window, cx));
            }
            ItemKind::Submenu(_) | ItemKind::Separator | ItemKind::Section => {}
        }
    }

    fn open_submenu_at(&mut self, parent: usize, select_first: bool, cx: &mut Context<Self>) {
        let Some(items) = self
            .items
            .get(parent)
            .and_then(|item| item.selectable().then(|| item.submenu_items()).flatten())
        else {
            return;
        };
        let selected = if select_first {
            first_selectable(items)
        } else {
            None
        };
        if self.submenu.as_ref().map(|open| open.parent) != Some(parent) {
            self.submenu_bounds.set(None);
        }
        self.submenu = Some(OpenSubmenu { parent, selected });
        self.selected = Some(parent);
        self.hover_task = None;
        cx.notify();
    }

    fn close_submenu(&mut self, cx: &mut Context<Self>) {
        if self.submenu.take().is_some() {
            self.submenu_bounds.set(None);
            cx.notify();
        }
    }

    /// Chooses the marked item of a level: opens a submenu, runs anything
    /// else.
    fn confirm(
        &mut self,
        submenu: bool,
        ix: usize,
        from_keyboard: bool,
        focused: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.level_items(submenu).get(ix).cloned() else {
            return;
        };
        if !item.selectable() {
            return;
        }
        if item.submenu_items().is_some() {
            if !submenu {
                self.open_submenu_at(ix, from_keyboard, cx);
            }
            return;
        }
        self.run(item, focused, window, cx);
    }

    // -- keyboard ---------------------------------------------------------

    fn handle_keystroke(
        &mut self,
        keystroke: &Keystroke,
        focused: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = keystroke.modifiers;
        let in_submenu = self.submenu.is_some();
        let plain = !modifiers.control && !modifiers.alt && !modifiers.function;
        let key = keystroke.key.as_str();
        match key {
            "down" | "up" if plain => self.move_selection(in_submenu, key == "down", cx),
            "n" | "p" if modifiers.control && !modifiers.alt => {
                self.move_selection(in_submenu, key == "n", cx)
            }
            "home" | "pageup" if plain => {
                let first = first_selectable(self.level_items(in_submenu));
                self.set_selected(in_submenu, first, cx);
            }
            "end" | "pagedown" if plain => {
                let last = last_selectable(self.level_items(in_submenu));
                self.set_selected(in_submenu, last, cx);
            }
            // ctrl-enter opened the menu from the keyboard; pressed again it closes it
            "enter" if modifiers.control => self.close(window, cx),
            "enter" | "space" if plain && self.type_ahead_running(key) => {
                self.type_ahead(' ', in_submenu, cx)
            }
            "enter" | "space" if plain => {
                if let Some(ix) = self.level_selected(in_submenu) {
                    self.confirm(in_submenu, ix, true, focused, window, cx);
                }
            }
            "right" if plain && !in_submenu => {
                if let Some(ix) = self.selected {
                    self.open_submenu_at(ix, true, cx);
                }
            }
            "left" if plain && in_submenu => self.close_submenu(cx),
            "escape" => {
                if in_submenu {
                    self.close_submenu(cx);
                } else {
                    self.close(window, cx);
                }
            }
            _ if plain => {
                let typed = keystroke
                    .key_char
                    .as_deref()
                    .filter(|text| text.chars().count() == 1)
                    .or_else(|| (key.chars().count() == 1).then_some(key))
                    .and_then(|text| text.chars().next());
                if let Some(ch) = typed.filter(|ch| !ch.is_control()) {
                    self.type_ahead(ch, in_submenu, cx);
                }
            }
            _ => {}
        }
    }

    fn move_selection(&mut self, submenu: bool, forward: bool, cx: &mut Context<Self>) {
        let items = self.level_items(submenu);
        let count = items.len();
        let current = self.level_selected(submenu);
        let next = (1..=count)
            .map(|step| match (current, forward) {
                (Some(current), true) => (current + step) % count,
                (Some(current), false) => (current + count - step) % count,
                (None, true) => step - 1,
                (None, false) => count - step,
            })
            .find(|ix| items[*ix].selectable());
        if next.is_some() {
            self.set_selected(submenu, next, cx);
        }
    }

    /// Whether `space` belongs to what is being typed ("move to", "new f").
    fn type_ahead_running(&self, key: &str) -> bool {
        key == "space"
            && !self.type_ahead.is_empty()
            && self
                .type_ahead_at
                .is_some_and(|at| at.elapsed() < TYPE_AHEAD_PAUSE)
    }

    fn type_ahead(&mut self, ch: char, submenu: bool, cx: &mut Context<Self>) {
        let fresh = self
            .type_ahead_at
            .is_none_or(|at| at.elapsed() >= TYPE_AHEAD_PAUSE);
        if fresh {
            self.type_ahead.clear();
        }
        self.type_ahead.extend(ch.to_lowercase());
        self.type_ahead_at = Some(Instant::now());

        let typed = self.type_ahead.clone();
        let items = self.level_items(submenu);
        let count = items.len();
        let current = self.level_selected(submenu);
        let find = |prefix: &str, start: usize| {
            (0..count).map(|step| (start + step) % count).find(|ix| {
                items[*ix].selectable() && items[*ix].title.to_lowercase().starts_with(prefix)
            })
        };
        let after = current.map_or(0, |current| current + 1);
        let mut restart = None;
        let found = if typed.chars().count() == 1 {
            // one letter: the next item that starts with it, round and round
            find(&typed, after)
        } else {
            // more letters narrow down, starting at the marked item
            find(&typed, current.unwrap_or(0)).or_else(|| {
                // no item starts with all of it: the letter alone starts again
                let last: String = ch.to_lowercase().collect();
                let found = find(&last, after);
                restart = found.map(|_| last);
                found
            })
        };
        if let Some(restart) = restart {
            self.type_ahead = restart;
        }
        if found.is_some() {
            self.set_selected(submenu, found, cx);
        }
    }

    // -- mouse ------------------------------------------------------------

    fn hover_root(&mut self, ix: usize, cx: &mut Context<Self>) {
        let selectable = self.items.get(ix).is_some_and(|item| item.selectable());
        let wanted = selectable.then_some(ix);
        if self.selected != wanted {
            self.selected = wanted;
            cx.notify();
        }
        let is_submenu = selectable && self.items[ix].submenu_items().is_some();
        let open = self.submenu.as_ref().map(|open| open.parent);
        if (is_submenu && open == Some(ix)) || (!is_submenu && open.is_none()) {
            self.hover_task = None;
            return;
        }
        // after a rest on the row: open its submenu, or close the one that
        // belongs to another row
        self.hover_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SUBMENU_DELAY).await;
            this.update(cx, |this, cx| {
                if this.selected != wanted {
                    return;
                }
                if is_submenu {
                    this.open_submenu_at(ix, false, cx);
                } else {
                    this.close_submenu(cx);
                }
            })
            .ok();
        }));
    }

    fn hover_submenu(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.hover_task = None;
        let selectable = self
            .level_items(true)
            .get(ix)
            .is_some_and(|item| item.selectable());
        let Some(open) = self.submenu.as_mut() else {
            return;
        };
        let wanted = selectable.then_some(ix);
        if open.selected != wanted || self.selected != Some(open.parent) {
            open.selected = wanted;
            self.selected = Some(open.parent);
            cx.notify();
        }
    }

    // -- drawing ----------------------------------------------------------

    fn render_panel(
        &self,
        submenu: bool,
        focused: Option<&FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let items = self.level_items(submenu);
        let selected = self.level_selected(submenu);
        let leading = items
            .iter()
            .any(|item| item.icon.is_some() || item.checked.is_some());
        let max_height = (window.viewport_size().height - WINDOW_MARGIN * 2.).max(ROW_HEIGHT * 3.);

        let rows = items.iter().enumerate().map(|(ix, item)| {
            match item.kind {
                ItemKind::Separator => {
                    return div()
                        .h(SEPARATOR_HEIGHT)
                        .flex_none()
                        .flex()
                        .items_center()
                        .child(div().h(px(1.)).flex_1().mx(px(10.)).bg(colors.hairline))
                        .into_any_element();
                }
                ItemKind::Section => {
                    return div()
                        .h(SECTION_HEIGHT)
                        .flex_none()
                        .flex()
                        .items_end()
                        .px(px(12.))
                        .pb(px(3.))
                        .text_size(UI_TEXT_XS)
                        .line_height(px(14.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .whitespace_nowrap()
                        .child(item.title.clone())
                        .into_any_element();
                }
                _ => {}
            }
            let enabled = item.selectable();
            let is_selected = enabled && selected == Some(ix);
            // a submenu row stays marked while the keyboard is in its submenu
            let text = if !enabled {
                colors.faint
            } else if item.danger {
                colors.danger
            } else {
                colors.ink
            };
            let fill = if item.danger {
                colors.danger_subtle
            } else {
                colors.accent_subtle
            };
            let hint = item.key_hint.clone().or_else(|| {
                let action = item.action_ref()?;
                self.target
                    .as_ref()
                    .or(focused)
                    .and_then(|focus| KeyHint::for_action_in(action, focus, window))
                    .or_else(|| KeyHint::for_action(action, cx))
            });
            let is_submenu = item.submenu_items().is_some();
            let selector = if submenu {
                format!("context-submenu-item-{ix}")
            } else {
                format!("context-menu-item-{ix}")
            };
            div()
                .id(("context-menu-row", ix))
                .debug_selector(|| selector)
                .h(ROW_HEIGHT)
                .flex_none()
                .px(PANEL_PADDING)
                .flex()
                .when(enabled, |el| el.cursor_pointer())
                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                    if submenu {
                        this.hover_submenu(ix, cx);
                    } else {
                        this.hover_root(ix, cx);
                    }
                }))
                .on_click(cx.listener(move |this, _, window, cx| {
                    let focused = window.focused(cx);
                    this.confirm(submenu, ix, false, focused, window, cx);
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .rounded(RADIUS_SM)
                        .text_color(text)
                        .when(is_selected, |el| el.bg(fill))
                        .when(leading, |el| {
                            let mark = if item.checked == Some(true) {
                                Some(IconName::Check)
                            } else {
                                item.icon
                            };
                            el.child(div().flex_none().size(px(16.)).children(mark.map(|icon| {
                                Icon::new(icon).color(if enabled && !item.danger {
                                    if is_selected {
                                        colors.ink
                                    } else {
                                        colors.muted
                                    }
                                } else {
                                    text
                                })
                            })))
                        })
                        .when_some(item.swatch, |el, color| {
                            el.child(
                                div()
                                    .flex_none()
                                    .size(px(12.))
                                    .rounded(px(3.))
                                    .bg(color)
                                    .border_1()
                                    .border_color(colors.hairline)
                                    .when(!enabled, |el| el.opacity(0.5)),
                            )
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(item.title.clone()),
                        )
                        .when_some(hint, |el, hint| {
                            el.child(div().flex_none().pl(px(16.)).child(hint.color(if enabled {
                                colors.muted
                            } else {
                                colors.faint
                            })))
                        })
                        .when(is_submenu, |el| {
                            el.child(
                                Icon::new(IconName::ChevronRight)
                                    .size(px(12.))
                                    .color(if enabled { colors.muted } else { colors.faint }),
                            )
                        }),
                )
                .into_any_element()
        });

        div()
            .id(if submenu {
                "context-submenu"
            } else {
                "context-menu"
            })
            .debug_selector(|| {
                if submenu {
                    "context-submenu"
                } else {
                    "context-menu"
                }
                .to_string()
            })
            .occlude()
            .flex()
            .flex_col()
            .min_w(px(180.))
            .max_w(px(340.))
            .max_h(max_height)
            .overflow_y_scroll()
            .when(!submenu, |el| el.track_scroll(&self.scroll))
            .py(PANEL_PADDING)
            .rounded(RADIUS_LG)
            .bg(colors.elevated)
            .border_1()
            .border_color(if colors.appearance.is_dark() {
                colors.border_strong
            } else {
                colors.hairline
            })
            .shadow(theme.shadow_overlay())
            .font_family(theme.fonts.ui.clone())
            .text_size(UI_TEXT_BASE)
            .line_height(px(18.))
            .text_color(colors.ink)
            .cursor_default()
            .children(rows)
            .into_any_element()
    }
}

fn first_selectable(items: &[ContextMenuItem]) -> Option<usize> {
    items.iter().position(|item| item.selectable())
}

fn last_selectable(items: &[ContextMenuItem]) -> Option<usize> {
    items.iter().rposition(|item| item.selectable())
}

/// The offset of item `ix` below the top edge of its panel.
fn row_top(items: &[ContextMenuItem], ix: usize) -> Pixels {
    items[..ix]
        .iter()
        .fold(PANEL_BORDER + PANEL_PADDING, |top, item| {
            top + item.height()
        })
}

fn row_bounds(
    items: &[ContextMenuItem],
    ix: usize,
    panel: Bounds<Pixels>,
    scrolled: Pixels,
) -> Option<Bounds<Pixels>> {
    let item = items.get(ix)?;
    Some(Bounds::new(
        point(panel.left(), panel.top() + row_top(items, ix) + scrolled),
        size(panel.size.width, item.height()),
    ))
}

impl Render for ContextMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = window.focused(cx);
        let root = self.render_panel(false, focused.as_ref(), window, cx);
        let submenu = self
            .submenu
            .as_ref()
            .map(|open| open.parent)
            .map(|parent| Placed {
                at: PlacedAt::RootRow {
                    root: self.bounds.clone(),
                    row_top: row_top(&self.items, parent) + self.scroll.offset().y,
                },
                child: self.render_panel(true, focused.as_ref(), window, cx),
                bounds: self.submenu_bounds.clone(),
                watch: None,
            });
        div()
            .child(Placed {
                at: PlacedAt::Anchor(self.anchor),
                child: root,
                bounds: self.bounds.clone(),
                watch: Some(vec![self.bounds.clone(), self.submenu_bounds.clone()]),
            })
            .children(submenu)
    }
}

// ---------------------------------------------------------------------------
// The layer
// ---------------------------------------------------------------------------

/// Draws the open [`ContextMenu`] of its window, above everything else.
/// [`crate::ToastLayer`] contains one; mount it yourself only in a window
/// without a toast layer. See the [module docs](self).
#[derive(IntoElement, Clone, Copy, Debug, Default)]
pub struct ContextMenuLayer;

impl RenderOnce for ContextMenuLayer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = window_id(window);
        let registry = cx.default_global::<Registry>();
        let menu = registry.open.get(&id).cloned();
        let swallow = registry.swallow_mouse_up.contains(&id);
        // One layer per frame draws the menu, whichever is asked first. The
        // mark is taken off right after the frame (a deferred effect of a
        // draw runs as soon as the draw is done).
        let first = (menu.is_some() || swallow) && registry.drawn.insert(id);
        if first {
            cx.defer(move |cx| {
                cx.default_global::<Registry>().drawn.remove(&id);
            });
        }
        div().absolute().when(first, |el| {
            el.children(menu.map(|menu| deferred(menu).with_priority(DRAW_PRIORITY)))
                .when(swallow, |el| {
                    el.child(gpui::canvas(
                        |_, _, _| (),
                        |_, _, window, _| install_mouse_up_swallow(window),
                    ))
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, y: f32) -> Point<Pixels> {
        point(px(x), px(y))
    }

    #[test]
    fn placement_at_a_point_flips_and_clamps() {
        let viewport = size(px(800.), px(600.));
        let menu = size(px(200.), px(300.));
        let m = px(6.);
        // fits: top left at the point
        assert_eq!(
            place(MenuAnchor::Point(at(100., 100.)), menu, viewport, m),
            at(100., 100.)
        );
        // no room to the right: the menu ends at the point
        assert_eq!(
            place(MenuAnchor::Point(at(700., 100.)), menu, viewport, m),
            at(500., 100.)
        );
        // no room below: the menu ends at the point
        assert_eq!(
            place(MenuAnchor::Point(at(100., 500.)), menu, viewport, m),
            at(100., 200.)
        );
        // neither side has room (a tall menu): clamped into the window
        let tall = size(px(200.), px(560.));
        assert_eq!(
            place(MenuAnchor::Point(at(100., 300.)), tall, viewport, m),
            at(100., 34.)
        );
        // larger than the window: starts at the margin
        let huge = size(px(900.), px(700.));
        assert_eq!(
            place(MenuAnchor::Point(at(100., 300.)), huge, viewport, m),
            at(6., 6.)
        );
        // a point outside the window still gives a menu inside it
        assert_eq!(
            place(MenuAnchor::Point(at(-50., -50.)), menu, viewport, m),
            at(6., 6.)
        );
    }

    #[test]
    fn placement_below_and_beside_bounds() {
        let viewport = size(px(800.), px(600.));
        let menu = size(px(200.), px(300.));
        let m = px(6.);
        let button = Bounds::new(at(100., 40.), size(px(80.), px(28.)));
        assert_eq!(
            place(MenuAnchor::Below(button), menu, viewport, m),
            at(100., 72.)
        );
        // no room below: above the bounds
        let low = Bounds::new(at(100., 520.), size(px(80.), px(28.)));
        assert_eq!(
            place(MenuAnchor::Below(low), menu, viewport, m),
            at(100., 216.)
        );
        // right edge: pushed left into the window
        let right = Bounds::new(at(760., 40.), size(px(30.), px(28.)));
        assert_eq!(
            place(MenuAnchor::Below(right), menu, viewport, m),
            at(594., 72.)
        );

        let row = Bounds::new(at(100., 200.), size(px(220.), px(24.)));
        assert_eq!(
            place(MenuAnchor::Beside(row), menu, viewport, m),
            at(317., 195.)
        );
        // no room on the right: on the left of the row
        let row = Bounds::new(at(500., 200.), size(px(220.), px(24.)));
        assert_eq!(
            place(MenuAnchor::Beside(row), menu, viewport, m),
            at(303., 195.)
        );
        // near the bottom: pushed up
        let row = Bounds::new(at(100., 560.), size(px(220.), px(24.)));
        assert_eq!(
            place(MenuAnchor::Beside(row), menu, viewport, m),
            at(317., 294.)
        );
    }

    #[test]
    fn separators_that_separate_nothing_are_dropped() {
        let items = normalize(
            vec![
                ContextMenuItem::separator(),
                ContextMenuItem::new("A", |_, _| {}),
                ContextMenuItem::separator(),
                ContextMenuItem::separator(),
                ContextMenuItem::submenu(
                    "More",
                    vec![
                        ContextMenuItem::separator(),
                        ContextMenuItem::new("B", |_, _| {}),
                        ContextMenuItem::submenu(
                            "Deeper",
                            vec![ContextMenuItem::new("C", |_, _| {})],
                        ),
                    ],
                ),
                ContextMenuItem::submenu("Empty", vec![]),
                ContextMenuItem::separator(),
            ],
            false,
        );
        let titles: Vec<_> = items.iter().map(|item| item.title().to_string()).collect();
        assert_eq!(titles, ["A", "", "More", "Empty"]);
        let nested = items[2].submenu_items().unwrap();
        assert_eq!(nested.len(), 2);
        assert!(nested[1].is_disabled(), "one level of submenus");
        assert!(items[3].is_disabled(), "an empty submenu cannot be opened");
    }
}
