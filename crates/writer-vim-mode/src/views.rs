//! The two views a host mounts: the status bar item and the command line.
//!
//! Both only draw what [`VimMode::status`] holds and neither takes the
//! keyboard focus: the keys of the command line arrive through the editor's
//! key interception, like every other Vim key.

use std::{cell::Cell, rc::Rc};

use gpui::{
    Action, AnyView, App, Bounds, Context, ElementInputHandler, Entity, Focusable, FontWeight,
    IntoElement, MouseButton, MouseDownEvent, Pixels, Point, Render, SharedString, Subscription,
    Window, canvas, div, prelude::*, px,
};
use writer_ui::theme::{
    ActiveTheme, RADIUS_MD, RADIUS_SM, SPACE_0_5, SPACE_1, SPACE_2, SPACE_3, UI_TEXT_BASE,
    UI_TEXT_SM, UI_TEXT_XS,
};
use writer_ui::{ContextMenu, ContextMenuItem, Icon, IconName, KeyHint, MenuAnchor};
use writer_vim::Mode;

use crate::{ToggleVimMode, mode::VimMode};

/// Height of the command-line strip.
const COMMAND_LINE_HEIGHT: f32 = 28.;
/// The widest a status message gets before it is cut with an ellipsis.
const MESSAGE_MAX_WIDTH: f32 = 420.;

/// Adds the host's items to the menu of the mode chip.
type MenuHook = Rc<dyn Fn(&mut Vec<ContextMenuItem>, &mut Window, &mut App)>;

/// The Vim item for a status bar: the mode, the keys of an unfinished
/// command, and the last message or error. Draws nothing while Vim mode is
/// off, so it can stay mounted.
///
/// The mode is a control for the mouse: a click or a right-click on it
/// opens a menu ([`VimStatusItem::menu_items`]) with what the keys of the
/// mode do: back to Normal mode, Insert mode, the command line, a search,
/// clearing the search highlight, and turning Vim mode off. The menu is a
/// `writer_ui::ContextMenu`, drawn by the window's toast layer.
///
/// ```ignore
/// let item = cx.new(|cx| VimStatusItem::new(vim.clone(), cx));
/// item.update(cx, |item, _| {
///     // the application's own toggle, so the menu shows its key (cmd-alt-v)
///     item.set_toggle_action(app::ToggleVimMode);
///     // more items after this crate's
///     item.extend_menu(|items, _, _| items.push(ContextMenuItem::action("Settings", OpenSettings)));
/// });
/// workspace.set_status_item("vim", Some(item.into()), cx);
/// ```
pub struct VimStatusItem {
    vim: Entity<VimMode>,
    /// The action of "Turn off Vim mode".
    toggle: Box<dyn Action>,
    menu_hook: Option<MenuHook>,
    /// Where the mode chip was drawn: the menu drops from it.
    chip_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    _observe: Subscription,
}

impl VimStatusItem {
    /// A status item for `vim`.
    pub fn new(vim: Entity<VimMode>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&vim, |_, _, cx| cx.notify());
        Self {
            vim,
            toggle: Box::new(ToggleVimMode),
            menu_hook: None,
            chip_bounds: Rc::default(),
            _observe: observe,
        }
    }

    /// The action behind "Turn off Vim mode" in the menu of the mode chip.
    /// By default this crate's [`ToggleVimMode`], which has no key. An
    /// application that binds an action of its own to the key (and stores
    /// the setting there) names it here: the menu then shows that key and
    /// choosing the item does what the key does. The action is dispatched
    /// to the element that has the keyboard.
    pub fn set_toggle_action(&mut self, action: impl Action) {
        self.toggle = Box::new(action);
    }

    /// Lets the host add items to the menu of the mode chip (or change
    /// the list): `hook` gets the items of this crate each time the menu
    /// is about to open.
    pub fn extend_menu(
        &mut self,
        hook: impl Fn(&mut Vec<ContextMenuItem>, &mut Window, &mut App) + 'static,
    ) {
        self.menu_hook = Some(Rc::new(hook));
    }

    /// The items of the menu of the mode chip for the state Vim is in:
    /// the two modes a writer switches between (the current one checked),
    /// the command line and the search with their keys, "Clear search
    /// highlight" (disabled without one) and "Turn off Vim mode", then
    /// what the host added. Empty while Vim mode is off.
    ///
    /// Each item does what its key does, typed in the text: the keyboard
    /// goes to the editor first.
    pub fn menu_items(&self, window: &mut Window, cx: &mut App) -> Vec<ContextMenuItem> {
        let vim = self.vim.read(cx);
        let status = vim.status();
        if !status.attached {
            return Vec::new();
        }
        let idle =
            status.mode == Mode::Normal && status.pending_keys.is_empty() && !status.insert_pending;
        let inserting = status.mode.is_insert();
        let visual = matches!(status.mode, Mode::Visual | Mode::VisualLine);
        let read_only = vim.editor().read(cx).is_read_only();
        // From a mode in which the key would mean something else, `escape`
        // comes first. The command line and a search keep a Visual
        // selection: it is their range.
        let from_normal = |keys: &'static str, with_escape: &'static str| {
            if idle { keys } else { with_escape }
        };
        let keys = |keys: &'static str| {
            let vim = self.vim.downgrade();
            move |window: &mut Window, cx: &mut App| {
                let Some(vim) = vim.upgrade() else {
                    return;
                };
                vim.read(cx).editor().read(cx).focus(window);
                vim.update(cx, |vim, cx| vim.feed_keys(keys, cx));
            }
        };
        let clear = {
            let vim = self.vim.downgrade();
            move |_: &mut Window, cx: &mut App| {
                vim.update(cx, |vim, cx| vim.execute("nohlsearch", cx)).ok();
            }
        };
        let mut items = vec![
            ContextMenuItem::new("Normal mode", keys(if idle { "" } else { "<Esc>" }))
                .checked(idle)
                .key_hint(KeyHint::parse("escape")),
            ContextMenuItem::new(
                "Insert mode",
                keys(if inserting {
                    ""
                } else {
                    from_normal("i", "<Esc>i")
                }),
            )
            .checked(inserting)
            .disabled(read_only)
            .key_hint(KeyHint::parse("i")),
            ContextMenuItem::separator(),
            ContextMenuItem::new(
                "Command line",
                keys(if visual {
                    ":"
                } else {
                    from_normal(":", "<Esc>:")
                }),
            )
            .key_hint(KeyHint::parse(":")),
            ContextMenuItem::new(
                "Search",
                keys(if visual {
                    "/"
                } else {
                    from_normal("/", "<Esc>/")
                }),
            )
            .key_hint(KeyHint::parse("/")),
            ContextMenuItem::new("Clear search highlight", clear)
                .disabled(!status.search_highlighted),
            ContextMenuItem::separator(),
            ContextMenuItem::action_boxed("Turn off Vim mode", self.toggle.boxed_clone()),
        ];
        if let Some(hook) = self.menu_hook.clone() {
            hook(&mut items, window, cx);
        }
        items
    }

    /// Opens the menu of the mode chip, dropping from the chip (or at
    /// `fallback` before the chip was drawn). Does nothing while Vim mode
    /// is off.
    pub fn open_menu(
        &mut self,
        fallback: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = self.menu_items(window, cx);
        if items.is_empty() {
            return;
        }
        let anchor = match self.chip_bounds.get() {
            Some(bounds) => MenuAnchor::Below(bounds),
            None => MenuAnchor::Point(fallback),
        };
        ContextMenu::open(anchor, items, window, cx);
    }
}

impl Render for VimStatusItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.vim.read(cx).status().clone();
        let root = div().flex().flex_none().items_center().gap(SPACE_2);
        if !status.attached {
            return root;
        }
        let theme = cx.theme();
        let colors = theme.colors;
        // Normal mode is the resting state and stays quiet; the modes in
        // which keys do something else are marked.
        let (chip_background, chip_text) = match status.mode {
            // The bell: the chip is filled for a moment.
            _ if status.bell => (colors.accent, colors.canvas),
            Mode::Normal | Mode::OperatorPending | Mode::CommandLine => (colors.hover, colors.ink),
            Mode::Insert | Mode::Visual | Mode::VisualLine => (colors.accent_subtle, colors.accent),
            Mode::Replace => (colors.danger_subtle, colors.danger),
        };
        let chip_bounds = self.chip_bounds.clone();
        let open_menu = |this: &mut Self,
                         event: &MouseDownEvent,
                         window: &mut Window,
                         cx: &mut Context<Self>| {
            // The press is this control's: the status bar around it does
            // not open a menu of its own for it.
            cx.stop_propagation();
            this.open_menu(event.position, window, cx);
        };
        let mode = div()
            .id("vim-mode-chip")
            .debug_selector(|| "vim-mode-chip".into())
            .relative()
            .flex_none()
            .px(SPACE_1 + SPACE_0_5)
            .rounded(RADIUS_SM)
            .bg(chip_background)
            .text_color(chip_text)
            .text_size(UI_TEXT_XS)
            .font_weight(FontWeight::SEMIBOLD)
            .whitespace_nowrap()
            .cursor_pointer()
            .hover(|style| style.opacity(0.8))
            .tooltip(tip("Vim mode: click for its commands", None))
            .on_mouse_down(MouseButton::Left, cx.listener(open_menu))
            .on_mouse_down(MouseButton::Right, cx.listener(open_menu))
            .child(status.mode_label())
            .child(
                canvas(
                    move |bounds, _, _| chip_bounds.set(Some(bounds)),
                    |_, (), _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );
        root.text_size(UI_TEXT_SM)
            .child(mode)
            // The command line shows its own text in its strip.
            .when(
                !status.pending_keys.is_empty() && status.command_line.is_none(),
                |root| {
                    root.child(
                        div()
                            .flex_none()
                            .font_family(theme.fonts.mono.clone())
                            .text_color(colors.ink)
                            .whitespace_nowrap()
                            .child(SharedString::from(status.pending_keys.clone())),
                    )
                },
            )
            .when_some(status.message.clone(), |root, message| {
                root.child(
                    div()
                        .max_w(px(MESSAGE_MAX_WIDTH))
                        .truncate()
                        .text_color(if message.is_error {
                            colors.danger
                        } else {
                            colors.muted
                        })
                        .child(SharedString::from(message.text)),
                )
            })
    }
}

/// The Vim command line: one line that shows what is being typed after
/// `:`, `/` or `?`, with the cursor. It has no height while the command
/// line is closed, so a host mounts it once as a strip under the editor.
///
/// The keyboard focus stays in the editor. `enter` runs the command,
/// `escape` cancels, `up` / `down` walk the history, `left` / `right` /
/// `home` / `end` move the cursor, `ctrl-w` and `ctrl-u` delete, `ctrl-r`
/// plus a register name inserts a register and `cmd-v` pastes.
///
/// ```ignore
/// let line = cx.new(|cx| VimCommandLine::new(vim.clone(), cx));
/// workspace.set_editor_strip("vim-command-line", Some(line.into()), cx);
/// ```
pub struct VimCommandLine {
    vim: Entity<VimMode>,
    _observe: Subscription,
}

impl VimCommandLine {
    /// A command line for `vim`.
    pub fn new(vim: Entity<VimMode>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&vim, |_, _, cx| cx.notify());
        Self {
            vim,
            _observe: observe,
        }
    }

    /// Whether the strip is showing (the command line is open).
    pub fn is_open(&self, cx: &gpui::App) -> bool {
        self.vim.read(cx).status().command_line.is_some()
    }
}

impl VimCommandLine {
    /// Gives the keyboard back to the text, where the keys of the command
    /// line are typed.
    fn focus_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.vim.read(cx).editor().read(cx).focus(window);
    }

    /// The close control: what `escape` does.
    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_editor(window, cx);
        self.vim.update(cx, |vim, cx| vim.feed_keys("<Esc>", cx));
    }

    /// Registers the Vim mode as the window's text input while it has a
    /// use for typed text outside Insert mode (see the `input` module),
    /// and tells it whether this frame did. An element without size: only
    /// its paint matters. It comes after the editor in the window, and
    /// the editor registers nothing outside Insert mode.
    fn text_input(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let vim = self.vim.clone();
        let (focus, ready) = {
            let vim = vim.read(cx);
            (
                vim.editor().read(cx).focus_handle(cx),
                vim.input_ready.clone(),
            )
        };
        canvas(
            |_, _, _| (),
            move |bounds, (), window, cx| {
                let wanted = vim.read(cx).wants_text_input() && focus.is_focused(window);
                ready.set(wanted);
                if wanted {
                    window.handle_input(&focus, ElementInputHandler::new(bounds, vim), cx);
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}

impl Render for VimCommandLine {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.vim.read(cx).status();
        let line = status.command_line.clone();
        let composing = status.composing.clone();
        let text_input = self.text_input(cx);
        let root = div()
            .id("vim-command-line")
            .relative()
            .w_full()
            .flex_none()
            .child(text_input);
        let Some(line) = line else {
            return root;
        };
        let theme = cx.theme();
        let colors = theme.colors;
        let cursor = line.cursor.min(line.text.len());
        let cursor = (0..=cursor)
            .rev()
            .find(|offset| line.text.is_char_boundary(*offset))
            .unwrap_or(0);
        let (before, after) = line.text.split_at(cursor);
        let part = |text: &str| {
            div()
                .flex_none()
                .whitespace_nowrap()
                .child(SharedString::from(text.to_string()))
        };
        root.debug_selector(|| "vim-command-line".into())
            .h(px(COMMAND_LINE_HEIGHT))
            .flex()
            .items_center()
            .overflow_hidden()
            .pl(SPACE_3)
            .pr(SPACE_1)
            .border_t_1()
            .border_color(colors.hairline)
            .bg(colors.canvas)
            .font_family(theme.fonts.mono.clone())
            .text_size(UI_TEXT_BASE)
            .text_color(colors.ink)
            // A click anywhere on the line brings the keyboard back to it.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.focus_editor(window, cx)),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(colors.muted)
                    .child(SharedString::from(line.prefix.to_string())),
            )
            .when(!before.is_empty(), |root| root.child(part(before)))
            // The accent of a dead key, until its letter follows.
            .when_some(composing, |root, composing| {
                root.child(part(&composing).underline())
            })
            .child(div().flex_none().w(px(1.5)).h(px(16.)).bg(colors.caret))
            .when(!after.is_empty(), |root| root.child(part(after)))
            .child(div().flex_1().min_w(SPACE_2))
            // For the mouse: what `escape` does.
            .child(
                div()
                    .id("vim-command-line-close")
                    .debug_selector(|| "vim-command-line-close".into())
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(20.))
                    .rounded(RADIUS_SM)
                    .cursor_pointer()
                    .hover(|style| style.bg(colors.hover))
                    .tooltip(tip("Close the command line", Some("escape")))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx)))
                    .child(Icon::new(IconName::Close).size(px(12.)).color(colors.muted)),
            )
    }
}

/// A small floating label with the key that does the same: where a mouse
/// user learns the key.
struct Tip {
    label: SharedString,
    hint: Option<KeyHint>,
}

impl Render for Tip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        // GPUI puts the tooltip at the pointer; the margin keeps it clear
        // of the arrow.
        div().pt(px(6.)).child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .h(px(24.))
                .px(SPACE_2)
                .rounded(RADIUS_MD)
                .bg(colors.elevated)
                .border_1()
                .border_color(colors.border_strong)
                .shadow(theme.shadow_popover())
                .font_family(theme.fonts.ui.clone())
                .text_size(UI_TEXT_SM)
                .line_height(px(16.))
                .text_color(colors.ink)
                .child(self.label.clone())
                .children(self.hint.clone().map(|hint| hint.size(UI_TEXT_XS))),
        )
    }
}

/// A tooltip builder for `.tooltip(..)`: `label`, followed by the key
/// `keystroke` (GPUI spelling).
fn tip(
    label: &'static str,
    keystroke: Option<&'static str>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| {
        cx.new(|_| Tip {
            label: label.into(),
            hint: keystroke.and_then(KeyHint::parse),
        })
        .into()
    }
}
