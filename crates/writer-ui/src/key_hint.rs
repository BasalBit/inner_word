//! [`KeyHint`]: a key binding drawn as compact macOS glyphs ("⇧⌘P", "⌃X ⌃S").
//!
//! Hints are never hand-written: look them up from the keymap so they always
//! show what is actually bound.
//!
//! ```ignore
//! use writer_ui::KeyHint;
//!
//! // keymap-wide: the highest-precedence binding of the action (last added)
//! row.children(KeyHint::for_action(&OpenCommandPalette, cx));
//!
//! // as seen from a focus handle (respects key contexts such as "Editor")
//! KeyHint::for_action_in(&ToggleBold, &editor_focus, window);
//!
//! // from a binding or a keystroke string
//! KeyHint::from_binding(&binding);
//! KeyHint::parse("cmd-shift-p").unwrap();
//! KeyHint::parse("ctrl-x ctrl-s").unwrap();       // two-stroke chord
//!
//! // as text (menus, tooltips, tests)
//! writer_ui::key_hint::format_keystroke(&Keystroke::parse("cmd-alt-s")?);  // "⌥⌘S"
//! ```
//!
//! Modifier order is the macOS menu order: ⌃ ⌥ ⇧ ⌘. This differs from GPUI's
//! own `Keystroke` display ("^⌥⌘⇧"), which does not match the system menus.

use gpui::{
    Action, App, FocusHandle, Hsla, KeyBinding, Keystroke, Pixels, Window, div, prelude::*, px,
};

use crate::theme::{ActiveTheme, RADIUS_SM, UI_TEXT_SM};

/// The glyph for a GPUI key name: `enter` → "↩", `a` → "A", `f5` → "F5".
pub fn key_glyph(key: &str) -> String {
    let glyph = match key {
        "enter" => "↩",
        "backspace" => "⌫",
        "delete" => "⌦",
        "escape" => "⎋",
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
    if m.shift {
        out.push('⇧');
    }
    if m.platform {
        out.push('⌘');
    }
    out.push_str(&key_glyph(&keystroke.key));
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
    pub fn for_action_in(action: &dyn Action, focus: &FocusHandle, window: &Window) -> Option<Self> {
        window
            .highest_precedence_binding_for_action_in(action, focus)
            .map(|binding| Self::from_binding(&binding))
    }

    /// The highest-precedence binding for `action` from the currently focused
    /// element.
    pub fn for_action_focused(action: &dyn Action, window: &Window) -> Option<Self> {
        window
            .highest_precedence_binding_for_action(action)
            .map(|binding| Self::from_binding(&binding))
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
        assert_eq!(fmt("escape"), "⎋");
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
}
