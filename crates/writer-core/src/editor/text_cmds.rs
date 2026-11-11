//! Text insertion and deletion, Enter and Backspace semantics.

use crate::attrs::InlineAttrs;
use crate::block::{Block, BlockKind};
use crate::history::EditKind;
use crate::position::{Position, Selection, TextRange};
use crate::text::{self, char_class};

use super::{EditorState, inherited_attrs};

/// Which way a removed void block hands over the caret.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    Backward,
    Forward,
}

/// The kind of the block created after `kind` when a block is split (Enter):
/// list items continue (a to-do starts unchecked), a heading split at its end
/// continues as a paragraph, everything else keeps its kind.
pub(crate) fn continuation_kind(kind: &BlockKind, at_end: bool) -> BlockKind {
    match kind {
        BlockKind::Heading { .. } if at_end => BlockKind::Paragraph,
        BlockKind::TodoItem { .. } => BlockKind::TodoItem { checked: false },
        BlockKind::Divider | BlockKind::PageBreak => BlockKind::Paragraph,
        k => k.clone(),
    }
}

impl EditorState {
    // ----- insertion ------------------------------------------------------------------

    /// Types `text` at the selection (replacing it), as keyboard or IME input.
    ///
    /// Control characters are normalised (`\r\n` and `\r` become `\n`). The
    /// new text takes the pending attributes, else the attributes of the
    /// preceding character; a link or comment anchor is only continued when
    /// typing strictly inside it. A `\n` splits the block like Enter (inside
    /// a code block it is a new line). Typing on a void block inserts a new
    /// paragraph after it. Consecutive typing coalesces into one undo step
    /// (broken at word starts). When a single character is typed, input
    /// rules and smart punctuation run afterwards as their own undo step.
    pub fn insert_text(&mut self, text: &str) -> bool {
        let text = text::normalize_input(text).into_owned();
        if text.is_empty() {
            return false;
        }
        let mut chars = text.chars();
        let single = match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        };
        let first = text.chars().next().map(char_class);
        let last = text.chars().next_back().map(char_class);
        let changed = self.edit(EditKind::Typing, |ed| {
            ed.set_tx_classes(first, last);
            ed.insert_text_impl(&text, true);
        });
        if changed && let Some(c) = single {
            self.apply_input_rules(c);
        }
        changed
    }

    /// Pastes plain `text` at the selection: like [`EditorState::insert_text`]
    /// but as its own undo step and without input rules.
    pub fn paste_plain_text(&mut self, text: &str) -> bool {
        let text = text::normalize_input(text).into_owned();
        self.edit(EditKind::Other, |ed| {
            if text.is_empty() {
                ed.delete_selection_impl();
            } else {
                ed.insert_text_impl(&text, true);
            }
        })
    }

    /// Inserts text at the selection; `\n` splits blocks when `split` and the
    /// block is not a code block.
    pub(crate) fn insert_text_impl(&mut self, text: &str, split: bool) {
        if !self.selection.is_collapsed() {
            self.delete_selection_impl();
        }
        let mut pos = self.leave_void(self.selection.head);
        let attrs = self
            .pending
            .take()
            .unwrap_or_else(|| inherited_attrs(&self.doc.blocks[pos.block], pos.offset));
        let is_code = self.doc.blocks[pos.block].kind.is_code();
        if is_code || !split || !text.contains('\n') {
            pos = self.insert_in_block(pos, text, &attrs);
        } else {
            for (i, line) in text.split('\n').enumerate() {
                if i > 0 {
                    pos = self.split_block(pos);
                }
                if !line.is_empty() {
                    pos = self.insert_in_block(pos, line, &attrs);
                }
            }
        }
        let b = &self.doc.blocks[pos.block];
        let o = text::ceil_grapheme_boundary(&b.text, pos.offset);
        self.caret_to(Position::new(pos.block, o));
    }

    /// If `pos` is on a void block, inserts an empty paragraph after it and
    /// returns its start; otherwise returns `pos`.
    pub(crate) fn leave_void(&mut self, pos: Position) -> Position {
        if self.doc.blocks[pos.block].is_void() {
            self.splice(pos.block + 1, 0, vec![Block::new(BlockKind::Paragraph)]);
            Position::new(pos.block + 1, 0)
        } else {
            pos
        }
    }

    /// Inserts already-normalised single-block text; returns the end position.
    pub(crate) fn insert_in_block(&mut self, pos: Position, text: &str, attrs: &InlineAttrs) -> Position {
        if text.is_empty() {
            return pos;
        }
        self.update_block(pos.block, |b| b.insert(pos.offset, text, attrs));
        Position::new(pos.block, pos.offset + text.len())
    }

    /// Inserts a soft line break (`\n` inside the block; shift-enter). On a
    /// void block this behaves like Enter.
    pub fn insert_soft_break(&mut self) -> bool {
        if self.selection.is_collapsed() && self.doc.blocks[self.selection.head.block].is_void() {
            return self.insert_paragraph_break();
        }
        self.edit(EditKind::Typing, |ed| {
            ed.set_tx_classes(Some(text::CharClass::Space), Some(text::CharClass::Space));
            ed.insert_text_impl("\n", false);
        })
    }

    /// Enter. Deletes the selection, then:
    /// - on a void block: inserts a paragraph after it;
    /// - in a code block: inserts a new line keeping the current line's
    ///   leading whitespace; on an empty last line that follows an empty
    ///   line it removes those two lines and exits into a new paragraph;
    /// - on an empty list item: outdents it one level, or turns it into a
    ///   paragraph at indent 0; on an empty quote: turns it into a paragraph;
    /// - at the start of a non-empty block: inserts an empty block before it
    ///   (a paragraph before a heading, an item of the same kind before an
    ///   item) and keeps the caret on the text;
    /// - otherwise splits the block. The second half keeps the kind (a list
    ///   item continues, a to-do starts unchecked), except that splitting a
    ///   heading at its end starts a paragraph.
    ///
    /// The inline attributes active at the split carry over as pending
    /// attributes (without link and comment anchors).
    pub fn insert_paragraph_break(&mut self) -> bool {
        self.edit(EditKind::Other, |ed| ed.paragraph_break_impl())
    }

    fn paragraph_break_impl(&mut self) {
        if !self.selection.is_collapsed() {
            self.delete_selection_impl();
        }
        let pos = self.selection.head;
        let b = &self.doc.blocks[pos.block];
        let mut carry = self
            .pending
            .clone()
            .unwrap_or_else(|| inherited_attrs(b, pos.offset));
        carry.link = None;
        carry.comments.clear();
        if b.is_void() {
            let p = self.leave_void(pos);
            self.caret_to(p);
            return;
        }
        if b.kind.is_code() {
            self.code_enter(pos);
            return;
        }
        if b.is_empty() {
            if b.kind.is_list_item() {
                if b.indent > 0 {
                    let n = b.indent - 1;
                    self.update_block(pos.block, |b| b.set_indent(n));
                } else {
                    self.update_block(pos.block, |b| b.set_kind(BlockKind::Paragraph));
                }
                return;
            }
            if b.kind == BlockKind::Quote {
                self.update_block(pos.block, |b| b.set_kind(BlockKind::Paragraph));
                return;
            }
        }
        let p = self.split_block(pos);
        self.caret_to(p);
        self.set_pending(carry);
    }

    fn code_enter(&mut self, pos: Position) {
        let b = &self.doc.blocks[pos.block];
        let t = &b.text;
        let o = pos.offset;
        let exit = o == t.len() && (t.ends_with("\n\n") || t == "\n");
        if exit {
            let k = if t == "\n" { 1 } else { 2 };
            let len = t.len();
            self.update_block(pos.block, |b| b.remove(len - k..len));
            self.splice(pos.block + 1, 0, vec![Block::new(BlockKind::Paragraph)]);
            self.caret_to(Position::new(pos.block + 1, 0));
            return;
        }
        let ls = text::line_start(t, o);
        let indent: String = t[ls..o].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let ins = format!("\n{indent}");
        let attrs = inherited_attrs(b, o);
        let p = self.insert_in_block(pos, &ins, &attrs);
        self.caret_to(p);
    }

    /// Leaves the current block: inserts an empty paragraph after the block
    /// at the selection end and moves the caret there (for code blocks,
    /// quotes and lists).
    pub fn exit_block(&mut self) -> bool {
        self.edit(EditKind::Other, |ed| {
            let i = ed.selection.end().block;
            ed.splice(i + 1, 0, vec![Block::new(BlockKind::Paragraph)]);
            ed.caret_to(Position::new(i + 1, 0));
        })
    }

    /// Splits the block at `pos` (see [`EditorState::insert_paragraph_break`]
    /// for the kind rules) and returns the start of the second block.
    pub(crate) fn split_block(&mut self, pos: Position) -> Position {
        let b = &self.doc.blocks[pos.block];
        if b.is_void() {
            return self.leave_void(pos);
        }
        if pos.offset == 0 && !b.is_empty() {
            let kind = match &b.kind {
                BlockKind::Heading { .. } => BlockKind::Paragraph,
                k => continuation_kind(k, false),
            };
            let mut nb = Block::new(kind);
            nb.alignment = b.alignment;
            nb.indent = b.indent;
            if nb.kind.same_type(&b.kind) {
                nb.text_size = b.text_size;
            }
            self.splice(pos.block, 0, vec![nb]);
            return Position::new(pos.block + 1, 0);
        }
        let at_end = pos.offset >= b.len();
        let kind = continuation_kind(&b.kind, at_end);
        if at_end {
            let mut nb = Block::new(kind);
            nb.alignment = b.alignment;
            nb.indent = b.indent;
            if nb.kind.same_type(&b.kind) {
                nb.text_size = b.text_size;
            }
            self.splice(pos.block + 1, 0, vec![nb]);
        } else {
            let mut first = b.clone();
            let mut second = first.split_off(pos.offset);
            if second.kind != kind {
                if !second.kind.same_type(&kind) {
                    second.text_size = None;
                }
                second.set_kind(kind);
            }
            self.splice(pos.block, 1, vec![first, second]);
        }
        Position::new(pos.block + 1, 0)
    }

    // ----- deletion -------------------------------------------------------------------

    /// Deletes the selection. Returns false when it is collapsed.
    pub fn delete_selection(&mut self) -> bool {
        self.edit(EditKind::Other, |ed| ed.delete_selection_impl())
    }

    /// Deletes `range` (clamped) and puts the caret at its start.
    pub fn delete_range(&mut self, range: TextRange) -> bool {
        let s = self.doc.clamp_position(range.start);
        let e = self.doc.clamp_position(range.end);
        self.edit(EditKind::Other, |ed| {
            ed.set_sel(Selection::new(s, e));
            ed.delete_selection_impl();
        })
    }

    pub(crate) fn delete_selection_impl(&mut self) {
        let r = self.selection.range();
        if r.is_empty() {
            return;
        }
        let p = self.delete_range_impl(r.start, r.end);
        self.caret_to(p);
    }

    /// Deletes `s..e` (valid, ordered) and returns where the caret goes.
    ///
    /// Across blocks: the head of the first block and the tail of the last
    /// block merge into the first block (keeping its id and kind). A void
    /// block at the start of the range is removed; a void block at the end
    /// is not part of the range and stays (an emptied first block before it
    /// disappears).
    pub(crate) fn delete_range_impl(&mut self, s: Position, e: Position) -> Position {
        if s.block == e.block {
            if s.offset < e.offset {
                self.update_block(s.block, |b| b.remove(s.offset..e.offset));
            }
            return s;
        }
        let first_void = self.doc.blocks[s.block].is_void();
        let last_void = self.doc.blocks[e.block].is_void();
        if first_void {
            if !last_void && e.offset > 0 {
                self.update_block(e.block, |b| b.remove(0..e.offset));
            }
            self.splice(s.block, e.block - s.block, Vec::new());
            return Position::new(s.block, 0);
        }
        if last_void {
            if s.offset == 0 {
                self.splice(s.block, e.block - s.block, Vec::new());
                return Position::new(s.block, 0);
            }
            let len = self.doc.blocks[s.block].len();
            self.update_block(s.block, |b| b.remove(s.offset..len));
            self.splice(s.block + 1, e.block - s.block - 1, Vec::new());
            return s;
        }
        let mut merged = self.doc.blocks[s.block].clone();
        let len = merged.len();
        merged.remove(s.offset..len);
        let last = &self.doc.blocks[e.block];
        let tail = last.slice(e.offset..last.len());
        merged.append(&tail);
        merged.mark_dirty();
        self.splice(s.block, e.block - s.block + 1, vec![merged]);
        s
    }

    /// Removes the void block at `index`; the caret goes to the end of the
    /// previous block (`Backward`) or the start of the next (`Forward`).
    fn remove_void(&mut self, index: usize, dir: Dir) {
        if self.doc.blocks.len() == 1 {
            self.splice(0, 1, vec![Block::new(BlockKind::Paragraph)]);
            self.caret_to(Position::ZERO);
            return;
        }
        self.splice(index, 1, Vec::new());
        let n = self.doc.blocks.len();
        let p = match dir {
            Dir::Backward if index > 0 => Position::new(index - 1, self.doc.blocks[index - 1].len()),
            Dir::Backward => Position::ZERO,
            Dir::Forward if index < n => Position::new(index, 0),
            Dir::Forward => Position::new(n - 1, self.doc.blocks[n - 1].len()),
        };
        self.caret_to(p);
    }

    /// Merges block `i + 1` into block `i` (keeping block `i`'s id and kind).
    fn merge_with_next(&mut self, i: usize) {
        let mut a = self.doc.blocks[i].clone();
        let b = self.doc.blocks[i + 1].clone();
        a.append(&b);
        a.mark_dirty();
        self.splice(i, 2, vec![a]);
    }

    /// Backspace. With a selection, deletes it. Inside text, deletes the
    /// previous grapheme. On a void block, removes the block. At the start of
    /// a block: an indented list item or paragraph outdents; a list item,
    /// heading or quote becomes a paragraph; a code block becomes paragraphs
    /// (one per line); a paragraph merges into the previous block, or removes
    /// it if that is a void block.
    pub fn backspace(&mut self) -> bool {
        if !self.selection.is_collapsed() {
            return self.delete_selection();
        }
        let pos = self.selection.head;
        let b = &self.doc.blocks[pos.block];
        if !b.is_void() && pos.offset > 0 {
            let start = text::prev_grapheme_boundary(&b.text, pos.offset);
            let class = b.text[start..pos.offset].chars().next().map(char_class);
            return self.edit(EditKind::Backspace, |ed| {
                ed.set_tx_classes(class, class);
                ed.update_block(pos.block, |b| b.remove(start..pos.offset));
                ed.caret_to(Position::new(pos.block, start));
            });
        }
        self.edit(EditKind::Other, |ed| ed.backspace_at_block_start(pos.block))
    }

    fn backspace_at_block_start(&mut self, i: usize) {
        let b = &self.doc.blocks[i];
        if b.is_void() {
            self.remove_void(i, Dir::Backward);
            return;
        }
        match &b.kind {
            k if k.is_list_item() && b.indent > 0 => {
                let n = b.indent - 1;
                self.update_block(i, |b| b.set_indent(n));
            }
            k if k.is_list_item() || matches!(k, BlockKind::Heading { .. } | BlockKind::Quote) => {
                self.update_block(i, |b| b.set_kind(BlockKind::Paragraph));
            }
            BlockKind::CodeBlock { .. } => {
                self.convert_blocks(i..i + 1, &BlockKind::Paragraph);
                self.caret_to(Position::new(i, 0));
            }
            _ if b.indent > 0 => {
                let n = b.indent - 1;
                self.update_block(i, |b| b.set_indent(n));
            }
            _ => {
                if i == 0 {
                    return;
                }
                if self.doc.blocks[i - 1].is_void() {
                    self.splice(i - 1, 1, Vec::new());
                    self.caret_to(Position::new(i - 1, 0));
                } else {
                    let len = self.doc.blocks[i - 1].len();
                    self.merge_with_next(i - 1);
                    self.caret_to(Position::new(i - 1, len));
                }
            }
        }
    }

    /// Forward delete. With a selection, deletes it. Inside text, deletes the
    /// next grapheme. On a void block, removes the block. At the end of a
    /// block, merges the next block into this one (or removes it if it is a
    /// void block).
    pub fn delete_forward(&mut self) -> bool {
        if !self.selection.is_collapsed() {
            return self.delete_selection();
        }
        let pos = self.selection.head;
        let b = &self.doc.blocks[pos.block];
        if b.is_void() {
            return self.edit(EditKind::Other, |ed| ed.remove_void(pos.block, Dir::Forward));
        }
        if pos.offset < b.len() {
            let end = text::next_grapheme_boundary(&b.text, pos.offset);
            let class = b.text[pos.offset..end].chars().next().map(char_class);
            return self.edit(EditKind::DeleteForward, |ed| {
                ed.set_tx_classes(class, class);
                ed.update_block(pos.block, |b| b.remove(pos.offset..end));
                ed.caret_to(pos);
            });
        }
        if pos.block + 1 >= self.doc.blocks.len() {
            return false;
        }
        self.edit(EditKind::Other, |ed| {
            if ed.doc.blocks[pos.block + 1].is_void() {
                ed.splice(pos.block + 1, 1, Vec::new());
            } else {
                ed.merge_with_next(pos.block);
            }
            ed.caret_to(pos);
        })
    }

    /// Option-backspace: deletes back to the start of the word before the
    /// caret within the block; at the start of a block behaves like
    /// [`EditorState::backspace`].
    pub fn delete_word_backward(&mut self) -> bool {
        if !self.selection.is_collapsed() {
            return self.delete_selection();
        }
        let pos = self.selection.head;
        let b = &self.doc.blocks[pos.block];
        if b.is_void() || pos.offset == 0 {
            return self.backspace();
        }
        let start = text::word_start_before(&b.text, pos.offset).unwrap_or(0);
        self.delete_range(TextRange::in_block(pos.block, start, pos.offset))
    }

    /// Option-delete: deletes to the end of the word after the caret within
    /// the block; at the end of a block behaves like
    /// [`EditorState::delete_forward`].
    pub fn delete_word_forward(&mut self) -> bool {
        if !self.selection.is_collapsed() {
            return self.delete_selection();
        }
        let pos = self.selection.head;
        let b = &self.doc.blocks[pos.block];
        if b.is_void() || pos.offset >= b.len() {
            return self.delete_forward();
        }
        let end = text::word_end_after(&b.text, pos.offset).unwrap_or(b.len());
        self.delete_range(TextRange::in_block(pos.block, pos.offset, end))
    }

    /// Deletes back to the start of the hard line (cmd-backspace uses the
    /// visual line, which the UI computes and passes to
    /// [`EditorState::delete_range`]). At a line start behaves like backspace.
    pub fn delete_to_line_start(&mut self) -> bool {
        if !self.selection.is_collapsed() {
            return self.delete_selection();
        }
        let pos = self.selection.head;
        let b = &self.doc.blocks[pos.block];
        let start = text::line_start(&b.text, pos.offset);
        if b.is_void() || start == pos.offset {
            return self.backspace();
        }
        self.delete_range(TextRange::in_block(pos.block, start, pos.offset))
    }

    /// Deletes to the end of the hard line (ctrl-k). At a line end behaves
    /// like forward delete.
    pub fn delete_to_line_end(&mut self) -> bool {
        if !self.selection.is_collapsed() {
            return self.delete_selection();
        }
        let pos = self.selection.head;
        let b = &self.doc.blocks[pos.block];
        let end = text::line_end(&b.text, pos.offset);
        if b.is_void() || end == pos.offset {
            return self.delete_forward();
        }
        self.delete_range(TextRange::in_block(pos.block, pos.offset, end))
    }
}
