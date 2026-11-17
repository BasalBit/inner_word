//! The command line: `:` commands and `/` `?` searches.

use std::ops::Range;

use innerword_core::{Position, SearchOptions, TextRange};

use crate::buf::{self, Lines};
use crate::effect::{CommandLine, Effect, HostRequest, SearchPattern};
use crate::ex::{self, Addr, AddrBase, ExCommand, RangeSpec, RepPiece, SubstArgs};
use crate::key::{Key, KeyCode};
use crate::search;
use crate::textobj::Target;

use super::motions::Motion;
use super::operators::OpInfo;
use super::{Base, Cx, Operator, Vim};

/// What the command line is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CmdKind {
    Ex,
    Search { forward: bool },
}

/// The command line while it is open.
#[derive(Clone, Debug)]
pub(crate) struct CmdLineState {
    pub kind: CmdKind,
    pub text: String,
    /// Byte offset of the cursor in `text`.
    pub cursor: usize,
    /// Position in the history while browsing it with up / down.
    history_index: Option<usize>,
    /// What was typed before browsing the history.
    typed: String,
    /// The cursor when the command line was opened (a search starts here,
    /// and escape returns here after an incremental search moved away).
    pub origin: Position,
    /// The Visual selection when it was opened.
    origin_visual: Option<super::VisualState>,
    /// `ctrl-r` was pressed; the next key names a register.
    await_register: bool,
    /// Whether search highlights were on when it was opened (an abandoned
    /// search puts them back as they were).
    saved_highlight: bool,
}

impl CmdLineState {
    pub fn view(&self) -> CommandLine {
        CommandLine {
            prefix: match self.kind {
                CmdKind::Ex => ':',
                CmdKind::Search { forward: true } => '/',
                CmdKind::Search { forward: false } => '?',
            },
            text: self.text.clone(),
            cursor: self.cursor,
        }
    }

    fn prev_char(&self) -> usize {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next_char(&self) -> usize {
        self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.cursor, |c| self.cursor + c.len_utf8())
    }
}

impl Vim {
    /// Opens the command line.
    pub(crate) fn open_cmdline(&mut self, cx: &mut Cx, kind: CmdKind, initial: &str) {
        let origin = self.cursor(cx);
        self.cmdline = Some(CmdLineState {
            kind,
            text: initial.to_string(),
            cursor: initial.len(),
            history_index: None,
            typed: String::new(),
            origin,
            origin_visual: self.is_visual().then_some(self.visual),
            await_register: false,
            saved_highlight: self.search.highlight,
        });
    }

    fn history(&self, kind: CmdKind) -> &Vec<String> {
        match kind {
            CmdKind::Ex => &self.ex_history,
            CmdKind::Search { .. } => &self.search.history,
        }
    }

    fn push_history(&mut self, kind: CmdKind, entry: &str) {
        if entry.is_empty() {
            return;
        }
        let h = match kind {
            CmdKind::Ex => &mut self.ex_history,
            CmdKind::Search { .. } => &mut self.search.history,
        };
        h.retain(|e| e != entry);
        h.push(entry.to_string());
        if h.len() > 100 {
            h.remove(0);
        }
    }

    /// Inserts text at the command-line cursor.
    pub(crate) fn cmdline_insert(&mut self, s: &str, cx: &mut Cx) {
        if let Some(c) = self.cmdline.as_mut() {
            // The command line is one line.
            let clean: String = s.chars().filter(|ch| !ch.is_control()).collect();
            c.text.insert_str(c.cursor, &clean);
            c.cursor += clean.len();
            c.history_index = None;
        }
        self.search_preview(cx);
    }

    /// Restores the cursor (and Visual selection) the command line was
    /// opened with.
    fn cmdline_restore_origin(&mut self, cx: &mut Cx) {
        let Some(c) = self.cmdline.as_ref() else {
            return;
        };
        let origin = c.origin;
        if let Some(v) = c.origin_visual {
            if self.is_visual() {
                self.visual = v;
                self.sync_visual_selection(cx);
            }
        } else if !self.is_visual() {
            self.set_normal_caret(cx, origin);
        }
    }

    /// Incremental search: shows where the pattern typed so far would
    /// take the caret.
    fn search_preview(&mut self, cx: &mut Cx) {
        if !self.options.incremental_search {
            return;
        }
        let Some(c) = self.cmdline.as_ref() else {
            return;
        };
        let CmdKind::Search { forward } = c.kind else {
            return;
        };
        let origin = c.origin;
        let pattern = search::parse_pattern(&c.text);
        self.cmdline_restore_origin(cx);
        let Some(pat) = pattern else {
            return;
        };
        let found = search::find(cx.ed.document(), &pat, origin, forward);
        // Show the matches of what is being typed.
        self.search.pattern = Some(pat);
        self.search.highlight = true;
        if let Some(f) = found
            && self.pending.operator.is_none()
        {
            self.move_cursor(cx, f.range.start);
        }
    }

    /// A key while the command line is open. Every key is consumed.
    pub(crate) fn cmdline_key(&mut self, key: Key, cx: &mut Cx) -> bool {
        self.pending.keys.push(key);
        let Some(c) = self.cmdline.as_mut() else {
            return false;
        };
        if c.await_register {
            c.await_register = false;
            let text = if key.ctrl_char() == Some('w') {
                let cur = c.origin;
                self.word_under(cx, cur, false).map(|(w, _, _)| w)
            } else if let Some(name) = key.plain_char() {
                self.read_register(cx, Some(name)).map(|r| r.text())
            } else {
                None
            };
            if let Some(t) = text {
                let line = t.lines().next().unwrap_or("").to_string();
                self.cmdline_insert(&line, cx);
            }
            return true;
        }
        if key.is_escape() {
            self.cancel_cmdline(cx);
            return true;
        }
        if let Some(ch) = key.ctrl_char() {
            match ch {
                'h' => self.cmdline_backspace(cx),
                'u' => {
                    c.text.replace_range(..c.cursor, "");
                    c.cursor = 0;
                    self.search_preview(cx);
                }
                'w' => {
                    let head = &c.text[..c.cursor];
                    let trimmed = head.trim_end();
                    let start = trimmed
                        .char_indices()
                        .rev()
                        .take_while(|(_, ch)| ch.is_alphanumeric() || *ch == '_')
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or_else(|| {
                            trimmed.char_indices().next_back().map_or(0, |(i, _)| i)
                        });
                    c.text.replace_range(start..c.cursor, "");
                    c.cursor = start;
                    self.search_preview(cx);
                }
                'b' => c.cursor = 0,
                'e' => c.cursor = c.text.len(),
                'p' => self.cmdline_history(cx, true),
                'n' => self.cmdline_history(cx, false),
                'r' => c.await_register = true,
                'm' | 'j' => self.accept_cmdline(cx),
                _ => self.bell(),
            }
            return true;
        }
        match key.code {
            KeyCode::Char(ch) => {
                let mut buf = [0u8; 4];
                self.cmdline_insert(ch.encode_utf8(&mut buf), cx);
            }
            KeyCode::Enter => self.accept_cmdline(cx),
            KeyCode::Backspace => self.cmdline_backspace(cx),
            KeyCode::Delete => {
                let n = c.next_char();
                c.text.replace_range(c.cursor..n, "");
                self.search_preview(cx);
            }
            KeyCode::Left => c.cursor = c.prev_char(),
            KeyCode::Right => c.cursor = c.next_char(),
            KeyCode::Home => c.cursor = 0,
            KeyCode::End => c.cursor = c.text.len(),
            KeyCode::Up => self.cmdline_history(cx, true),
            KeyCode::Down => self.cmdline_history(cx, false),
            KeyCode::Tab | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Escape => {}
        }
        true
    }

    fn cmdline_backspace(&mut self, cx: &mut Cx) {
        let Some(c) = self.cmdline.as_mut() else {
            return;
        };
        if c.text.is_empty() {
            // Backspace on an empty command line closes it.
            self.cancel_cmdline(cx);
            return;
        }
        let p = c.prev_char();
        c.text.replace_range(p..c.cursor, "");
        c.cursor = p;
        c.history_index = None;
        self.search_preview(cx);
    }

    fn cmdline_history(&mut self, cx: &mut Cx, older: bool) {
        let Some(c) = self.cmdline.as_ref() else {
            return;
        };
        let kind = c.kind;
        let len = self.history(kind).len();
        let next = match (c.history_index, older) {
            (None, true) if len > 0 => Some(len - 1),
            (None, _) => None,
            (Some(0), true) => Some(0),
            (Some(i), true) => Some(i - 1),
            (Some(i), false) if i + 1 < len => Some(i + 1),
            (Some(_), false) => None,
        };
        let entry = next.map(|i| self.history(kind)[i].clone());
        if let Some(c) = self.cmdline.as_mut() {
            if c.history_index.is_none() {
                c.typed = c.text.clone();
            }
            c.text = entry.unwrap_or_else(|| c.typed.clone());
            c.cursor = c.text.len();
            c.history_index = next;
        }
        self.search_preview(cx);
    }

    fn cancel_cmdline(&mut self, cx: &mut Cx) {
        self.cmdline_restore_origin(cx);
        let Some(c) = self.cmdline.take() else {
            return;
        };
        if matches!(c.kind, CmdKind::Search { .. }) {
            // The preview replaced the pattern: put the last real one back.
            self.search.pattern = search::parse_pattern(&self.search.raw);
            self.search.highlight = c.saved_highlight;
        }
        self.push_history(c.kind, &c.text);
        self.cancel_pending();
    }

    fn accept_cmdline(&mut self, cx: &mut Cx) {
        self.cmdline_restore_origin(cx);
        let Some(c) = self.cmdline.take() else {
            return;
        };
        self.push_history(c.kind, &c.text);
        match c.kind {
            CmdKind::Ex => {
                self.pending.operator = None;
                let text = c.text.clone();
                self.run_ex(&text, cx);
                if self.is_visual() {
                    self.exit_visual(cx);
                }
                self.finish_command(cx, false);
            }
            CmdKind::Search { forward } => {
                if !c.text.is_empty() {
                    match search::parse_pattern(&c.text) {
                        Some(p) => {
                            self.search.pattern = Some(p);
                            self.search.raw = c.text.clone();
                        }
                        None => {
                            self.search.pattern = search::parse_pattern(&self.search.raw);
                        }
                    }
                } else {
                    self.search.pattern = search::parse_pattern(&self.search.raw);
                }
                self.search.forward = forward;
                self.run_motion(cx, Motion::Search { reverse: false });
            }
        }
    }

    // ----- ex commands --------------------------------------------------------------------

    fn resolve_addr(&mut self, cx: &Cx, a: Addr) -> Option<usize> {
        let doc = cx.ed.document();
        let last = doc.block_count() - 1;
        let base = match a.base {
            AddrBase::Number(n) => n.saturating_sub(1).min(last) as isize,
            AddrBase::Current => self.cursor(cx).block as isize,
            AddrBase::Last => last as isize,
            AddrBase::Mark(m) => match self.mark_position(cx, m) {
                Some(p) => p.block as isize,
                None => {
                    self.error("E20: Mark not set");
                    return None;
                }
            },
        };
        Some(base.saturating_add(a.offset).clamp(0, last as isize) as usize)
    }

    /// The line an address names as a destination (`:m`, `:t`, `:pu`):
    /// the number of lines the text goes below, from 0 (the top) to the
    /// number of lines in the document. Outside of that it is an error, as
    /// in Vim.
    fn resolve_destination(&mut self, cx: &Cx, a: Addr) -> Option<usize> {
        let n = cx.ed.document().block_count() as isize;
        let base = match a.base {
            AddrBase::Number(line) => isize::try_from(line).unwrap_or(isize::MAX / 4),
            AddrBase::Current => self.cursor(cx).block as isize + 1,
            AddrBase::Last => n,
            AddrBase::Mark(m) => match self.mark_position(cx, m) {
                Some(p) => p.block as isize + 1,
                None => {
                    self.error("E20: Mark not set");
                    return None;
                }
            },
        };
        let line = base.saturating_add(a.offset);
        if !(0..=n).contains(&line) {
            self.error("E16: Invalid range");
            return None;
        }
        Some(line as usize)
    }

    /// The blocks a range names; `default` when no range was given.
    fn resolve_range(
        &mut self,
        cx: &Cx,
        r: RangeSpec,
        default: Range<usize>,
    ) -> Option<Range<usize>> {
        let n = cx.ed.document().block_count();
        Some(match r {
            RangeSpec::None => default,
            RangeSpec::All => 0..n,
            RangeSpec::One(a) => {
                let b = self.resolve_addr(cx, a)?;
                b..b + 1
            }
            RangeSpec::Two(a, b) => {
                let x = self.resolve_addr(cx, a)?;
                let y = self.resolve_addr(cx, b)?;
                x.min(y)..x.max(y) + 1
            }
        })
    }

    /// Runs one ex command line.
    pub(crate) fn run_ex(&mut self, line: &str, cx: &mut Cx) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        self.last_ex = trimmed.to_string();
        let parsed = match ex::parse(trimmed) {
            Ok(p) => p,
            Err(e) => {
                self.error(e);
                return;
            }
        };
        let edits = match &parsed.command {
            ExCommand::Delete { .. }
            | ExCommand::Join
            | ExCommand::ShiftRight(_)
            | ExCommand::ShiftLeft(_)
            | ExCommand::RepeatSubstitute { .. }
            | ExCommand::Undo
            | ExCommand::Redo
            | ExCommand::Move(_)
            | ExCommand::Copy(_)
            | ExCommand::Put { .. } => true,
            ExCommand::Substitute(args) => !args.count_only,
            _ => false,
        };
        if edits && !self.writable() {
            return;
        }
        let cur = self.cursor(cx).block;
        let here = cur..cur + 1;
        let origin = self.cursor(cx);
        // A count takes that many lines, starting with the last of the
        // range.
        let counted = |r: Range<usize>, blocks: usize| match parsed.count {
            Some(n) => {
                let first = r.end - 1;
                first..(first + n.max(1)).min(blocks)
            }
            None => r,
        };
        let info = |register: Option<char>| OpInfo {
            register,
            big: false,
            yank_caret: origin,
            shift_times: 1,
            widen: false,
        };
        match parsed.command {
            ExCommand::Goto => {
                let Some(r) = self.resolve_range(cx, parsed.range, here) else {
                    return;
                };
                let doc = cx.ed.document();
                let target =
                    buf::unit_first_non_blank(doc, buf::unit_at(doc, Position::new(r.end - 1, 0)));
                self.push_jump(cx, origin);
                self.leave_visual_quietly(cx);
                self.set_normal_caret(cx, target);
            }
            ExCommand::Write { args } => {
                if args.is_empty() {
                    self.effects.push(Effect::Request(HostRequest::Save));
                } else {
                    self.effects.push(Effect::Request(HostRequest::Command {
                        name: "write".into(),
                        args,
                        bang: false,
                        range: None,
                    }));
                }
            }
            ExCommand::Quit { force } => {
                self.effects
                    .push(Effect::Request(HostRequest::Close { force }));
            }
            ExCommand::WriteQuit => {
                self.effects
                    .push(Effect::Request(HostRequest::SaveAndClose));
            }
            ExCommand::Delete { register } | ExCommand::Yank { register } => {
                let Some(r) = self.resolve_range(cx, parsed.range, here) else {
                    return;
                };
                if register.is_some_and(|c| !crate::register::is_writable(c)) {
                    self.error("E488: Trailing characters");
                    return;
                }
                let op = if matches!(parsed.command, ExCommand::Delete { .. }) {
                    Operator::Delete
                } else {
                    Operator::Yank
                };
                let r = counted(r, cx.ed.document().block_count());
                self.leave_visual_quietly(cx);
                // Vim goes to the first line of the range before deleting.
                let doc = cx.ed.document();
                let first =
                    buf::unit_first_non_blank(doc, buf::unit_at(doc, Position::new(r.start, 0)));
                self.hint_undo(cx, first);
                self.apply_operator(cx, op, Target::Lines(Lines::Blocks(r)), info(register));
            }
            ExCommand::ShiftRight(n) | ExCommand::ShiftLeft(n) => {
                let Some(r) = self.resolve_range(cx, parsed.range, here) else {
                    return;
                };
                let right = matches!(parsed.command, ExCommand::ShiftRight(_));
                let r = counted(r, cx.ed.document().block_count());
                self.leave_visual_quietly(cx);
                self.hint_undo(cx, origin);
                self.shift_lines(cx, &Lines::Blocks(r), right, n.max(1));
            }
            ExCommand::Join => {
                let Some(r) = self.resolve_range(cx, parsed.range, here) else {
                    return;
                };
                self.leave_visual_quietly(cx);
                // `:j N` joins N lines starting with the last of the range.
                let (first, count) = match parsed.count {
                    Some(n) => (r.end - 1, n.max(2)),
                    None => (r.start, r.len().max(2)),
                };
                let at = Position::new(first, 0);
                // Vim goes to the first line and keeps the column.
                self.hint_undo(cx, Position::new(first, origin.offset));
                let has_next = first + 1 < cx.ed.document().block_count();
                // `:j!` adds and removes no spaces.
                if has_next && self.join_lines(cx, at, count, !parsed.bang) {
                    // The cursor goes to the first non-blank of the line.
                    let doc = cx.ed.document();
                    let p = buf::unit_first_non_blank(doc, buf::unit_at(doc, at));
                    self.set_normal_caret(cx, p);
                } else {
                    self.set_normal_caret(cx, origin);
                    self.bell();
                }
            }
            ExCommand::Substitute(args) => {
                let Some(r) = self.resolve_range(cx, parsed.range, here) else {
                    return;
                };
                self.leave_visual_quietly(cx);
                self.substitute(cx, r, args);
            }
            ExCommand::RepeatSubstitute { keep_flags } => {
                let Some(r) = self.resolve_range(cx, parsed.range, here) else {
                    return;
                };
                let Some(mut args) = self.last_subst.clone() else {
                    self.error("E35: No previous regular expression");
                    return;
                };
                if !keep_flags {
                    args.global = false;
                    args.count_only = false;
                }
                self.leave_visual_quietly(cx);
                self.substitute(cx, r, args);
            }
            ExCommand::Move(to) | ExCommand::Copy(to) => {
                let moving = matches!(parsed.command, ExCommand::Move(_));
                let Some(r) = self.resolve_range(cx, parsed.range, here) else {
                    return;
                };
                let Some(dest) = to.and_then(|to| self.resolve_destination(cx, to)) else {
                    if to.is_none() {
                        self.error("E14: Invalid address");
                    }
                    return;
                };
                if moving && dest > r.start && dest < r.end {
                    self.error("E134: Cannot move a range of lines into itself");
                    return;
                }
                self.leave_visual_quietly(cx);
                // Undoing a move returns to where the cursor was; undoing a
                // copy to the line the text went below.
                let back = if moving {
                    origin
                } else {
                    Position::new(dest.saturating_sub(1), 0)
                };
                self.hint_undo(cx, back);
                let len = r.len();
                // The lines travel as a register would hold them: kinds,
                // indent and formatting come along.
                let lines = cx.ed.document().block_fragment(r.clone());
                let first = if !moving {
                    cx.ed.insert_blocks_at(dest, &lines);
                    dest
                } else if dest >= r.end && dest > r.start {
                    if dest > r.end {
                        cx.ed.insert_blocks_at(dest, &lines);
                        super::operators::delete_blocks(cx, &r);
                    }
                    dest - len
                } else {
                    if dest < r.start {
                        cx.ed.insert_blocks_at(dest, &lines);
                        super::operators::delete_blocks(cx, &(r.start + len..r.end + len));
                    }
                    dest
                };
                // The cursor goes to the last of the lines.
                let last = first + len - 1;
                self.set_mark(cx, '[', Position::new(first, 0));
                self.set_mark(cx, ']', Position::new(last, 0));
                let doc = cx.ed.document();
                let target =
                    buf::unit_first_non_blank(doc, buf::unit_at(doc, Position::new(last, 0)));
                self.set_normal_caret(cx, target);
                if len > 2 {
                    let what = if moving { "moved" } else { "more lines" };
                    self.message(format!("{len} {what}"));
                }
            }
            ExCommand::Put { register } => {
                let n = cx.ed.document().block_count();
                // Below the line of the range (`:0pu`: at the top), with
                // `!` above it.
                let at = match parsed.range {
                    RangeSpec::None => cur + usize::from(!parsed.bang),
                    RangeSpec::One(a) | RangeSpec::Two(_, a) => {
                        let Some(below) = self.resolve_destination(cx, a) else {
                            return;
                        };
                        if parsed.bang {
                            below.saturating_sub(1)
                        } else {
                            below
                        }
                    }
                    RangeSpec::All => n - usize::from(parsed.bang),
                };
                let Some(content) = self.read_register(cx, register) else {
                    return;
                };
                self.leave_visual_quietly(cx);
                self.hint_undo(cx, Position::new(at.saturating_sub(1), 0));
                // Whatever the register holds goes in as whole lines.
                let lines = content.fragment.blocks().len();
                cx.ed.insert_blocks_at(at, &content.fragment);
                let last = at + lines - 1;
                self.set_mark(cx, '[', Position::new(at, 0));
                self.set_mark(cx, ']', Position::new(last, 0));
                let doc = cx.ed.document();
                let target =
                    buf::unit_first_non_blank(doc, buf::unit_at(doc, Position::new(last, 0)));
                self.set_normal_caret(cx, target);
            }
            ExCommand::NoHighlight => self.search.highlight = false,
            ExCommand::Undo => {
                self.leave_visual_quietly(cx);
                self.undo(cx, 1);
            }
            ExCommand::Redo => {
                self.leave_visual_quietly(cx);
                self.redo(cx, 1);
            }
            ExCommand::Host { name, args, bang } => {
                let range = match parsed.range {
                    RangeSpec::None => None,
                    r => match self.resolve_range(cx, r, here) {
                        Some(r) => Some(r),
                        None => return,
                    },
                };
                self.effects.push(Effect::Request(HostRequest::Command {
                    name,
                    args,
                    bang,
                    range,
                }));
            }
        }
    }

    /// `:s`: replaces matches of a literal pattern in the blocks `range`.
    pub(crate) fn substitute(&mut self, cx: &mut Cx, range: Range<usize>, args: SubstArgs) {
        let raw = if args.pattern.is_empty() {
            self.search.raw.clone()
        } else {
            args.pattern.clone()
        };
        let Some(mut pat) = search::parse_pattern(&raw) else {
            self.error("E35: No previous regular expression");
            return;
        };
        if let Some(cs) = args.case_sensitive {
            pat.options = SearchOptions {
                case_sensitive: cs,
                ..pat.options
            };
        }
        // A substitution also sets the search pattern, as in Vim.
        self.search.raw = raw.clone();
        self.search.pattern = search::parse_pattern(&raw);
        self.search.highlight = true;
        let mut stored = args.clone();
        stored.pattern = raw.clone();
        self.last_subst = Some(stored);

        let origin = self.cursor(cx);
        let mut total = 0usize;
        let mut lines = 0usize;
        let mut last_block = None;
        cx.ed.set_caret(origin);
        // Blocks added by `\r` in the replacement, which breaks the line.
        let mut added = 0usize;
        for b0 in range {
            let b = b0 + added;
            if b >= cx.ed.document().block_count() {
                break;
            }
            let mut matches = pat.matches_in_block(cx.ed.document(), b);
            if matches.is_empty() {
                continue;
            }
            if !args.global {
                matches.truncate(1);
            }
            total += matches.len();
            lines += 1;
            if last_block.is_none() && !args.count_only {
                // Vim has the cursor at the start of the first changed line
                // when it makes the change: undo returns there.
                self.hint_undo(cx, Position::new(b, 0));
            }
            last_block = Some(b);
            if args.count_only {
                continue;
            }
            let mut splits = 0usize;
            for m in matches.into_iter().rev() {
                let matched = cx.ed.document().blocks()[b].text()[m.clone()].to_string();
                let mut rep = String::new();
                for piece in &args.replacement {
                    match piece {
                        RepPiece::Text(t) => rep.push_str(t),
                        RepPiece::Match => rep.push_str(&matched),
                    }
                }
                let r = TextRange::in_block(b, m.start, m.end);
                let mut parts = rep.split('\r');
                let first = parts.next().unwrap_or("");
                if first.is_empty() {
                    cx.ed.delete_range(r);
                } else {
                    cx.ed.replace_range(r, first);
                }
                // Every `\r` starts a new block with what follows it.
                let mut at = Position::new(b, m.start + first.len());
                for part in parts {
                    cx.ed.set_caret(at);
                    cx.ed.insert_paragraph_break();
                    at = Position::new(at.block + 1, 0);
                    if !part.is_empty() {
                        cx.ed
                            .replace_range(TextRange::in_block(at.block, 0, 0), part);
                        at.offset = part.len();
                    }
                    splits += 1;
                }
            }
            added += splits;
            last_block = Some(b + splits);
        }
        let Some(last) = last_block else {
            self.set_normal_caret(cx, origin);
            self.error(format!("E486: Pattern not found: {raw}"));
            return;
        };
        let plural = |n: usize, word: &str| {
            if n == 1 {
                format!("{n} {word}")
            } else {
                format!("{n} {word}s")
            }
        };
        if args.count_only {
            // (Vim leaves the cursor at the start of the last line with a
            // match.)
            self.set_normal_caret(cx, Position::new(last, 0));
            self.message(format!(
                "{} on {}",
                plural(total, "match").replace("matchs", "matches"),
                plural(lines, "line")
            ));
            return;
        }
        let doc = cx.ed.document();
        let caret = buf::unit_first_non_blank(doc, buf::unit_at(doc, Position::new(last, 0)));
        self.set_normal_caret(cx, caret);
        self.set_mark(cx, '.', caret);
        self.message(format!(
            "{} on {}",
            plural(total, "substitution"),
            plural(lines, "line")
        ));
    }

    /// Leaves Visual mode without touching the caret (an ex command took
    /// its range from the selection).
    fn leave_visual_quietly(&mut self, cx: &mut Cx) {
        if self.is_visual() {
            self.save_visual(cx);
            self.base = Base::Normal;
            let c = cx.ed.document().clamp_position(self.visual.cursor);
            self.set_normal_caret(cx, c);
        }
    }

    /// Makes `pat` the search pattern, as if it had been typed after `/`
    /// or `?` (used by `*` and `#`).
    pub(crate) fn set_search(&mut self, pat: SearchPattern, raw: String, forward: bool) {
        self.search.pattern = Some(pat);
        self.search.raw = raw.clone();
        self.search.forward = forward;
        self.search.highlight = true;
        self.push_history(CmdKind::Search { forward }, &raw);
    }
}
