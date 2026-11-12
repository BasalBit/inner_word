//! Markdown-style input rules and smart punctuation.
//!
//! Rules run after a single typed character has been inserted, each as its
//! own undo step, so one undo gives back the literally typed characters.
//! They never run in code blocks or on text with the inline code style.
//!
//! Block rules (in a paragraph, when the typed character completes a prefix
//! at the very start of the block):
//! `#`–`######` + space (heading), `-`/`*`/`+` + space (bullet), digits +
//! `.`/`)` + space (numbered), `[]`/`[ ]` + space (to-do), `[x]` + space
//! (checked to-do), `>` + space (quote), three backticks (code block), three
//! hyphens in an otherwise empty paragraph (divider). The to-do prefixes
//! also work at the start of a bullet item, so `- [ ] ` typed as in Markdown
//! ends up as a to-do.
//!
//! Inline rules (when a closing delimiter is typed): `**bold**`, `*italic*`,
//! `***bold italic***`,
//! `_italic_`, `` `code` ``, `~~strike~~`, `==highlight==`. The content must
//! not start or end with whitespace, the opening delimiter must not follow a
//! letter, digit or the same delimiter character, and for `*` and `_` the
//! closing delimiter must not be followed by a letter or digit. So
//! `2 * 3 * 4`, `snake_case_name`, `__init__` and a lone `word*` stay as typed.
//!
//! Smart punctuation: straight quotes become curly (opening after
//! whitespace, an opening bracket, a dash or the other kind of opening
//! quote, or at the start, looking through Markdown delimiters as in
//! `**"bold"**`; otherwise closing / apostrophe, so `""` is an empty pair),
//! `--` becomes an em dash (at the very start of a
//! block only once the next character is typed, so `---` can still make a
//! divider), `...` becomes an ellipsis.

use crate::attrs::{Highlight, InlineAttrs, InlineStyle};
use crate::block::BlockKind;
use crate::history::EditKind;
use crate::position::Position;
use crate::runs;
use crate::text::{self, char_after, char_before};

use super::{EditorState, inherited_attrs};

/// The opening delimiter `d` (made of `ch`) matching a closing one at
/// `close`, searched backwards on the line starting at `ls`.
///
/// A candidate is an opener when the content after it does not start with
/// whitespace or another `ch`, and it does not follow a letter, digit or
/// `ch`. Candidates that are not openers are skipped when they cannot close
/// either (they follow whitespace, as the middle `*` of `*a * b*`); the
/// search stops at one that could close an earlier span (it follows other
/// text, as the second `*` of `*a* b*`), so a span never jumps over it.
fn find_opener(t: &str, ls: usize, close: usize, d: &str, ch: char) -> Option<usize> {
    let mut end = close;
    while let Some(rel) = t[ls..end].rfind(d) {
        let open = ls + rel;
        let open_end = open + d.len();
        end = open;
        if open_end >= close {
            continue;
        }
        let content = &t[open_end..close];
        let before = char_before(t, open);
        let valid = !content.starts_with(char::is_whitespace)
            && !content.starts_with(ch)
            && !before.is_some_and(|c| c == ch || c.is_alphanumeric());
        if valid {
            return Some(open);
        }
        let could_close = before.is_some_and(|c| c != ch && !c.is_whitespace());
        if could_close {
            return None;
        }
    }
    None
}

#[derive(Clone, Copy)]
enum Effect {
    Style(InlineStyle),
    BoldItalic,
    Highlight,
}

impl Effect {
    fn set(self, a: &mut InlineAttrs, on: bool) {
        match self {
            Effect::Style(s) => a.set_style(s, on),
            Effect::BoldItalic => {
                a.bold = on;
                a.italic = on;
            }
            Effect::Highlight => a.highlight = on.then_some(Highlight::Yellow),
        }
    }

    fn apply(self, a: &mut InlineAttrs) {
        self.set(a, true);
    }

    fn unapply(self, a: &mut InlineAttrs) {
        self.set(a, false);
    }
}

impl EditorState {
    /// Runs the enabled rules for the just-typed character `ch`.
    pub(crate) fn apply_input_rules(&mut self, ch: char) {
        let opts = self.options;
        if (!opts.input_rules && !opts.smart_punctuation) || !self.selection.is_collapsed() {
            return;
        }
        let pos = self.selection.head;
        let b = &self.doc.blocks[pos.block];
        if b.kind.is_code() || b.is_void() {
            return;
        }
        let o = pos.offset;
        if !b.text[..o].ends_with(ch) {
            return;
        }
        let ch_start = o - ch.len_utf8();
        if b.attrs_at(ch_start).is_some_and(|a| a.code) {
            return;
        }
        if opts.input_rules {
            if self.try_block_rule(pos, ch) {
                return;
            }
            if self.try_inline_rule(pos, ch) {
                return;
            }
        }
        if opts.smart_punctuation {
            self.try_smart_punctuation(pos, ch);
        }
    }

    fn try_block_rule(&mut self, pos: Position, ch: char) -> bool {
        let b = &self.doc.blocks[pos.block];
        let prefix = &b.text[..pos.offset];
        if b.kind == BlockKind::BulletItem && ch == ' ' {
            // "- [ ] " as Markdown writes a task: the bullet becomes a to-do.
            let checked = match prefix {
                "[] " | "[ ] " => false,
                "[x] " | "[X] " => true,
                _ => return false,
            };
            let i = pos.block;
            let cut = pos.offset;
            return self.edit(EditKind::Other, |ed| {
                ed.update_block(i, |b| {
                    b.remove(0..cut);
                    b.set_kind(BlockKind::TodoItem { checked });
                });
                ed.caret_to(Position::new(i, 0));
            });
        }
        if b.kind != BlockKind::Paragraph {
            return false;
        }
        let kind = match ch {
            ' ' => {
                let p = &prefix[..prefix.len() - 1];
                let digits = p.len() >= 2
                    && p.len() <= 10
                    && (p.ends_with('.') || p.ends_with(')'))
                    && p[..p.len() - 1].bytes().all(|c| c.is_ascii_digit());
                if !p.is_empty() && p.len() <= 6 && p.bytes().all(|c| c == b'#') {
                    BlockKind::heading(p.len() as u8)
                } else if p == "-" || p == "*" || p == "+" {
                    BlockKind::BulletItem
                } else if digits {
                    BlockKind::NumberedItem
                } else if p == "[]" || p == "[ ]" {
                    BlockKind::todo()
                } else if p == "[x]" || p == "[X]" {
                    BlockKind::TodoItem { checked: true }
                } else if p == ">" {
                    BlockKind::Quote
                } else {
                    return false;
                }
            }
            '`' if prefix == "```" => BlockKind::code(),
            '-' if prefix == "---" && pos.offset == b.len() => {
                let i = pos.block;
                return self.edit(EditKind::Other, |ed| {
                    ed.update_block(i, |b| b.set_kind(BlockKind::Divider));
                    let p = ed.paragraph_after(i);
                    ed.caret_to(p);
                });
            }
            _ => return false,
        };
        let i = pos.block;
        let cut = pos.offset;
        self.edit(EditKind::Other, |ed| {
            ed.update_block(i, |b| {
                b.remove(0..cut);
                b.set_kind(kind);
            });
            ed.caret_to(Position::new(i, 0));
        })
    }

    fn try_inline_rule(&mut self, pos: Position, ch: char) -> bool {
        let delims: &[(&str, Effect)] = match ch {
            '*' => &[
                ("***", Effect::BoldItalic),
                ("**", Effect::Style(InlineStyle::Bold)),
                ("*", Effect::Style(InlineStyle::Italic)),
            ],
            '_' => &[("_", Effect::Style(InlineStyle::Italic))],
            '`' => &[("`", Effect::Style(InlineStyle::Code))],
            '~' => &[("~~", Effect::Style(InlineStyle::Strikethrough))],
            '=' => &[("==", Effect::Highlight)],
            _ => return false,
        };
        let b = &self.doc.blocks[pos.block];
        let t = &b.text;
        let o = pos.offset;
        let ls = text::line_start(t, o);
        for &(d, effect) in delims {
            if !t[ls..o].ends_with(d) {
                continue;
            }
            let close = o - d.len();
            let before_close = char_before(t, close);
            if before_close.is_none_or(|c| c == ch || c.is_whitespace()) {
                continue;
            }
            if (ch == '*' || ch == '_') && char_after(t, o).is_some_and(char::is_alphanumeric) {
                continue;
            }
            let Some(open) = find_opener(t, ls, close, d, ch) else {
                continue;
            };
            let open_end = open + d.len();
            if runs::spans_in(&b.runs, open..o).any(|(_, a)| a.code) {
                continue;
            }
            let i = pos.block;
            let content_end = close - d.len();
            return self.edit(EditKind::Other, |ed| {
                ed.update_block(i, |b| {
                    b.remove(close..o);
                    b.remove(open..open_end);
                    b.apply_attrs(open..content_end, |a| effect.apply(a));
                });
                let caret = Position::new(i, content_end);
                ed.caret_to(caret);
                let mut pending = inherited_attrs(&ed.doc.blocks[i], content_end);
                effect.unapply(&mut pending);
                ed.set_pending(pending);
            });
        }
        false
    }

    fn try_smart_punctuation(&mut self, pos: Position, ch: char) -> bool {
        let b = &self.doc.blocks[pos.block];
        let t = &b.text;
        let o = pos.offset;
        let start = o - ch.len_utf8();
        // (range, replacement) pairs, later ranges first.
        let mut reps: Vec<(std::ops::Range<usize>, &str)> = Vec::new();
        match ch {
            '"' | '\'' => {
                // Opening after nothing, whitespace, an opening bracket, a
                // dash or the *other* kind of opening quote (nested quotes);
                // Markdown delimiters in between are looked through, so the
                // quote in `**"bold"**` opens. Right after an opening quote
                // of its own kind it closes, so typing `""` gives an empty
                // pair of curly quotes.
                let own_open = if ch == '"' { '\u{201C}' } else { '\u{2018}' };
                let lead = t[..start].trim_end_matches(['*', '_', '~', '=', '`']);
                let opening = char_before(t, start) != Some(own_open)
                    && lead.chars().next_back().is_none_or(|c| {
                        c.is_whitespace()
                            || "([{<\u{201C}\u{2018}\u{AB}\u{2014}\u{2013}-/".contains(c)
                    });
                let rep = match (ch, opening) {
                    ('"', true) => "\u{201C}",
                    ('"', false) => "\u{201D}",
                    ('\'', true) => "\u{2018}",
                    _ => "\u{2019}",
                };
                reps.push((start..o, rep));
            }
            '-' => {
                if t[..o].ends_with("--") {
                    let s = o - 2;
                    if s > 0 && char_before(t, s) != Some('-') {
                        reps.push((s..o, "\u{2014}"));
                    }
                }
            }
            '.' if t[..o].ends_with("...") && char_before(t, o - 3) != Some('.') => {
                reps.push((o - 3..o, "\u{2026}"));
            }
            _ => {}
        }
        // Deferred em dash at the start of a block: "--" then any other char.
        if ch != '-' && &t[..start] == "--" {
            reps.push((0..2, "\u{2014}"));
        }
        if reps.is_empty() {
            return false;
        }
        let i = pos.block;
        let mut caret = o;
        for (r, rep) in &reps {
            caret = caret + rep.len() - r.len();
        }
        self.edit(EditKind::Other, |ed| {
            ed.update_block(i, |b| {
                for (r, rep) in &reps {
                    let attrs = b.attrs_at(r.start).cloned().unwrap_or_default();
                    b.remove(r.clone());
                    b.insert(r.start, rep, &attrs);
                }
            });
            ed.caret_to(Position::new(i, caret));
        })
    }
}
