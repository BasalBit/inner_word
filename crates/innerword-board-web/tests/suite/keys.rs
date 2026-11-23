//! The whiteboard's own keys, read as the keymap reads a key.

use std::collections::BTreeSet;

use innerword_board_web::keys::{WHITEBOARD_KEYS, is_whiteboard_key};
use innerword_keymap::keystroke::{KeySequence, Keystroke};

#[test]
fn every_key_is_gpui_keystroke_text_in_normal_form() {
    for key in WHITEBOARD_KEYS {
        let read = Keystroke::parse(key.keys).unwrap_or_else(|e| panic!("{}: {e}", key.keys));
        assert_eq!(read.to_string(), key.keys, "{} is not written in normal form", key.keys);
        // A key a keyboard sends, as a person would write it in the shortcuts file.
        KeySequence::parse_written(key.keys).unwrap_or_else(|e| panic!("{}: {e}", key.keys));
        assert!(!key.what.is_empty() && key.what.chars().next().unwrap().is_uppercase(), "{}", key.keys);
    }
}

#[test]
fn no_key_is_listed_twice() {
    let mut seen = BTreeSet::new();
    for key in WHITEBOARD_KEYS {
        assert!(seen.insert(Keystroke::parse(key.keys).unwrap()), "{} twice", key.keys);
    }
}

#[test]
fn every_key_has_a_modifier() {
    // A bare key, or shift with one, is typed: it can never be bound where text is typed.
    for key in WHITEBOARD_KEYS {
        assert!(key.keys.starts_with("cmd-") || key.keys.starts_with("alt-"), "{}", key.keys);
    }
}

#[test]
fn the_keys_of_section_4_7() {
    for keys in [
        "cmd-z", "cmd-shift-z", "cmd-c", "cmd-x", "cmd-v", "cmd-shift-v", "cmd-a", "cmd-d", "cmd-g", "cmd-shift-g", "cmd-[", "cmd-]", "cmd-alt-[",
        "cmd-alt-]", "cmd-alt-c", "cmd-alt-v", "cmd-shift-up", "cmd-shift-down", "cmd-shift-left", "cmd-shift-right", "cmd-shift-,", "cmd-shift-.",
        "cmd-'", "cmd-k", "cmd-shift-l", "cmd-up", "cmd-down", "cmd-left", "cmd-right", "cmd-enter", "cmd-backspace", "cmd-delete", "alt-z", "alt-s",
        "alt-/", "alt-r", "alt-h", "alt-v", "alt-shift-c",
    ] {
        assert!(is_whiteboard_key(keys), "{keys}");
    }
    // The window keeps these on a whiteboard.
    for keys in ["cmd-shift-p", "cmd-p", "cmd-s", "cmd-shift-s", "cmd-f", "cmd-=", "cmd--", "cmd-0", "cmd-/", "cmd-w", "cmd-q", "ctrl-enter"] {
        assert!(!is_whiteboard_key(keys), "{keys}");
    }
}
