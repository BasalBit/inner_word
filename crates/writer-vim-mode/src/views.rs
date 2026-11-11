//! The two views a host mounts: the status bar item and the command line.
//!
//! Both only draw what [`VimMode::status`] holds and neither takes the
//! keyboard focus: the keys of the command line arrive through the editor's
//! key interception, like every other Vim key.

use gpui::{
    AnyView, App, Context, ElementInputHandler, Entity, Focusable, FontWeight, IntoElement,
    MouseButton, Render, SharedString, Subscription, Window, canvas, div, prelude::*, px,
};
use writer_ui::theme::{
    ActiveTheme, RADIUS_SM, SPACE_0_5, SPACE_1, SPACE_2, SPACE_3, UI_TEXT_BASE, UI_TEXT_SM,
    UI_TEXT_XS,
};
use writer_ui::{Icon, IconName, KeyHint, theme::RADIUS_MD};
use writer_vim::Mode;

use crate::mode::VimMode;

/// Height of the command-line strip.
const COMMAND_LINE_HEIGHT: f32 = 28.;
/// The widest a status message gets before it is cut with an ellipsis.
const MESSAGE_MAX_WIDTH: f32 = 420.;

/// The Vim item for a status bar: the mode, the keys of an unfinished
/// command, and the last message or error. Draws nothing while Vim mode is
/// off, so it can stay mounted.
///
/// ```ignore
/// let item = cx.new(|cx| VimStatusItem::new(vim.clone(), cx));
/// workspace.set_status_item("vim", Some(item.into()), cx);
/// ```
pub struct VimStatusItem {
    vim: Entity<VimMode>,
    _observe: Subscription,
}

impl VimStatusItem {
    /// A status item for `vim`.
    pub fn new(vim: Entity<VimMode>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&vim, |_, _, cx| cx.notify());
        Self {
            vim,
            _observe: observe,
        }
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
        let mode = div()
            .debug_selector(|| "vim-mode-chip".into())
            .flex_none()
            .px(SPACE_1 + SPACE_0_5)
            .rounded(RADIUS_SM)
            .bg(chip_background)
            .text_color(chip_text)
            .text_size(UI_TEXT_XS)
            .font_weight(FontWeight::SEMIBOLD)
            .whitespace_nowrap()
            .child(status.mode_label());
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
    /// use for typed text that plain keys cannot give it (a dead key, an
    /// input method; see the `input` module). An element without size:
    /// only its paint matters. It comes after the editor in the window,
    /// and the editor registers nothing outside Insert mode.
    fn text_input(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let vim = self.vim.read(cx);
        if !vim.wants_text_input() {
            return None;
        }
        let focus = vim.editor().read(cx).focus_handle(cx);
        let vim = self.vim.clone();
        Some(
            canvas(
                |_, _, _| (),
                move |bounds, (), window, cx| {
                    window.handle_input(&focus, ElementInputHandler::new(bounds, vim), cx);
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
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
            .children(text_input);
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
                    .tooltip(tip("Close the command line", "escape"))
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
    keystroke: &'static str,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| {
        cx.new(|_| Tip {
            label: label.into(),
            hint: KeyHint::parse(keystroke),
        })
        .into()
    }
}
