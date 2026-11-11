//! Normal, Visual and operator-pending mode: turning keys into commands.

use writer_core::{Position, Selection, TextRange};

use crate::buf::{self, Class, Lines};
use crate::host::{GoalColumn, Scroll, ScrollAlign};
use crate::effect::{Effect, HostRequest};
use crate::key::{Key, KeyCode};
use crate::motion;
use crate::register;
use crate::search;
use crate::textobj::{self, Target};

use super::cmdline::CmdKind;
use super::marks::Mark;
use super::motions::{GoalUpdate, Motion, MotionKind, MotionOut};
use super::operators::OpInfo;
use super::{Await, Base, ChangeRecord, Cx, FindSpec, Operator, Pending, SavedVisual, Vim, VisualRepeat, VisualState};

/// A key reduced to what the command tables look at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tok {
    Ch(char),
    Ctrl(char),
    Named(KeyCode),
}

fn tok(key: Key) -> Tok {
    match key.code {
        KeyCode::Char(c) if key.ctrl => Tok::Ctrl(c),
        KeyCode::Char(c) => Tok::Ch(c),
        code => Tok::Named(code),
    }
}

/// The exclusive end of a Visual selection whose last cell is `e`: a
/// newline cell takes its line break along.
fn visual_end(doc: &writer_core::Document, e: Position) -> Position {
    if buf::is_newline(doc, e) {
        buf::next_cell(doc, e).unwrap_or(e)
    } else {
        buf::cell_end(doc, e)
    }
}

impl Vim {
    // ----- Visual mode plumbing -----------------------------------------------------------

    /// Makes the editor's selection show the Visual selection.
    pub(crate) fn sync_visual_selection(&mut self, cx: &mut Cx) {
        let Base::Visual { line } = self.base else {
            return;
        };
        let doc = cx.ed.document();
        let a = doc.clamp_position(self.visual.anchor);
        let c = doc.clamp_position(self.visual.cursor);
        let (s, e) = if a <= c { (a, c) } else { (c, a) };
        let (start, end) = if line {
            match buf::lines_between(doc, s, e) {
                Lines::Blocks(r) => (
                    Position::new(r.start, 0),
                    Position::new(r.end - 1, buf::len_of(doc, r.end - 1)),
                ),
                Lines::Code { block, start, end } => (Position::new(block, start), Position::new(block, end)),
            }
        } else {
            (s, visual_end(doc, e))
        };
        let sel = if c >= a {
            Selection::new(start, end)
        } else {
            Selection::new(end, start)
        };
        cx.ed.set_selection(sel);
    }

    /// What a Visual-mode operator acts on.
    fn visual_target(&self, cx: &Cx) -> Target {
        let doc = cx.ed.document();
        let a = doc.clamp_position(self.visual.anchor);
        let c = doc.clamp_position(self.visual.cursor);
        let (s, e) = if a <= c { (a, c) } else { (c, a) };
        if matches!(self.base, Base::Visual { line: true }) {
            Target::Lines(buf::lines_between(doc, s, e))
        } else if s == e && doc.blocks()[s.block].is_void() {
            // A divider selected on its own is the whole block (a text
            // range could not hold it at the end of the document).
            Target::Lines(Lines::Blocks(s.block..s.block + 1))
        } else {
            Target::Chars(TextRange::new(s, visual_end(doc, e)))
        }
    }

    fn enter_visual(&mut self, cx: &mut Cx, line: bool) {
        let c = self.cursor(cx);
        self.visual = VisualState { anchor: c, cursor: c };
        self.base = Base::Visual { line };
        self.sync_visual_selection(cx);
    }

    /// Remembers the Visual selection for `gv` and the `<` `>` marks.
    pub(crate) fn save_visual(&mut self, cx: &Cx) {
        let Base::Visual { line } = self.base else {
            return;
        };
        let doc = cx.ed.document();
        let a = doc.clamp_position(self.visual.anchor);
        let c = doc.clamp_position(self.visual.cursor);
        self.last_visual = Some(SavedVisual {
            anchor: Mark::at(doc, a),
            cursor: Mark::at(doc, c),
            line,
        });
        self.marks.insert('<', Mark::at(doc, a.min(c)));
        self.marks.insert('>', Mark::at(doc, a.max(c)));
    }

    /// Leaves Visual mode; the caret stays where the cursor was.
    pub(crate) fn exit_visual(&mut self, cx: &mut Cx) {
        if !self.is_visual() {
            return;
        }
        self.save_visual(cx);
        let c = cx.ed.document().clamp_position(self.visual.cursor);
        self.base = Base::Normal;
        self.set_normal_caret(cx, c);
    }

    /// Moves the Vim cursor: the caret in Normal mode, the moving end of
    /// the selection in Visual mode.
    pub(crate) fn move_cursor(&mut self, cx: &mut Cx, p: Position) {
        self.move_cursor_eol(cx, p, false);
    }

    fn move_cursor_eol(&mut self, cx: &mut Cx, p: Position, allow_eol: bool) {
        if self.is_visual() {
            let doc = cx.ed.document();
            self.visual.cursor = if allow_eol {
                doc.clamp_position(p)
            } else {
                buf::clamp_normal(doc, p)
            };
            self.sync_visual_selection(cx);
        } else {
            self.set_normal_caret(cx, p);
        }
    }

    // ----- command bookkeeping -------------------------------------------------------------

    /// Drops the command being typed.
    pub(crate) fn cancel_pending(&mut self) {
        self.pending = Pending::default();
        self.visual_repeat = None;
        self.undo_hint = None;
    }

    /// The command failed: ring the bell and forget it.
    fn fail(&mut self) {
        self.bell();
        self.cancel_pending();
    }

    /// The command is complete. A repeatable one becomes the change that
    /// `.` repeats (once its Insert session ends, if it started one).
    pub(crate) fn finish_command(&mut self, cx: &mut Cx, repeatable: bool) {
        let p = std::mem::take(&mut self.pending);
        let visual = self.visual_repeat.take();
        let changed = cx.ed.revision() != self.command_revision;
        if changed && !std::mem::take(&mut self.keep_redo_carets) {
            // A new change: nothing is left to redo.
            self.redo_carets.clear();
        }
        self.keep_redo_carets = false;
        if let Some(mark) = self.undo_hint.take()
            && changed
        {
            let depth = cx.ed.undo_depth();
            self.undo_hints.retain(|(d, _)| *d < depth);
            self.undo_hints.push((depth, mark));
            if self.undo_hints.len() > 2000 {
                self.undo_hints.drain(..1000);
            }
        }
        if repeatable && !self.replaying {
            let rec = ChangeRecord {
                register: p.register,
                count: p.count_opt(),
                keys: p.keys,
                visual,
                insert: Vec::new(),
            };
            match self.insert.as_mut() {
                Some(s) => s.record = Some(rec),
                None => self.last_change = Some(rec),
            }
        }
    }

    /// Asks that `u` put the caret back at `p` (the caret as it is now)
    /// after undoing the command being executed.
    pub(crate) fn hint_undo(&mut self, cx: &Cx, p: Position) {
        self.undo_hint = Some(Mark::at(cx.ed.document(), p));
    }

    // ----- key entry -------------------------------------------------------------------------

    /// A key in Normal, Visual or operator-pending mode.
    pub(crate) fn normal_key(&mut self, key: Key, cx: &mut Cx) -> bool {
        // Whatever one key changes is one undo step. A command that starts
        // Insert mode opens its own group inside this one, so the step
        // lasts until the session ends.
        cx.ed.begin_undo_group();
        if !self.pending.active() {
            self.command_revision = cx.ed.revision();
        }
        let consumed = self.normal_key_inner(key, cx);
        cx.ed.end_undo_group();
        consumed
    }

    fn normal_key_inner(&mut self, key: Key, cx: &mut Cx) -> bool {
        if key.is_escape() {
            if self.pending.active() {
                self.cancel_pending();
                return true;
            }
            if self.is_visual() {
                self.exit_visual(cx);
                return true;
            }
            // Nothing to cancel: the host may use the key (after ctrl-o
            // from Insert mode it simply returns there).
            return self.resume.is_some();
        }
        // Left to the host: option chords, and shifted navigation keys
        // (which extend the selection; `sync` then starts Visual mode).
        let shifted_nav = key.shift
            && matches!(
                key.code,
                KeyCode::Left
                    | KeyCode::Right
                    | KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Home
                    | KeyCode::End
                    | KeyCode::PageUp
                    | KeyCode::PageDown
            );
        if key.alt || shifted_nav {
            if self.pending.active() {
                self.fail();
                return true;
            }
            return false;
        }
        if let Some(aw) = self.pending.awaiting.take() {
            self.pending.display.push_str(&key.to_string());
            if aw != Await::Register {
                self.pending.keys.push(key);
            }
            self.argument_key(aw, key, cx);
            return true;
        }
        if let Some(c) = key.plain_char()
            && c.is_ascii_digit()
            && (c != '0' || self.pending.count.is_some())
        {
            let d = c as usize - '0' as usize;
            let n = self.pending.count.unwrap_or(0).saturating_mul(10).saturating_add(d);
            self.pending.count = Some(n.min(999_999));
            self.pending.display.push(c);
            return true;
        }
        let was_active = self.pending.active();
        self.pending.display.push_str(&key.to_string());
        self.pending.keys.push(key);
        let used = self.command_key(key, cx);
        if !used {
            // Not a command of the engine. Character keys are swallowed
            // all the same (they must not be typed into the document);
            // other keys go back to the host unless a command was underway.
            let is_char = key.plain_char().is_some();
            self.fail();
            if !is_char && !was_active {
                // No bell for a key that is simply the host's.
                self.effects.retain(|e| !matches!(e, Effect::Bell));
                return false;
            }
        }
        true
    }

    /// The motion a key stands for, if any.
    fn motion_for(&self, t: Tok) -> Option<Motion> {
        Some(match t {
            Tok::Ch('h') | Tok::Named(KeyCode::Left) => Motion::Left,
            Tok::Ch('l') | Tok::Named(KeyCode::Right) => Motion::Right,
            Tok::Named(KeyCode::Backspace) | Tok::Ctrl('h') => Motion::WrapLeft,
            Tok::Ch(' ') => Motion::WrapRight,
            Tok::Ch('j') | Tok::Named(KeyCode::Down) | Tok::Ctrl('n') | Tok::Ctrl('j') => Motion::Down,
            Tok::Ch('k') | Tok::Named(KeyCode::Up) | Tok::Ctrl('p') => Motion::Up,
            Tok::Ch('+') | Tok::Named(KeyCode::Enter) | Tok::Ctrl('m') => Motion::LineDown,
            Tok::Ch('-') => Motion::LineUp,
            Tok::Ch('_') => Motion::LineHere,
            Tok::Ch('w') => Motion::WordForward { big: false },
            Tok::Ch('W') => Motion::WordForward { big: true },
            Tok::Ch('b') => Motion::WordBackward { big: false },
            Tok::Ch('B') => Motion::WordBackward { big: true },
            Tok::Ch('e') => Motion::WordEnd { big: false },
            Tok::Ch('E') => Motion::WordEnd { big: true },
            Tok::Ch('0') | Tok::Named(KeyCode::Home) => Motion::LineStart,
            Tok::Ch('^') => Motion::FirstNonBlank,
            Tok::Ch('$') | Tok::Named(KeyCode::End) => Motion::LineEnd,
            Tok::Ch('|') => Motion::Column,
            Tok::Ch('G') => Motion::LastLine,
            Tok::Ch('{') => Motion::ParagraphBackward,
            Tok::Ch('}') => Motion::ParagraphForward,
            Tok::Ch('(') => Motion::SentenceBackward,
            Tok::Ch(')') => Motion::SentenceForward,
            Tok::Ch(';') => Motion::RepeatFind { reverse: false },
            Tok::Ch(',') => Motion::RepeatFind { reverse: true },
            Tok::Ch('%') => {
                if self.pending.count_opt().is_some() {
                    Motion::Percent
                } else {
                    Motion::MatchPair
                }
            }
            Tok::Ch('H') => Motion::ViewTop,
            Tok::Ch('M') => Motion::ViewMiddle,
            Tok::Ch('L') => Motion::ViewBottom,
            Tok::Ch('n') => Motion::Search { reverse: false },
            Tok::Ch('N') => Motion::Search { reverse: true },
            _ => return None,
        })
    }

    /// A key that starts or completes a command. Returns false for a key
    /// that means nothing here.
    fn command_key(&mut self, key: Key, cx: &mut Cx) -> bool {
        let t = tok(key);
        let visual = self.is_visual();
        let op = self.pending.operator;

        // Prefixes and arguments first: they are the same in every mode.
        match t {
            Tok::Ch('"') => {
                self.pending.keys.pop();
                self.pending.awaiting = Some(Await::Register);
                return true;
            }
            Tok::Ch('g') => {
                self.pending.awaiting = Some(Await::G);
                return true;
            }
            Tok::Ch('f') | Tok::Ch('F') | Tok::Ch('t') | Tok::Ch('T') => {
                self.pending.awaiting = Some(Await::Find {
                    forward: matches!(t, Tok::Ch('f' | 't')),
                    till: matches!(t, Tok::Ch('t' | 'T')),
                });
                return true;
            }
            Tok::Ch('\'') | Tok::Ch('`') => {
                self.pending.awaiting = Some(Await::Jump {
                    exact: t == Tok::Ch('`'),
                });
                return true;
            }
            Tok::Ch('i') | Tok::Ch('a') if op.is_some() || visual => {
                self.pending.awaiting = Some(Await::Object {
                    around: t == Tok::Ch('a'),
                });
                return true;
            }
            Tok::Ch('/') | Tok::Ch('?') => {
                self.open_cmdline(
                    cx,
                    CmdKind::Search {
                        forward: t == Tok::Ch('/'),
                    },
                    "",
                );
                return true;
            }
            Tok::Ch('*') | Tok::Ch('#') => {
                self.search_word(cx, t == Tok::Ch('*'), true);
                return true;
            }
            _ => {}
        }

        // Operators.
        let operator = match t {
            Tok::Ch('d') => Some(Operator::Delete),
            Tok::Ch('c') => Some(Operator::Change),
            Tok::Ch('y') => Some(Operator::Yank),
            Tok::Ch('>') => Some(Operator::ShiftRight),
            Tok::Ch('<') => Some(Operator::ShiftLeft),
            // `g~~`, `guu`, `gUU`.
            Tok::Ch('~') if op == Some(Operator::ToggleCase) => op,
            Tok::Ch('u') if op == Some(Operator::Lower) => op,
            Tok::Ch('U') if op == Some(Operator::Upper) => op,
            _ => None,
        };
        if let Some(o) = operator {
            self.operator_key(cx, o);
            return true;
        }

        // Motions.
        if let Some(m) = self.motion_for(t) {
            self.run_motion(cx, m);
            return true;
        }

        if op.is_some() {
            // An operator is waiting and this is not a motion.
            return false;
        }
        if visual {
            return self.visual_key(t, cx);
        }
        self.plain_key(t, cx)
    }

    /// An operator key: starts operator-pending mode, or applies the
    /// operator to lines when doubled (`dd`), or to the selection.
    fn operator_key(&mut self, cx: &mut Cx, op: Operator) {
        if self.is_visual() {
            self.visual_operator(cx, op, false);
            return;
        }
        match self.pending.operator {
            None => {
                self.pending.operator = Some(op);
                self.pending.op_count = self.pending.count.take();
            }
            Some(cur) if cur == op => {
                let origin = self.cursor(cx);
                let lines = self.lines_from_cursor(cx, origin, self.pending.count1());
                self.run_operator(cx, op, Target::Lines(lines), origin, false, true);
            }
            Some(_) => self.fail(),
        }
    }

    /// Applies an operator and completes the command.
    fn run_operator(
        &mut self,
        cx: &mut Cx,
        op: Operator,
        target: Target,
        yank_caret: Position,
        big: bool,
        widen: bool,
    ) {
        let reg = self.pending.register;
        if reg.is_some_and(|r| !register::is_writable(r)) {
            self.fail();
            return;
        }
        let shift_times = if self.visual_repeat.is_some() {
            self.pending.count1()
        } else {
            1
        };
        let info = OpInfo {
            register: reg,
            big,
            yank_caret,
            shift_times,
            widen,
        };
        if op != Operator::Yank
            && let Target::Lines(lines) = &target
        {
            // Undoing a change of whole lines puts the caret back where it
            // was, if it was on the first of them (as Vim does); undoing a
            // change of text puts it at the start of that text, which the
            // editor's own undo record already gives.
            let c = cx.ed.selection().head;
            let doc = cx.ed.document();
            let first = lines.start();
            let on_first = buf::unit_at(doc, c) == buf::unit_at(doc, first);
            self.hint_undo(cx, if on_first { c } else { first });
        }
        let ok = self.apply_operator(cx, op, target, info);
        self.goal = None;
        if ok {
            self.finish_command(cx, op != Operator::Yank);
        } else {
            self.fail();
        }
    }

    /// An operator in Visual mode. `linewise` forces whole lines (`X`,
    /// `Y`, `C`, …).
    fn visual_operator(&mut self, cx: &mut Cx, op: Operator, linewise: bool) {
        let doc = cx.ed.document();
        let a = doc.clamp_position(self.visual.anchor);
        let c = doc.clamp_position(self.visual.cursor);
        let (s, e) = if a <= c { (a, c) } else { (c, a) };
        let line = linewise || matches!(self.base, Base::Visual { line: true });
        let target = if line {
            Target::Lines(buf::lines_between(doc, s, e))
        } else {
            self.visual_target(cx)
        };
        let cols = if s.block == e.block {
            buf::graphemes_between(buf::text_of(doc, s.block), s.offset, e.offset) + 1
        } else {
            let ls = buf::line_start(doc, e);
            buf::graphemes_between(buf::text_of(doc, e.block), ls, e.offset) + 1
        };
        self.visual_repeat = Some(VisualRepeat {
            line,
            blocks: e.block - s.block,
            cols,
        });
        self.save_visual(cx);
        self.base = Base::Normal;
        cx.ed.set_caret(s);
        // A linewise selection starts in column one, as in Vim.
        let start = match &target {
            Target::Lines(l) => l.start(),
            Target::Chars(_) => s,
        };
        self.run_operator(cx, op, target, start, false, false);
    }

    /// Runs a motion: moves the cursor, or hands the range to the pending
    /// operator.
    pub(crate) fn run_motion(&mut self, cx: &mut Cx, m: Motion) {
        let origin = self.cursor(cx);
        let count = self.pending.count_opt();
        let op = self.pending.operator;
        let mut result = None;
        // `cw` on a word changes to the end of the word, not up to the
        // next one.
        if op == Some(Operator::Change)
            && let Motion::WordForward { big } = m
        {
            let opts = self.class_opts(big);
            let doc = cx.ed.document();
            if buf::is_empty_line(doc, origin) {
                // Nothing to change on an empty line: just start typing.
                result = Some(MotionOut {
                    pos: origin,
                    kind: MotionKind::Exclusive,
                    goal: GoalUpdate::Clear,
                    jump: false,
                    big: false,
                    exact: false,
                });
            } else if buf::class_at(doc, origin, opts) != Class::Blank {
                let p = motion::word_end(doc, origin, count.unwrap_or(1), opts, true);
                result = Some(MotionOut {
                    pos: p,
                    kind: MotionKind::Inclusive,
                    goal: GoalUpdate::Clear,
                    jump: false,
                    big: false,
                    exact: false,
                });
            }
        }
        let out = match result {
            Some(o) => o,
            None => match self.compute_motion(cx, m, count, origin, op.is_some()) {
                Some(o) => o,
                None => {
                    self.fail();
                    return;
                }
            },
        };
        if let Some(op) = op {
            let target = self.motion_target(cx, origin, &out);
            // A yank leaves the caret at the start of what was yanked.
            let yank_caret = match &target {
                Target::Chars(r) => r.start,
                Target::Lines(_) => origin.min(out.pos),
            };
            self.run_operator(cx, op, target, yank_caret, out.big, true);
            return;
        }
        if out.jump && out.pos != origin {
            self.push_jump(cx, origin);
        }
        let before = self.cursor(cx);
        self.move_cursor_eol(cx, out.pos, m == Motion::LineEnd);
        match out.goal {
            GoalUpdate::Keep => {}
            GoalUpdate::Clear => self.goal = None,
            GoalUpdate::End => self.goal = Some(GoalColumn::EndOfRow),
        }
        if self.cursor(cx) == before && !matches!(m, Motion::LineStart | Motion::FirstNonBlank | Motion::LineEnd | Motion::LineHere | Motion::Column | Motion::RowStart | Motion::RowFirstNonBlank | Motion::RowEnd | Motion::LastNonBlank | Motion::FirstLine | Motion::LastLine | Motion::Percent) {
            self.bell();
        }
        self.finish_command(cx, false);
    }

    /// The word a `*` at `at` searches for: the keyword under or after the
    /// caret on its line, else the run of other non-blank characters.
    /// Returns the text, whether it is a keyword, and where it starts.
    pub(crate) fn word_under(&self, cx: &Cx, at: Position, big: bool) -> Option<(String, bool, Position)> {
        let doc = cx.ed.document();
        let opts = self.class_opts(big);
        let t = buf::text_of(doc, at.block);
        let ls = buf::line_start(doc, at);
        let le = buf::line_end(doc, at);
        let class = |o: usize| buf::class_at(doc, Position::new(at.block, o), opts);
        let is_keyword = |c: Class| !matches!(c, Class::Blank | Class::Punct);
        let find_from = |want_keyword: bool| {
            let mut o = at.offset.min(le);
            while o < le {
                let c = class(o);
                if (want_keyword && is_keyword(c)) || (!want_keyword && c != Class::Blank) {
                    return Some(o);
                }
                o = buf::next_boundary(t, o);
            }
            None
        };
        let (start, keyword) = match find_from(true) {
            Some(o) => (o, true),
            None => (find_from(false)?, false),
        };
        let c = class(start);
        let mut s = start;
        while s > ls {
            let q = buf::prev_boundary(t, s);
            if class(q) != c {
                break;
            }
            s = q;
        }
        let mut e = start;
        while e < le && class(e) == c {
            e = buf::next_boundary(t, e);
        }
        Some((t[s..e].to_string(), keyword, Position::new(at.block, s)))
    }

    /// `*`, `#`, `g*`, `g#`: searches for the word under the caret,
    /// starting from the start of that word.
    fn search_word(&mut self, cx: &mut Cx, forward: bool, whole: bool) {
        let at = self.cursor(cx);
        let Some((word, keyword, start)) = self.word_under(cx, at, false) else {
            self.error("E348: No string under cursor");
            self.fail();
            return;
        };
        let whole = whole && keyword;
        let raw = search::escape_literal(&word, whole);
        let pattern = search::literal_pattern(&word, whole);
        self.set_search(pattern.clone(), raw, forward);
        if self.pending.operator.is_some() {
            self.run_motion(cx, Motion::Search { reverse: false });
            return;
        }
        let mut p = start;
        let mut wrapped = false;
        for _ in 0..self.pending.count1() {
            let Some(found) = search::find(cx.ed.document(), &pattern, p, forward) else {
                break;
            };
            wrapped |= found.wrapped;
            p = found.range.start;
        }
        if wrapped {
            self.message(if forward {
                "search hit BOTTOM, continuing at TOP"
            } else {
                "search hit TOP, continuing at BOTTOM"
            });
        }
        if p != at {
            self.push_jump(cx, at);
        }
        self.move_cursor(cx, p);
        self.goal = None;
        self.finish_command(cx, false);
    }

    // ----- arguments ------------------------------------------------------------------------

    /// The key after a prefix (`g`, `z`, `"`, `f`, `r`, `m`, `'`, `i`, …).
    fn argument_key(&mut self, aw: Await, key: Key, cx: &mut Cx) {
        let t = tok(key);
        let visual = self.is_visual();
        let op = self.pending.operator;
        match aw {
            Await::Register => match key.plain_char() {
                Some(c) if register::is_register_name(c) => self.pending.register = Some(c),
                _ => self.fail(),
            },
            Await::Find { forward, till } => {
                let ch = match t {
                    Tok::Ch(c) => c,
                    Tok::Named(KeyCode::Tab) => '\t',
                    _ => {
                        self.fail();
                        return;
                    }
                };
                let spec = FindSpec { ch, forward, till };
                self.last_find = Some(spec);
                self.run_motion(cx, Motion::Find(spec));
            }
            Await::Jump { exact } => {
                let Tok::Ch(c) = t else {
                    self.fail();
                    return;
                };
                self.run_motion(cx, if exact { Motion::MarkExact(c) } else { Motion::MarkLine(c) });
            }
            Await::SetMark => {
                let Tok::Ch(c) = t else {
                    self.fail();
                    return;
                };
                if c.is_ascii_alphabetic() || matches!(c, '\'' | '`' | '[' | ']' | '<' | '>') {
                    let p = self.cursor(cx);
                    let name = if c == '`' { '\'' } else { c };
                    self.set_mark(cx, name, p);
                    self.finish_command(cx, false);
                } else {
                    self.fail();
                }
            }
            Await::Replace => {
                let ch = match t {
                    Tok::Ch(c) => Some(c),
                    Tok::Named(KeyCode::Enter) => None,
                    Tok::Named(KeyCode::Tab) => Some('\t'),
                    _ => {
                        self.fail();
                        return;
                    }
                };
                if visual {
                    let Some(c) = ch else {
                        self.fail();
                        return;
                    };
                    let target = self.visual_target(cx);
                    let start = match &target {
                        Target::Chars(r) => r.start,
                        Target::Lines(l) => l.start(),
                    };
                    self.visual_operator_prepare(cx);
                    cx.ed.set_caret(start);
                    self.replace_in_target(cx, &target, c);
                    self.set_normal_caret(cx, start);
                    self.finish_command(cx, true);
                } else {
                    let n = self.pending.count1();
                    if self.replace_chars(cx, n, ch) {
                        self.finish_command(cx, true);
                    } else {
                        self.fail();
                    }
                }
            }
            Await::Object { around } => {
                let Tok::Ch(c) = t else {
                    self.fail();
                    return;
                };
                self.text_object(cx, c, around);
            }
            Await::BigZ => match t {
                Tok::Ch('Z') => {
                    self.effects.push(Effect::Request(HostRequest::SaveAndClose));
                    self.finish_command(cx, false);
                }
                Tok::Ch('Q') => {
                    self.effects.push(Effect::Request(HostRequest::Close { force: true }));
                    self.finish_command(cx, false);
                }
                _ => self.fail(),
            },
            Await::Z => {
                let cur = self.cursor(cx);
                let align = match t {
                    Tok::Ch('z') | Tok::Ch('.') => ScrollAlign::Center,
                    Tok::Ch('t') | Tok::Named(KeyCode::Enter) => ScrollAlign::Top,
                    Tok::Ch('b') | Tok::Ch('-') => ScrollAlign::Bottom,
                    _ => {
                        self.fail();
                        return;
                    }
                };
                cx.host.scroll(cx.ed, Scroll::Align { caret: cur, align });
                if matches!(t, Tok::Ch('.') | Tok::Ch('-') | Tok::Named(KeyCode::Enter)) {
                    let doc = cx.ed.document();
                    let ls = buf::line_start(doc, cur);
                    let le = buf::line_end(doc, cur);
                    let o = buf::first_non_blank(doc, cur.block, ls).min(le);
                    self.move_cursor(cx, Position::new(cur.block, o));
                }
                self.finish_command(cx, false);
            }
            Await::G => match t {
                Tok::Ch('g') => self.run_motion(cx, Motion::FirstLine),
                Tok::Ch('e') => self.run_motion(cx, Motion::WordEndBackward { big: false }),
                Tok::Ch('E') => self.run_motion(cx, Motion::WordEndBackward { big: true }),
                Tok::Ch('j') | Tok::Named(KeyCode::Down) => self.run_motion(cx, Motion::Down),
                Tok::Ch('k') | Tok::Named(KeyCode::Up) => self.run_motion(cx, Motion::Up),
                Tok::Ch('0') | Tok::Named(KeyCode::Home) => self.run_motion(cx, Motion::RowStart),
                Tok::Ch('^') => self.run_motion(cx, Motion::RowFirstNonBlank),
                Tok::Ch('$') | Tok::Named(KeyCode::End) => self.run_motion(cx, Motion::RowEnd),
                Tok::Ch('_') => self.run_motion(cx, Motion::LastNonBlank),
                Tok::Ch('*') => self.search_word(cx, true, false),
                Tok::Ch('#') => self.search_word(cx, false, false),
                Tok::Ch('u') => self.operator_key(cx, Operator::Lower),
                Tok::Ch('U') => self.operator_key(cx, Operator::Upper),
                Tok::Ch('~') => self.operator_key(cx, Operator::ToggleCase),
                Tok::Ch('v') if op.is_none() => {
                    self.reselect_visual(cx);
                    self.finish_command(cx, false);
                }
                Tok::Ch('J') if op.is_none() => self.join_command(cx, false),
                Tok::Ch('i') if op.is_none() && !visual => {
                    if let Some(p) = self.mark_position(cx, '^') {
                        cx.ed.set_caret(p);
                    }
                    let n = self.pending.count1();
                    self.begin_insert(cx, false, n, None, false);
                    self.finish_command(cx, true);
                }
                Tok::Ch('I') if op.is_none() && !visual => {
                    let cur = self.cursor(cx);
                    let ls = buf::line_start(cx.ed.document(), cur);
                    cx.ed.set_caret(Position::new(cur.block, ls));
                    let n = self.pending.count1();
                    self.begin_insert(cx, false, n, None, false);
                    self.finish_command(cx, true);
                }
                Tok::Ch('p') | Tok::Ch('P') if op.is_none() && !visual => {
                    self.put_command(cx, t == Tok::Ch('P'), true);
                }
                Tok::Ch('&') if op.is_none() && !visual => {
                    self.run_ex("%&&", cx);
                    self.finish_command(cx, false);
                }
                _ => self.fail(),
            },
        }
    }

    /// `iw`, `a"`, `ip`, … after an operator or in Visual mode.
    fn text_object(&mut self, cx: &mut Cx, c: char, around: bool) {
        let visual = self.is_visual();
        let mut at = self.cursor(cx);
        let count = self.pending.count1();
        let doc = cx.ed.document();
        // With something already selected, a word, sentence or paragraph
        // object adds the next one.
        let extending = visual
            && self.visual.anchor < self.visual.cursor
            && matches!(c, 'w' | 'W' | 's' | 'p');
        if extending {
            let next = if c == 'p' {
                (at.block + 1 < doc.block_count()).then(|| Position::new(at.block + 1, 0))
            } else {
                buf::next_cell(doc, at).filter(|n| n.block == at.block && !buf::is_newline(doc, *n))
            };
            match next {
                Some(n) => at = n,
                None => {
                    self.fail();
                    return;
                }
            }
        }
        let brackets = |c: char| match c {
            '(' | ')' | 'b' => Some(('(', ')')),
            '[' | ']' => Some(('[', ']')),
            '{' | '}' | 'B' => Some(('{', '}')),
            '<' | '>' => Some(('<', '>')),
            _ => None,
        };
        let mut target = match c {
            'w' => textobj::word(doc, at, around, count, self.class_opts(false)).map(Target::Chars),
            'W' => textobj::word(doc, at, around, count, self.class_opts(true)).map(Target::Chars),
            's' => textobj::sentence(doc, at, around, count).map(Target::Chars),
            'p' => textobj::paragraph(doc, at, around, count).map(Target::Lines),
            '"' | '\'' | '`' => textobj::quote(doc, at, c, around).map(Target::Chars),
            _ => match brackets(c) {
                Some((o, cl)) => textobj::bracket(doc, at, o, cl, around, count).map(Target::Chars),
                None => None,
            },
        };
        if visual
            && let Some((o, cl)) = brackets(c)
            && let Target::Chars(current) = self.visual_target(cx)
            && self.visual.anchor != self.visual.cursor
            && let Some(Target::Chars(found)) = &target
            && found.start >= current.start
            && found.end <= current.end
        {
            // The selection already covers that pair: take the next one out.
            target = textobj::bracket_growing(doc, current, o, cl, around).map(Target::Chars);
        }
        let Some(target) = target else {
            self.fail();
            return;
        };
        if let Some(op) = self.pending.operator {
            let start = match &target {
                Target::Chars(r) => r.start,
                Target::Lines(l) => l.start(),
            };
            self.run_operator(cx, op, target, start, false, false);
            return;
        }
        // Visual mode: select the object.
        match target {
            Target::Chars(r) => {
                if r.is_empty() {
                    self.fail();
                    return;
                }
                let last = buf::prev_cell(doc, r.end).unwrap_or(r.start).max(r.start);
                if !extending {
                    self.visual.anchor = r.start;
                }
                self.visual.cursor = last;
                if matches!(self.base, Base::Visual { line: true }) {
                    self.base = Base::Visual { line: false };
                }
            }
            Target::Lines(l) => {
                let (s, e) = match l {
                    Lines::Blocks(r) => (Position::new(r.start, 0), Position::new(r.end - 1, 0)),
                    Lines::Code { block, start, end } => (Position::new(block, start), Position::new(block, end)),
                };
                if !extending {
                    self.visual.anchor = s;
                }
                self.visual.cursor = e;
                self.base = Base::Visual { line: true };
            }
        }
        self.sync_visual_selection(cx);
        self.finish_command(cx, false);
    }

    /// `gv`: selects the last Visual selection again (in Visual mode: swaps
    /// with it).
    fn reselect_visual(&mut self, cx: &mut Cx) {
        let Some(saved) = self.last_visual else {
            self.bell();
            return;
        };
        let doc = cx.ed.document();
        let (Some(a), Some(c)) = (saved.anchor.resolve(doc), saved.cursor.resolve(doc)) else {
            self.bell();
            return;
        };
        if self.is_visual() {
            self.save_visual(cx);
        }
        self.visual = VisualState { anchor: a, cursor: c };
        self.base = Base::Visual { line: saved.line };
        self.sync_visual_selection(cx);
    }

    /// Bookkeeping shared by Visual-mode commands that are not operators:
    /// remembers the selection for `.` and `gv` and returns to Normal mode.
    fn visual_operator_prepare(&mut self, cx: &mut Cx) {
        let doc = cx.ed.document();
        let a = doc.clamp_position(self.visual.anchor);
        let c = doc.clamp_position(self.visual.cursor);
        let (s, e) = if a <= c { (a, c) } else { (c, a) };
        let line = matches!(self.base, Base::Visual { line: true });
        let cols = if s.block == e.block {
            buf::graphemes_between(buf::text_of(doc, s.block), s.offset, e.offset) + 1
        } else {
            let ls = buf::line_start(doc, e);
            buf::graphemes_between(buf::text_of(doc, e.block), ls, e.offset) + 1
        };
        self.visual_repeat = Some(VisualRepeat {
            line,
            blocks: e.block - s.block,
            cols,
        });
        self.save_visual(cx);
        self.base = Base::Normal;
    }

    /// Rebuilds a Visual selection of the recorded size at the caret, for
    /// `.` after a Visual-mode command.
    fn visual_for_repeat(&mut self, cx: &mut Cx, v: VisualRepeat) {
        let start = cx.ed.selection().head;
        let doc = cx.ed.document();
        let last_block = (start.block + v.blocks).min(doc.block_count() - 1);
        let cursor = if v.line {
            Position::new(last_block, 0)
        } else if last_block == start.block {
            let t = buf::text_of(doc, start.block);
            let le = buf::line_end(doc, start);
            let o = buf::advance_graphemes(t, start.offset, v.cols.saturating_sub(1), le);
            buf::clamp_normal(doc, Position::new(start.block, o))
        } else {
            let t = buf::text_of(doc, last_block);
            let le = buf::line_end(doc, Position::new(last_block, 0));
            let o = buf::advance_graphemes(t, 0, v.cols.saturating_sub(1), le);
            buf::clamp_normal(doc, Position::new(last_block, o))
        };
        self.visual = VisualState { anchor: start, cursor };
        self.base = Base::Visual { line: v.line };
        self.sync_visual_selection(cx);
    }

    // ----- Visual-mode keys ---------------------------------------------------------------

    fn visual_key(&mut self, t: Tok, cx: &mut Cx) -> bool {
        match t {
            Tok::Ch('x') | Tok::Named(KeyCode::Delete) => self.visual_operator(cx, Operator::Delete, false),
            Tok::Ch('X') | Tok::Ch('D') => self.visual_operator(cx, Operator::Delete, true),
            Tok::Ch('s') => self.visual_operator(cx, Operator::Change, false),
            Tok::Ch('C') | Tok::Ch('S') | Tok::Ch('R') => self.visual_operator(cx, Operator::Change, true),
            Tok::Ch('Y') => self.visual_operator(cx, Operator::Yank, true),
            Tok::Ch('u') => self.visual_operator(cx, Operator::Lower, false),
            Tok::Ch('U') => self.visual_operator(cx, Operator::Upper, false),
            Tok::Ch('~') => self.visual_operator(cx, Operator::ToggleCase, false),
            Tok::Ch('J') => self.join_command(cx, true),
            Tok::Ch('p') | Tok::Ch('P') => {
                let reg = self.pending.register;
                let Some(content) = self.read_register(cx, reg) else {
                    self.fail();
                    return true;
                };
                let target = self.visual_target(cx);
                let n = self.pending.count1();
                self.visual_operator_prepare(cx);
                let c = cx.ed.selection().start();
                cx.ed.set_caret(c);
                self.hint_undo(cx, c);
                self.put_over(cx, target, &content, n, t == Tok::Ch('P'));
                self.finish_command(cx, true);
            }
            Tok::Ch('r') => self.pending.awaiting = Some(Await::Replace),
            Tok::Ch('o') | Tok::Ch('O') => {
                std::mem::swap(&mut self.visual.anchor, &mut self.visual.cursor);
                self.sync_visual_selection(cx);
                self.finish_command(cx, false);
            }
            Tok::Ch('v') | Tok::Ch('V') => {
                let want_line = t == Tok::Ch('V');
                if self.base == (Base::Visual { line: want_line }) {
                    self.exit_visual(cx);
                } else {
                    self.base = Base::Visual { line: want_line };
                    if !want_line {
                        // Back on a character.
                        let c = buf::clamp_normal(cx.ed.document(), self.visual.cursor);
                        self.visual.cursor = c;
                    }
                    self.sync_visual_selection(cx);
                }
                self.finish_command(cx, false);
            }
            Tok::Ch(':') => {
                self.save_visual(cx);
                self.open_cmdline(cx, CmdKind::Ex, "'<,'>");
            }
            Tok::Ch('m') => self.pending.awaiting = Some(Await::SetMark),
            Tok::Ch('z') => self.pending.awaiting = Some(Await::Z),
            Tok::Ctrl('d' | 'u' | 'f' | 'b' | 'e' | 'y')
            | Tok::Named(KeyCode::PageUp | KeyCode::PageDown) => self.scroll_command(cx, t),
            _ => return false,
        }
        true
    }

    // ----- Normal-mode keys ---------------------------------------------------------------

    fn plain_key(&mut self, t: Tok, cx: &mut Cx) -> bool {
        let n = self.pending.count1();
        let cur = self.cursor(cx);
        match t {
            Tok::Ch('x') | Tok::Named(KeyCode::Delete) | Tok::Ch('X') => {
                let doc = cx.ed.document();
                let text = buf::text_of(doc, cur.block);
                let ls = buf::line_start(doc, cur);
                let le = buf::line_end(doc, cur);
                let range = if t == Tok::Ch('X') {
                    let mut o = cur.offset;
                    for _ in 0..n {
                        if o <= ls {
                            break;
                        }
                        o = buf::prev_boundary(text, o);
                    }
                    TextRange::in_block(cur.block, o, cur.offset)
                } else {
                    let o = buf::advance_graphemes(text, cur.offset, n, le);
                    TextRange::in_block(cur.block, cur.offset, o)
                };
                if range.is_empty() {
                    self.fail();
                    return true;
                }
                self.run_operator(cx, Operator::Delete, Target::Chars(range), range.start, false, false);
            }
            Tok::Ch('s') => {
                let doc = cx.ed.document();
                let le = buf::line_end(doc, cur);
                let o = buf::advance_graphemes(buf::text_of(doc, cur.block), cur.offset, n, le);
                let range = TextRange::in_block(cur.block, cur.offset, o);
                self.run_operator(cx, Operator::Change, Target::Chars(range), cur, false, false);
            }
            Tok::Ch('S') => {
                let lines = self.lines_from_cursor(cx, cur, n);
                self.run_operator(cx, Operator::Change, Target::Lines(lines), cur, false, false);
            }
            Tok::Ch('D') | Tok::Ch('C') => {
                // To the end of the line, and `count - 1` more lines.
                let Some(m) = self.compute_motion(cx, Motion::LineEnd, Some(n), cur, true) else {
                    self.fail();
                    return true;
                };
                let target = Target::Chars(TextRange::new(cur, m.pos));
                let op = if t == Tok::Ch('D') { Operator::Delete } else { Operator::Change };
                self.run_operator(cx, op, target, cur, false, false);
            }
            Tok::Ch('Y') => {
                let lines = self.lines_from_cursor(cx, cur, n);
                self.run_operator(cx, Operator::Yank, Target::Lines(lines), cur, false, false);
            }
            Tok::Ch('r') => self.pending.awaiting = Some(Await::Replace),
            Tok::Ch('R') => {
                cx.ed.set_caret(cur);
                self.begin_insert(cx, true, n, None, false);
                self.finish_command(cx, true);
            }
            Tok::Ch('~') => {
                if self.tilde(cx, n) {
                    self.finish_command(cx, true);
                } else {
                    self.fail();
                }
            }
            Tok::Ch('J') => self.join_command(cx, true),
            Tok::Ch('p') | Tok::Ch('P') => self.put_command(cx, t == Tok::Ch('P'), false),
            Tok::Ch('u') => {
                self.undo(cx, n);
                self.finish_command(cx, false);
            }
            Tok::Ctrl('r') => {
                self.redo(cx, n);
                self.finish_command(cx, false);
            }
            Tok::Ch('.') => self.repeat_change(cx),
            Tok::Ch('i') | Tok::Ch('a') | Tok::Ch('I') | Tok::Ch('A') => {
                let doc = cx.ed.document();
                let ls = buf::line_start(doc, cur);
                let le = buf::line_end(doc, cur);
                let at = match t {
                    Tok::Ch('i') => cur,
                    Tok::Ch('a') => buf::cell_end(doc, cur),
                    Tok::Ch('I') => Position::new(cur.block, buf::first_non_blank(doc, cur.block, ls).min(le)),
                    _ => Position::new(cur.block, le),
                };
                cx.ed.set_caret(at);
                self.begin_insert(cx, false, n, None, false);
                self.finish_command(cx, true);
            }
            Tok::Ch('o') | Tok::Ch('O') => {
                let below = t == Tok::Ch('o');
                cx.ed.set_caret(cur);
                self.begin_insert(cx, false, n, Some(below), false);
                self.open_line(cx, below);
                self.finish_command(cx, true);
            }
            Tok::Ch('v') | Tok::Ch('V') => {
                self.enter_visual(cx, t == Tok::Ch('V'));
                if n > 1 {
                    // `3v` selects three characters, `3V` three lines.
                    if t == Tok::Ch('V') {
                        let doc = cx.ed.document();
                        let u = buf::unit_offset(doc, buf::unit_at(doc, cur), n as isize - 1);
                        self.visual.cursor = Position::new(u.block, u.start);
                    } else {
                        let doc = cx.ed.document();
                        let le = buf::line_end(doc, cur);
                        let o = buf::advance_graphemes(buf::text_of(doc, cur.block), cur.offset, n - 1, le);
                        self.visual.cursor = buf::clamp_normal(doc, Position::new(cur.block, o));
                    }
                    self.sync_visual_selection(cx);
                }
                self.finish_command(cx, false);
            }
            Tok::Ch('&') => {
                self.run_ex("&", cx);
                self.finish_command(cx, true);
            }
            Tok::Ch(':') => {
                let initial = if self.pending.count.is_some() {
                    // `3:` starts with a range of three lines.
                    if n > 1 { format!(".,.+{}", n - 1) } else { ".".to_string() }
                } else {
                    String::new()
                };
                self.pending = Pending::default();
                self.open_cmdline(cx, CmdKind::Ex, &initial);
            }
            Tok::Ch('m') => self.pending.awaiting = Some(Await::SetMark),
            Tok::Ch('z') => self.pending.awaiting = Some(Await::Z),
            Tok::Ch('Z') => self.pending.awaiting = Some(Await::BigZ),
            Tok::Ctrl('o') => {
                let doc = cx.ed.document();
                match self.jumps.back(doc, cur, n) {
                    Some(p) => {
                        self.set_normal_caret(cx, p);
                        self.goal = None;
                        self.finish_command(cx, false);
                    }
                    None => self.fail(),
                }
            }
            Tok::Ctrl('i') | Tok::Named(KeyCode::Tab) => {
                let doc = cx.ed.document();
                match self.jumps.forward(doc, n) {
                    Some(p) => {
                        self.set_normal_caret(cx, p);
                        self.goal = None;
                        self.finish_command(cx, false);
                    }
                    None => self.fail(),
                }
            }
            Tok::Ctrl('d' | 'u' | 'f' | 'b' | 'e' | 'y')
            | Tok::Named(KeyCode::PageUp | KeyCode::PageDown) => self.scroll_command(cx, t),
            Tok::Ctrl('l') => {
                self.search.highlight = false;
                self.finish_command(cx, false);
            }
            Tok::Ctrl('g') => {
                let doc = cx.ed.document();
                let blocks = doc.block_count();
                let col = buf::graphemes_between(
                    buf::text_of(doc, cur.block),
                    buf::line_start(doc, cur),
                    cur.offset,
                ) + 1;
                self.message(format!(
                    "line {} of {} --{}%-- col {}",
                    cur.block + 1,
                    blocks,
                    (cur.block + 1) * 100 / blocks,
                    col
                ));
                self.finish_command(cx, false);
            }
            _ => return false,
        }
        true
    }

    /// `J` and `gJ`, in Normal and Visual mode.
    fn join_command(&mut self, cx: &mut Cx, spaces: bool) {
        let (origin, count) = if self.is_visual() {
            let doc = cx.ed.document();
            let a = doc.clamp_position(self.visual.anchor);
            let c = doc.clamp_position(self.visual.cursor);
            let (s, e) = if a <= c { (a, c) } else { (c, a) };
            let lines = buf::lines_between(doc, s, e).count(doc);
            self.visual_operator_prepare(cx);
            cx.ed.set_caret(s);
            (s, lines.max(2))
        } else {
            (self.cursor(cx), self.pending.count1())
        };
        self.hint_undo(cx, origin);
        if self.join_lines(cx, origin, count, spaces) {
            self.finish_command(cx, true);
        } else {
            self.set_normal_caret(cx, origin);
            self.fail();
        }
    }

    /// `p`, `P`, `gp`, `gP` in Normal mode.
    fn put_command(&mut self, cx: &mut Cx, before: bool, caret_after: bool) {
        let reg = self.pending.register;
        let Some(content) = self.read_register(cx, reg) else {
            self.fail();
            return;
        };
        let n = self.pending.count1();
        let c = cx.ed.selection().head;
        self.hint_undo(cx, c);
        self.put(cx, &content, before, n, caret_after);
        self.goal = None;
        self.finish_command(cx, true);
    }

    /// `u`. The caret goes to where the change began: the selection the
    /// editor recorded before it, or the caret the command asked to be
    /// remembered when that lies in the restored blocks.
    pub(crate) fn undo(&mut self, cx: &mut Cx, count: usize) {
        let mut caret = None;
        for _ in 0..count.max(1) {
            let depth = cx.ed.undo_depth();
            while self.undo_hints.last().is_some_and(|(d, _)| *d > depth) {
                self.undo_hints.pop();
            }
            let hint = match self.undo_hints.last() {
                Some((d, _)) if *d == depth => self.undo_hints.pop().map(|(_, m)| m),
                _ => None,
            };
            if !cx.ed.undo() {
                break;
            }
            let mut p = cx.ed.selection().start();
            if let Some(m) = hint
                && let Some(q) = m.resolve(cx.ed.document())
                && let Some(splice) = cx.ed.last_change().and_then(|c| c.blocks)
                && q.block >= splice.start
                && q.block < splice.start + splice.new_len.max(1)
            {
                p = q;
            }
            let p = buf::clamp_normal(cx.ed.document(), p);
            // Redo comes back to the same place, as in Vim.
            self.redo_carets.push(Mark::at(cx.ed.document(), p));
            caret = Some(p);
        }
        self.keep_redo_carets = true;
        let Some(p) = caret else {
            self.message("Already at oldest change");
            self.bell();
            return;
        };
        self.set_normal_caret(cx, p);
        self.goal = None;
    }

    /// `ctrl-r`.
    pub(crate) fn redo(&mut self, cx: &mut Cx, count: usize) {
        let mut caret = None;
        for _ in 0..count.max(1) {
            if !cx.ed.redo() {
                break;
            }
            let mut p = cx.ed.selection().start();
            if let Some(m) = self.redo_carets.pop() {
                // The next undo of this step returns to the same caret.
                self.undo_hints.push((cx.ed.undo_depth(), m));
                if let Some(q) = m.resolve(cx.ed.document())
                    && let Some(splice) = cx.ed.last_change().and_then(|c| c.blocks)
                    && q.block >= splice.start
                    && q.block < splice.start + splice.new_len.max(1)
                {
                    p = q;
                }
            }
            caret = Some(p);
        }
        self.keep_redo_carets = true;
        let Some(p) = caret else {
            self.message("Already at newest change");
            self.bell();
            return;
        };
        self.set_normal_caret(cx, p);
        self.goal = None;
    }

    /// `.`: does the last change again, with a new count if one is given.
    fn repeat_change(&mut self, cx: &mut Cx) {
        let Some(mut rec) = self.last_change.clone() else {
            self.fail();
            return;
        };
        if let Some(n) = self.pending.count_opt() {
            rec.count = Some(n);
        }
        self.pending = Pending::default();
        self.replaying = true;
        if let Some(v) = rec.visual {
            self.visual_for_repeat(cx, v);
        }
        self.pending.register = rec.register;
        self.pending.count = rec.count;
        for k in &rec.keys {
            self.dispatch(*k, cx);
        }
        if matches!(self.base, Base::Insert | Base::Replace) {
            let replace = self.base == Base::Replace;
            if let Some(s) = self.insert.as_mut() {
                s.actions = rec.insert.clone();
            }
            self.apply_insert_actions(cx, &rec.insert, replace);
            self.leave_insert(cx);
        }
        // Anything left half-typed (a failed command) is dropped.
        self.cmdline = None;
        if self.is_visual() {
            self.exit_visual(cx);
        }
        self.pending = Pending::default();
        self.replaying = false;
        // The new count sticks for the next repeat.
        if let Some(last) = self.last_change.as_mut() {
            last.count = rec.count;
        }
    }

    /// The scroll commands.
    fn scroll_command(&mut self, cx: &mut Cx, t: Tok) {
        let counted = self.pending.count_opt();
        let cur = self.cursor(cx);
        let doc = cx.ed.document();
        let vp = cx.host.viewport(cx.ed);
        let page = vp.map_or(20, |v| v.rows.max(1));
        let (rows, with_caret): (isize, bool) = match t {
            Tok::Ctrl('e') => (counted.unwrap_or(1) as isize, false),
            Tok::Ctrl('y') => (-(counted.unwrap_or(1) as isize), false),
            Tok::Ctrl('d') => (counted.unwrap_or((page / 2).max(1)) as isize, true),
            Tok::Ctrl('u') => (-(counted.unwrap_or((page / 2).max(1)) as isize), true),
            Tok::Ctrl('f') | Tok::Named(KeyCode::PageDown) => {
                ((page.saturating_sub(2).max(1) * counted.unwrap_or(1)) as isize, true)
            }
            _ => (-((page.saturating_sub(2).max(1) * counted.unwrap_or(1)) as isize), true),
        };
        cx.host.scroll(cx.ed, Scroll::Rows(rows));
        if with_caret {
            // The caret travels with the text.
            match self.move_rows(cx, cur, rows) {
                Some(p) => self.move_cursor(cx, p),
                None => self.bell(),
            }
        } else if let Some(vp) = vp {
            // The caret stays unless it would leave the screen.
            let goal = match self.goal {
                Some(g) => g,
                None => GoalColumn::X(cx.host.column_of(cx.ed, cur)),
            };
            let x0 = GoalColumn::X(0.0);
            // Where the viewport will be: its top moves, its height stays
            // (so a scroll that stops at the document start changes nothing).
            let top = doc.clamp_position(cx.host.move_by_rows(cx.ed, vp.top, rows, x0));
            let bottom = doc.clamp_position(cx.host.move_by_rows(
                cx.ed,
                top,
                vp.rows.max(1) as isize - 1,
                x0,
            ));
            let top_row = cx.host.visual_row(cx.ed, top);
            let bottom_row = cx.host.visual_row(cx.ed, bottom);
            let above = cur.block < top.block || (cur.block == top.block && cur.offset < top_row.start);
            let below = cur.block > bottom.block
                || (cur.block == bottom.block
                    && if bottom_row.is_empty() {
                        cur.offset > bottom_row.start
                    } else {
                        cur.offset >= bottom_row.end
                    });
            if above {
                let p = cx.host.move_by_rows(cx.ed, top, 0, goal);
                self.goal = Some(goal);
                self.move_cursor(cx, p);
            } else if below {
                let p = cx.host.move_by_rows(cx.ed, bottom, 0, goal);
                self.goal = Some(goal);
                self.move_cursor(cx, p);
            }
        }
        self.finish_command(cx, false);
    }
}
