//! Insert and Replace mode.
//!
//! Typing itself is the host's business: the engine only consumes the keys
//! Vim owns. What it keeps is a record of the session, so that the session
//! can be played again for counts (`3ihello`) and for `.`.

use writer_core::{Block, BlockKind, Fragment, Position, TextRange, text};

use crate::buf;
use crate::key::{Key, KeyCode};
use crate::register::RegisterContent;

use super::{Base, ChangeRecord, Cx, Resume, Vim};

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
        });
        self.base = if replace { Base::Replace } else { Base::Insert };
        self.goal = None;
        // Typing is a new change: nothing is left to redo.
        self.redo_carets.clear();
    }

    /// A key in Insert or Replace mode. Returns whether Vim used it.
    pub(crate) fn insert_key(&mut self, key: Key, cx: &mut Cx) -> bool {
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
                'w' => {
                    self.insert_delete_word(cx);
                    self.record_insert(InsertAction::DeleteWord);
                    true
                }
                'u' => {
                    self.insert_delete_to_line_start(cx);
                    self.record_insert(InsertAction::DeleteToLineStart);
                    true
                }
                'h' => {
                    if replace {
                        self.replace_backspace(cx);
                    } else {
                        cx.ed.backspace();
                        self.record_insert(InsertAction::Backspace);
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
                    self.record_insert(InsertAction::DeleteForward);
                    false
                }
                'k' => {
                    self.record_insert(InsertAction::DeleteToLineEnd);
                    false
                }
                'a' | 'e' | 'f' | 'b' | 'n' | 'p' => {
                    if let Some(s) = self.insert.as_mut() {
                        s.interrupted();
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
                self.record_insert(if key.alt {
                    InsertAction::HostDeleteWord
                } else {
                    InsertAction::Backspace
                });
                false
            }
            KeyCode::Delete if !key.ctrl && !key.alt => {
                self.record_insert(InsertAction::DeleteForward);
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
                if let Some(s) = self.insert.as_mut() {
                    s.interrupted();
                }
                false
            }
            _ => false,
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
        let Some(s) = self.insert.as_mut() else {
            return;
        };
        if typed.is_empty() {
            return;
        }
        s.push_text(typed);
        if s.replace {
            let n = text::grapheme_count(typed);
            for _ in 0..n {
                let over = Self::overwrite_one(cx);
                if let Some(s) = self.insert.as_mut() {
                    s.overwritten.push(over);
                }
            }
        }
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
        let prev = Position::new(p.block, buf::prev_boundary(buf::text_of(doc, p.block), p.offset));
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
    fn insert_delete_word(&mut self, cx: &mut Cx) {
        let p = cx.ed.selection().head;
        let doc = cx.ed.document();
        let ls = buf::line_start(doc, p);
        if p.offset <= ls {
            cx.ed.backspace();
            return;
        }
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
        cx.ed.delete_range(TextRange::in_block(p.block, o, p.offset));
    }

    /// `ctrl-u`: deletes back to the start of the line.
    fn insert_delete_to_line_start(&mut self, cx: &mut Cx) {
        let p = cx.ed.selection().head;
        let ls = buf::line_start(cx.ed.document(), p);
        if p.offset <= ls {
            cx.ed.backspace();
        } else {
            cx.ed.delete_range(TextRange::in_block(p.block, ls, p.offset));
        }
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
    pub(crate) fn apply_insert_actions(&mut self, cx: &mut Cx, actions: &[InsertAction], replace: bool) {
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
                            Self::overwrite_one(cx);
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
                cx.ed.replace_range(TextRange::in_block(p.block, le, le), &ins);
                cx.ed.set_caret(Position::new(p.block, le + ins.len()));
            } else {
                let ins = format!("{indent}\n");
                cx.ed.replace_range(TextRange::in_block(p.block, ls, ls), &ins);
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
        let Some(mut s) = self.insert.take() else {
            self.base = Base::Normal;
            return;
        };
        if s.count > 1 && !s.actions.is_empty() {
            let actions = s.actions.clone();
            for _ in 1..s.count {
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
        if !s.actions.is_empty() {
            self.set_mark(cx, '.', head);
            self.set_mark(cx, ']', head);
            self.last_inserted = Self::actions_text(&s.actions);
        }
        // The caret steps back onto the last typed character.
        let doc = cx.ed.document();
        let ls = buf::line_start(doc, head);
        let back = if head.offset > ls {
            Position::new(head.block, buf::prev_boundary(buf::text_of(doc, head.block), head.offset))
        } else {
            head
        };
        self.base = Base::Normal;
        self.set_normal_caret(cx, back);
        if s.group_open {
            cx.ed.end_undo_group();
        }
        self.goal = None;
        if let Some(mut rec) = s.record.take() {
            rec.insert = std::mem::take(&mut s.actions);
            if !self.replaying {
                self.last_change = Some(rec);
            }
        }
    }

    /// `ctrl-o`: one Normal-mode command, then back to Insert mode.
    fn insert_one_shot(&mut self, cx: &mut Cx) {
        let Some(s) = self.insert.take() else {
            return;
        };
        let head = cx.ed.selection().head;
        if s.group_open {
            cx.ed.end_undo_group();
        }
        if !s.actions.is_empty() {
            self.last_inserted = Self::actions_text(&s.actions);
        }
        self.set_mark(cx, '^', head);
        // The caret stays where it is, even after the last character of a
        // line: `A<C-o>h` then lands on that character, as in Vim.
        cx.ed.set_caret(head);
        self.base = Base::Normal;
        self.resume = Some(Resume { replace: s.replace });
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
        self.begin_insert(cx, r.replace, 1, None, false);
    }
}
