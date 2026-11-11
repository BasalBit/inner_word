//! Operators and the other commands that change text.

use std::ops::Range;

use writer_core::{Block, BlockKind, Fragment, InlineAttrs, InlineStyle, Position, TextRange};

use crate::buf::{self, Lines};
use crate::register::RegisterContent;
use crate::textobj::Target;

use super::insert::MAX_REPEAT;
use super::marks::PlaceMark;
use super::{Cx, Operator, Vim};

/// How a case operator maps text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaseMode {
    Lower,
    Upper,
    Toggle,
}

/// The upper-case form of one character. Vim maps character to character:
/// `ß` becomes the capital sharp s, and a character whose upper case is
/// several characters stays as it is. That also keeps every character's
/// formatting and comment anchors in place.
fn upper_char(c: char) -> char {
    if c == 'ß' {
        return 'ẞ';
    }
    let mut it = c.to_uppercase();
    match (it.next(), it.next()) {
        (Some(u), None) => u,
        _ => c,
    }
}

fn lower_char(c: char) -> char {
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

fn map_case(s: &str, mode: CaseMode) -> String {
    s.chars()
        .map(|c| match mode {
            CaseMode::Lower => lower_char(c),
            CaseMode::Upper => upper_char(c),
            CaseMode::Toggle => {
                if c.is_lowercase() {
                    upper_char(c)
                } else if c.is_uppercase() {
                    lower_char(c)
                } else {
                    c
                }
            }
        })
        .collect()
}

/// A fragment holding the hard lines `range` of code block `block`.
fn code_fragment(cx: &Cx, block: usize, range: Range<usize>) -> Fragment {
    let b = &cx.ed.document().blocks()[block];
    Fragment::new(vec![
        Block::with_text(b.kind().clone(), &b.text()[range])
            .with_indent(b.indent())
            .with_alignment(b.alignment()),
    ])
}

/// The pieces of `range` that one block selection each can cover exactly.
///
/// A selection that ends at offset 0 of a block does not touch that block,
/// so a block range ending with empty blocks cannot be selected as a whole:
/// the trailing empty blocks come one at a time (a caret in an empty block
/// does select it), last block first, followed by the rest.
fn selectable_pieces(cx: &Cx, range: &Range<usize>) -> Vec<Range<usize>> {
    let blocks = cx.ed.document().blocks();
    let mut end = range.end.min(blocks.len());
    let mut pieces = Vec::new();
    while end > range.start + 1 && blocks[end - 1].is_empty() && !blocks[end - 1].is_void() {
        pieces.push(end - 1..end);
        end -= 1;
    }
    pieces.push(range.start..end);
    pieces
}

/// Deletes exactly the blocks `range`.
pub(crate) fn delete_blocks(cx: &mut Cx, range: &Range<usize>) {
    for piece in selectable_pieces(cx, range) {
        cx.ed.select_blocks(piece);
        cx.ed.delete_blocks();
    }
}

/// What an operator needs to know besides its target.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OpInfo {
    pub register: Option<char>,
    /// Deletes always go to register 1 (motions like `}` and `/`).
    pub big: bool,
    /// Where a yank leaves the caret.
    pub yank_caret: Position,
    /// How often `>` and `<` shift (Visual mode count).
    pub shift_times: usize,
    /// Apply Vim's rule that widens a multi-line `d{motion}` to whole
    /// lines when only blanks surround it.
    pub widen: bool,
}

/// Whether `target` is the only line of a document that holds nothing but
/// one empty paragraph. Vim does not delete (or put in a register) the
/// single empty line of an empty buffer.
fn is_empty_document(cx: &Cx, target: &Target) -> bool {
    let doc = cx.ed.document();
    matches!(target, Target::Lines(Lines::Blocks(_)))
        && doc.block_count() == 1
        && doc.blocks()[0].is_empty()
        && !doc.blocks()[0].is_void()
        && *doc.blocks()[0].kind() == BlockKind::Paragraph
}

impl Vim {
    /// The register content of `target`.
    fn target_content(&self, cx: &Cx, target: &Target) -> RegisterContent {
        let doc = cx.ed.document();
        match target {
            Target::Chars(r) => RegisterContent::charwise(doc.fragment(*r)),
            Target::Lines(Lines::Blocks(r)) => {
                RegisterContent::linewise(doc.block_fragment(r.clone()))
            }
            Target::Lines(Lines::Code { block, start, end }) => {
                RegisterContent::linewise(code_fragment(cx, *block, *start..*end))
            }
        }
    }

    fn set_change_marks(&mut self, cx: &Cx, start: Position, end: Position) {
        self.set_mark(cx, '[', start);
        self.set_mark(cx, ']', end);
        self.set_mark(cx, '.', start);
    }

    /// Applies `op` to `target`. Returns false when nothing could be done.
    pub(crate) fn apply_operator(
        &mut self,
        cx: &mut Cx,
        op: Operator,
        target: Target,
        info: OpInfo,
    ) -> bool {
        match op {
            Operator::Yank => {
                let content = self.target_content(cx, &target);
                let lines = match &target {
                    Target::Lines(l) => l.count(cx.ed.document()),
                    Target::Chars(_) => 0,
                };
                let (s, e) = self.target_ends(cx, &target);
                self.registers.yank(info.register, content, cx.host);
                self.set_mark(cx, '[', s);
                self.set_mark(cx, ']', e);
                if lines > 2 {
                    self.message(format!("{lines} lines yanked"));
                }
                self.set_normal_caret(cx, info.yank_caret);
                true
            }
            Operator::Delete => {
                if is_empty_document(cx, &target) {
                    return true;
                }
                let target = if info.widen {
                    self.widen_delete(cx, target)
                } else {
                    target
                };
                let before = cx.ed.document().block_count();
                let caret = self.delete_target(cx, &target, info);
                let linewise = matches!(target, Target::Lines(_));
                let caret = if linewise {
                    let doc = cx.ed.document();
                    buf::unit_first_non_blank(doc, buf::unit_at(doc, caret))
                } else {
                    caret
                };
                self.set_normal_caret(cx, caret);
                let after = cx.ed.document().block_count();
                if linewise && before > after + 2 {
                    self.message(format!("{} fewer lines", before - after));
                }
                let c = cx.ed.selection().head;
                self.set_change_marks(cx, c, c);
                true
            }
            Operator::Change => {
                self.change_target(cx, &target, info);
                true
            }
            Operator::ShiftRight | Operator::ShiftLeft => {
                let lines = match target {
                    Target::Lines(l) => l,
                    Target::Chars(r) => buf::lines_between(cx.ed.document(), r.start, {
                        // A range ending at the start of a line does not
                        // include that line.
                        let doc = cx.ed.document();
                        if r.end > r.start && buf::at_line_start(doc, r.end) {
                            buf::prev_cell(doc, r.end).unwrap_or(r.end)
                        } else {
                            r.end
                        }
                    }),
                };
                self.shift_lines(
                    cx,
                    &lines,
                    op == Operator::ShiftRight,
                    info.shift_times.max(1),
                );
                true
            }
            Operator::Lower | Operator::Upper | Operator::ToggleCase => {
                let mode = match op {
                    Operator::Lower => CaseMode::Lower,
                    Operator::Upper => CaseMode::Upper,
                    _ => CaseMode::Toggle,
                };
                let (start, last) = self.target_ends(cx, &target);
                self.case_target(cx, &target, mode);
                // A linewise case change leaves the caret where the motion
                // started; a characterwise one at the start of the text.
                let caret = match &target {
                    Target::Chars(_) => start,
                    Target::Lines(_) => info.yank_caret,
                };
                self.set_normal_caret(cx, caret);
                self.set_change_marks(cx, start, last);
                true
            }
        }
    }

    /// The first and the last position of `target`, for the `[` and `]`
    /// marks: the last character of a run of text, the last line of a
    /// linewise range.
    fn target_ends(&self, cx: &Cx, target: &Target) -> (Position, Position) {
        let doc = cx.ed.document();
        match target {
            Target::Chars(r) => {
                let last = buf::prev_cell(doc, r.end)
                    .filter(|p| *p >= r.start)
                    .unwrap_or(r.start);
                (r.start, last)
            }
            Target::Lines(Lines::Blocks(r)) => (
                Position::new(r.start, 0),
                Position::new(r.end - 1, buf::len_of(doc, r.end - 1)),
            ),
            Target::Lines(Lines::Code { block, start, end }) => {
                (Position::new(*block, *start), Position::new(*block, *end))
            }
        }
    }

    /// Deletes `target` into the registers and returns where the caret
    /// ends up (not yet adjusted for Normal mode).
    pub(crate) fn delete_target(&mut self, cx: &mut Cx, target: &Target, info: OpInfo) -> Position {
        let content = self.target_content(cx, target);
        match target {
            Target::Chars(r) => {
                if r.is_empty() {
                    // Nothing to delete: the registers stay as they are.
                    return r.start;
                }
                let doc = cx.ed.document();
                let small = self.visual_big
                    || (r.start.block == r.end.block
                        && !buf::text_of(doc, r.start.block)[r.start.offset..r.end.offset]
                            .contains('\n'));
                self.registers
                    .delete(info.register, content, small, info.big, cx.host);
                cx.ed.set_caret(r.start);
                cx.ed.delete_range(*r);
                cx.ed.selection().head
            }
            Target::Lines(Lines::Blocks(range)) => {
                // (Visual backspace fills `-` as well as register 1.)
                self.registers.delete(
                    info.register,
                    content,
                    self.visual_big,
                    self.visual_big,
                    cx.host,
                );
                delete_blocks(cx, range);
                cx.ed.selection().head
            }
            Target::Lines(Lines::Code { block, start, end }) => {
                self.registers
                    .delete(info.register, content, false, false, cx.host);
                let len = buf::len_of(cx.ed.document(), *block);
                if *start == 0 && *end >= len {
                    // Every line: the code block itself goes.
                    delete_blocks(cx, &(*block..*block + 1));
                    return cx.ed.selection().head;
                }
                let (rs, re) = if *end < len {
                    (*start, *end + 1)
                } else {
                    (start.saturating_sub(1), *end)
                };
                cx.ed.set_caret(Position::new(*block, *start));
                cx.ed.delete_range(TextRange::in_block(*block, rs, re));
                let doc = cx.ed.document();
                let at = Position::new(*block, (*start).min(buf::len_of(doc, *block)));
                Position::new(*block, buf::line_start(doc, at))
            }
        }
    }

    /// `c`: deletes `target` and starts Insert mode in its place. A
    /// linewise change keeps one empty line: the first block stays, with
    /// its kind, so `cc` on a heading or a list item keeps that structure.
    fn change_target(&mut self, cx: &mut Cx, target: &Target, info: OpInfo) {
        cx.ed.begin_undo_group();
        // What is typed instead takes the formatting of the first changed
        // character, as typing over a selection does in the editor.
        let first = match target {
            Target::Chars(r) => r.start,
            Target::Lines(l) => l.start(),
        };
        let carry = {
            let block = &cx.ed.document().blocks()[first.block];
            if block.kind().is_code() {
                None
            } else {
                block.attrs_at(first.offset).cloned()
            }
        };
        match target {
            Target::Chars(r) => {
                if r.is_empty()
                    && std::mem::take(&mut self.op_inclusive)
                    && buf::is_empty_line(cx.ed.document(), r.start)
                {
                    // `C` or `ciw` on an empty line: nothing goes, and the
                    // registers hold that nothing.
                    let content = self.target_content(cx, target);
                    self.registers
                        .delete(info.register, content, true, false, cx.host);
                }
                let p = self.delete_target(cx, target, info);
                cx.ed.set_caret(p);
            }
            Target::Lines(Lines::Blocks(range)) => {
                if !is_empty_document(cx, target) {
                    let content = self.target_content(cx, target);
                    self.registers
                        .delete(info.register, content, false, false, cx.host);
                }
                let first = range.start;
                let first_void = cx.ed.document().blocks()[first].is_void();
                if first_void {
                    let all = range.len() == cx.ed.document().block_count();
                    delete_blocks(cx, range);
                    if !all {
                        let empty = Fragment::new(vec![Block::new(BlockKind::Paragraph)]);
                        cx.ed.insert_blocks_at(first, &empty);
                    }
                    cx.ed.set_caret(Position::new(first, 0));
                } else {
                    if range.len() > 1 {
                        delete_blocks(cx, &(first + 1..range.end));
                    }
                    let whole = cx.ed.document().block_range(first);
                    cx.ed.set_caret(whole.start);
                    if !whole.is_empty() {
                        cx.ed.delete_range(whole);
                    }
                    cx.ed.set_caret(Position::new(first, 0));
                }
            }
            Target::Lines(Lines::Code { block, start, end }) => {
                let content = self.target_content(cx, target);
                self.registers
                    .delete(info.register, content, false, false, cx.host);
                // Keep the indent of the first line, as autoindent would.
                let indent: String = buf::text_of(cx.ed.document(), *block)[*start..*end]
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .collect();
                cx.ed.set_caret(Position::new(*block, *start));
                cx.ed
                    .replace_range(TextRange::in_block(*block, *start, *end), &indent);
                cx.ed
                    .set_caret(Position::new(*block, *start + indent.len()));
            }
        }
        let c = cx.ed.selection().head;
        self.set_change_marks(cx, c, c);
        if let Some(wanted) = carry {
            Self::carry_format(cx, &wanted);
        }
        self.begin_insert(cx, false, 1, None, true);
    }

    /// Makes the next typed text take the character formatting of `wanted`
    /// (styles, highlight, colour, font). Links and comment anchors are
    /// left to the editor's rule: they continue only inside themselves.
    fn carry_format(cx: &mut Cx, wanted: &InlineAttrs) {
        let now = cx.ed.current_attrs();
        for style in InlineStyle::ALL {
            if now.has_style(style) != wanted.has_style(style) {
                cx.ed.toggle_style(style);
            }
        }
        if now.highlight != wanted.highlight {
            cx.ed.set_highlight(wanted.highlight);
        }
        if now.color != wanted.color {
            cx.ed.set_text_color(wanted.color);
        }
        if now.font != wanted.font {
            cx.ed.set_font_family(wanted.font.as_deref());
        }
    }

    /// `>` and `<`: block indent for blocks, leading spaces for the lines
    /// of a code block.
    pub(crate) fn shift_lines(&mut self, cx: &mut Cx, lines: &Lines, right: bool, times: usize) {
        let times = times.clamp(1, 64);
        let caret;
        match lines {
            Lines::Blocks(range) => {
                for piece in selectable_pieces(cx, range) {
                    cx.ed.select_blocks(piece);
                    for _ in 0..times {
                        if right {
                            cx.ed.indent();
                        } else {
                            cx.ed.outdent();
                        }
                    }
                }
                let doc = cx.ed.document();
                caret = buf::unit_first_non_blank(
                    doc,
                    buf::unit_at(doc, Position::new(range.start, 0)),
                );
                let n = range.len();
                if n > 2 {
                    self.message(format!(
                        "{n} lines {}ed {times} time{}",
                        if right { ">" } else { "<" },
                        if times == 1 { "" } else { "s" }
                    ));
                }
            }
            Lines::Code { block, start, end } => {
                let width = self.options.shift_width.max(1) * times;
                let text = buf::text_of(cx.ed.document(), *block).to_string();
                // Line starts, last first, so earlier offsets stay valid.
                let mut starts: Vec<usize> = vec![*start];
                starts.extend(
                    text[*start..*end]
                        .match_indices('\n')
                        .map(|(i, _)| *start + i + 1),
                );
                cx.ed.set_caret(Position::new(*block, *start));
                for ls in starts.into_iter().rev() {
                    let le = text[ls..].find('\n').map_or(text.len(), |i| ls + i);
                    if right {
                        if le > ls {
                            let pad = " ".repeat(width);
                            cx.ed
                                .replace_range(TextRange::in_block(*block, ls, ls), &pad);
                        }
                    } else {
                        let mut remove = 0;
                        let mut cols = 0;
                        for c in text[ls..le].chars() {
                            if cols >= width {
                                break;
                            }
                            match c {
                                ' ' => cols += 1,
                                '\t' => cols += self.options.shift_width.max(1),
                                _ => break,
                            }
                            remove += c.len_utf8();
                        }
                        if remove > 0 {
                            cx.ed
                                .delete_range(TextRange::in_block(*block, ls, ls + remove));
                        }
                    }
                }
                let doc = cx.ed.document();
                let o = buf::first_non_blank(doc, *block, *start);
                caret = Position::new(
                    *block,
                    o.min(buf::line_end(doc, Position::new(*block, *start))),
                );
            }
        }
        self.set_normal_caret(cx, caret);
        let c = cx.ed.selection().head;
        self.set_change_marks(cx, c, c);
    }

    /// Changes the case of the text in `target`, keeping the formatting of
    /// every character.
    fn case_target(&mut self, cx: &mut Cx, target: &Target, mode: CaseMode) {
        let doc = cx.ed.document();
        let mut parts: Vec<(usize, Range<usize>)> = Vec::new();
        match target {
            Target::Chars(r) => {
                for b in r.start.block..=r.end.block {
                    let from = if b == r.start.block {
                        r.start.offset
                    } else {
                        0
                    };
                    let to = if b == r.end.block {
                        r.end.offset
                    } else {
                        buf::len_of(doc, b)
                    };
                    if from < to {
                        parts.push((b, from..to));
                    }
                }
            }
            Target::Lines(Lines::Blocks(range)) => {
                for b in range.clone() {
                    let len = buf::len_of(doc, b);
                    if len > 0 {
                        parts.push((b, 0..len));
                    }
                }
            }
            Target::Lines(Lines::Code { block, start, end }) => {
                if start < end {
                    parts.push((*block, *start..*end));
                }
            }
        }
        for (b, range) in parts {
            self.recase(cx, b, range, mode);
        }
    }

    fn recase(&mut self, cx: &mut Cx, block: usize, range: Range<usize>, mode: CaseMode) {
        let blk = &cx.ed.document().blocks()[block];
        let old = &blk.text()[range.clone()];
        let new = map_case(old, mode);
        if new == old {
            return;
        }
        if old.chars().count() == new.chars().count() {
            // Same length in characters: every character keeps its own
            // formatting.
            cx.ed
                .replace_range(TextRange::in_block(block, range.start, range.end), &new);
            return;
        }
        // The length changes (ß to SS): go run by run so that each piece
        // has a single formatting, last piece first.
        let pieces: Vec<Range<usize>> = blk
            .spans()
            .map(|(r, _)| r.start.max(range.start)..r.end.min(range.end))
            .filter(|r| r.start < r.end)
            .collect();
        for r in pieces.into_iter().rev() {
            let old = cx.ed.document().blocks()[block].text()[r.clone()].to_string();
            let new = map_case(&old, mode);
            if new != old {
                cx.ed
                    .replace_range(TextRange::in_block(block, r.start, r.end), &new);
            }
        }
    }

    /// `~`: toggles the case of `count` characters and moves past them.
    pub(crate) fn tilde(&mut self, cx: &mut Cx, count: usize) -> bool {
        let origin = cx.ed.selection().head;
        let doc = cx.ed.document();
        let le = buf::line_end(doc, origin);
        if origin.offset >= le {
            return false;
        }
        let end = buf::advance_graphemes(buf::text_of(doc, origin.block), origin.offset, count, le);
        cx.ed.set_caret(origin);
        self.recase(cx, origin.block, origin.offset..end, CaseMode::Toggle);
        // Case mapping can change byte lengths: find the end again by
        // counting characters from the start.
        let doc = cx.ed.document();
        let le = buf::line_end(doc, origin);
        let after =
            buf::advance_graphemes(buf::text_of(doc, origin.block), origin.offset, count, le);
        self.set_normal_caret(cx, Position::new(origin.block, after));
        self.set_change_marks(cx, origin, origin);
        true
    }

    /// `r{char}`: replaces `count` characters. `None` for `ch` means a
    /// line break (`r<CR>`).
    pub(crate) fn replace_chars(&mut self, cx: &mut Cx, count: usize, ch: Option<char>) -> bool {
        let origin = cx.ed.selection().head;
        let doc = cx.ed.document();
        let t = buf::text_of(doc, origin.block);
        let le = buf::line_end(doc, origin);
        let available = buf::graphemes_between(t, origin.offset, le);
        if available < count || count == 0 {
            return false;
        }
        let end = buf::advance_graphemes(t, origin.offset, count, le);
        let range = TextRange::in_block(origin.block, origin.offset, end);
        cx.ed.set_caret(origin);
        match ch {
            Some(c) => {
                let rep: String = std::iter::repeat_n(c, count).collect();
                cx.ed.replace_range(range, &rep);
                let last = origin.offset + rep.len() - c.len_utf8();
                self.set_normal_caret(cx, Position::new(origin.block, last));
            }
            None => {
                cx.ed.delete_range(range);
                cx.ed.insert_paragraph_break();
                let p = cx.ed.selection().head;
                self.set_normal_caret(cx, p);
            }
        }
        self.set_change_marks(cx, origin, origin);
        true
    }

    /// Visual `r{char}`: replaces every selected character.
    pub(crate) fn replace_in_target(&mut self, cx: &mut Cx, target: &Target, ch: char) {
        let doc = cx.ed.document();
        let mut parts: Vec<(usize, Range<usize>)> = Vec::new();
        match target {
            Target::Chars(r) => {
                for b in r.start.block..=r.end.block {
                    let from = if b == r.start.block {
                        r.start.offset
                    } else {
                        0
                    };
                    let to = if b == r.end.block {
                        r.end.offset
                    } else {
                        buf::len_of(doc, b)
                    };
                    if from < to {
                        parts.push((b, from..to));
                    }
                }
            }
            Target::Lines(Lines::Blocks(range)) => {
                for b in range.clone() {
                    parts.push((b, 0..buf::len_of(doc, b)));
                }
            }
            Target::Lines(Lines::Code { block, start, end }) => parts.push((*block, *start..*end)),
        }
        for (b, range) in parts {
            let old = cx.ed.document().blocks()[b].text()[range.clone()].to_string();
            // One replacement character per grapheme; line breaks stay.
            let mut new = String::with_capacity(old.len());
            let mut o = 0;
            while o < old.len() {
                let next = buf::next_boundary(&old, o);
                if &old[o..next] == "\n" {
                    new.push('\n');
                } else {
                    new.push(ch);
                }
                o = next;
            }
            if new != old {
                cx.ed
                    .replace_range(TextRange::in_block(b, range.start, range.end), &new);
            }
        }
    }

    /// `J`: joins `count` lines (at least two) starting at block `first`.
    /// Blocks are joined with one space (none before `)`, after white
    /// space, or when either side is empty); the leading white space of
    /// the joined block goes. A void block joins like an empty line: it
    /// disappears. Inside a code block the hard lines are joined instead.
    pub(crate) fn join_lines(
        &mut self,
        cx: &mut Cx,
        origin: Position,
        count: usize,
        spaces: bool,
    ) -> bool {
        /// What one join does.
        enum Step {
            /// Remove a void block; the caret goes to this position.
            DropBlock(usize, Position),
            /// Replace the range with the separator; the caret goes to the
            /// position.
            Replace(TextRange, &'static str, Position),
        }
        let joins = count.max(2) - 1;
        let mut caret = origin;
        let mut done = 0;
        cx.ed.set_caret(origin);
        // The last character of the line joined before (of the first line
        // to begin with), `None` for an empty line: Vim adds no space after
        // a line that ends in white space, but an empty line in between
        // makes it forget that.
        let mut prev_last: Option<char> = {
            let doc = cx.ed.document();
            let ls = buf::line_start(doc, origin);
            let le = buf::line_end(doc, origin);
            buf::text_of(doc, origin.block)[ls..le].chars().next_back()
        };
        for _ in 0..joins {
            let step = {
                let doc = cx.ed.document();
                let b = caret.block;
                let blk = &doc.blocks()[b];
                let t = blk.text();
                let le = buf::line_end(doc, caret);
                // Where the line break is, and what follows it.
                let (brk, next_start, next_block) = if blk.kind().is_code() && le < t.len() {
                    (Position::new(b, le), le + 1, b)
                } else if b + 1 < doc.block_count() {
                    (Position::new(b, t.len()), 0, b + 1)
                } else {
                    break;
                };
                let next = &doc.blocks()[next_block];
                if next_block != b && next.is_void() {
                    prev_last = None;
                    Step::DropBlock(next_block, brk)
                } else if next_block != b && blk.is_void() {
                    prev_last = next.text().chars().next_back();
                    Step::DropBlock(b, Position::new(b, 0))
                } else {
                    let nt = next.text();
                    let next_le = nt[next_start..]
                        .find('\n')
                        .map_or(nt.len(), |i| next_start + i);
                    let ws: usize = nt[next_start..next_le]
                        .chars()
                        .take_while(|c| c.is_whitespace())
                        .map(char::len_utf8)
                        .sum();
                    let rest_empty = next_start + ws >= next_le;
                    let ls = buf::line_start(doc, brk);
                    let head_empty = brk.offset == ls;
                    let ends_white = prev_last.is_some_and(char::is_whitespace);
                    let before_paren = nt[next_start + ws..].starts_with(')');
                    let sep = if !spaces || head_empty || rest_empty || ends_white || before_paren {
                        ""
                    } else {
                        " "
                    };
                    prev_last = if spaces {
                        nt[next_start + ws..next_le].chars().next_back()
                    } else {
                        nt[next_start..next_le].chars().next_back()
                    };
                    let end = if spaces {
                        Position::new(next_block, next_start + ws)
                    } else {
                        Position::new(next_block, next_start)
                    };
                    Step::Replace(TextRange::new(brk, end), sep, brk)
                }
            };
            match step {
                Step::DropBlock(index, at) => {
                    cx.ed.select_blocks(index..index + 1);
                    cx.ed.delete_blocks();
                    caret = at;
                }
                Step::Replace(range, sep, at) => {
                    cx.ed.replace_range(range, sep);
                    caret = at;
                }
            }
            done += 1;
            caret = cx.ed.document().clamp_position(caret);
        }
        if done == 0 {
            if count > 2 {
                // Vim's `3J` on the last line: nothing to join, no error,
                // and the cursor goes to the start of the line.
                let ls = buf::line_start(cx.ed.document(), origin);
                self.set_normal_caret(cx, Position::new(origin.block, ls));
                return true;
            }
            return false;
        }
        self.set_normal_caret(cx, caret);
        let c = cx.ed.selection().head;
        self.set_change_marks(cx, c, c);
        true
    }

    // ----- put ---------------------------------------------------------------------------

    /// Reads register `name` for a put; reports an empty register.
    pub(crate) fn read_register(
        &mut self,
        cx: &mut Cx,
        name: Option<char>,
    ) -> Option<RegisterContent> {
        let content = match name {
            Some('.') => Some(RegisterContent::charwise(Fragment::from_plain_text(
                &self.last_inserted,
            ))),
            Some(':') => Some(RegisterContent::charwise(Fragment::from_plain_text(
                &self.last_ex,
            ))),
            Some('/') => Some(RegisterContent::charwise(Fragment::from_plain_text(
                &self.search.raw,
            ))),
            Some('_') => None,
            other => self.registers.get(other, cx.host),
        };
        match content {
            Some(c) if !c.is_empty() => Some(c),
            _ => {
                let shown = name.unwrap_or('"');
                self.error(format!("E353: Nothing in register {shown}"));
                None
            }
        }
    }

    /// Register `name` for a Visual-mode put, where an empty register is
    /// not an error.
    pub(crate) fn registers_content(
        &mut self,
        cx: &mut Cx,
        name: Option<char>,
    ) -> Option<RegisterContent> {
        let before = self.effects.len();
        let content = self.read_register(cx, name);
        if content.is_none() {
            self.effects.truncate(before);
        }
        content
    }

    /// Whether `content` should be put as lines inside the code block the
    /// caret is in.
    fn puts_code_lines(cx: &Cx, content: &RegisterContent, at: Position) -> bool {
        let blocks = content.fragment.blocks();
        content.linewise
            && blocks.len() == 1
            && blocks[0].kind().is_code()
            && cx.ed.document().blocks()[at.block].kind().is_code()
    }

    /// `p`, `P`, `gp`, `gP` in Normal mode.
    pub(crate) fn put(
        &mut self,
        cx: &mut Cx,
        content: &RegisterContent,
        before: bool,
        count: usize,
        caret_after: bool,
    ) {
        let origin = cx.ed.selection().head;
        let count = count.clamp(1, MAX_REPEAT);
        cx.ed.set_caret(origin);
        if content.linewise {
            if Self::puts_code_lines(cx, content, origin) {
                let doc = cx.ed.document();
                let text = content.text();
                let (at, ins, first_line) = if before {
                    let ls = buf::line_start(doc, origin);
                    let ins = format!("{text}\n").repeat(count);
                    (ls, ins, ls)
                } else {
                    let le = buf::line_end(doc, origin);
                    let ins = format!("\n{text}").repeat(count);
                    (le, ins, le + 1)
                };
                let len = ins.len();
                cx.ed
                    .replace_range(TextRange::in_block(origin.block, at, at), &ins);
                let doc = cx.ed.document();
                let caret = if caret_after {
                    let after = (first_line + len).min(buf::len_of(doc, origin.block));
                    Position::new(origin.block, after)
                } else {
                    let le = buf::line_end(doc, Position::new(origin.block, first_line));
                    Position::new(
                        origin.block,
                        buf::first_non_blank(doc, origin.block, first_line).min(le),
                    )
                };
                self.set_mark(cx, '[', Position::new(origin.block, first_line));
                self.set_mark(cx, ']', Position::new(origin.block, first_line + len - 1));
                self.set_normal_caret(cx, caret);
                return;
            }
            let index = if before {
                origin.block
            } else {
                origin.block + 1
            };
            let n = content.fragment.blocks().len();
            for _ in 0..count {
                cx.ed.insert_blocks_at(index, &content.fragment);
            }
            let doc = cx.ed.document();
            let total = n * count;
            let caret = if caret_after {
                // The line after the new ones; at the end of the document
                // the last line, as in Vim.
                Position::new((index + total).min(doc.block_count() - 1), 0)
            } else {
                buf::unit_first_non_blank(doc, buf::unit_at(doc, Position::new(index, 0)))
            };
            self.set_mark(cx, '[', Position::new(index, 0));
            // The end of the put text is its last character.
            let last = index + total - 1;
            let last_text = buf::text_of(doc, last);
            let last_char = buf::prev_boundary(last_text, last_text.len());
            self.set_mark(cx, ']', Position::new(last, last_char));
            self.set_mark(cx, '.', Position::new(index, 0));
            if total > 2 {
                self.message(format!("{total} more lines"));
            }
            self.set_normal_caret(cx, caret);
            return;
        }
        let doc = cx.ed.document();
        let at = if before {
            origin
        } else {
            buf::cell_end(doc, origin)
        };
        cx.ed.set_caret(at);
        for _ in 0..count {
            cx.ed.paste(&content.fragment);
        }
        let end = cx.ed.selection().head;
        let multi = content.fragment.blocks().len() > 1;
        let doc = cx.ed.document();
        let caret = if caret_after {
            end
        } else if multi {
            // Vim leaves the caret at the start of a multi-line put.
            doc.clamp_position(at)
        } else {
            buf::prev_cell(doc, end)
                .filter(|p| p.block == end.block)
                .unwrap_or(end)
        };
        // `[` and `]` are the first and the last character put.
        let last = buf::prev_cell(doc, end).filter(|p| *p >= at).unwrap_or(at);
        self.set_mark(cx, '[', at);
        self.set_mark(cx, ']', last);
        self.set_mark(cx, '.', at);
        self.set_normal_caret(cx, caret);
    }

    /// `p` / `P` over a Visual selection: the selection is replaced by the
    /// register. With `keep` (`P`) the registers stay as they are;
    /// otherwise the replaced text goes to the unnamed register.
    pub(crate) fn put_over(
        &mut self,
        cx: &mut Cx,
        target: Target,
        content: &RegisterContent,
        count: usize,
        keep: bool,
    ) {
        let count = count.clamp(1, MAX_REPEAT);
        let replaced = self.target_content(cx, &target);
        // The first and last position of what was put, for `[`, `]`, `gv`.
        let mut put_range: Option<(Position, Position)> = None;
        match &target {
            Target::Chars(r) => {
                cx.ed.set_caret(r.start);
                cx.ed.delete_range(*r);
                let p = cx.ed.selection().head;
                if content.is_empty() {
                    // Nothing to put: the selection is just deleted.
                    self.set_normal_caret(cx, p);
                } else if !content.linewise {
                    for _ in 0..count {
                        cx.ed.paste(&content.fragment);
                    }
                    let end = cx.ed.selection().head;
                    let doc = cx.ed.document();
                    let last = buf::prev_cell(doc, end).filter(|q| *q >= p).unwrap_or(p);
                    put_range = Some((p, last));
                    let caret = if content.fragment.blocks().len() > 1 {
                        // Vim leaves the caret at the start of a multi-line
                        // put.
                        p
                    } else {
                        buf::prev_cell(doc, end)
                            .filter(|q| q.block == end.block)
                            .unwrap_or(end)
                    };
                    self.set_normal_caret(cx, caret);
                } else {
                    // Lines go between the two halves of the line, which is
                    // split where the selection was, as in Vim: an empty
                    // half stays as an empty line.
                    let mut blocks: Vec<Block> = vec![Block::new(BlockKind::Paragraph)];
                    let mut n = 0;
                    for _ in 0..count {
                        blocks.extend(content.fragment.blocks().iter().cloned());
                        n += content.fragment.blocks().len();
                    }
                    blocks.push(Block::new(BlockKind::Paragraph));
                    cx.ed.paste(&Fragment::new(blocks));
                    let first = p.block + 1;
                    put_range = Some((Position::new(first, 0), Position::new(first + n - 1, 0)));
                    let doc = cx.ed.document();
                    let c =
                        buf::unit_first_non_blank(doc, buf::unit_at(doc, Position::new(first, 0)));
                    self.set_normal_caret(cx, c);
                }
            }
            Target::Lines(Lines::Blocks(range)) => {
                let mut lines: Vec<Block> = Vec::new();
                for _ in 0..count {
                    lines.extend(content.fragment.blocks().iter().cloned());
                }
                let n = lines.len();
                // Insert first, then remove the old blocks, so the document
                // never runs empty in between.
                cx.ed.set_caret(Position::new(range.start, 0));
                if n > 0 {
                    cx.ed.insert_blocks_at(range.start, &Fragment::new(lines));
                    put_range = Some((
                        Position::new(range.start, 0),
                        Position::new(range.start + n - 1, 0),
                    ));
                }
                delete_blocks(cx, &(range.start + n..range.end + n));
                let doc = cx.ed.document();
                let c = buf::unit_first_non_blank(
                    doc,
                    buf::unit_at(doc, Position::new(range.start, 0)),
                );
                self.set_normal_caret(cx, c);
            }
            Target::Lines(Lines::Code { block, start, end }) => {
                let text = vec![content.text(); count].join("\n");
                cx.ed.set_caret(Position::new(*block, *start));
                cx.ed
                    .replace_range(TextRange::in_block(*block, *start, *end), &text);
                let doc = cx.ed.document();
                let o = buf::first_non_blank(doc, *block, *start);
                let le = buf::line_end(doc, Position::new(*block, *start));
                self.set_normal_caret(cx, Position::new(*block, o.min(le)));
            }
        }
        let nothing_replaced = matches!(&target, Target::Chars(r) if r.is_empty());
        if !keep && !nothing_replaced {
            let small = match &target {
                Target::Chars(r) => r.start.block == r.end.block,
                Target::Lines(_) => false,
            };
            self.registers.delete(None, replaced, small, false, cx.host);
        }
        let c = cx.ed.selection().head;
        let (first, last) = put_range.unwrap_or((c, c));
        self.set_mark(cx, '[', first);
        self.set_mark(cx, ']', last);
        self.set_mark(cx, '.', first);
        // `gv` selects what was put, in the mode the selection had.
        if let Some(v) = self.last_visual.as_mut() {
            let doc = cx.ed.document();
            v.anchor = PlaceMark::at(doc, first);
            v.cursor = PlaceMark::at(doc, last);
        }
        self.set_mark(cx, '<', first);
        self.set_mark(cx, '>', last);
    }
}
