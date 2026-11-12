//! Keystroke text: GPUI's spelling of a key, read and written without GPUI.
//!
//! A [`Keystroke`] is one press (`cmd-shift-p`), a [`KeySequence`] one or
//! more presses in a row (`g g`, `ctrl-x ctrl-s`). Both are always in
//! **normal form**, so two spellings of the same key compare equal and
//! print the same.
//!
//! # What is read
//!
//! What `gpui::Keystroke::parse` of gpui 0.2.2 reads, the same way:
//! modifiers and a key joined by `-`, the modifiers in any order and any
//! case (`ctrl`, `alt`, `shift`, `fn`, `cmd`; `super`, `win` and
//! `secondary` are `cmd` on a Mac), the key last. A single upper-case
//! letter means shift with that letter (`cmd-B` is `cmd-shift-b`). The
//! minus key is written as itself (`cmd--`). A modifier with no key after
//! it is the key (`shift` alone), as in GPUI. The form `a->b` (a key with
//! the character it types, which GPUI has for tests) is read and the
//! character dropped.
//!
//! Read in addition, because a person edits the shortcuts file by hand:
//! `opt` and `option` for `alt`, `command` for `cmd`, `control` for `ctrl`,
//! `esc` for `escape` and `return` for `enter`. They are never written.
//!
//! Not read: a keystroke without a key (`cmd-`), two keys in one keystroke
//! (`cmd-a-b`), a word before the key that is no modifier (`hyper-e`), an
//! empty text.
//!
//! # What a person wrote
//!
//! GPUI takes any word as the name of a key (`cmd-entr` is the key "entr"
//! with command), and so does [`Keystroke::parse`], which has to read
//! whatever a keyboard reports. A text a person typed into the shortcuts
//! file is read with [`KeySequence::parse_written`], which reads the same
//! and then refuses what no keyboard sends: a key whose name is not one
//! ([`KeystrokeProblem::UnknownKey`]) and a modifier without a key
//! ([`KeystrokeProblem::ModifierAlone`]). Every [`KeystrokeError`] says
//! what is wrong in words for whoever edits the file.
//!
//! # Glyphs
//!
//! [`Keystroke::glyphs`] and [`KeySequence::glyphs`] write a key the way
//! the app shows keys ("⇧⌘P", "⌃↩", "G G"): the modifiers in the order of
//! the macOS menus (⌃⌥⇧⌘), then the key. It is the same text
//! `writer_ui::key_hint::format_keystrokes` gives (a test of the app holds
//! the two against each other for every key of the keymap); it is here so
//! that a sentence made without GPUI names a key as the list does.
//!
//! # What is written (the normal form)
//!
//! The modifiers in the order the app's own sources and documentation
//! write them, `fn-ctrl-cmd-alt-shift-`, then the key in lower case:
//! `cmd-alt-s`, `ctrl-cmd-space`, `cmd-alt-shift-;`. GPUI reads every
//! order, so the text can be handed to `KeyBinding::load` as it is. (GPUI's
//! own `Keystroke::unparse` writes `ctrl-alt-cmd-shift-`; reading its
//! output here gives the same keystroke.)
//!
//! # One key, two spellings
//!
//! A real keyboard reports shift with a digit or a punctuation key as the
//! shifted character without the shift flag (`cmd-shift-7` reaches the app
//! as `cmd-&`, see `parse_keystroke` in GPUI's macOS events). A binding
//! spelled `cmd-shift-7` only matches scripted keystrokes, one spelled
//! `cmd-&` only a real keyboard, so the app binds both, the readable one
//! last because key hints show the last.
//!
//! Here the pair is **one key**. The normal form is the readable spelling:
//! `cmd-&` and `cmd-shift-7` both read as the keystroke that prints
//! `cmd-shift-7`. [`Keystroke::spellings`] gives back what has to be bound:
//! the character a keyboard sends first, the readable form last
//! (`["cmd-&", "cmd-shift-7"]`). A key without such a pair has one
//! spelling.
//!
//! The pairs are those of the US layout (`1 !`, `2 @`, ... `/ ?`, 21 of
//! them), which is what the app's defaults assume too. On another layout
//! the label can be off (a key recorded as `cmd-&` is shown as shift with
//! 7 wherever `&` really is), but what was recorded still works: the
//! character the keyboard sent is always one of the spellings bound.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The modifier keys held with a key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Modifiers {
    /// The control key (`ctrl`).
    pub control: bool,
    /// The option key (`alt` in GPUI's spelling).
    pub alt: bool,
    /// The shift key.
    pub shift: bool,
    /// The command key (`cmd`; GPUI calls it `platform`).
    pub command: bool,
    /// The fn (Globe) key.
    pub function: bool,
}

impl Modifiers {
    /// Whether any modifier is held.
    pub fn any(self) -> bool {
        self.control || self.alt || self.shift || self.command || self.function
    }

    /// Whether nothing is held, or shift alone: what a keyboard needs to
    /// type a character.
    pub fn at_most_shift(self) -> bool {
        !(self.control || self.alt || self.command || self.function)
    }
}

/// What is wrong with a text that should be a keystroke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeystrokeProblem {
    /// The text is empty.
    Empty,
    /// There are modifiers and a `-`, and no key after them (`cmd-`).
    NoKey,
    /// More than one key in one keystroke (`cmd-a-b`). A sequence of keys
    /// is written with spaces (`cmd-k cmd-b`).
    MoreThanOneKey,
    /// White space inside one keystroke.
    Space,
    /// A word before the key that is not a modifier (`hyper-e`, `ctl-s`).
    UnknownModifier,
    /// The key has a name no key has (`cmd-entr`, `cmd+b`). Only
    /// [`KeySequence::parse_written`] says this: GPUI takes any name.
    UnknownKey,
    /// Modifiers and no key (`shift`, `cmd-shift`), which GPUI reads as a
    /// key and no keyboard sends. Only [`KeySequence::parse_written`] says
    /// this.
    ModifierAlone,
}

/// The modifiers as a person may write them, for a sentence.
const MODIFIERS_IN_WORDS: &str = "cmd, ctrl, alt (the option key), shift and fn";

/// The names of the keys that are not one character, for a sentence.
const KEY_NAMES_IN_WORDS: &str = "enter, tab, space, escape, backspace, delete, insert, up, down, \
    left, right, home, end, pageup, pagedown, or a function key like f5";

/// The names a key that is not one character can have: what GPUI's macOS
/// events report (gpui 0.2.2, `parse_keystroke`), and the two read in
/// addition.
const KEY_NAMES: [&str; 17] = [
    "space",
    "enter",
    "return",
    "tab",
    "backspace",
    "escape",
    "esc",
    "delete",
    "insert",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    "pageup",
    "pagedown",
];

/// Whether `word` (in lower case) is one of the spellings of a modifier.
fn is_modifier_word(word: &str) -> bool {
    matches!(
        word,
        "ctrl"
            | "control"
            | "alt"
            | "opt"
            | "option"
            | "shift"
            | "fn"
            | "cmd"
            | "command"
            | "super"
            | "win"
            | "secondary"
    )
}

/// Whether `name` (in lower case) is the name of a key: one character, a
/// function key (`f` and a number from 1; a Mac reports up to `f35`, and
/// a higher number is taken as written), or one of [`KEY_NAMES`].
fn is_key_name(name: &str) -> bool {
    single_char(name).is_some()
        || KEY_NAMES.contains(&name)
        || name
            .strip_prefix('f')
            .filter(|number| number.chars().all(|digit| digit.is_ascii_digit()))
            .and_then(|number| number.parse::<u16>().ok())
            .is_some_and(|number| number >= 1)
}

/// A text that is not a keystroke, with the reason.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeystrokeError {
    /// The text as it was given.
    pub text: String,
    /// What is wrong with it.
    pub problem: KeystrokeProblem,
}

impl KeystrokeError {
    /// The words of the text between its `-`, without the empty ones.
    fn words(&self) -> impl Iterator<Item = &str> {
        self.text.split('-').filter(|word| !word.is_empty())
    }
}

impl fmt::Display for KeystrokeError {
    /// What is wrong, for a person who wrote the text by hand: the text in
    /// quotes, the part of it that is wrong, and how it is written right.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = &self.text;
        match self.problem {
            KeystrokeProblem::Empty => write!(
                f,
                "an empty text stands where a key should (a key is written like cmd-shift-p; \
                 a command without a key has an empty list)"
            ),
            KeystrokeProblem::NoKey => write!(
                f,
                "\"{text}\" has no key after its modifiers (a key is written like cmd-shift-p)"
            ),
            KeystrokeProblem::ModifierAlone => write!(
                f,
                "\"{text}\" is modifiers alone, without a key (a key is written like \
                 cmd-shift-p)"
            ),
            KeystrokeProblem::UnknownModifier => {
                match self
                    .words()
                    .find(|word| !is_modifier_word(&word.to_ascii_lowercase()))
                {
                    Some(word) => write!(
                        f,
                        "\"{text}\": \"{word}\" is not a modifier (the modifiers are \
                         {MODIFIERS_IN_WORDS})"
                    ),
                    None => write!(
                        f,
                        "\"{text}\" has a modifier that is none (the modifiers are \
                         {MODIFIERS_IN_WORDS})"
                    ),
                }
            }
            KeystrokeProblem::UnknownKey => {
                let name = self.words().last().unwrap_or(text);
                if name.contains('+') {
                    write!(
                        f,
                        "\"{text}\" is joined with +: modifiers and a key are joined by - \
                         (a key is written like cmd-shift-p)"
                    )
                } else {
                    write!(
                        f,
                        "\"{text}\": \"{name}\" is not the name of a key (a key is one \
                         character, or one of {KEY_NAMES_IN_WORDS})"
                    )
                }
            }
            KeystrokeProblem::MoreThanOneKey => {
                let keys: Vec<String> = self
                    .words()
                    .filter(|word| !is_modifier_word(&word.to_ascii_lowercase()))
                    .map(|word| format!("\"{word}\""))
                    .collect();
                let named = match keys.as_slice() {
                    [] | [_] => String::new(),
                    [first, second] => format!(": {first} and {second}"),
                    many => format!(": {}", many.join(", ")),
                };
                write!(
                    f,
                    "\"{text}\" has more than one key in one keystroke{named} (modifiers and \
                     one key are joined by -, keys pressed one after the other are separated \
                     by a space)"
                )
            }
            KeystrokeProblem::Space => {
                write!(f, "\"{text}\" has a space inside one keystroke")
            }
        }
    }
}

impl std::error::Error for KeystrokeError {}

/// What macOS does with a key, for the keys it uses itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SystemUse {
    /// What the key does, as the end of a sentence that starts with the
    /// key: "switches between apps".
    pub what: &'static str,
    /// `true`: the key never reaches an app and no setting frees it (app
    /// switching, Force Quit, locking the screen, logging out): a shortcut
    /// on it would never run. `false`: macOS uses it unless the user
    /// switched that off or moved it in System Settings (Spotlight, the
    /// screenshots, Mission Control, the input source, ...), so a shortcut
    /// on it runs only then.
    pub kept: bool,
}

/// One key press: the modifiers and the key. Always in normal form (see
/// the [module docs](self)): equal keystrokes are the same key to the
/// user, whichever way they were spelled.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Keystroke {
    modifiers: Modifiers,
    key: String,
}

/// The US layout's pairs: the key, and the character shift makes of it.
const SHIFTED: [(char, char); 21] = [
    ('`', '~'),
    ('1', '!'),
    ('2', '@'),
    ('3', '#'),
    ('4', '$'),
    ('5', '%'),
    ('6', '^'),
    ('7', '&'),
    ('8', '*'),
    ('9', '('),
    ('0', ')'),
    ('-', '_'),
    ('=', '+'),
    ('[', '{'),
    (']', '}'),
    ('\\', '|'),
    (';', ':'),
    ('\'', '"'),
    (',', '<'),
    ('.', '>'),
    ('/', '?'),
];

/// The key under a shifted character (`&` gives `7`).
fn unshifted(character: char) -> Option<char> {
    SHIFTED
        .iter()
        .find(|(_, shifted)| *shifted == character)
        .map(|(plain, _)| *plain)
}

/// The character shift makes of a key (`7` gives `&`).
fn shifted(character: char) -> Option<char> {
    SHIFTED
        .iter()
        .find(|(plain, _)| *plain == character)
        .map(|(_, shifted)| *shifted)
}

fn single_char(key: &str) -> Option<char> {
    let mut chars = key.chars();
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

/// Keys that edit or move in a text without typing a character.
const EDITING_KEYS: [&str; 13] = [
    "enter",
    "tab",
    "backspace",
    "delete",
    "escape",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    "pageup",
    "pagedown",
];

/// The keys macOS uses itself: the key, what it does, and whether it is
/// kept from apps altogether (`true`: there is no setting that frees the
/// key; `false`: System Settings > Keyboard > Keyboard Shortcuts, or
/// another setting, can switch the system's use off).
const SYSTEM_KEYS: [(&str, &str, bool); 30] = [
    ("cmd-tab", "switches between apps", true),
    ("cmd-shift-tab", "switches between apps", true),
    ("cmd-space", "opens Spotlight", false),
    ("cmd-alt-escape", "opens Force Quit", true),
    ("ctrl-cmd-q", "locks the screen", true),
    ("cmd-shift-q", "logs out", true),
    ("cmd-alt-shift-q", "logs out", true),
    ("cmd-shift-3", "takes a screenshot", false),
    ("cmd-shift-4", "takes a screenshot", false),
    ("cmd-shift-5", "opens the screenshot tools", false),
    ("ctrl-cmd-shift-3", "takes a screenshot", false),
    ("ctrl-cmd-shift-4", "takes a screenshot", false),
    ("cmd-alt-space", "opens a Finder search window", false),
    ("ctrl-space", "switches the input source", false),
    ("ctrl-alt-space", "switches the input source", false),
    ("ctrl-up", "opens Mission Control", false),
    ("ctrl-down", "shows the windows of the app", false),
    ("ctrl-left", "moves to the desktop on the left", false),
    ("ctrl-right", "moves to the desktop on the right", false),
    ("f11", "shows the desktop", false),
    ("cmd-alt-d", "shows or hides the Dock", false),
    ("cmd-`", "moves to the next window of the app", false),
    (
        "cmd-shift-`",
        "moves to the previous window of the app",
        false,
    ),
    ("cmd-f5", "turns VoiceOver on or off", false),
    ("cmd-alt-f5", "opens the accessibility shortcuts", false),
    ("cmd-alt-8", "turns zoom on or off", false),
    ("ctrl-f2", "moves the keyboard to the menu bar", false),
    ("ctrl-f3", "moves the keyboard to the Dock", false),
    ("cmd-shift-/", "opens the Help menu", false),
    ("ctrl-cmd-d", "looks up the word under the pointer", false),
];

impl Keystroke {
    /// Reads one keystroke in GPUI's spelling. See the [module
    /// docs](self) for what is read.
    pub fn parse(text: &str) -> Result<Self, KeystrokeError> {
        let error = |problem| KeystrokeError {
            text: text.to_string(),
            problem,
        };
        if text.is_empty() {
            return Err(error(KeystrokeProblem::Empty));
        }
        if text.chars().any(char::is_whitespace) {
            return Err(error(KeystrokeProblem::Space));
        }

        let mut modifiers = Modifiers::default();
        let mut key: Option<String> = None;
        let mut parts = text.split('-').peekable();
        while let Some(part) = parts.next() {
            // A modifier, unless it is the last part and something has to
            // be the key (handled after the loop, as GPUI does).
            let lower = part.to_ascii_lowercase();
            match lower.as_str() {
                "ctrl" | "control" => {
                    modifiers.control = true;
                    continue;
                }
                "alt" | "opt" | "option" => {
                    modifiers.alt = true;
                    continue;
                }
                "shift" => {
                    modifiers.shift = true;
                    continue;
                }
                "fn" => {
                    modifiers.function = true;
                    continue;
                }
                "cmd" | "command" | "super" | "win" | "secondary" => {
                    modifiers.command = true;
                    continue;
                }
                _ => {}
            }

            if let Some(next) = parts.peek() {
                if part.is_empty() && next.is_empty() && text.ends_with('-') {
                    // "cmd--": the minus key.
                    key = Some("-".to_string());
                    break;
                } else if next.len() > 1 && next.starts_with('>') {
                    // "a->b": the key and the character it types; the
                    // character is dropped.
                    parts.next();
                    if parts.peek().is_some() {
                        return Err(error(KeystrokeProblem::MoreThanOneKey));
                    }
                } else if is_key_name(&lower) || part.is_empty() {
                    // A key, and more after it.
                    return Err(error(KeystrokeProblem::MoreThanOneKey));
                } else {
                    // A word before the key that is no key either: it was
                    // meant as a modifier.
                    return Err(error(KeystrokeProblem::UnknownModifier));
                }
            }

            let mut name = part.to_string();
            if name.len() == 1 && name.as_bytes()[0].is_ascii_uppercase() {
                modifiers.shift = true;
            }
            name.make_ascii_lowercase();
            key = Some(name);
        }

        // A modifier that stands alone is the key (GPUI's rule and order).
        let key = match key {
            Some(key) => key,
            None if text.ends_with('-') => return Err(error(KeystrokeProblem::NoKey)),
            None => {
                if std::mem::take(&mut modifiers.shift) {
                    "shift".to_string()
                } else if std::mem::take(&mut modifiers.control) {
                    "control".to_string()
                } else if std::mem::take(&mut modifiers.alt) {
                    "alt".to_string()
                } else if std::mem::take(&mut modifiers.command) {
                    "platform".to_string()
                } else if std::mem::take(&mut modifiers.function) {
                    "function".to_string()
                } else {
                    return Err(error(KeystrokeProblem::NoKey));
                }
            }
        };
        if key.is_empty() {
            return Err(error(KeystrokeProblem::NoKey));
        }
        Ok(Self::normal(modifiers, key))
    }

    /// A keystroke from its parts, as a GPUI `Keystroke` has them (its
    /// `modifiers` and its `key`; `platform` is [`Modifiers::command`]).
    /// The key is taken as GPUI names it (`"p"`, `"enter"`, `"&"`).
    pub fn from_parts(modifiers: Modifiers, key: &str) -> Result<Self, KeystrokeError> {
        let error = |problem| KeystrokeError {
            text: key.to_string(),
            problem,
        };
        if key.is_empty() {
            return Err(error(KeystrokeProblem::NoKey));
        }
        if key.chars().any(char::is_whitespace) {
            return Err(error(KeystrokeProblem::Space));
        }
        let mut modifiers = modifiers;
        let mut key = key.to_string();
        if key.len() == 1 && key.as_bytes()[0].is_ascii_uppercase() {
            modifiers.shift = true;
        }
        key.make_ascii_lowercase();
        Ok(Self::normal(modifiers, key))
    }

    /// Brings the parts into normal form: the names read in addition
    /// become GPUI's, and a shifted character becomes shift with its key.
    fn normal(mut modifiers: Modifiers, key: String) -> Self {
        let mut key = match key.as_str() {
            "esc" => "escape".to_string(),
            "return" => "enter".to_string(),
            _ => key,
        };
        if let Some(plain) = single_char(&key).and_then(unshifted) {
            modifiers.shift = true;
            key = plain.to_string();
        }
        Self { modifiers, key }
    }

    /// The modifiers held. For a key bound in two spellings this is the
    /// readable one's: `cmd-&` has shift and command.
    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// The key as GPUI names it, in lower case: `"p"`, `"enter"`, `"f10"`,
    /// `"7"` (never the shifted character).
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Whether the key is one that types a character: a letter, a digit,
    /// punctuation, or the space bar (not `enter`, `tab`, an arrow or a
    /// function key).
    pub fn is_character(&self) -> bool {
        self.key == "space" || single_char(&self.key).is_some()
    }

    /// The guard rail's first question: is this a bare character, or shift
    /// with a character? Bound where text is typed, such a key could no
    /// longer be typed.
    pub fn types_text(&self) -> bool {
        self.is_character() && self.modifiers.at_most_shift()
    }

    /// Whether this is option with a character, with or without shift: on
    /// a Mac keyboard that types a special character or starts an accent
    /// (`alt-e` then a letter writes é). Bound where text is typed, that
    /// character could no longer be typed.
    pub fn types_special_character(&self) -> bool {
        self.is_character()
            && self.modifiers.alt
            && !(self.modifiers.control || self.modifiers.command || self.modifiers.function)
    }

    /// Whether this is a key a text needs to be edited at all, alone or
    /// with shift: `enter`, `tab`, `backspace`, `delete`, `escape`, the
    /// arrows, `home`, `end`, `pageup`, `pagedown`.
    pub fn is_editing_key(&self) -> bool {
        self.modifiers.at_most_shift() && EDITING_KEYS.contains(&self.key.as_str())
    }

    /// Whether the key is a modifier pressed alone (`shift`), which GPUI
    /// can read and no keyboard sends as a key press.
    pub fn is_modifier_alone(&self) -> bool {
        matches!(
            self.key.as_str(),
            "shift" | "control" | "alt" | "platform" | "function"
        )
    }

    /// The guard rail's second question: does macOS use this key itself?
    /// `None` for a key it leaves to apps. Any key with the fn (Globe)
    /// key held counts as used by macOS (not kept).
    pub fn system_use(&self) -> Option<SystemUse> {
        for (text, what, kept) in SYSTEM_KEYS {
            if Keystroke::parse(text).is_ok_and(|key| key == *self) {
                return Some(SystemUse { what, kept });
            }
        }
        if self.modifiers.function {
            return Some(SystemUse {
                what: "is one of the Globe key's shortcuts",
                kept: false,
            });
        }
        None
    }

    /// Whether the key has a name a key can have: one character, a
    /// function key (`f1`, `f2`, ...), or one of the named keys a Mac
    /// keyboard reports (`enter`, `tab`, `space`, `escape`, `backspace`,
    /// `delete`, `insert`, the arrows, `home`, `end`, `pageup`,
    /// `pagedown`). GPUI reads any word as a key; a word that is none of
    /// these is a key no keyboard sends.
    pub fn has_known_key(&self) -> bool {
        is_key_name(&self.key)
    }

    /// The key as the app shows keys: the modifiers as glyphs in the order
    /// of the macOS menus (⌃⌥⇧⌘, "fn " before them), then the key ("⇧⌘P",
    /// "⌃↩", "F5", "Esc"). See the [module docs](self).
    pub fn glyphs(&self) -> String {
        let mut out = String::new();
        if self.modifiers.function {
            out.push_str("fn ");
        }
        if self.modifiers.control {
            out.push('\u{2303}');
        }
        if self.modifiers.alt {
            out.push('\u{2325}');
        }
        if self.modifiers.shift {
            out.push('\u{21e7}');
        }
        if self.modifiers.command {
            out.push('\u{2318}');
        }
        out.push_str(&key_glyph(&self.key));
        out
    }

    /// Every spelling that has to be bound for this key to work, each one
    /// a text GPUI reads: for shift with a digit or a punctuation key the
    /// character a keyboard sends, then the readable form
    /// (`["cmd-&", "cmd-shift-7"]`); for every other key the one normal
    /// form. Bind them in this order: key hints show the last.
    pub fn spellings(&self) -> Vec<String> {
        let readable = self.to_string();
        if self.modifiers.shift
            && let Some(sent) = single_char(&self.key).and_then(shifted)
        {
            let mut modifiers = self.modifiers;
            modifiers.shift = false;
            return vec![write(modifiers, &sent.to_string()), readable];
        }
        vec![readable]
    }
}

/// The glyph of a key by its GPUI name: `enter` is "↩", `a` is "A", `f5`
/// is "F5". Escape is written "Esc" (its glyph reads as an undo arrow at
/// interface sizes), as the app's key hints write it.
fn key_glyph(key: &str) -> String {
    let glyph = match key {
        "enter" => "\u{21a9}",
        "backspace" => "\u{232b}",
        "delete" => "\u{2326}",
        "escape" => "Esc",
        "tab" => "\u{21e5}",
        "space" => "Space",
        "up" => "\u{2191}",
        "down" => "\u{2193}",
        "left" => "\u{2190}",
        "right" => "\u{2192}",
        "pageup" => "\u{21de}",
        "pagedown" => "\u{21df}",
        "home" => "\u{2196}",
        "end" => "\u{2198}",
        "insert" => "Ins",
        _ => {
            return if key.chars().count() == 1 {
                key.to_uppercase()
            } else if let Some(number) = key
                .strip_prefix('f')
                .filter(|number| number.parse::<u8>().is_ok())
            {
                format!("F{number}")
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

fn write(modifiers: Modifiers, key: &str) -> String {
    let mut text = String::new();
    if modifiers.function {
        text.push_str("fn-");
    }
    if modifiers.control {
        text.push_str("ctrl-");
    }
    if modifiers.command {
        text.push_str("cmd-");
    }
    if modifiers.alt {
        text.push_str("alt-");
    }
    if modifiers.shift {
        text.push_str("shift-");
    }
    text.push_str(key);
    text
}

impl fmt::Display for Keystroke {
    /// The normal form, which GPUI reads: `cmd-alt-s`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&write(self.modifiers, &self.key))
    }
}

impl FromStr for Keystroke {
    type Err = KeystrokeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

/// One or more keystrokes pressed one after the other: what a binding is
/// bound to. Never empty. Written as its keystrokes separated by one
/// space (`g g`), which is what `KeyBinding::load` reads.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeySequence(Vec<Keystroke>);

impl KeySequence {
    /// Reads keystrokes separated by white space.
    pub fn parse(text: &str) -> Result<Self, KeystrokeError> {
        let strokes = text
            .split_whitespace()
            .map(Keystroke::parse)
            .collect::<Result<Vec<_>, _>>()?;
        if strokes.is_empty() {
            return Err(KeystrokeError {
                text: text.to_string(),
                problem: KeystrokeProblem::Empty,
            });
        }
        Ok(Self(strokes))
    }

    /// Reads keystrokes a person wrote (an entry of the shortcuts file):
    /// what [`KeySequence::parse`] reads, without the two things GPUI
    /// reads and no keyboard sends. A key whose name is none
    /// ([`KeystrokeProblem::UnknownKey`]: `cmd-entr`, `cmd+b`) and
    /// modifiers without a key ([`KeystrokeProblem::ModifierAlone`]:
    /// `shift`) are errors here. The error has the keystroke that is
    /// wrong as its text.
    pub fn parse_written(text: &str) -> Result<Self, KeystrokeError> {
        let sequence = Self::parse(text)?;
        for (stroke, written) in sequence.0.iter().zip(text.split_whitespace()) {
            let problem = if stroke.is_modifier_alone() {
                KeystrokeProblem::ModifierAlone
            } else if !stroke.has_known_key() {
                KeystrokeProblem::UnknownKey
            } else {
                continue;
            };
            return Err(KeystrokeError {
                text: written.to_string(),
                problem,
            });
        }
        Ok(sequence)
    }

    /// The keys as the app shows them, one after the other: "⇧⌘P",
    /// "G G". See [`Keystroke::glyphs`].
    pub fn glyphs(&self) -> String {
        self.0
            .iter()
            .map(Keystroke::glyphs)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// A sequence of one keystroke.
    pub fn single(keystroke: Keystroke) -> Self {
        Self(vec![keystroke])
    }

    /// A sequence from its keystrokes; `None` when there are none.
    pub fn from_strokes(strokes: Vec<Keystroke>) -> Option<Self> {
        (!strokes.is_empty()).then_some(Self(strokes))
    }

    /// The keystrokes, in the order they are pressed.
    pub fn strokes(&self) -> &[Keystroke] {
        &self.0
    }

    /// The first keystroke.
    pub fn first(&self) -> &Keystroke {
        &self.0[0]
    }

    /// Whether this is one keystroke and not several in a row.
    pub fn is_single(&self) -> bool {
        self.0.len() == 1
    }

    /// Whether `other` begins this sequence (or is the same). Two
    /// bindings of which one begins the other get in each other's way:
    /// the short one is pressed on the way to the long one.
    pub fn starts_with(&self, other: &KeySequence) -> bool {
        self.0.starts_with(&other.0)
    }

    /// Whether the two sequences get in each other's way: the same, or
    /// one begins the other.
    pub fn collides_with(&self, other: &KeySequence) -> bool {
        self.starts_with(other) || other.starts_with(self)
    }

    /// Every spelling that has to be bound (see [`Keystroke::spellings`]):
    /// one for most keys, two for shift with a digit or punctuation, and
    /// for a sequence every combination of its keystrokes' spellings. The
    /// readable one is last.
    pub fn spellings(&self) -> Vec<String> {
        let mut texts = vec![String::new()];
        for stroke in &self.0 {
            let spellings = stroke.spellings();
            let mut next = Vec::with_capacity(texts.len() * spellings.len());
            // The readable spelling of a stroke is its last: keeping the
            // outer loop on the stroke's spellings puts the all-readable
            // sequence at the very end.
            for spelling in &spellings {
                for text in &texts {
                    let mut joined = text.clone();
                    if !joined.is_empty() {
                        joined.push(' ');
                    }
                    joined.push_str(spelling);
                    next.push(joined);
                }
            }
            texts = next;
        }
        texts
    }
}

impl fmt::Display for KeySequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (ix, stroke) in self.0.iter().enumerate() {
            if ix > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{stroke}")?;
        }
        Ok(())
    }
}

impl FromStr for KeySequence {
    type Err = KeystrokeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl From<Keystroke> for KeySequence {
    fn from(keystroke: Keystroke) -> Self {
        Self::single(keystroke)
    }
}

impl Serialize for Keystroke {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Keystroke {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Keystroke::parse(&text).map_err(serde::de::Error::custom)
    }
}

impl Serialize for KeySequence {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for KeySequence {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        KeySequence::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> Keystroke {
        Keystroke::parse(text).unwrap()
    }

    #[test]
    fn the_normal_form_orders_modifiers_and_lowers_the_key() {
        assert_eq!(key("shift-cmd-P").to_string(), "cmd-shift-p");
        assert_eq!(key("alt-cmd-s").to_string(), "cmd-alt-s");
        assert_eq!(key("CMD-ALT-s"), key("cmd-alt-s"));
        assert_eq!(key("CMD-ALT-S"), key("cmd-alt-shift-s"));
        assert_eq!(key("cmd-B"), key("cmd-shift-b"));
        assert_eq!(key("Enter").to_string(), "enter");
        assert_eq!(
            key("fn-shift-alt-cmd-ctrl-f5").to_string(),
            "fn-ctrl-cmd-alt-shift-f5"
        );
        assert_eq!(key("super-k"), key("cmd-k"));
        assert_eq!(key("win-k"), key("cmd-k"));
        assert_eq!(key("secondary-k"), key("cmd-k"));
    }

    #[test]
    fn the_names_a_person_writes_are_read_and_never_written() {
        assert_eq!(key("opt-cmd-s").to_string(), "cmd-alt-s");
        assert_eq!(key("option-s"), key("alt-s"));
        assert_eq!(key("command-s"), key("cmd-s"));
        assert_eq!(key("control-s"), key("ctrl-s"));
        assert_eq!(key("esc").to_string(), "escape");
        assert_eq!(key("cmd-return").to_string(), "cmd-enter");
    }

    #[test]
    fn the_minus_key_and_the_greater_than_key() {
        assert_eq!(key("cmd--").key(), "-");
        assert_eq!(key("cmd--").to_string(), "cmd--");
        assert_eq!(key("-").key(), "-");
        assert_eq!(key("cmd->"), key("cmd-shift-."));
        assert_eq!(key(">"), key("shift-."));
        // shift with minus: the underscore.
        assert_eq!(key("cmd-_").to_string(), "cmd-shift--");
        assert_eq!(key("cmd-shift--"), key("cmd-_"));
    }

    #[test]
    fn a_shifted_character_is_shift_with_its_key() {
        for (plain, shifted) in SHIFTED {
            let readable = key(&format!("cmd-shift-{plain}"));
            let sent = key(&format!("cmd-{shifted}"));
            assert_eq!(readable, sent, "{plain} {shifted}");
            assert_eq!(readable.key(), plain.to_string());
            assert!(readable.modifiers().shift);
            assert_eq!(
                readable.spellings(),
                vec![format!("cmd-{shifted}"), format!("cmd-shift-{plain}")]
            );
        }
        // Without shift a digit is one key with one spelling.
        assert_eq!(key("cmd-7").spellings(), vec!["cmd-7"]);
        // Letters keep the shift flag on a real keyboard.
        assert_eq!(key("shift-g").spellings(), vec!["shift-g"]);
        assert_eq!(key("cmd-shift-enter").spellings(), vec!["cmd-shift-enter"]);
        // Option does not change the rule.
        assert_eq!(
            key("cmd-alt-shift-;").spellings(),
            vec!["cmd-alt-:", "cmd-alt-shift-;"]
        );
    }

    #[test]
    fn a_modifier_alone_is_the_key_as_in_gpui() {
        assert_eq!(key("shift").key(), "shift");
        assert!(!key("shift").modifiers().any());
        assert!(key("shift").is_modifier_alone());
        assert_eq!(key("cmd-shift").key(), "shift");
        assert!(key("cmd-shift").modifiers().command);
        assert_eq!(key("cmd").key(), "platform");
    }

    #[test]
    fn what_is_not_a_keystroke() {
        let problem = |text: &str| Keystroke::parse(text).unwrap_err().problem;
        assert_eq!(problem(""), KeystrokeProblem::Empty);
        assert_eq!(problem("cmd-"), KeystrokeProblem::NoKey);
        assert_eq!(problem("cmd-a-b"), KeystrokeProblem::MoreThanOneKey);
        assert_eq!(problem("cmd-a b"), KeystrokeProblem::Space);
        assert_eq!(problem("a--"), KeystrokeProblem::MoreThanOneKey);
        assert!(
            Keystroke::parse("cmd-a-b")
                .unwrap_err()
                .to_string()
                .contains("cmd-a-b")
        );
        // A word before the key that is neither a modifier nor a key.
        assert_eq!(problem("hyper-e"), KeystrokeProblem::UnknownModifier);
        assert_eq!(problem("cmd-meta-e"), KeystrokeProblem::UnknownModifier);
        assert_eq!(problem("ctl-s"), KeystrokeProblem::UnknownModifier);
        // A key and one more: two keys.
        assert_eq!(problem("enter-b"), KeystrokeProblem::MoreThanOneKey);
        assert_eq!(problem("f5-f6"), KeystrokeProblem::MoreThanOneKey);
    }

    /// Each way a key text can be wrong is said in its own words, with the
    /// part that is wrong named: this is what a person who edited the
    /// shortcuts file by hand reads.
    #[test]
    fn every_way_a_written_key_can_be_wrong_is_said_in_its_own_words() {
        let says = |text: &str| KeySequence::parse_written(text).unwrap_err().to_string();
        let problem = |text: &str| KeySequence::parse_written(text).unwrap_err().problem;

        // A modifier the app does not know: one key, not several.
        assert_eq!(problem("hyper-e"), KeystrokeProblem::UnknownModifier);
        assert_eq!(
            says("hyper-e"),
            "\"hyper-e\": \"hyper\" is not a modifier (the modifiers are cmd, ctrl, alt (the \
             option key), shift and fn)"
        );
        assert!(!says("hyper-e").contains("more than one key"));
        assert!(says("cmd-meta-e").contains("\"meta\" is not a modifier"));

        // A key name no key has.
        assert_eq!(problem("cmd-entr"), KeystrokeProblem::UnknownKey);
        assert_eq!(
            says("cmd-entr"),
            "\"cmd-entr\": \"entr\" is not the name of a key (a key is one character, or one \
             of enter, tab, space, escape, backspace, delete, insert, up, down, left, right, \
             home, end, pageup, pagedown, or a function key like f5)"
        );
        assert_eq!(problem("cmd-f0"), KeystrokeProblem::UnknownKey);
        assert_eq!(problem("f5x"), KeystrokeProblem::UnknownKey);
        // Joined with a plus sign, as other programs write keys.
        assert_eq!(problem("cmd+b"), KeystrokeProblem::UnknownKey);
        assert_eq!(
            says("cmd+b"),
            "\"cmd+b\" is joined with +: modifiers and a key are joined by - (a key is written \
             like cmd-shift-p)"
        );

        // An empty text.
        assert_eq!(problem(""), KeystrokeProblem::Empty);
        assert_eq!(problem("   "), KeystrokeProblem::Empty);
        assert_eq!(
            says(""),
            "an empty text stands where a key should (a key is written like cmd-shift-p; a \
             command without a key has an empty list)"
        );

        // Modifiers and no key.
        assert_eq!(problem("cmd-"), KeystrokeProblem::NoKey);
        assert_eq!(
            says("cmd-"),
            "\"cmd-\" has no key after its modifiers (a key is written like cmd-shift-p)"
        );
        assert_eq!(problem("shift"), KeystrokeProblem::ModifierAlone);
        assert_eq!(problem("cmd-shift"), KeystrokeProblem::ModifierAlone);
        assert_eq!(
            says("cmd-shift"),
            "\"cmd-shift\" is modifiers alone, without a key (a key is written like cmd-shift-p)"
        );

        // Two keys in one keystroke.
        assert_eq!(problem("cmd-a-b"), KeystrokeProblem::MoreThanOneKey);
        assert_eq!(
            says("cmd-a-b"),
            "\"cmd-a-b\" has more than one key in one keystroke: \"a\" and \"b\" (modifiers and \
             one key are joined by -, keys pressed one after the other are separated by a space)"
        );

        // In a sequence the keystroke that is wrong is the one named.
        assert_eq!(problem("cmd-k hyper-e"), KeystrokeProblem::UnknownModifier);
        assert!(says("cmd-k hyper-e").starts_with("\"hyper-e\": "));
        assert!(says("cmd-k entr").starts_with("\"entr\": "));

        // Several keystrokes in one text are a sequence, and read.
        for text in [
            "g g",
            "cmd-k cmd-b",
            "cmd-shift-p",
            "Cmd-Opt-S",
            "esc",
            "cmd-return",
            "cmd--",
            "cmd-&",
            "ö",
            "f35",
            "ctrl-f91",
            "shift-f10",
            "ctrl-insert",
        ] {
            assert_eq!(
                KeySequence::parse_written(text).ok(),
                KeySequence::parse(text).ok(),
                "{text}"
            );
            assert!(KeySequence::parse_written(text).is_ok(), "{text}");
        }
    }

    #[test]
    fn glyphs_are_written_as_the_app_shows_keys() {
        let glyphs = |text: &str| KeySequence::parse(text).unwrap().glyphs();
        assert_eq!(glyphs("cmd-shift-p"), "⇧⌘P");
        assert_eq!(glyphs("cmd-alt-b"), "⌥⌘B");
        assert_eq!(glyphs("ctrl-cmd-alt-shift-f5"), "⌃⌥⇧⌘F5");
        assert_eq!(glyphs("fn-f"), "fn F");
        assert_eq!(glyphs("cmd-&"), "⇧⌘7");
        assert_eq!(glyphs("i"), "I");
        assert_eq!(glyphs("g g"), "G G");
        assert_eq!(glyphs("ctrl-enter"), "⌃↩");
        assert_eq!(glyphs("cmd-tab"), "⌘⇥");
        assert_eq!(glyphs("escape"), "Esc");
        assert_eq!(glyphs("cmd-space"), "⌘Space");
        assert_eq!(glyphs("alt-up"), "⌥↑");
        assert_eq!(glyphs("cmd-backspace"), "⌘⌫");
        assert_eq!(glyphs("shift-pagedown"), "⇧⇟");
        assert_eq!(key("cmd-shift-7").glyphs(), "⇧⌘7");
    }

    #[test]
    fn the_character_a_test_keystroke_types_is_dropped() {
        assert_eq!(key("a->b"), key("a"));
        assert_eq!(key("alt-s->ß"), key("alt-s"));
    }

    #[test]
    fn the_guard_rail_questions() {
        for text in [
            "a",
            "shift-a",
            "A",
            "7",
            "?",
            "space",
            "shift-space",
            "ö",
            "-",
        ] {
            assert!(key(text).types_text(), "{text}");
        }
        for text in [
            "cmd-a", "ctrl-a", "alt-a", "enter", "tab", "f5", "escape", "fn-a", "up",
        ] {
            assert!(!key(text).types_text(), "{text}");
        }
        assert!(key("alt-e").types_special_character());
        assert!(key("alt-shift-2").types_special_character());
        assert!(!key("cmd-alt-e").types_special_character());
        assert!(!key("alt-up").types_special_character());
        assert!(key("enter").is_editing_key());
        assert!(key("shift-tab").is_editing_key());
        assert!(!key("cmd-enter").is_editing_key());
        assert!(!key("a").is_editing_key());
    }

    #[test]
    fn the_keys_of_macos() {
        let kept = key("cmd-tab").system_use().unwrap();
        assert!(kept.kept);
        assert_eq!(kept.what, "switches between apps");
        assert!(key("cmd-shift-tab").system_use().unwrap().kept);
        // Spotlight and the screenshots can be switched off or moved.
        let spotlight = key("cmd-space").system_use().unwrap();
        assert!(!spotlight.kept);
        assert_eq!(spotlight.what, "opens Spotlight");
        assert!(
            !key("cmd-$").system_use().unwrap().kept,
            "cmd-shift-4 as a keyboard sends it"
        );
        let usual = key("ctrl-up").system_use().unwrap();
        assert!(!usual.kept);
        assert!(!key("cmd-~").system_use().unwrap().kept);
        assert!(!key("fn-f").system_use().unwrap().kept);
        // Keys the app binds itself are not the system's.
        for text in [
            "cmd-q",
            "cmd-h",
            "cmd-m",
            "ctrl-cmd-f",
            "ctrl-cmd-space",
            "cmd-b",
        ] {
            assert_eq!(key(text).system_use(), None, "{text}");
        }
        // Every key of the table is read.
        for (text, _, _) in SYSTEM_KEYS {
            assert!(Keystroke::parse(text).is_ok(), "{text}");
        }
    }

    #[test]
    fn sequences() {
        let sequence = KeySequence::parse("ctrl-x  ctrl-s").unwrap();
        assert_eq!(sequence.to_string(), "ctrl-x ctrl-s");
        assert_eq!(sequence.strokes().len(), 2);
        assert!(!sequence.is_single());
        assert!(KeySequence::parse("cmd-p").unwrap().is_single());
        assert!(KeySequence::parse("  ").is_err());
        assert!(KeySequence::parse("cmd-p cmd-").is_err());

        let g = KeySequence::parse("g").unwrap();
        let gg = KeySequence::parse("g g").unwrap();
        assert!(gg.starts_with(&g));
        assert!(!g.starts_with(&gg));
        assert!(g.collides_with(&gg) && gg.collides_with(&g));
        assert!(!gg.collides_with(&KeySequence::parse("g h").unwrap()));

        assert_eq!(gg.spellings(), vec!["g g"]);
        assert_eq!(
            KeySequence::parse("cmd-k cmd-shift-7").unwrap().spellings(),
            vec!["cmd-k cmd-&", "cmd-k cmd-shift-7"]
        );
        let both = KeySequence::parse("shift-1 shift-2").unwrap().spellings();
        assert_eq!(both.len(), 4);
        assert_eq!(both.last().unwrap(), "shift-1 shift-2");
        assert_eq!(both.first().unwrap(), "! @");
    }

    #[test]
    fn from_the_parts_of_a_gpui_keystroke() {
        let command = Modifiers {
            command: true,
            ..Modifiers::default()
        };
        assert_eq!(
            Keystroke::from_parts(command, "&").unwrap(),
            key("cmd-shift-7")
        );
        assert_eq!(Keystroke::from_parts(command, "p").unwrap(), key("cmd-p"));
        assert_eq!(
            Keystroke::from_parts(Modifiers::default(), "-").unwrap(),
            key("-")
        );
        assert!(Keystroke::from_parts(command, "").is_err());
    }

    #[test]
    fn as_json_a_key_is_its_text() {
        let sequence = KeySequence::parse("cmd-&").unwrap();
        assert_eq!(serde_json::to_string(&sequence).unwrap(), "\"cmd-shift-7\"");
        let back: KeySequence = serde_json::from_str("\"cmd-&\"").unwrap();
        assert_eq!(back, sequence);
        assert!(serde_json::from_str::<KeySequence>("\"cmd-\"").is_err());
        assert!(serde_json::from_str::<Keystroke>("\"cmd-a cmd-b\"").is_err());
    }
}
