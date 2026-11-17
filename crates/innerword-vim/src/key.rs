//! Keys as the engine sees them.

use std::fmt;

/// What was pressed, without modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyCode {
    /// A character key. The character is the one the key produces, with
    /// shift already applied (`'A'`, `'$'`, `' '`). With `ctrl` set it is the
    /// base letter in lower case (`ctrl-w` is `Char('w')`).
    Char(char),
    /// Escape.
    Escape,
    /// Return / Enter.
    Enter,
    /// Tab.
    Tab,
    /// Backspace (delete backward).
    Backspace,
    /// Forward delete.
    Delete,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
}

/// One key press: a [`KeyCode`] plus the control, alt (option) and shift
/// flags.
///
/// The shift flag only matters for named keys (`shift-enter` is a soft line
/// break in Insert mode, shifted arrows are left to the host); for
/// [`KeyCode::Char`] the character already carries it. Keys held with the
/// command key are application shortcuts and must not be sent to the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    /// The key.
    pub code: KeyCode,
    /// Control is held.
    pub ctrl: bool,
    /// Alt (option) is held.
    pub alt: bool,
    /// Shift is held (meaningful for named keys only).
    pub shift: bool,
}

impl Key {
    /// The escape key.
    pub const ESCAPE: Key = Key::named(KeyCode::Escape);
    /// The enter key.
    pub const ENTER: Key = Key::named(KeyCode::Enter);
    /// The backspace key.
    pub const BACKSPACE: Key = Key::named(KeyCode::Backspace);
    /// The tab key.
    pub const TAB: Key = Key::named(KeyCode::Tab);

    /// A plain character key.
    pub const fn char(c: char) -> Key {
        Key {
            code: KeyCode::Char(c),
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    /// A character key with control held (`Key::ctrl('w')`). The letter is
    /// stored in lower case.
    pub fn ctrl(c: char) -> Key {
        Key {
            code: KeyCode::Char(c.to_ascii_lowercase()),
            ctrl: true,
            alt: false,
            shift: false,
        }
    }

    /// A key without modifiers.
    pub const fn named(code: KeyCode) -> Key {
        Key {
            code,
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    /// Builder: sets the shift flag.
    pub const fn with_shift(mut self) -> Key {
        self.shift = true;
        self
    }

    /// Builder: sets the alt flag.
    pub const fn with_alt(mut self) -> Key {
        self.alt = true;
        self
    }

    /// The character of an unmodified character key.
    pub fn plain_char(&self) -> Option<char> {
        match self.code {
            KeyCode::Char(c) if !self.ctrl && !self.alt => Some(c),
            _ => None,
        }
    }

    /// The letter of a control chord (`ctrl-w` gives `'w'`).
    pub fn ctrl_char(&self) -> Option<char> {
        match self.code {
            KeyCode::Char(c) if self.ctrl && !self.alt => Some(c),
            _ => None,
        }
    }

    /// Whether this key leaves a mode or cancels a command: escape,
    /// `ctrl-[` or `ctrl-c`.
    pub fn is_escape(&self) -> bool {
        (self.code == KeyCode::Escape && !self.ctrl && !self.alt)
            || matches!(self.ctrl_char(), Some('[' | 'c'))
    }

    /// Builds a key from the parts of a platform keystroke, following the
    /// conventions of GPUI's `Keystroke`: `key` is the name of the key
    /// (`"a"`, `"escape"`, `"enter"`, `"backspace"`, `"left"`, `"space"`, …),
    /// `key_char` the text the key would type, if any.
    ///
    /// Returns `None` for keys the engine has no use for (function keys,
    /// bare modifiers). Callers should not pass keystrokes held with the
    /// command key at all.
    ///
    /// Rules: a named key keeps its modifiers. With control held, the key is
    /// `Char(key)` in lower case with `ctrl` set. Otherwise the typed
    /// character wins (`key_char`, so `shift-4` is `'$'` and a dead-key
    /// result is the composed character); without one, a single-character
    /// `key` is used, upper-cased when shift is held.
    pub fn from_keystroke(
        key: &str,
        key_char: Option<&str>,
        ctrl: bool,
        alt: bool,
        shift: bool,
    ) -> Option<Key> {
        let named = match key {
            "escape" => Some(KeyCode::Escape),
            "enter" | "return" => Some(KeyCode::Enter),
            "tab" => Some(KeyCode::Tab),
            "backspace" => Some(KeyCode::Backspace),
            "delete" => Some(KeyCode::Delete),
            "left" => Some(KeyCode::Left),
            "right" => Some(KeyCode::Right),
            "up" => Some(KeyCode::Up),
            "down" => Some(KeyCode::Down),
            "home" => Some(KeyCode::Home),
            "end" => Some(KeyCode::End),
            "pageup" => Some(KeyCode::PageUp),
            "pagedown" => Some(KeyCode::PageDown),
            _ => None,
        };
        if let Some(code) = named {
            return Some(Key {
                code,
                ctrl,
                alt,
                shift,
            });
        }
        let single = |s: &str| {
            let mut it = s.chars();
            match (it.next(), it.next()) {
                (Some(c), None) => Some(c),
                _ => None,
            }
        };
        let base = if key == "space" {
            Some(' ')
        } else {
            single(key)
        };
        if ctrl {
            let c = base?;
            return Some(Key {
                code: KeyCode::Char(c.to_ascii_lowercase()),
                ctrl: true,
                alt,
                shift: false,
            });
        }
        let typed = key_char.and_then(single);
        let c = match typed {
            Some(c) => c,
            None => {
                let c = base?;
                if shift {
                    c.to_uppercase().next().unwrap_or(c)
                } else {
                    c
                }
            }
        };
        Some(Key {
            code: KeyCode::Char(c),
            ctrl: false,
            alt,
            shift: false,
        })
    }

    /// Parses a key sequence in Vim notation: plain characters stand for
    /// themselves, special keys are written in angle brackets, case
    /// insensitive: `<Esc>`, `<CR>` / `<Enter>`, `<Tab>`, `<BS>`, `<Del>`,
    /// `<Left>`, `<Right>`, `<Up>`, `<Down>`, `<Home>`, `<End>`, `<PageUp>`,
    /// `<PageDown>`, `<Space>`, `<lt>` (a literal `<`), and the modified
    /// forms `<C-x>`, `<A-x>` / `<M-x>`, `<S-Left>`, which combine
    /// (`<C-S-Left>`). A `<` that does not start a known name is a literal
    /// `<`.
    ///
    /// ```
    /// use innerword_vim::Key;
    /// let keys = Key::parse_sequence("ciwnew<Esc><C-r>");
    /// assert_eq!(keys.len(), 8);
    /// assert_eq!(keys[6], Key::ESCAPE);
    /// assert_eq!(keys[7], Key::ctrl('r'));
    /// ```
    pub fn parse_sequence(s: &str) -> Vec<Key> {
        let mut out = Vec::new();
        let mut rest = s;
        while let Some(c) = rest.chars().next() {
            if c == '<'
                && let Some(end) = rest.find('>')
                && let Some(k) = parse_bracketed(&rest[1..end])
            {
                out.push(k);
                rest = &rest[end + 1..];
                continue;
            }
            out.push(Key::char(c));
            rest = &rest[c.len_utf8()..];
        }
        out
    }
}

fn parse_bracketed(inner: &str) -> Option<Key> {
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut name = inner;
    loop {
        let b = name.as_bytes();
        if b.len() > 2 && b[1] == b'-' {
            match b[0].to_ascii_lowercase() {
                b'c' => ctrl = true,
                b'a' | b'm' => alt = true,
                b's' => shift = true,
                _ => return None,
            }
            name = &name[2..];
        } else {
            break;
        }
    }
    let code = match name.to_ascii_lowercase().as_str() {
        "esc" | "escape" => KeyCode::Escape,
        "cr" | "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "bs" | "backspace" => KeyCode::Backspace,
        "del" | "delete" => KeyCode::Delete,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "space" => KeyCode::Char(' '),
        "lt" => KeyCode::Char('<'),
        _ => {
            let mut it = name.chars();
            match (it.next(), it.next()) {
                (Some(c), None) if ctrl || alt => {
                    KeyCode::Char(if ctrl { c.to_ascii_lowercase() } else { c })
                }
                _ => return None,
            }
        }
    };
    let is_char = matches!(code, KeyCode::Char(_));
    Some(Key {
        code,
        ctrl,
        alt,
        shift: shift && !is_char,
    })
}

impl fmt::Display for Key {
    /// Vim notation, the inverse of [`Key::parse_sequence`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self.code {
            KeyCode::Char(c) => {
                if !self.ctrl && !self.alt {
                    return match c {
                        '<' => f.write_str("<lt>"),
                        ' ' => f.write_str("<Space>"),
                        c => write!(f, "{c}"),
                    };
                }
                c.to_string()
            }
            KeyCode::Escape => "Esc".into(),
            KeyCode::Enter => "CR".into(),
            KeyCode::Tab => "Tab".into(),
            KeyCode::Backspace => "BS".into(),
            KeyCode::Delete => "Del".into(),
            KeyCode::Left => "Left".into(),
            KeyCode::Right => "Right".into(),
            KeyCode::Up => "Up".into(),
            KeyCode::Down => "Down".into(),
            KeyCode::Home => "Home".into(),
            KeyCode::End => "End".into(),
            KeyCode::PageUp => "PageUp".into(),
            KeyCode::PageDown => "PageDown".into(),
        };
        let plain = !self.ctrl && !self.alt && !self.shift;
        if plain {
            return write!(f, "<{name}>");
        }
        f.write_str("<")?;
        if self.ctrl {
            f.write_str("C-")?;
        }
        if self.alt {
            f.write_str("A-")?;
        }
        if self.shift {
            f.write_str("S-")?;
        }
        write!(f, "{name}>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vim_notation() {
        let k = Key::parse_sequence("d2w<Esc><C-w><cr><S-Left><A-x><lt><Space>a<b");
        assert_eq!(k[0], Key::char('d'));
        assert_eq!(k[1], Key::char('2'));
        assert_eq!(k[3], Key::ESCAPE);
        assert_eq!(k[4], Key::ctrl('w'));
        assert_eq!(k[5], Key::ENTER);
        assert_eq!(k[6], Key::named(KeyCode::Left).with_shift());
        assert_eq!(k[7], Key::char('x').with_alt());
        assert_eq!(k[8], Key::char('<'));
        assert_eq!(k[9], Key::char(' '));
        assert_eq!(k[10], Key::char('a'));
        assert_eq!(k[11], Key::char('<'));
        assert_eq!(k[12], Key::char('b'));
        assert_eq!(k.len(), 13);
    }

    #[test]
    fn display_round_trips() {
        let src = "x<Esc><C-r><CR><S-Left><lt><Space><A-x><BS><Tab>";
        let keys = Key::parse_sequence(src);
        let shown: String = keys.iter().map(|k| k.to_string()).collect();
        assert_eq!(Key::parse_sequence(&shown), keys);
    }

    #[test]
    fn from_keystroke_rules() {
        let k = Key::from_keystroke;
        assert_eq!(k("a", Some("a"), false, false, false), Some(Key::char('a')));
        assert_eq!(k("a", Some("A"), false, false, true), Some(Key::char('A')));
        assert_eq!(k("4", Some("$"), false, false, true), Some(Key::char('$')));
        assert_eq!(k("w", None, true, false, false), Some(Key::ctrl('w')));
        assert_eq!(k("[", None, true, false, false), Some(Key::ctrl('[')));
        assert_eq!(
            k("space", Some(" "), false, false, false),
            Some(Key::char(' '))
        );
        assert_eq!(k("escape", None, false, false, false), Some(Key::ESCAPE));
        assert_eq!(
            k("enter", None, false, false, true),
            Some(Key::ENTER.with_shift())
        );
        assert_eq!(k("a", None, false, false, true), Some(Key::char('A')));
        assert_eq!(k("f5", None, false, false, false), None);
        assert!(Key::ctrl('[').is_escape());
        assert!(Key::ESCAPE.is_escape());
        assert!(!Key::char('[').is_escape());
    }
}
