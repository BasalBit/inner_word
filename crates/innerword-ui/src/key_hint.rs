//! [`KeyHint`]: a key binding drawn as compact macOS glyphs ("⇧⌘P", "⌃X ⌃S").
//!
//! Hints are never hand-written: look them up from the keymap so they always
//! show what is actually bound.
//!
//! ```ignore
//! use innerword_ui::KeyHint;
//!
//! // keymap-wide: the highest-precedence binding of the action (last added)
//! row.children(KeyHint::for_action(&OpenCommandPalette, cx));
//!
//! // as seen from a focus handle (respects key contexts such as "Editor")
//! KeyHint::for_action_in(&ToggleBold, &editor_focus, window);
//! // the same, falling back to the keymap-wide binding: for a control of
//! // a panel whose key applies in that panel
//! KeyHint::for_action_at(&OpenContextMenu, &list_focus, window, cx);
//!
//! // from a binding or a keystroke string
//! KeyHint::from_binding(&binding);
//! KeyHint::parse("cmd-shift-p").unwrap();
//! KeyHint::parse("ctrl-x ctrl-s").unwrap();       // two-stroke chord
//!
//! // as text (menus, tooltips, tests)
//! innerword_ui::key_hint::format_keystroke(&Keystroke::parse("cmd-alt-s")?);  // "⌥⌘S"
//! ```
//!
//! # One way to show a key
//!
//! Wherever a key is shown next to what it does (footers of popups, buttons
//! of a panel, empty states), the label comes first and the key second, the
//! key in the plain muted style: "Apply ↩", "Cancel Esc". [`KeyLabel`] draws
//! exactly that; [`crate::Button::key_hint`] and the rows of
//! [`crate::Picker`] and [`crate::ContextMenu`] follow the same order. The
//! boxed style is for the shortcuts overlay and for a key that stands alone
//! in running text.
//!
//! ```ignore
//! footer.child(KeyLabel::new("Apply", KeyHint::parse("enter")))
//!       .child(KeyLabel::new("Cancel", KeyHint::parse("escape")))
//! ```
//!
//! Escape is written "Esc": its glyph (U+238B) reads as an undo or refresh
//! arrow at interface sizes.
//!
//! Modifier order is the macOS menu order: ⌃ ⌥ ⇧ ⌘. This differs from GPUI's
//! own `Keystroke` display ("^⌥⌘⇧"), which does not match the system menus.

use gpui::{
    Action, App, FocusHandle, Hsla, KeyBinding, Keystroke, Pixels, SharedString, Window, div,
    prelude::*, px,
};

use crate::theme::{ActiveTheme, RADIUS_SM, UI_TEXT_SM};

/// The glyph for a GPUI key name: `enter` → "↩", `a` → "A", `f5` → "F5".
pub fn key_glyph(key: &str) -> String {
    let glyph = match key {
        "enter" => "↩",
        "backspace" => "⌫",
        "delete" => "⌦",
        // not the glyph ⎋: at 12px it reads as an undo or refresh arrow
        "escape" => "Esc",
        "tab" => "⇥",
        "space" => "Space",
        "up" => "↑",
        "down" => "↓",
        "left" => "←",
        "right" => "→",
        "pageup" => "⇞",
        "pagedown" => "⇟",
        "home" => "↖",
        "end" => "↘",
        "insert" => "Ins",
        _ => {
            return if key.chars().count() == 1 {
                key.to_uppercase()
            } else if let Some(n) = key.strip_prefix('f').filter(|n| n.parse::<u8>().is_ok()) {
                format!("F{n}")
            } else {
                let mut chars = key.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().chain(chars).collect())
                    .unwrap_or_default()
            };
        }
    };
    glyph.to_string()
}

/// One keystroke as macOS glyphs: modifiers in menu order (⌃⌥⇧⌘), then the key.
pub fn format_keystroke(keystroke: &Keystroke) -> String {
    let m = &keystroke.modifiers;
    let mut out = String::new();
    if m.function {
        out.push_str("fn ");
    }
    if m.control {
        out.push('⌃');
    }
    if m.alt {
        out.push('⌥');
    }
    // The plus key (shift with `=`) is written as macOS writes it beside
    // "Zoom In": the character, without the shift that makes it. A real
    // keyboard reports it as `+` without the shift flag, which reads "⌘+"
    // by itself; both spellings are one key and are shown as one.
    let plus = m.shift && keystroke.key == "=";
    if m.shift && !plus {
        out.push('⇧');
    }
    if m.platform {
        out.push('⌘');
    }
    if plus {
        out.push('+');
    } else {
        out.push_str(&key_glyph(&keystroke.key));
    }
    out
}

/// Several keystrokes (a chord), separated by spaces.
pub fn format_keystrokes(keystrokes: &[Keystroke]) -> String {
    keystrokes
        .iter()
        .map(format_keystroke)
        .collect::<Vec<_>>()
        .join(" ")
}

fn binding_keystrokes(binding: &KeyBinding) -> Vec<Keystroke> {
    binding
        .keystrokes()
        .iter()
        .map(|stroke| stroke.inner().clone())
        .collect()
}

/// Visual style of a [`KeyHint`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyHintStyle {
    /// Plain muted glyphs, for list rows and menus.
    #[default]
    Plain,
    /// Each keystroke in a small outlined chip, for the shortcuts overlay
    /// and buttons.
    Boxed,
}

/// A key binding rendered as glyphs. See the [module docs](self).
#[derive(IntoElement, Clone, Debug, PartialEq)]
pub struct KeyHint {
    keystrokes: Vec<Keystroke>,
    style: KeyHintStyle,
    color: Option<Hsla>,
    size: Option<Pixels>,
}

impl KeyHint {
    pub fn new(keystrokes: impl IntoIterator<Item = Keystroke>) -> Self {
        Self {
            keystrokes: keystrokes.into_iter().collect(),
            style: KeyHintStyle::Plain,
            color: None,
            size: None,
        }
    }

    /// Parses GPUI keystroke syntax ("cmd-shift-p", "ctrl-x ctrl-s").
    pub fn parse(source: &str) -> Option<Self> {
        let keystrokes = source
            .split_whitespace()
            .map(Keystroke::parse)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        (!keystrokes.is_empty()).then(|| Self::new(keystrokes))
    }

    pub fn from_binding(binding: &KeyBinding) -> Self {
        Self::new(binding_keystrokes(binding))
    }

    /// The highest-precedence binding for `action` anywhere in the keymap
    /// (the binding added last), ignoring key contexts. Actions are compared
    /// by value, so `SetHeading { level: 2 }` finds its own binding.
    pub fn for_action(action: &dyn Action, cx: &App) -> Option<Self> {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        keymap
            .bindings_for_action(action)
            .last()
            .map(Self::from_binding)
    }

    /// The highest-precedence binding for `action` that would fire if
    /// `focus` were focused (respects key contexts). Uses the last rendered
    /// frame, so `focus` must have been rendered.
    pub fn for_action_in(
        action: &dyn Action,
        focus: &FocusHandle,
        window: &Window,
    ) -> Option<Self> {
        window
            .highest_precedence_binding_for_action_in(action, focus)
            .map(|binding| Self::from_binding(&binding))
    }

    /// GPUI's own answer for "the binding of `action` from the currently
    /// focused element" (`Window::highest_precedence_binding_for_action`).
    ///
    /// In gpui 0.2.2 that answer is not about the focused element: it is
    /// taken from the key context stack the last frame's dispatch tree was
    /// left with when the frame was finished, which is the place of the
    /// element that was painted last. It is right where that happens to be
    /// the focused one (a window with one focusable element) and wrong
    /// elsewhere: a binding in the focused element's context is not found
    /// when a layer drawn over it was painted after it (tested in
    /// `a_hint_is_asked_in_the_place_it_applies`). Use
    /// [`KeyHint::for_action_here`] for where the keyboard is, or
    /// [`KeyHint::for_action_in`] for a focus handle.
    pub fn for_action_focused(action: &dyn Action, window: &Window) -> Option<Self> {
        window
            .highest_precedence_binding_for_action(action)
            .map(|binding| Self::from_binding(&binding))
    }

    /// The key to show for `action` on a control that belongs to the place
    /// `focus` is in (a panel's own list, a chat's own composer): the
    /// binding that fires when `focus` has the keyboard
    /// ([`KeyHint::for_action_in`]), and only where that place has none, or
    /// was not drawn yet, the keymap-wide one ([`KeyHint::for_action`]).
    ///
    /// An action can be bound in several key contexts, and each of those
    /// bindings can be given another key (the context menu in the library,
    /// in the comment list, in a text field). The keymap-wide answer is the
    /// binding added last, which is some place's and not necessarily this
    /// one's; so a control that knows where its key applies asks there.
    pub fn for_action_at(
        action: &dyn Action,
        focus: &FocusHandle,
        window: &Window,
        cx: &App,
    ) -> Option<Self> {
        Self::for_action_in(action, focus, window).or_else(|| Self::for_action(action, cx))
    }

    /// The key to show for `action` where the keyboard is now: the binding
    /// of it that fires from the focused element
    /// ([`KeyHint::for_action_in`] with the focused handle), else the
    /// keymap-wide one ([`KeyHint::for_action`]). What
    /// [`crate::Tooltip::for_action`] and the button of a toast show.
    ///
    /// Safe before the first frame and with nothing focused: it then
    /// answers keymap-wide. (Not [`KeyHint::for_action_focused`]: see
    /// there. And not `Window::context_stack`, which GPUI answers from the
    /// root of the last frame when nothing in it is focused, and asserts
    /// that there is one: on the first frame there is none.)
    pub fn for_action_here(action: &dyn Action, window: &Window, cx: &App) -> Option<Self> {
        window
            .focused(cx)
            .and_then(|focus| Self::for_action_in(action, &focus, window))
            .or_else(|| Self::for_action(action, cx))
    }

    pub fn keystrokes(&self) -> &[Keystroke] {
        &self.keystrokes
    }

    /// The hint as text, e.g. "⇧⌘P".
    pub fn text(&self) -> String {
        format_keystrokes(&self.keystrokes)
    }

    pub fn style(mut self, style: KeyHintStyle) -> Self {
        self.style = style;
        self
    }

    pub fn boxed(self) -> Self {
        self.style(KeyHintStyle::Boxed)
    }

    /// Overrides the colour (default: the theme's muted colour).
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    /// Overrides the text size (default 12px).
    pub fn size(mut self, size: Pixels) -> Self {
        self.size = Some(size);
        self
    }
}

impl RenderOnce for KeyHint {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.theme().colors;
        let color = self.color.unwrap_or(colors.muted);
        let size = self.size.unwrap_or(UI_TEXT_SM);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .text_size(size)
            .line_height(size + px(4.))
            .text_color(color)
            .whitespace_nowrap()
            .children(self.keystrokes.iter().map(|keystroke| {
                let text = format_keystroke(keystroke);
                match self.style {
                    KeyHintStyle::Plain => div().child(text),
                    KeyHintStyle::Boxed => div()
                        .px(px(5.))
                        .min_w(px(18.))
                        .flex()
                        .justify_center()
                        .rounded(RADIUS_SM)
                        .border_1()
                        .border_color(colors.hairline)
                        .bg(colors.canvas)
                        .child(text),
                }
            }))
    }
}

/// A label followed by its key: "Apply ↩". The one way to show what a key
/// does in footers, hint rows and empty states (see the [module docs](self)).
/// Without a key (an unbound action) only the label is drawn.
#[derive(IntoElement, Clone, Debug, PartialEq)]
pub struct KeyLabel {
    label: SharedString,
    hint: Option<KeyHint>,
    color: Option<Hsla>,
}

impl KeyLabel {
    /// Accepts an `Option` so `KeyHint::for_action(..)` can be passed directly.
    pub fn new(label: impl Into<SharedString>, hint: impl Into<Option<KeyHint>>) -> Self {
        Self {
            label: label.into(),
            hint: hint.into(),
            color: None,
        }
    }

    /// Overrides the label colour (default: the theme's muted colour).
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    /// "Apply ↩", for tests and tooltips.
    pub fn text(&self) -> String {
        match &self.hint {
            Some(hint) => format!("{} {}", self.label, hint.text()),
            None => self.label.to_string(),
        }
    }
}

impl RenderOnce for KeyLabel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.theme().colors;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(5.))
            .text_size(UI_TEXT_SM)
            .line_height(UI_TEXT_SM + px(4.))
            .text_color(self.color.unwrap_or(colors.muted))
            .whitespace_nowrap()
            .child(self.label)
            .children(self.hint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(source: &str) -> String {
        KeyHint::parse(source).unwrap().text()
    }

    #[test]
    fn glyphs_in_menu_order() {
        assert_eq!(fmt("cmd-shift-p"), "⇧⌘P");
        assert_eq!(fmt("cmd-alt-s"), "⌥⌘S");
        assert_eq!(fmt("ctrl-alt-shift-cmd-k"), "⌃⌥⇧⌘K");
        assert_eq!(fmt("cmd-enter"), "⌘↩");
        assert_eq!(fmt("alt-backspace"), "⌥⌫");
        assert_eq!(fmt("escape"), "Esc");
        assert_eq!(
            KeyLabel::new("Cancel", KeyHint::parse("escape")).text(),
            "Cancel Esc"
        );
        assert_eq!(KeyLabel::new("Apply", None).text(), "Apply");
        assert_eq!(fmt("shift-tab"), "⇧⇥");
        assert_eq!(fmt("cmd-up"), "⌘↑");
        assert_eq!(fmt("cmd-,"), "⌘,");
        assert_eq!(fmt("cmd-\\"), "⌘\\");
        assert_eq!(fmt("f12"), "F12");
        assert_eq!(fmt("ctrl-cmd-space"), "⌃⌘Space");
        assert_eq!(fmt("ctrl-x ctrl-s"), "⌃X ⌃S");
        assert!(KeyHint::parse("cmd-opt-s").is_none());
        assert!(KeyHint::parse("").is_none());
    }

    /// The key of "Zoom In" reads "⌘+" in both of its spellings, as the
    /// menu bar draws it; no other shifted character is written so.
    #[test]
    fn the_plus_key_is_written_as_the_mac_writes_it() {
        assert_eq!(fmt("cmd-+"), "⌘+");
        assert_eq!(fmt("cmd-shift-="), "⌘+");
        assert_eq!(fmt("cmd-="), "⌘=");
        assert_eq!(fmt("cmd-alt-shift-="), "⌥⌘+");
        assert_eq!(fmt("cmd-shift-7"), "⇧⌘7");
        assert_eq!(fmt("cmd-shift-0"), "⇧⌘0");
        assert_eq!(fmt("cmd-shift--"), "⇧⌘-");
    }
}
