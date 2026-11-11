//! Table-driven test harness.
//!
//! A case is a starting buffer, a key sequence and the expected buffer,
//! written in a small notation:
//!
//! - Blocks are separated by line breaks. A soft line break inside a block
//!   is written `↵`.
//! - `|` stands before the character the caret is on. In Visual mode `«`
//!   stands before the character at the other end of the selection.
//! - A block may start with an indent `{2}` and then a kind: `# ` `## `
//!   `### ` headings, `- ` bullet, `1. ` numbered, `[ ] ` / `[x] ` to-do,
//!   `> ` quote, `~ ` code block. `---` alone is a divider, `===` a page
//!   break.
//! - `**bold**` and `__italic__` mark formatted text.
//!
//! Keys use Vim notation (`<Esc>`, `<C-r>`, `<CR>`, …). Keys the engine does
//! not consume in Insert mode are carried out by a stand-in for the editor
//! (typing, enter, backspace, arrows), which also sends the inserted-text
//! notification, as the real host does.

#![allow(dead_code)]

use std::ops::Range;

use writer_core::{
    Block, BlockKind, Document, EditorOptions, EditorState, Fragment, InlineAttrs, Movement, Position,
    Selection, text,
};
use writer_vim::{
    Effect, GoalColumn, HostRequest, Key, KeyCode, Mode, Outcome, RegisterContent, Scroll, ScrollAlign,
    Viewport, Vim, VimHost, VimOptions,
};

// ----- the mock host -------------------------------------------------------------------------

/// One visual row: a byte range of a block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub block: usize,
    pub start: usize,
    pub end: usize,
}

/// A host that wraps every hard line at a fixed number of characters.
pub struct MockHost {
    pub width: usize,
    /// Index of the first visible row.
    pub top: usize,
    /// Number of visible rows.
    pub height: usize,
    pub clipboard: Option<RegisterContent>,
    pub scrolls: Vec<Scroll>,
}

impl MockHost {
    pub fn new(width: usize, height: usize) -> MockHost {
        MockHost {
            width,
            top: 0,
            height,
            clipboard: None,
            scrolls: Vec::new(),
        }
    }

    pub fn rows(&self, doc: &Document) -> Vec<Row> {
        let mut out = Vec::new();
        for (bi, b) in doc.blocks().iter().enumerate() {
            let t = b.text();
            let mut line_start = 0;
            loop {
                let line_end = t[line_start..].find('\n').map_or(t.len(), |i| line_start + i);
                if line_start == line_end {
                    out.push(Row {
                        block: bi,
                        start: line_start,
                        end: line_end,
                    });
                } else {
                    let mut o = line_start;
                    while o < line_end {
                        let mut e = o;
                        for _ in 0..self.width {
                            if e >= line_end {
                                break;
                            }
                            e = text::next_grapheme_boundary(t, e);
                        }
                        out.push(Row {
                            block: bi,
                            start: o,
                            end: e.min(line_end),
                        });
                        o = e;
                    }
                }
                if line_end >= t.len() {
                    break;
                }
                line_start = line_end + 1;
            }
        }
        out
    }

    fn row_index(&self, doc: &Document, pos: Position) -> usize {
        let rows = self.rows(doc);
        let t = doc.blocks()[pos.block].text();
        let mut best = 0;
        for (i, r) in rows.iter().enumerate() {
            if r.block != pos.block {
                continue;
            }
            best = i;
            let at_line_end = r.end >= t.len() || t.as_bytes()[r.end] == b'\n';
            if (r.start <= pos.offset && pos.offset < r.end) || (pos.offset == r.end && at_line_end) {
                return i;
            }
        }
        best
    }

    fn at_column(doc: &Document, row: &Row, col: usize) -> Position {
        let t = doc.blocks()[row.block].text();
        if row.start == row.end {
            return Position::new(row.block, row.start);
        }
        let mut o = row.start;
        for _ in 0..col {
            let n = text::next_grapheme_boundary(t, o);
            if n >= row.end {
                break;
            }
            o = n;
        }
        Position::new(row.block, o)
    }
}

impl VimHost for MockHost {
    fn visual_row(&mut self, doc: &Document, pos: Position) -> Range<usize> {
        let rows = self.rows(doc);
        let r = &rows[self.row_index(doc, pos)];
        r.start..r.end
    }

    fn column_of(&mut self, doc: &Document, pos: Position) -> f32 {
        let rows = self.rows(doc);
        let r = &rows[self.row_index(doc, pos)];
        let t = doc.blocks()[r.block].text();
        let upto = pos.offset.clamp(r.start, r.end);
        text::grapheme_count(&t[r.start..upto]) as f32
    }

    fn move_by_rows(&mut self, doc: &Document, from: Position, rows: isize, goal: GoalColumn) -> Position {
        let all = self.rows(doc);
        let i = self.row_index(doc, from) as isize;
        let j = (i + rows).clamp(0, all.len() as isize - 1) as usize;
        let col = match goal {
            GoalColumn::X(x) => x.max(0.0) as usize,
            GoalColumn::EndOfRow => usize::MAX,
        };
        MockHost::at_column(doc, &all[j], col)
    }

    fn viewport(&mut self, doc: &Document) -> Option<Viewport> {
        let rows = self.rows(doc);
        let top = self.top.min(rows.len() - 1);
        let bottom = (top + self.height.max(1) - 1).min(rows.len() - 1);
        Some(Viewport {
            top: Position::new(rows[top].block, rows[top].start),
            bottom: Position::new(rows[bottom].block, rows[bottom].start),
            rows: self.height.max(1),
        })
    }

    fn scroll(&mut self, doc: &Document, scroll: Scroll) {
        self.scrolls.push(scroll);
        let n = self.rows(doc).len() as isize;
        let top = match scroll {
            Scroll::Rows(r) => self.top as isize + r,
            Scroll::Align { caret, align } => {
                let r = self.row_index(doc, caret) as isize;
                match align {
                    ScrollAlign::Top => r,
                    ScrollAlign::Center => r - (self.height as isize) / 2,
                    ScrollAlign::Bottom => r - (self.height as isize - 1),
                }
            }
        };
        self.top = top.clamp(0, n - 1) as usize;
    }

    fn clipboard_read(&mut self) -> Option<RegisterContent> {
        self.clipboard.clone()
    }

    fn clipboard_write(&mut self, content: &RegisterContent) {
        self.clipboard = Some(content.clone());
    }
}

// ----- the buffer notation ---------------------------------------------------------------------

const CARET: char = '|';
const ANCHOR: char = '«';
const SOFT_BREAK: char = '↵';

struct ParsedBuffer {
    blocks: Vec<Block>,
    caret: Option<Position>,
    anchor: Option<Position>,
}

fn parse_kind(line: &str) -> (BlockKind, &str) {
    let prefixes: [(&str, BlockKind); 9] = [
        ("### ", BlockKind::heading(3)),
        ("## ", BlockKind::heading(2)),
        ("# ", BlockKind::heading(1)),
        ("- ", BlockKind::BulletItem),
        ("1. ", BlockKind::NumberedItem),
        ("[ ] ", BlockKind::TodoItem { checked: false }),
        ("[x] ", BlockKind::TodoItem { checked: true }),
        ("> ", BlockKind::Quote),
        ("~ ", BlockKind::code()),
    ];
    for (p, k) in prefixes {
        if let Some(rest) = line.strip_prefix(p) {
            return (k, rest);
        }
    }
    (BlockKind::Paragraph, line)
}

fn parse_buffer(src: &str) -> ParsedBuffer {
    let mut blocks = Vec::new();
    let mut caret = None;
    let mut anchor = None;
    for (bi, raw) in src.split('\n').enumerate() {
        let mut line = raw;
        let mut indent = 0u8;
        if let Some(rest) = line.strip_prefix('{')
            && let Some(close) = rest.find('}')
            && let Ok(n) = rest[..close].parse::<u8>()
        {
            indent = n;
            line = &rest[close + 1..];
        }
        let bare: String = line.chars().filter(|c| *c != CARET && *c != ANCHOR).collect();
        if bare == "---" || bare == "===" {
            let kind = if bare == "---" { BlockKind::Divider } else { BlockKind::PageBreak };
            if line.contains(CARET) {
                caret = Some(Position::new(bi, 0));
            }
            if line.contains(ANCHOR) {
                anchor = Some(Position::new(bi, 0));
            }
            blocks.push(Block::new(kind));
            continue;
        }
        let (kind, body) = parse_kind(line);
        let mut block = Block::new(kind).with_indent(indent);
        let mut attrs = InlineAttrs::default();
        let mut pending = String::new();
        let mut len = 0usize;
        let mut chars = body.chars().peekable();
        let flush = |block: Block, pending: &mut String, attrs: &InlineAttrs| -> Block {
            if pending.is_empty() {
                block
            } else {
                let b = block.with_span(pending, attrs.clone());
                pending.clear();
                b
            }
        };
        while let Some(c) = chars.next() {
            match c {
                CARET => caret = Some(Position::new(bi, len)),
                ANCHOR => anchor = Some(Position::new(bi, len)),
                '*' if chars.peek() == Some(&'*') => {
                    chars.next();
                    block = flush(block, &mut pending, &attrs);
                    attrs.bold = !attrs.bold;
                }
                '_' if chars.peek() == Some(&'_') => {
                    chars.next();
                    block = flush(block, &mut pending, &attrs);
                    attrs.italic = !attrs.italic;
                }
                SOFT_BREAK => {
                    pending.push('\n');
                    len += 1;
                }
                c => {
                    pending.push(c);
                    len += c.len_utf8();
                }
            }
        }
        block = flush(block, &mut pending, &attrs);
        blocks.push(block);
    }
    ParsedBuffer {
        blocks,
        caret,
        anchor,
    }
}

fn render_block(b: &Block, caret: Option<usize>, anchor: Option<usize>) -> String {
    let mut out = String::new();
    if b.indent() > 0 {
        out.push_str(&format!("{{{}}}", b.indent()));
    }
    let mark = |out: &mut String, at: usize| {
        if anchor == Some(at) {
            out.push(ANCHOR);
        }
        if caret == Some(at) {
            out.push(CARET);
        }
    };
    match b.kind() {
        BlockKind::Divider | BlockKind::PageBreak => {
            mark(&mut out, 0);
            out.push_str(if *b.kind() == BlockKind::Divider { "---" } else { "===" });
            return out;
        }
        BlockKind::Paragraph => {}
        BlockKind::Heading { level } => {
            out.push_str(&"#".repeat(*level as usize));
            out.push(' ');
        }
        BlockKind::BulletItem => out.push_str("- "),
        BlockKind::NumberedItem => out.push_str("1. "),
        BlockKind::TodoItem { checked: false } => out.push_str("[ ] "),
        BlockKind::TodoItem { checked: true } => out.push_str("[x] "),
        BlockKind::Quote => out.push_str("> "),
        BlockKind::CodeBlock { .. } => out.push_str("~ "),
    }
    let t = b.text();
    let mut bold = false;
    let mut italic = false;
    for (i, c) in t.char_indices() {
        let a = b.attrs_at(i).cloned().unwrap_or_default();
        if a.bold != bold {
            out.push_str("**");
            bold = a.bold;
        }
        if a.italic != italic {
            out.push_str("__");
            italic = a.italic;
        }
        mark(&mut out, i);
        out.push(if c == '\n' { SOFT_BREAK } else { c });
    }
    if bold {
        out.push_str("**");
    }
    if italic {
        out.push_str("__");
    }
    mark(&mut out, t.len());
    out
}

/// The document in the notation, with the caret (and Visual anchor).
pub fn render(doc: &Document, caret: Option<Position>, anchor: Option<Position>) -> String {
    doc.blocks()
        .iter()
        .enumerate()
        .map(|(i, b)| {
            render_block(
                b,
                caret.filter(|p| p.block == i).map(|p| p.offset),
                anchor.filter(|p| p.block == i).map(|p| p.offset),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A register in the notation; a linewise register ends with a line break.
pub fn render_register(r: &RegisterContent) -> String {
    let mut s = r
        .fragment
        .blocks()
        .iter()
        .map(|b| render_block(b, None, None))
        .collect::<Vec<_>>()
        .join("\n");
    if r.linewise {
        s.push('\n');
    }
    s
}

/// A register from the notation (a trailing line break makes it linewise).
pub fn register(src: &str) -> RegisterContent {
    let (body, linewise) = match src.strip_suffix('\n') {
        Some(b) => (b, true),
        None => (src, false),
    };
    let fragment = Fragment::new(parse_buffer(body).blocks);
    RegisterContent { fragment, linewise }
}

// ----- a running session -----------------------------------------------------------------------

/// An editor, a mock host and an engine, driven by keys.
pub struct Session {
    pub ed: EditorState,
    pub host: MockHost,
    pub vim: Vim,
    pub effects: Vec<Effect>,
    /// Keys nobody handled (not consumed in Normal mode, unknown to the
    /// stand-in editor).
    pub unhandled: Vec<Key>,
}

impl Session {
    pub fn new(buffer: &str) -> Session {
        Session::with(buffer, 1000, 10, VimOptions::default())
    }

    pub fn with(buffer: &str, width: usize, height: usize, options: VimOptions) -> Session {
        let parsed = parse_buffer(buffer);
        let mut ed = EditorState::new(Document::from_blocks(parsed.blocks));
        ed.set_options(EditorOptions {
            input_rules: false,
            smart_punctuation: false,
        });
        let mut host = MockHost::new(width, height);
        let mut vim = Vim::with_options(options);
        let caret = parsed.caret.unwrap_or(Position::ZERO);
        ed.set_caret(caret);
        let mut effects = vim.attach(&mut ed, &mut host).effects;
        if let Some(anchor) = parsed.anchor {
            // A selection made by the host: `sync` turns it into Visual mode.
            let next = |ed: &EditorState, p: Position| ed.document().moved(p, Movement::Right);
            let sel = if caret >= anchor {
                Selection::new(anchor, next(&ed, caret))
            } else {
                Selection::new(next(&ed, anchor), caret)
            };
            ed.set_selection(sel);
            effects.extend(vim.sync(&mut ed, &mut host).effects);
        }
        Session {
            ed,
            host,
            vim,
            effects,
            unhandled: Vec::new(),
        }
    }

    fn absorb(&mut self, o: Outcome) -> bool {
        self.effects.extend(o.effects);
        o.consumed
    }

    /// Types keys in Vim notation.
    pub fn keys(&mut self, keys: &str) -> &mut Session {
        for k in Key::parse_sequence(keys) {
            self.key(k);
        }
        self
    }

    pub fn key(&mut self, k: Key) {
        let o = self.vim.handle_key(k, &mut self.ed, &mut self.host);
        if self.absorb(o) {
            return;
        }
        if self.vim.mode().is_insert() {
            self.host_key(k);
        } else {
            self.host_key_normal(k);
        }
    }

    /// The stand-in editor: what the real one does with a key in Insert mode.
    fn host_key(&mut self, k: Key) {
        match k.code {
            KeyCode::Char(c) if !k.ctrl && !k.alt => {
                let mut b = [0u8; 4];
                let s = c.encode_utf8(&mut b);
                self.ed.insert_text(s);
                let o = self.vim.text_inserted(s, &mut self.ed, &mut self.host);
                self.absorb(o);
            }
            KeyCode::Char('d') if k.ctrl => {
                self.ed.delete_forward();
            }
            KeyCode::Char('k') if k.ctrl => {
                self.ed.delete_to_line_end();
            }
            KeyCode::Char('a') if k.ctrl => self.ed.move_caret(Movement::LineStart, false),
            KeyCode::Char('e') if k.ctrl => self.ed.move_caret(Movement::LineEnd, false),
            KeyCode::Enter if k.shift => {
                self.ed.insert_soft_break();
            }
            KeyCode::Enter => {
                self.ed.insert_paragraph_break();
            }
            KeyCode::Backspace if k.alt => {
                self.ed.delete_word_backward();
            }
            KeyCode::Backspace => {
                self.ed.backspace();
            }
            KeyCode::Delete => {
                self.ed.delete_forward();
            }
            KeyCode::Tab => {
                self.ed.insert_text("\t");
                let o = self.vim.text_inserted("\t", &mut self.ed, &mut self.host);
                self.absorb(o);
            }
            KeyCode::Left => self.ed.move_caret(Movement::Left, k.shift),
            KeyCode::Right => self.ed.move_caret(Movement::Right, k.shift),
            KeyCode::Home => self.ed.move_caret(Movement::LineStart, k.shift),
            KeyCode::End => self.ed.move_caret(Movement::LineEnd, k.shift),
            _ => self.unhandled.push(k),
        }
    }

    /// Outside Insert mode the editor only sees what the engine left:
    /// shifted arrows extend the selection, then the engine is synced.
    fn host_key_normal(&mut self, k: Key) {
        let movement = match k.code {
            KeyCode::Left => Some(Movement::Left),
            KeyCode::Right => Some(Movement::Right),
            KeyCode::Home => Some(Movement::LineStart),
            KeyCode::End => Some(Movement::LineEnd),
            _ => None,
        };
        match movement {
            Some(m) if k.shift => {
                self.ed.move_caret(m, true);
                let o = self.vim.sync(&mut self.ed, &mut self.host);
                self.absorb(o);
            }
            _ => self.unhandled.push(k),
        }
    }

    /// Types text the way an input method commits it: one insertion, one
    /// notification.
    pub fn commit(&mut self, s: &str) -> &mut Session {
        self.ed.insert_text(s);
        let o = self.vim.text_inserted(s, &mut self.ed, &mut self.host);
        self.absorb(o);
        self
    }

    pub fn mode(&self) -> Mode {
        self.vim.mode()
    }

    /// The buffer with the caret (and Visual anchor) marked.
    pub fn buffer(&self) -> String {
        let caret = self.vim.caret(&self.ed);
        let anchor = if self.vim.mode().is_visual() {
            let sel = self.ed.selection();
            // The other end of the selection, as a cell.
            let doc = self.ed.document();
            let a = if sel.head >= sel.anchor {
                sel.anchor
            } else {
                // The anchor cell is the last selected one.
                let end = sel.anchor;
                if end.offset > 0 {
                    let t = doc.blocks()[end.block].text();
                    Position::new(end.block, text::prev_grapheme_boundary(t, end.offset))
                } else if end.block > 0 {
                    Position::new(end.block - 1, doc.blocks()[end.block - 1].len())
                } else {
                    end
                }
            };
            Some(a)
        } else {
            None
        };
        render(self.ed.document(), Some(caret), anchor)
    }

    /// The buffer without markers.
    pub fn text(&self) -> String {
        render(self.ed.document(), None, None)
    }

    pub fn reg(&self, name: char) -> String {
        match self.vim.register(name) {
            Some(r) => render_register(r),
            None => "<unset>".into(),
        }
    }

    pub fn selection(&self) -> Selection {
        self.ed.selection()
    }

    pub fn messages(&self) -> Vec<String> {
        self.effects
            .iter()
            .filter_map(|e| match e {
                Effect::Message(m) | Effect::Error(m) => Some(m.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn requests(&self) -> Vec<HostRequest> {
        self.effects
            .iter()
            .filter_map(|e| match e {
                Effect::Request(r) => Some(r.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn bells(&self) -> usize {
        self.effects.iter().filter(|e| matches!(e, Effect::Bell)).count()
    }

    pub fn clear_effects(&mut self) {
        self.effects.clear();
    }

    /// Checks what must hold after every command.
    pub fn check_invariants(&self) -> Result<(), String> {
        self.ed.check_invariants()?;
        let doc = self.ed.document();
        let sel = self.ed.selection();
        if self.vim.mode() == Mode::Normal {
            if !sel.is_collapsed() {
                return Err(format!("Normal mode with a selection: {sel:?}"));
            }
            let p = sel.head;
            let t = doc.blocks()[p.block].text();
            let ls = text::line_start(t, p.offset);
            let le = text::line_end(t, p.offset);
            if p.offset == le && le > ls {
                return Err(format!("Normal-mode caret after the end of a line: {p:?}"));
            }
        }
        Ok(())
    }
}

// ----- table cases -------------------------------------------------------------------------------

/// One table row.
#[derive(Clone, Copy, Debug)]
pub struct Case {
    pub start: &'static str,
    pub keys: &'static str,
    pub expect: &'static str,
    pub mode: Mode,
    pub regs: &'static [(char, &'static str)],
    pub width: usize,
    pub height: usize,
    pub rules: bool,
    pub clip: Option<&'static str>,
    pub expect_clip: Option<&'static str>,
    pub message: Option<&'static str>,
    pub bell: Option<bool>,
    pub strict_words: bool,
}

/// A case that ends in Normal mode.
pub const fn c(start: &'static str, keys: &'static str, expect: &'static str) -> Case {
    Case {
        start,
        keys,
        expect,
        mode: Mode::Normal,
        regs: &[],
        width: 1000,
        height: 10,
        rules: false,
        clip: None,
        expect_clip: None,
        message: None,
        bell: None,
        strict_words: false,
    }
}

impl Case {
    /// The mode the case must end in.
    pub const fn mode(mut self, m: Mode) -> Case {
        self.mode = m;
        self
    }

    /// Ends in Insert mode.
    pub const fn insert(self) -> Case {
        self.mode(Mode::Insert)
    }

    /// Ends in Visual mode.
    pub const fn visual(self) -> Case {
        self.mode(Mode::Visual)
    }

    /// Ends in Visual Line mode.
    pub const fn vline(self) -> Case {
        self.mode(Mode::VisualLine)
    }

    /// Expected registers, in the buffer notation.
    pub const fn regs(mut self, r: &'static [(char, &'static str)]) -> Case {
        self.regs = r;
        self
    }

    /// Wrap width of the mock host.
    pub const fn width(mut self, w: usize) -> Case {
        self.width = w;
        self
    }

    /// Visible rows of the mock host.
    pub const fn height(mut self, h: usize) -> Case {
        self.height = h;
        self
    }

    /// Turns input rules and smart punctuation on.
    pub const fn rules(mut self) -> Case {
        self.rules = true;
        self
    }

    /// The system clipboard before the keys.
    pub const fn clip(mut self, text: &'static str) -> Case {
        self.clip = Some(text);
        self
    }

    /// The system clipboard after the keys.
    pub const fn expect_clip(mut self, text: &'static str) -> Case {
        self.expect_clip = Some(text);
        self
    }

    /// A message or error containing this text must have been reported.
    pub const fn message(mut self, text: &'static str) -> Case {
        self.message = Some(text);
        self
    }

    /// Whether a bell must (or must not) have rung.
    pub const fn bell(mut self, b: bool) -> Case {
        self.bell = Some(b);
        self
    }

    /// Vim's own word rule: an apostrophe is punctuation.
    pub const fn strict_words(mut self) -> Case {
        self.strict_words = true;
        self
    }

    pub fn run(&self) -> Result<(), String> {
        let options = VimOptions {
            apostrophe_in_words: !self.strict_words,
            ..VimOptions::default()
        };
        let mut s = Session::with(self.start, self.width, self.height, options);
        if self.rules {
            s.ed.set_options(EditorOptions::default());
        }
        if let Some(t) = self.clip {
            s.host.clipboard = Some(RegisterContent::from_plain_text(t));
        }
        s.clear_effects();
        s.keys(self.keys);
        let mut problems = Vec::new();
        let got = s.buffer();
        if got != self.expect {
            problems.push(format!("buffer: expected {:?}\n              got {:?}", self.expect, got));
        }
        if s.mode() != self.mode {
            problems.push(format!("mode: expected {:?}, got {:?}", self.mode, s.mode()));
        }
        for (name, want) in self.regs {
            let got = s.reg(*name);
            if got != *want {
                problems.push(format!("register {name}: expected {want:?}, got {got:?}"));
            }
        }
        if let Some(want) = self.expect_clip {
            let got = s.host.clipboard.as_ref().map(|c| c.to_plain_text());
            if got.as_deref() != Some(want) {
                problems.push(format!("clipboard: expected {want:?}, got {got:?}"));
            }
        }
        if let Some(want) = self.message
            && !s.messages().iter().any(|m| m.contains(want))
        {
            problems.push(format!("message: expected one containing {want:?}, got {:?}", s.messages()));
        }
        if let Some(want) = self.bell
            && (s.bells() > 0) != want
        {
            problems.push(format!("bell: expected {want}, rang {} time(s)", s.bells()));
        }
        if let Err(e) = s.check_invariants() {
            problems.push(format!("invariant: {e}"));
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "start {:?}  keys {:?}\n    {}",
                self.start,
                self.keys,
                problems.join("\n    ")
            ))
        }
    }
}

/// Runs a table and reports every failing row.
#[track_caller]
pub fn run(cases: &[Case]) {
    let mut failures = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        let result = std::panic::catch_unwind(|| case.run());
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => failures.push(format!("case {i}: {e}")),
            Err(_) => failures.push(format!(
                "case {i}: PANIC  start {:?}  keys {:?}",
                case.start, case.keys
            )),
        }
    }
    if !failures.is_empty() {
        panic!(
            "{} of {} cases failed:\n\n{}\n",
            failures.len(),
            cases.len(),
            failures.join("\n\n")
        );
    }
}

/// Number of cases run through [`run`] is counted by the tables
/// themselves; this helper keeps a quick single-case form for tests that
/// need a session afterwards.
pub fn session(start: &str, keys: &str) -> Session {
    let mut s = Session::new(start);
    s.clear_effects();
    s.keys(keys);
    s
}
