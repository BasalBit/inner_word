//! Keys between the three forms they have here: what GPUI hands over when
//! a key is pressed ([`gpui::Keystroke`]), the key as the keymap's data has
//! it ([`innerword_keymap::KeySequence`], in one normal form), and the glyphs
//! a person reads ("⇧⌘P").

use gpui::{Action, App, KeyContext};
use innerword_keymap::{KeySequence, KeystrokeError};
use innerword_ui::{KeyHint, key_hint};

/// The key GPUI reports for a press, as the keymap's data has it. A real
/// keyboard reports shift with a digit or punctuation as the shifted
/// character (`cmd-&`); the result is the readable form (`cmd-shift-7`),
/// the same key.
pub fn from_pressed(keystroke: &gpui::Keystroke) -> Result<KeySequence, KeystrokeError> {
    KeySequence::parse(&keystroke.unparse())
}

/// The keystrokes of `keys` as GPUI reads them.
pub fn to_gpui(keys: &KeySequence) -> Vec<gpui::Keystroke> {
    keys.strokes()
        .iter()
        .filter_map(|stroke| gpui::Keystroke::parse(&stroke.to_string()).ok())
        .collect()
}

/// `keys` as the glyphs the app shows everywhere: "⇧⌘P", "G G".
pub fn glyphs(keys: &KeySequence) -> String {
    key_hint::format_keystrokes(&to_gpui(keys))
}

/// Several keys as one text: "⌃↩, ⇧F10"; "none" for no key.
pub fn glyph_list(keys: &[KeySequence]) -> String {
    if keys.is_empty() {
        "none".to_string()
    } else {
        keys.iter().map(glyphs).collect::<Vec<_>>().join(", ")
    }
}

/// `keys` as a key hint.
pub fn hint(keys: &KeySequence) -> KeyHint {
    KeyHint::new(to_gpui(keys))
}

/// Whether two presses are the same key (the character a key types is not
/// part of it).
pub(crate) fn same_press(a: &gpui::Keystroke, b: &gpui::Keystroke) -> bool {
    a.modifiers == b.modifiers && a.key == b.key
}

/// Whether GPUI reports a modifier pressed alone, which is not a key yet.
pub(crate) fn is_modifier_alone(keystroke: &gpui::Keystroke) -> bool {
    matches!(
        keystroke.key.as_str(),
        "" | "shift" | "control" | "alt" | "platform" | "function"
    )
}

/// The modifiers held, as glyphs in menu order ("⌃⌥⇧⌘"), for the moment
/// before the key itself is pressed.
pub(crate) fn modifier_glyphs(modifiers: &gpui::Modifiers) -> String {
    let mut out = String::new();
    if modifiers.function {
        out.push_str("fn ");
    }
    if modifiers.control {
        out.push('⌃');
    }
    if modifiers.alt {
        out.push('⌥');
    }
    if modifiers.shift {
        out.push('⇧');
    }
    if modifiers.platform {
        out.push('⌘');
    }
    out
}

/// The key the view shows for one of its own actions: the binding of
/// `action` that applies in the key contexts `place` (outermost first),
/// the one bound last. Read from the keymap each time it is drawn, so a
/// footer follows a change at once; `None` when the action has no key
/// there, and the hint is then left out.
pub(crate) fn hint_in(action: &dyn Action, place: &[&str], cx: &App) -> Option<KeyHint> {
    let stack: Vec<KeyContext> = place
        .iter()
        .filter_map(|name| KeyContext::parse(name).ok())
        .collect();
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let mut anywhere = None;
    let mut here = None;
    for binding in keymap.bindings_for_action(action) {
        match binding.predicate() {
            None => anywhere = Some(binding),
            Some(predicate) if predicate.depth_of(&stack).is_some() => here = Some(binding),
            Some(_) => {}
        }
    }
    here.or(anywhere).map(KeyHint::from_binding)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pressed(text: &str) -> gpui::Keystroke {
        gpui::Keystroke::parse(text).unwrap()
    }

    #[test]
    fn a_pressed_key_becomes_the_normal_form() {
        assert_eq!(
            from_pressed(&pressed("cmd-alt-b")).unwrap().to_string(),
            "cmd-alt-b"
        );
        // What a real keyboard sends for shift with 7.
        assert_eq!(
            from_pressed(&pressed("cmd-&")).unwrap().to_string(),
            "cmd-shift-7"
        );
        assert_eq!(
            from_pressed(&pressed("escape")).unwrap().to_string(),
            "escape"
        );
        assert_eq!(
            from_pressed(&pressed("cmd--")).unwrap().to_string(),
            "cmd--"
        );
    }

    #[test]
    fn glyphs_are_the_apps() {
        let keys = |text: &str| KeySequence::parse(text).unwrap();
        assert_eq!(glyphs(&keys("cmd-shift-p")), "⇧⌘P");
        assert_eq!(glyphs(&keys("cmd-&")), "⇧⌘7");
        assert_eq!(glyphs(&keys("g g")), "G G");
        assert_eq!(glyphs(&keys("ctrl-enter")), "⌃↩");
        assert_eq!(
            glyph_list(&[keys("ctrl-enter"), keys("shift-f10")]),
            "⌃↩, ⇧F10"
        );
        assert_eq!(glyph_list(&[]), "none");
    }

    #[test]
    fn modifiers_alone_are_not_a_key() {
        assert!(is_modifier_alone(&pressed("shift")));
        assert!(!is_modifier_alone(&pressed("shift-a")));
        assert!(same_press(&pressed("cmd-b"), &pressed("cmd-b")));
        assert!(!same_press(&pressed("cmd-b"), &pressed("cmd-shift-b")));
    }
}
