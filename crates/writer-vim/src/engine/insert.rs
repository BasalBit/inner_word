//! Insert and Replace mode.
//!
//! Typing itself is the host's business: the engine only consumes the keys
//! Vim owns. What it keeps is a record of the session, so that the session
//! can be played again for counts (`3ihello`) and for `.`.

use writer_core::{Block, BlockKind, Fragment, Position, TextRange, text};

use crate::buf;
use crate::key::{Key, KeyCode};
use crate::register::RegisterContent;

use super::marks::Mark;
use super::{Base, ChangeRecord, Cx, Resume, Vim};
use crate::host::GoalColumn;

/// The most often a count repeats typed text or a put.
pub(crate) const MAX_REPEAT: usize = 10_000;

/// One thing that happened in an Insert session, in a form that can be
/// done again.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum InsertAction {
    /// Text typed through the host (as typed, before input rules).
    Text(String),
    /// Enter.
    ParagraphBreak,
    /// Shift-enter.
    SoftBreak,
    /// Backspace, `ctrl-h`.
    Backspace,
    /// Forward delete, `ctrl-d` of the host's Emacs keys.
    DeleteForward,
    /// `ctrl-w`: back to the start of the Vim word.
    DeleteWord,
    /// Option-backspace, handled by the host.
    HostDeleteWord,
    /// `ctrl-u`: back to the start of the line.
    DeleteToLineStart,
    /// `ctrl-k` of the host's Emacs keys.
    DeleteToLineEnd,
    /// `ctrl-r {register}`.
    Paste(RegisterContent),
}

/// An Insert or Replace session: from the command that started it to
/// escape.
#[derive(Clone, Debug)]
pub(crate) struct InsertSession {
    pub replace: bool,
    /// How often the typed text is wanted in total (`3i`).
    pub count: usize,
    /// `o` (`Some(true)`) or `O` (`Some(false)`): counts open more lines.
    pub open: Option<bool>,
    pub actions: Vec<InsertAction>,
    /// Replace mode: what each typed character overwrote.
    pub overwritten: Vec<Option<String>>,
    /// The command that started the session, for `.`.
    pub record: Option<ChangeRecord>,
    /// `ctrl-r` was pressed; the next key names a register.
    pub await_register: bool,
    /// The session holds an undo group open.
    pub group_open: bool,
    /// Where typing started. `ctrl-w` and `ctrl-u` stop here first, as in
    /// Vim, and go further only when pressed again.
    pub start: Position,
    /// A cursor key was left to the host with the caret here. If the caret
    /// has moved by the next event, the session was interrupted: what was
    /// typed before no longer repeats and is an undo step of its own.
    pub moved_check: Option<Position>,
    /// A deleting key was left to the host at this document revision, and
    /// recorded. If the revision is the same at the next event, the key
    /// deleted nothing (backspace at the start of the document) and the
    /// record drops it.
    pub unverified: Option<u64>,
    /// The session continues one that `ctrl-o` interrupted.
    pub resumed: bool,
}

impl InsertSession {
    /// Something happened that the record cannot express (the caret was
    /// moved): what was typed before no longer repeats.
    pub fn interrupted(&mut self) {
        self.actions.clear();
        self.overwritten.clear();
        self.count = 1;
        self.open = None;
    }

    fn push_text(&mut self, s: &str) {
        if let Some(InsertAction::Text(t)) = self.actions.last_mut() {
            t.push_str(s);
        } else {
            self.actions.push(InsertAction::Text(s.to_string()));
        }
    }
}

/// The graphemes of `s`.
fn graphemes(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut o = 0;
    while o < s.len() {
        let n = text::next_grapheme_boundary(s, o);
        out.push(&s[o..n]);
        o = n;
    }
    out
}

impl Vim {
    /// Starts an Insert (or Replace) session at the caret. The session is
    /// one undo step, together with whatever the starting command already
    /// changed inside the enclosing undo group.
    pub(crate) fn begin_insert(
        &mut self,
        cx: &mut Cx,
        replace: bool,
        count: usize,
        open: Option<bool>,
        group_open: bool,
    ) {
        if !group_open {
            cx.ed.begin_undo_group();
        }
        self.insert = Some(InsertSession {
            replace,
            count: count.max(1),
            open,
            actions: Vec::new(),
            overwritten: Vec::new(),
            record: None,
            await_register: false,
            group_open: true,
            start: cx.ed.selection().head,
            moved_check: None,
            unverified: None,
            resumed: false,
        });
        self.base = if replace { Base::Replace } else { Base::Insert };
        self.goal = None;
        // Typing is a new change: nothing is left to redo.
        self.redo_carets.clear();
    }

    /// The input method replaced `range` (which lay before `caret`, both
    /// in positions from before the replacement) with `text`. Corrects the
    /// typed text of the session when the replaced text is part of the end
    /// of it; returns whether it could.
    pub(crate) fn amend_typed(&mut self, range: TextRange, caret: Position, text: &str) -> bool {
        let Some(s) = self.insert.as_mut() else {
            return false;
        };
        // Not with a cursor key or a delete still to be accounted for.
        if s.replace || s.moved_check.is_some() || s.unverified.is_some() {
            return false;
        }
        let (start, end) = (range.start, range.end);
        if start.block != caret.block
            || end.block != caret.block
            || start.offset > end.offset
            || end.offset > caret.offset
        {
            return false;
        }
        let Some(InsertAction::Text(typed)) = s.actions.last_mut() else {
            return false;
        };
        // The replaced text as bytes from the end of what was typed. This
        // takes the typed text to be what stands in the document, which
        // holds unless a rule rewrote it (smart quotes): then the lengths
        // may not fit and the caller falls back.
        let from_end = caret.offset - start.offset;
        let Some(from) = typed.len().checked_sub(from_end) else {
            return false;
        };
        let to = from + (end.offset - start.offset);
        if !typed.is_char_boundary(from) || !typed.is_char_boundary(to) {
            return false;
        }
        typed.replace_range(from..to, text);
        true
    }

    /// Settles a cursor key that was left to the host: if it moved the
    /// caret, the session starts afresh there (Vim: a cursor key in Insert
    /// mode ends the undoable insert and what `.` repeats). `assume_moved`
    /// is for callers that cannot compare (text already inserted).
    pub(crate) fn settle_cursor_key(&mut self, cx: &mut Cx, assume_moved: bool) {
        let Some(s) = self.insert.as_mut() else {
            return;
        };
        if let Some(revision) = s.unverified.take()
            && revision == cx.ed.revision()
            && matches!(
                s.actions.last(),
                Some(
                    InsertAction::Backspace
                        | InsertAction::DeleteForward
                        | InsertAction::HostDeleteWord
                        | InsertAction::DeleteToLineEnd
                )
            )
        {
            // The key the host got deleted nothing.
            s.actions.pop();
        }
        let Some(before) = s.moved_check.take() else {
            return;
        };
        let head = cx.ed.selection().head;
        if !assume_moved && head == before {
            return;
        }
        // What was typed so far is what `.` repeats, until something is
        // typed at the new place; that then repeats as a plain insert.
        let replace = s.replace;
        let committed = s.record.take().and_then(|mut rec| {
            let keep_previous = s.resumed && s.actions.is_empty();
            rec.insert = std::mem::take(&mut s.actions);
            (!keep_previous).then_some(rec)
        });
        s.interrupted();
        s.resumed = true;
        if !assume_moved {
            s.start = head;
        }
        if s.group_open {
            // What was typed so far is one undo step, the rest another.
            cx.ed.end_undo_group();
            cx.ed.begin_undo_group();
        }
        if !self.replaying {
            if let Some(rec) = committed {
                self.last_change = Some(rec);
            }
            if let Some(s) = self.insert.as_mut() {
                s.record = Some(ChangeRecord {
                    register: None,
                    count: None,
                    keys: vec![Key::char(if replace { 'R' } else { 'i' })],
                    visual: None,
                    insert: Vec::new(),
                });
            }
        }
    }

    /// A key in Insert or Replace mode. Returns whether Vim used it.
    pub(crate) fn insert_key(&mut self, key: Key, cx: &mut Cx) -> bool {
        self.settle_cursor_key(cx, false);
        let Some(session) = self.insert.as_mut() else {
            // No session (should not happen): fall back to Normal mode.
            self.base = Base::Normal;
            return self.normal_key(key, cx);
        };
        if session.await_register {
            session.await_register = false;
            if let Some(name) = key.plain_char() {
                if let Some(content) = self.read_register(cx, Some(name)) {
                    self.insert_paste(cx, &content);
                    if let Some(s) = self.insert.as_mut() {
                        s.actions.push(InsertAction::Paste(content));
                    }
                }
            } else if !key.is_escape() {
                self.bell();
            }
            return true;
        }
        if key.is_escape() {
            self.leave_insert(cx);
            return true;
        }
        let replace = session.replace;
        if let Some(c) = key.ctrl_char() {
            return match c {
                'w' | 'u' if replace => {
                    // Replace mode: back over the word (the line), putting
                    // back what was overwritten; nothing else is deleted.
                    let p = cx.ed.selection().head;
                    let to = if c == 'w' {
                        self.word_start_before(cx, p)
                    } else {
                        self.insert_delete_limit(cx, p)
                    };
                    loop {
                        let h = cx.ed.selection().head;
                        if h.block != p.block || h.offset <= to {
                            break;
                        }
                        self.replace_backspace(cx);
                        if cx.ed.selection().head == h {
                            break;
                        }
                    }
                    true
                }
                'w' => {
                    let before = cx.ed.revision();
                    self.insert_delete_word(cx);
                    if cx.ed.revision() != before {
                        self.record_insert(InsertAction::DeleteWord);
                    }
                    true
                }
                'u' => {
                    let before = cx.ed.revision();
                    self.insert_delete_to_line_start(cx);
                    if cx.ed.revision() != before {
                        self.record_insert(InsertAction::DeleteToLineStart);
                    }
                    true
                }
                'h' => {
                    if replace {
                        self.replace_backspace(cx);
                    } else {
                        let before = cx.ed.revision();
                        cx.ed.backspace();
                        if cx.ed.revision() != before {
                            self.record_insert(InsertAction::Backspace);
                        }
                        self.insert_start_follows(cx);
                    }
                    true
                }
                'o' => {
                    self.insert_one_shot(cx);
                    true
                }
                'r' => {
                    if let Some(s) = self.insert.as_mut() {
                        s.await_register = true;
                    }
                    true
                }
                // The host's Emacs keys: remember what they do.
                'd' => {
                    self.record_host_delete(cx, InsertAction::DeleteForward);
                    false
                }
                'k' => {
                    self.record_host_delete(cx, InsertAction::DeleteToLineEnd);
                    false
                }
                'a' | 'e' | 'f' | 'b' | 'n' | 'p' => {
                    let head = cx.ed.selection().head;
                    if let Some(s) = self.insert.as_mut() {
                        s.moved_check = Some(head);
                    }
                    false
                }
                _ => false,
            };
        }
        match key.code {
            KeyCode::Enter if !key.alt && !key.ctrl => {
                self.record_insert(if key.shift {
                    InsertAction::SoftBreak
                } else {
                    InsertAction::ParagraphBreak
                });
                false
            }
            KeyCode::Backspace if !key.ctrl => {
                if replace && !key.alt {
                    self.replace_backspace(cx);
                    return true;
                }
                self.record_host_delete(
                    cx,
                    if key.alt {
                        InsertAction::HostDeleteWord
                    } else {
                        InsertAction::Backspace
                    },
                );
                false
            }
            KeyCode::Delete if !key.ctrl && !key.alt => {
                self.record_host_delete(cx, InsertAction::DeleteForward);
                false
            }
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown => {
                let head = cx.ed.selection().head;
                if let Some(s) = self.insert.as_mut() {
                    s.moved_check = Some(head);
                }
                false
            }
            _ => false,
        }
    }

    /// Records a deleting key that is left to the host, to be checked
    /// against the document at the next event.
    fn record_host_delete(&mut self, cx: &Cx, action: InsertAction) {
        let before = self.insert.as_ref().map_or(0, |s| s.actions.len());
        self.record_insert(action);
        if let Some(s) = self.insert.as_mut()
            && s.actions.len() > before
        {
            s.unverified = Some(cx.ed.revision());
        }
    }

    fn record_insert(&mut self, action: InsertAction) {
        if let Some(s) = self.insert.as_mut() {
            // A backspace that removes text typed in this session simply
            // takes it out of the record.
            if action == InsertAction::Backspace
                && let Some(InsertAction::Text(t)) = s.actions.last_mut()
            {
                let cut = text::prev_grapheme_boundary(t, t.len());
                t.truncate(cut);
                if t.is_empty() {
                    s.actions.pop();
                }
                return;
            }
            s.actions.push(action);
        }
    }

    /// The host inserted `text` (see [`Vim::text_inserted`]).
    pub(crate) fn note_inserted(&mut self, typed: &str, cx: &mut Cx) {
        // Text right after a cursor key, with no key in between to compare
        // carets at: take the cursor key as having moved.
        self.settle_cursor_key(cx, true);
        let Some(s) = self.insert.as_mut() else {
            return;
        };
        if typed.is_empty() {
            return;
        }
        s.push_text(typed);
        if s.replace {
            let over = Self::overwrite(cx, typed);
            if let Some(s) = self.insert.as_mut() {
                s.overwritten.extend(over);
            }
        }
    }

    /// Replace mode: makes the just typed `typed` overwrite what follows
    /// the caret (never a line break). Returns what each typed character
    /// overwrote.
    ///
    /// A single character that is already in the document takes the place
    /// of the character after it, formatting included: bold text
    /// overwritten in Replace mode stays bold.
    fn overwrite(cx: &mut Cx, typed: &str) -> Vec<Option<String>> {
        let p = cx.ed.selection().head;
        let doc = cx.ed.document();
        let t = buf::text_of(doc, p.block);
        let single = text::grapheme_count(typed) == 1;
        if single && !buf::is_newline(doc, p) && t[..p.offset].ends_with(typed) {
            let old = buf::grapheme_at(doc, p).to_string();
            let start = p.offset - typed.len();
            cx.ed
                .delete_range(TextRange::in_block(p.block, start, p.offset));
            cx.ed.replace_range(
                TextRange::in_block(p.block, start, start + old.len()),
                typed,
            );
            return vec![Some(old)];
        }
        (0..text::grapheme_count(typed))
            .map(|_| Self::overwrite_one(cx))
            .collect()
    }

    /// Replace mode: removes the character after the caret (not a line
    /// break) and returns it.
    fn overwrite_one(cx: &mut Cx) -> Option<String> {
        let p = cx.ed.selection().head;
        let doc = cx.ed.document();
        if buf::is_newline(doc, p) {
            return None;
        }
        let g = buf::grapheme_at(doc, p).to_string();
        let end = Position::new(p.block, p.offset + g.len());
        cx.ed.delete_range(TextRange::new(p, end));
        Some(g)
    }

    /// Backspace in Replace mode: takes back the last typed character and
    /// restores what it overwrote; before the start of the session it only
    /// moves left.
    fn replace_backspace(&mut self, cx: &mut Cx) {
        let p = cx.ed.selection().head;
        let doc = cx.ed.document();
        let Some(s) = self.insert.as_mut() else {
            return;
        };
        let ls = buf::line_start(doc, p);
        if p.offset <= ls {
            return;
        }
        let prev = Position::new(
            p.block,
            buf::prev_boundary(buf::text_of(doc, p.block), p.offset),
        );
        match s.overwritten.pop() {
            Some(original) => {
                if let Some(InsertAction::Text(t)) = s.actions.last_mut() {
                    let cut = text::prev_grapheme_boundary(t, t.len());
                    t.truncate(cut);
                    if t.is_empty() {
                        s.actions.pop();
                    }
                }
                match original {
                    Some(g) => {
                        cx.ed.replace_range(TextRange::new(prev, p), &g);
                    }
                    None => {
                        cx.ed.delete_range(TextRange::new(prev, p));
                    }
                }
                cx.ed.set_caret(prev);
            }
            None => cx.ed.set_caret(prev),
        }
    }

    /// `ctrl-w`: deletes back to the start of the word before the caret;
    /// at the start of a line it joins like backspace.
    /// How far back `ctrl-w` and `ctrl-u` may delete from `p`: to where
    /// typing started when that lies before the caret on the same line,
    /// else to the start of the line.
    fn insert_delete_limit(&self, cx: &Cx, p: Position) -> usize {
        let ls = buf::line_start(cx.ed.document(), p);
        match self.insert.as_ref().map(|s| s.start) {
            Some(start)
                if start.block == p.block && start.offset > ls && start.offset < p.offset =>
            {
                start.offset
            }
            _ => ls,
        }
    }

    /// After a deletion in the session: typing now starts no later than
    /// the caret.
    fn insert_start_follows(&mut self, cx: &Cx) {
        let head = cx.ed.selection().head;
        if let Some(s) = self.insert.as_mut()
            && (head.block != s.start.block || head.offset < s.start.offset)
        {
            s.start = head;
        }
    }

    fn insert_delete_word(&mut self, cx: &mut Cx) {
        let p = cx.ed.selection().head;
        if p.offset <= buf::line_start(cx.ed.document(), p) {
            cx.ed.backspace();
            self.insert_start_follows(cx);
            return;
        }
        let o = self.word_start_before(cx, p);
        cx.ed
            .delete_range(TextRange::in_block(p.block, o, p.offset));
        self.insert_start_follows(cx);
    }

    /// Where `ctrl-w` at `p` deletes back to: over white space and then
    /// one word, not past where typing started.
    fn word_start_before(&self, cx: &Cx, p: Position) -> usize {
        let doc = cx.ed.document();
        let ls = self.insert_delete_limit(cx, p);
        let opts = self.class_opts(false);
        let t = buf::text_of(doc, p.block);
        let mut o = p.offset;
        // White space first, then one run of the same class.
        while o > ls {
            let q = buf::prev_boundary(t, o);
            if buf::class_at(doc, Position::new(p.block, q), opts) != buf::Class::Blank {
                break;
            }
            o = q;
        }
        if o > ls {
            let class = buf::class_at(doc, Position::new(p.block, buf::prev_boundary(t, o)), opts);
            while o > ls {
                let q = buf::prev_boundary(t, o);
                if buf::class_at(doc, Position::new(p.block, q), opts) != class {
                    break;
                }
                o = q;
            }
        }
        o
    }

    /// `ctrl-u`: deletes what was typed on the line; pressed again (or with
    /// nothing typed), back to the start of the line.
    fn insert_delete_to_line_start(&mut self, cx: &mut Cx) {
        let p = cx.ed.selection().head;
        if p.offset <= buf::line_start(cx.ed.document(), p) {
            cx.ed.backspace();
        } else {
            let from = self.insert_delete_limit(cx, p);
            cx.ed
                .delete_range(TextRange::in_block(p.block, from, p.offset));
        }
        self.insert_start_follows(cx);
    }

    /// `ctrl-r {register}`: inserts the register at the caret. A linewise
    /// register is followed by a line break, as in Vim.
    fn insert_paste(&mut self, cx: &mut Cx, content: &RegisterContent) {
        if content.linewise {
            let mut blocks: Vec<Block> = content.fragment.blocks().to_vec();
            blocks.push(Block::new(BlockKind::Paragraph));
            cx.ed.paste(&Fragment::new(blocks));
        } else {
            cx.ed.paste(&content.fragment);
        }
    }

    /// Does `actions` again at the caret.
    pub(crate) fn apply_insert_actions(
        &mut self,
        cx: &mut Cx,
        actions: &[InsertAction],
        replace: bool,
    ) {
        for a in actions {
            match a {
                InsertAction::Text(s) => {
                    // One character at a time, so input rules and smart
                    // punctuation act as they did while typing.
                    for g in graphemes(s) {
                        if g == "\n" {
                            cx.ed.insert_paragraph_break();
                            continue;
                        }
                        cx.ed.insert_text(g);
                        if replace {
                            Self::overwrite(cx, g);
                        }
                    }
                }
                InsertAction::ParagraphBreak => {
                    cx.ed.insert_paragraph_break();
                }
                InsertAction::SoftBreak => {
                    cx.ed.insert_soft_break();
                }
                InsertAction::Backspace => {
                    cx.ed.backspace();
                }
                InsertAction::DeleteForward => {
                    cx.ed.delete_forward();
                }
                InsertAction::DeleteWord => self.insert_delete_word(cx),
                InsertAction::HostDeleteWord => {
                    cx.ed.delete_word_backward();
                }
                InsertAction::DeleteToLineStart => self.insert_delete_to_line_start(cx),
                InsertAction::DeleteToLineEnd => {
                    cx.ed.delete_to_line_end();
                }
                InsertAction::Paste(c) => self.insert_paste(cx, c),
            }
        }
    }

    /// The plain text of `actions` (register `.`).
    fn actions_text(actions: &[InsertAction]) -> String {
        let mut out = String::new();
        for a in actions {
            match a {
                InsertAction::Text(s) => out.push_str(s),
                InsertAction::ParagraphBreak | InsertAction::SoftBreak => out.push('\n'),
                InsertAction::Paste(c) => out.push_str(&c.text()),
                _ => {}
            }
        }
        out
    }

    /// Opens a line for `o` (`below`) or `O`: a new block next to the
    /// caret's block (a list item or quote continues, anything else gets a
    /// paragraph), or a new line inside a code block with the indent of the
    /// current one.
    pub(crate) fn open_line(&mut self, cx: &mut Cx, below: bool) {
        let p = cx.ed.selection().head;
        let doc = cx.ed.document();
        if doc.blocks()[p.block].kind().is_code() {
            let ls = buf::line_start(doc, p);
            let le = buf::line_end(doc, p);
            let indent: String = buf::text_of(doc, p.block)[ls..le]
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            if below {
                let ins = format!("\n{indent}");
                cx.ed
                    .replace_range(TextRange::in_block(p.block, le, le), &ins);
                cx.ed.set_caret(Position::new(p.block, le + ins.len()));
            } else {
                let ins = format!("{indent}\n");
                cx.ed
                    .replace_range(TextRange::in_block(p.block, ls, ls), &ins);
                cx.ed.set_caret(Position::new(p.block, ls + indent.len()));
            }
            return;
        }
        if below {
            cx.ed.insert_block_after();
        } else {
            cx.ed.insert_block_before();
        }
    }

    /// Escape: ends the session. Plays the session again for a count,
    /// closes the undo group, steps the caret back onto a character and
    /// stores the session for `.`.
    pub(crate) fn leave_insert(&mut self, cx: &mut Cx) {
        self.settle_cursor_key(cx, false);
        let Some(mut s) = self.insert.take() else {
            self.base = Base::Normal;
            return;
        };
        // A count repeats what was typed; `3o<Esc>` opens three lines even
        // with nothing typed.
        if s.count > 1 && (!s.actions.is_empty() || s.open.is_some()) {
            let actions = s.actions.clone();
            // A slip of the finger (`99999ix`) must not freeze the app.
            for _ in 1..s.count.min(MAX_REPEAT) {
                if s.open.is_some() {
                    // Every further copy goes below the one before, for
                    // `O` too, so the lines come out in typing order.
                    self.open_line(cx, true);
                }
                self.apply_insert_actions(cx, &actions, s.replace);
            }
        }
        let head = cx.ed.selection().head;
        cx.ed.set_caret(head);
        self.set_mark(cx, '^', head);
        // The caret steps back onto the last typed character.
        let doc = cx.ed.document();
        let ls = buf::line_start(doc, head);
        let back = if head.offset > ls {
            Position::new(
                head.block,
                buf::prev_boundary(buf::text_of(doc, head.block), head.offset),
            )
        } else {
            head
        };
        if !s.actions.is_empty() {
            // The last change is the last character typed.
            self.set_mark(cx, '.', back);
            self.set_mark(cx, '[', s.start.min(back));
            self.set_mark(cx, ']', back);
            self.last_inserted = Self::actions_text(&s.actions);
        }
        self.base = Base::Normal;
        self.set_normal_caret(cx, back);
        if s.group_open {
            cx.ed.end_undo_group();
        }
        self.goal = None;
        if let Some(mut rec) = s.record.take() {
            // A session resumed after `ctrl-o` repeats only if something
            // was typed in it; otherwise the change before stays.
            let keep_previous = s.resumed && s.actions.is_empty();
            rec.insert = std::mem::take(&mut s.actions);
            if !self.replaying && !keep_previous {
                self.last_change = Some(rec);
            }
        }
    }

    /// `ctrl-o`: one Normal-mode command, then back to Insert mode.
    fn insert_one_shot(&mut self, cx: &mut Cx) {
        self.settle_cursor_key(cx, false);
        let Some(mut s) = self.insert.take() else {
            return;
        };
        let head = cx.ed.selection().head;
        if s.group_open {
            cx.ed.end_undo_group();
        }
        if !s.actions.is_empty() {
            self.last_inserted = Self::actions_text(&s.actions);
        }
        // What was typed so far is what `.` repeats, unless more is typed
        // after the command.
        if let Some(mut rec) = s.record.take() {
            let keep_previous = s.resumed && s.actions.is_empty();
            rec.insert = std::mem::take(&mut s.actions);
            if !self.replaying && !keep_previous {
                self.last_change = Some(rec);
            }
        }
        self.set_mark(cx, '^', head);
        // After the last character of a line the caret steps onto that
        // character, as in Vim (`A<C-o>h` lands on the one before it); the
        // session resumes after it if the command leaves the caret there.
        let doc = cx.ed.document();
        let on = buf::clamp_normal(doc, head);
        let eol_line = (on != head)
            .then(|| Mark::at(doc, Position::new(head.block, buf::line_start(doc, head))));
        cx.ed.set_caret(on);
        self.base = Base::Normal;
        self.goal = None;
        self.resume = Some(Resume {
            replace: s.replace,
            eol_line,
        });
    }

    /// After the one command of `ctrl-o`: back to Insert mode, unless the
    /// command is still being typed or left Normal mode itself.
    pub(crate) fn maybe_resume_insert(&mut self, cx: &mut Cx) {
        if self.cmdline.is_some() || self.pending.active() {
            return;
        }
        match self.base {
            Base::Normal => {}
            // The command started its own session (`ctrl-o A`).
            Base::Insert | Base::Replace => {
                self.resume = None;
                return;
            }
            Base::Visual { .. } => return,
        }
        let Some(r) = self.resume.take() else {
            return;
        };
        // On the last character of the line the caret was at the end of,
        // or after `$`: go on typing after that character.
        let head = cx.ed.selection().head;
        let doc = cx.ed.document();
        let ls = buf::line_start(doc, head);
        let le = buf::line_end(doc, head);
        let on_last = head.offset < le && buf::cell_end(doc, head).offset == le;
        let same_line = r
            .eol_line
            .and_then(|m| m.resolve(doc))
            .is_some_and(|p| p == Position::new(head.block, ls));
        // (A command such as `D` leaves the caret after the end of what is
        // left of the line.)
        let left_past_end = self.clamped_from_eol == Some(head);
        if on_last && (same_line || left_past_end || self.goal == Some(GoalColumn::EndOfRow)) {
            cx.ed.set_caret(Position::new(head.block, le));
        }
        self.begin_insert(cx, r.replace, 1, None, false);
        // What `.` repeats from here on is a plain insert of what gets
        // typed now, as in Vim; a count given to the first command is spent.
        if let Some(s) = self.insert.as_mut() {
            s.resumed = true;
        }
        if !self.replaying
            && let Some(s) = self.insert.as_mut()
        {
            s.record = Some(ChangeRecord {
                register: None,
                count: None,
                keys: vec![Key::char(if r.replace { 'R' } else { 'i' })],
                visual: None,
                insert: Vec::new(),
            });
        }
    }
}
