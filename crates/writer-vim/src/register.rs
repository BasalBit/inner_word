//! Registers.

use std::collections::{HashMap, VecDeque};

use writer_core::{Block, Fragment};

use crate::host::VimHost;

/// What a register holds: a rich-text [`Fragment`] and whether it was
/// yanked or deleted as whole lines.
///
/// A linewise register holds whole blocks (kind, alignment, indent and
/// inline formatting included), so `ddp` moves a heading as a heading. A
/// characterwise register holds the copied part of each block it spans.
#[derive(Clone, Debug, PartialEq)]
pub struct RegisterContent {
    /// The content. Comment anchors never travel in fragments.
    pub fragment: Fragment,
    /// Whole lines (`yy`, `dd`, Visual Line) rather than a run of text.
    pub linewise: bool,
}

impl RegisterContent {
    /// A characterwise register.
    pub fn charwise(fragment: Fragment) -> RegisterContent {
        RegisterContent {
            fragment,
            linewise: false,
        }
    }

    /// A linewise register.
    pub fn linewise(fragment: Fragment) -> RegisterContent {
        RegisterContent {
            fragment,
            linewise: true,
        }
    }

    /// A register from plain text, following the Vim convention for the
    /// system clipboard: text that ends with a line break is linewise (the
    /// final break is not part of the content), anything else is
    /// characterwise. Each line becomes a paragraph.
    pub fn from_plain_text(text: &str) -> RegisterContent {
        let normalized = writer_core::text::normalize_input(text);
        match normalized.strip_suffix('\n') {
            Some(body) => {
                let fragment = if body.is_empty() {
                    Fragment::new(vec![Block::paragraph("")])
                } else {
                    Fragment::from_plain_text(body)
                };
                RegisterContent::linewise(fragment)
            }
            None => RegisterContent::charwise(Fragment::from_plain_text(&normalized)),
        }
    }

    /// The text without formatting: blocks joined by `\n`, void blocks as
    /// empty lines. A linewise register does not get a trailing line break
    /// here; see [`RegisterContent::to_plain_text`].
    pub fn text(&self) -> String {
        self.fragment.to_plain_text()
    }

    /// The text for the system clipboard: [`RegisterContent::text`], plus a
    /// final line break when the register is linewise, so that
    /// [`RegisterContent::from_plain_text`] restores the kind.
    pub fn to_plain_text(&self) -> String {
        let mut t = self.text();
        if self.linewise {
            t.push('\n');
        }
        t
    }

    /// Whether the register holds nothing at all (no block).
    pub fn is_empty(&self) -> bool {
        self.fragment.blocks().is_empty()
    }

    /// Appends `other` (an upper-case register name). Two characterwise
    /// parts are joined inline; as soon as either part is linewise the
    /// result is linewise and the parts stay separate lines.
    pub(crate) fn append(&mut self, other: &RegisterContent) {
        let mut blocks: Vec<Block> = self.fragment.blocks().to_vec();
        let add = other.fragment.blocks();
        if self.linewise || other.linewise || blocks.is_empty() || add.is_empty() {
            blocks.extend(add.iter().cloned());
            self.linewise = self.linewise || other.linewise;
        } else {
            let last = blocks.pop().expect("non-empty");
            let first = &add[0];
            if last.is_void() || first.is_void() {
                blocks.push(last);
                blocks.push(first.clone());
            } else {
                let mut merged = last;
                for (_, text, attrs) in first.text_spans() {
                    merged = merged.with_span(text, attrs.clone());
                }
                blocks.push(merged);
            }
            blocks.extend(add[1..].iter().cloned());
        }
        self.fragment = Fragment::new(blocks);
    }
}

/// Whether `c` names a register the engine knows.
pub(crate) fn is_register_name(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '"' | '-' | '+' | '*' | '_' | '.' | ':' | '/')
}

/// Whether register `c` can be written by yank, delete or put commands.
pub(crate) fn is_writable(c: char) -> bool {
    is_register_name(c) && !matches!(c, '.' | ':' | '/')
}

/// The register store.
#[derive(Clone, Debug, Default)]
pub(crate) struct Registers {
    unnamed: Option<RegisterContent>,
    yank: Option<RegisterContent>,
    /// `"1` (front) to `"9`.
    numbered: VecDeque<RegisterContent>,
    small: Option<RegisterContent>,
    named: HashMap<char, RegisterContent>,
    /// Mirror unnamed yanks, deletes and puts to the system clipboard.
    pub unnamed_clipboard: bool,
}

impl Registers {
    /// Stores `content` in an explicitly named register. Returns false for
    /// the black hole (nothing stored anywhere).
    fn store_named(
        &mut self,
        name: char,
        content: &RegisterContent,
        host: &mut dyn VimHost,
    ) -> bool {
        match name {
            '_' => return false,
            '+' | '*' => host.clipboard_write(content),
            '"' => {}
            c if c.is_ascii_lowercase() => {
                self.named.insert(c, content.clone());
            }
            c if c.is_ascii_uppercase() => {
                let lower = c.to_ascii_lowercase();
                match self.named.get_mut(&lower) {
                    Some(existing) => existing.append(content),
                    None => {
                        self.named.insert(lower, content.clone());
                    }
                }
            }
            c if c.is_ascii_digit() => {
                let i = c as usize - '0' as usize;
                if i == 0 {
                    self.yank = Some(content.clone());
                } else {
                    while self.numbered.len() < i {
                        self.numbered
                            .push_back(RegisterContent::charwise(Fragment::default()));
                    }
                    self.numbered[i - 1] = content.clone();
                }
            }
            '-' => self.small = Some(content.clone()),
            _ => {}
        }
        true
    }

    fn set_unnamed(
        &mut self,
        name: Option<char>,
        content: &RegisterContent,
        host: &mut dyn VimHost,
    ) {
        // The unnamed register shows the last written register (for an
        // upper-case append: the whole appended register).
        let shown = match name {
            Some(c) if c.is_ascii_uppercase() => self
                .named
                .get(&c.to_ascii_lowercase())
                .cloned()
                .unwrap_or_else(|| content.clone()),
            _ => content.clone(),
        };
        if self.unnamed_clipboard && name.is_none() {
            host.clipboard_write(&shown);
        }
        self.unnamed = Some(shown);
    }

    /// A yank into `name` (or the unnamed register).
    pub fn yank(&mut self, name: Option<char>, content: RegisterContent, host: &mut dyn VimHost) {
        match name {
            Some(c) if c != '"' => {
                if !self.store_named(c, &content, host) {
                    return;
                }
            }
            _ => self.yank = Some(content.clone()),
        }
        self.set_unnamed(name.filter(|c| *c != '"'), &content, host);
    }

    /// A delete or change into `name` (or the unnamed register). `small`
    /// means the text lies within one line; `numbered` forces register 1
    /// even then (deletes with `%`, `(`, `)`, `` ` ``, `/`, `?`, `n`, `N`,
    /// `{`, `}`).
    pub fn delete(
        &mut self,
        name: Option<char>,
        content: RegisterContent,
        small: bool,
        numbered: bool,
        host: &mut dyn VimHost,
    ) {
        match name {
            Some(c) if c != '"' => {
                if !self.store_named(c, &content, host) {
                    return;
                }
            }
            _ => {
                if !small || numbered {
                    self.numbered.push_front(content.clone());
                    self.numbered.truncate(9);
                }
                if small {
                    self.small = Some(content.clone());
                }
            }
        }
        self.set_unnamed(name.filter(|c| *c != '"'), &content, host);
    }

    /// Reads a register that holds document content.
    pub fn get(&self, name: Option<char>, host: &mut dyn VimHost) -> Option<RegisterContent> {
        match name {
            None | Some('"') => {
                if self.unnamed_clipboard
                    && let Some(c) = host.clipboard_read()
                {
                    return Some(c);
                }
                self.unnamed.clone()
            }
            Some('+' | '*') => host.clipboard_read(),
            Some(c) => self.peek(c).cloned(),
        }
    }

    /// Reads a register without touching the clipboard.
    pub fn peek(&self, name: char) -> Option<&RegisterContent> {
        match name {
            '"' => self.unnamed.as_ref(),
            '0' => self.yank.as_ref(),
            '-' => self.small.as_ref(),
            c if c.is_ascii_digit() => self.numbered.get(c as usize - '1' as usize),
            c if c.is_ascii_alphabetic() => self.named.get(&c.to_ascii_lowercase()),
            _ => None,
        }
    }

    /// Sets a register directly (host API, tests).
    pub fn set(&mut self, name: char, content: RegisterContent, host: &mut dyn VimHost) {
        if name == '"' {
            self.unnamed = Some(content);
        } else {
            self.store_named(name, &content, host);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use writer_core::{BlockKind, InlineAttrs};

    #[test]
    fn plain_text_round_trip() {
        let r = RegisterContent::from_plain_text("one\ntwo\n");
        assert!(r.linewise);
        assert_eq!(r.fragment.blocks().len(), 2);
        assert_eq!(r.to_plain_text(), "one\ntwo\n");
        let r = RegisterContent::from_plain_text("word");
        assert!(!r.linewise);
        assert_eq!(r.to_plain_text(), "word");
        let r = RegisterContent::from_plain_text("\n");
        assert!(r.linewise);
        assert_eq!(r.fragment.blocks().len(), 1);
        assert!(RegisterContent::from_plain_text("").is_empty());
    }

    #[test]
    fn append_joins_or_stacks() {
        let bold = InlineAttrs {
            bold: true,
            ..Default::default()
        };
        let mut a = RegisterContent::charwise(Fragment::new(vec![Block::paragraph("ab")]));
        let b = RegisterContent::charwise(Fragment::new(vec![
            Block::paragraph("").with_span("cd", bold),
            Block::paragraph("ef"),
        ]));
        a.append(&b);
        assert!(!a.linewise);
        assert_eq!(a.text(), "abcd\nef");
        assert!(a.fragment.blocks()[0].attrs_at(2).unwrap().bold);
        assert!(!a.fragment.blocks()[0].attrs_at(0).unwrap().bold);
        let line = RegisterContent::linewise(Fragment::new(vec![Block::with_text(
            BlockKind::heading(2),
            "Title",
        )]));
        a.append(&line);
        assert!(a.linewise);
        assert_eq!(a.text(), "abcd\nef\nTitle");
        assert_eq!(a.fragment.blocks()[2].kind(), &BlockKind::heading(2));
    }
}
