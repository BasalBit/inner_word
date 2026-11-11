//! Computing where a motion goes.

use writer_core::{EditorState, Position, TextRange};

use crate::buf::{self, Lines};
use crate::host::{GoalColumn, Viewport, VimHost};
use crate::motion;
use crate::search;
use crate::textobj::Target;

use super::{Cx, FindSpec, Vim};

/// A motion command, with its arguments.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Motion {
    Left,
    Right,
    /// Backspace: left, wrapping to the previous line.
    WrapLeft,
    /// Space: right, wrapping to the next line.
    WrapRight,
    /// `j`, `k`: visual rows as a caret movement, line units for an operator.
    Down,
    Up,
    /// `+`, enter, `-`: line units, to the first non-blank.
    LineDown,
    LineUp,
    /// `_`: `count - 1` line units down, to the first non-blank.
    LineHere,
    WordForward {
        big: bool,
    },
    WordBackward {
        big: bool,
    },
    WordEnd {
        big: bool,
    },
    WordEndBackward {
        big: bool,
    },
    LineStart,
    FirstNonBlank,
    LineEnd,
    LastNonBlank,
    RowStart,
    RowFirstNonBlank,
    RowEnd,
    Column,
    /// `gg`: line `count`, default the first.
    FirstLine,
    /// `G`: line `count`, default the last.
    LastLine,
    /// `N%`: the line at that percentage of the document.
    Percent,
    ParagraphForward,
    ParagraphBackward,
    SentenceForward,
    SentenceBackward,
    Find(FindSpec),
    RepeatFind {
        reverse: bool,
    },
    MatchPair,
    ViewTop,
    ViewMiddle,
    ViewBottom,
    /// `n`, `N`, and the motion of `/`, `?`, `*`, `#`.
    Search {
        reverse: bool,
    },
    MarkLine(char),
    MarkExact(char),
}

/// How an operator turns a motion into a range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MotionKind {
    Exclusive,
    Inclusive,
    Linewise,
}

/// What happens to the remembered goal column.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum GoalUpdate {
    /// Vertical motions keep it.
    Keep,
    /// Horizontal motions forget it.
    Clear,
    /// `$` makes the following vertical motions stick to row ends.
    End,
}

/// The result of a motion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MotionOut {
    pub pos: Position,
    pub kind: MotionKind,
    pub goal: GoalUpdate,
    /// The origin goes on the jump list.
    pub jump: bool,
    /// Deletes with this motion always go to register 1.
    pub big: bool,
    /// The range is taken as it is: no adjustment for an exclusive motion
    /// that ends at the start of a line (backspace over a line break).
    pub exact: bool,
}

fn out(pos: Position, kind: MotionKind) -> MotionOut {
    MotionOut {
        pos,
        kind,
        goal: GoalUpdate::Clear,
        jump: false,
        big: false,
        exact: false,
    }
}

fn jump(pos: Position, kind: MotionKind) -> MotionOut {
    MotionOut {
        jump: true,
        big: true,
        ..out(pos, kind)
    }
}

/// A position on the hard line `n` lines below the line of `from`, stopping
/// at the last line. `None` when `n` is not zero and `from` is already on
/// the last line (Vim's rule for a count that reaches past the end).
fn lines_down(doc: &writer_core::Document, from: Position, n: usize) -> Option<Position> {
    let mut p = from;
    for i in 0..n {
        let e = buf::line_end(doc, p);
        match buf::next_cell(doc, Position::new(p.block, e)) {
            Some(q) => p = q,
            None if i == 0 => return None,
            None => break,
        }
    }
    Some(p)
}

impl Vim {
    /// The rows [`VimOptions::scroll_off`](super::VimOptions::scroll_off)
    /// keeps free at the top and at the bottom of `vp`: none at the first
    /// and the last row of the document, and never so many that no row is
    /// left between them.
    pub(crate) fn scroll_margins(
        &self,
        ed: &EditorState,
        host: &mut dyn VimHost,
        vp: &Viewport,
    ) -> (isize, isize) {
        let off = self
            .options
            .scroll_off
            .min(vp.rows.max(1).saturating_sub(1) / 2) as isize;
        if off == 0 {
            return (0, 0);
        }
        let doc = ed.document();
        let top = if doc.clamp_position(vp.top) == Position::ZERO {
            0
        } else {
            off
        };
        let bottom = doc.clamp_position(vp.bottom);
        let last = doc.block_count() - 1;
        let at_end =
            bottom.block == last && host.visual_row(ed, bottom).end >= buf::len_of(doc, last);
        (top, if at_end { 0 } else { off })
    }

    /// Moves `rows` visual rows from `from`, keeping the goal column.
    /// `None` when the caret cannot move (first or last row).
    pub(crate) fn move_rows(
        &mut self,
        cx: &mut Cx,
        from: Position,
        rows: isize,
    ) -> Option<Position> {
        let doc = cx.ed.document();
        let goal = match self.goal {
            Some(g) => g,
            None => {
                let g = GoalColumn::X(cx.host.column_of(cx.ed, from));
                self.goal = Some(g);
                g
            }
        };
        let to = doc.clamp_position(cx.host.move_by_rows(cx.ed, from, rows, goal));
        let same_row = to.block == from.block
            && cx.host.visual_row(cx.ed, to) == cx.host.visual_row(cx.ed, from);
        if same_row { None } else { Some(to) }
    }

    /// Computes a motion from `origin`. `None` means the motion failed.
    pub(crate) fn compute_motion(
        &mut self,
        cx: &mut Cx,
        m: Motion,
        count: Option<usize>,
        origin: Position,
        op_pending: bool,
    ) -> Option<MotionOut> {
        use MotionKind::{Exclusive, Inclusive, Linewise};
        let n = count.unwrap_or(1).max(1);
        let doc = cx.ed.document();
        let b = origin.block;
        let t = buf::text_of(doc, b);
        let ls = buf::line_start(doc, origin);
        let le = buf::line_end(doc, origin);
        let at = |o: usize| Position::new(b, o);
        Some(match m {
            Motion::Left => {
                let mut o = origin.offset;
                for _ in 0..n {
                    if o <= ls {
                        break;
                    }
                    o = buf::prev_boundary(t, o);
                }
                // With an operator `h` in the first column is an empty
                // motion, not an error (Vim repeats `X` there with `.`).
                if o == origin.offset && !op_pending {
                    return None;
                }
                out(at(o), Exclusive)
            }
            Motion::Right => {
                // The caret stops on the last character; an operator may
                // reach the end of the line to include that character.
                // In Visual mode `l` can step onto the line break after the
                // last character, as in Vim.
                let limit = if op_pending || self.is_visual() {
                    le
                } else {
                    buf::last_char(doc, b, ls, le)
                };
                let o = buf::advance_graphemes(t, origin.offset, n, limit);
                if o == origin.offset && !op_pending {
                    return None;
                }
                out(at(o), Exclusive)
            }
            Motion::WrapLeft => {
                // Only `d` and `c` take the line break itself (`d<BS>` in
                // column one joins the lines); for a caret movement and for
                // the other operators the step goes to the last character
                // of the line before.
                let takes_break = matches!(
                    self.pending.operator,
                    Some(super::Operator::Delete | super::Operator::Change)
                );
                let mut p = origin;
                // The motion stepped onto the line break of a line with
                // text: the range is then taken as it is; otherwise the
                // usual rules for a motion ending in column one apply.
                let mut over_break = false;
                for _ in 0..n {
                    let Some(mut q) = buf::prev_cell(doc, p) else {
                        break;
                    };
                    if buf::is_newline(doc, q) && !buf::is_empty_line(doc, q) {
                        if takes_break {
                            over_break = true;
                        } else {
                            // Skip the newline cell of a non-empty line.
                            q = buf::prev_cell(doc, q).unwrap_or(q);
                        }
                    }
                    p = q;
                }
                // (With an operator the start of the document is an empty
                // motion, not an error.)
                if p == origin && !op_pending {
                    return None;
                }
                MotionOut {
                    exact: over_break,
                    ..out(p, Exclusive)
                }
            }
            Motion::WrapRight => {
                let mut p = origin;
                for _ in 0..n {
                    let Some(mut q) = buf::next_cell(doc, p) else {
                        break;
                    };
                    if !op_pending && buf::is_newline(doc, q) && !buf::is_empty_line(doc, q) {
                        match buf::next_cell(doc, q) {
                            Some(r) => q = r,
                            None => break,
                        }
                    }
                    p = q;
                }
                if p == origin && !op_pending {
                    return None;
                }
                out(p, Exclusive)
            }
            Motion::Down | Motion::Up => {
                let down = m == Motion::Down;
                if op_pending {
                    let u = buf::unit_at(doc, origin);
                    let step = if down { n as isize } else { -(n as isize) };
                    let target = buf::unit_offset(doc, u, step);
                    if target == u {
                        return None;
                    }
                    // Where the caret would land: the same column of the
                    // target line (it matters for `yk` and for undo).
                    let col = buf::graphemes_between(t, ls, origin.offset);
                    let tle = buf::line_end(doc, target.start_pos());
                    let o = buf::advance_graphemes(
                        buf::text_of(doc, target.block),
                        target.start,
                        col,
                        tle,
                    );
                    let landing = buf::clamp_normal(doc, Position::new(target.block, o));
                    MotionOut {
                        goal: GoalUpdate::Keep,
                        ..out(landing, Linewise)
                    }
                } else {
                    let rows = if down { n as isize } else { -(n as isize) };
                    let to = self.move_rows(cx, origin, rows)?;
                    MotionOut {
                        goal: GoalUpdate::Keep,
                        ..out(to, Linewise)
                    }
                }
            }
            Motion::LineDown | Motion::LineUp | Motion::LineHere => {
                let u = buf::unit_at(doc, origin);
                let step = match m {
                    Motion::LineDown => n as isize,
                    Motion::LineUp => -(n as isize),
                    _ => n as isize - 1,
                };
                let target = buf::unit_offset(doc, u, step);
                if target == u && step != 0 {
                    return None;
                }
                out(buf::unit_first_non_blank(doc, target), Linewise)
            }
            Motion::WordForward { big } => {
                let p = motion::word_forward(doc, origin, n, self.class_opts(big), op_pending);
                // (With an operator the end of the document is an empty
                // motion, not an error, as in Vim.)
                if p == origin && !op_pending {
                    return None;
                }
                out(p, Exclusive)
            }
            Motion::WordBackward { big } => {
                let p = motion::word_backward(doc, origin, n, self.class_opts(big));
                if p == origin {
                    return None;
                }
                out(p, Exclusive)
            }
            Motion::WordEnd { big } => {
                let p = motion::word_end(doc, origin, n, self.class_opts(big), false);
                if p == origin && !op_pending {
                    return None;
                }
                out(p, Inclusive)
            }
            Motion::WordEndBackward { big } => {
                let p = motion::word_end_backward(doc, origin, n, self.class_opts(big));
                if p == origin {
                    return None;
                }
                out(p, Inclusive)
            }
            Motion::LineStart => out(at(ls), Exclusive),
            Motion::FirstNonBlank => out(at(buf::first_non_blank_cell(doc, b, ls)), Exclusive),
            Motion::LineEnd => {
                // `count - 1` hard lines further down: as far as there are
                // lines, but not from the last one.
                let p = lines_down(doc, origin, n - 1)?;
                let e = buf::line_end(doc, p);
                let s = buf::line_start(doc, p);
                // The caret rests on the last character; an operator and
                // Visual mode use the end of the line.
                let o = if op_pending || self.is_visual() {
                    e
                } else {
                    buf::last_char(doc, p.block, s, e)
                };
                MotionOut {
                    goal: GoalUpdate::End,
                    // The newline cell itself is the exclusive end, and the
                    // range is taken as it is: `$` is inclusive in Vim, so
                    // the rules for a motion that ends in column one do not
                    // apply when the last line is empty.
                    exact: true,
                    ..out(Position::new(p.block, o), Exclusive)
                }
            }
            Motion::LastNonBlank => {
                let p = lines_down(doc, origin, n - 1)?;
                let e = buf::line_end(doc, p);
                let s = buf::line_start(doc, p);
                out(
                    Position::new(p.block, buf::last_non_blank(doc, p.block, s, e)),
                    Inclusive,
                )
            }
            Motion::RowStart | Motion::RowFirstNonBlank | Motion::RowEnd => {
                let row = cx.host.visual_row(cx.ed, origin);
                let rs = row.start.min(t.len());
                let re = row.end.clamp(rs, t.len());
                match m {
                    Motion::RowStart => out(at(rs), Exclusive),
                    Motion::RowFirstNonBlank => {
                        out(at(buf::first_non_blank(doc, b, rs).min(re)), Exclusive)
                    }
                    _ => {
                        let last = if re > rs {
                            buf::prev_boundary(t, re)
                        } else {
                            rs
                        };
                        MotionOut {
                            goal: GoalUpdate::End,
                            ..out(at(last), Inclusive)
                        }
                    }
                }
            }
            Motion::Column => {
                // Never past the last character, also for an operator.
                let o = buf::advance_graphemes(t, ls, n - 1, buf::last_char(doc, b, ls, le));
                out(at(o), Exclusive)
            }
            Motion::FirstLine | Motion::LastLine => {
                let blocks = doc.block_count();
                let target = match (count, m) {
                    (Some(c), _) => {
                        let blk = c.clamp(1, blocks) - 1;
                        buf::unit_at(doc, Position::new(blk, 0))
                    }
                    (None, Motion::FirstLine) => buf::unit_at(doc, Position::ZERO),
                    (None, _) => buf::unit_at(doc, doc.end()),
                };
                jump(buf::unit_first_non_blank(doc, target), Linewise)
            }
            Motion::Percent => {
                let blocks = doc.block_count();
                let c = count?.min(100);
                let line = (c * blocks).div_ceil(100).clamp(1, blocks);
                let u = buf::unit_at(doc, Position::new(line - 1, 0));
                jump(buf::unit_first_non_blank(doc, u), Linewise)
            }
            Motion::ParagraphForward => {
                // At the end of the document the result is the end of the
                // last block, so the last character counts for an operator.
                let (p, hit_end) = motion::paragraph_forward(doc, origin, n)?;
                if hit_end && buf::clamp_normal(doc, p) == origin && !op_pending {
                    return None;
                }
                jump(p, Exclusive)
            }
            Motion::ParagraphBackward => {
                let p = motion::paragraph_backward(doc, origin, n)?;
                if p == origin {
                    return None;
                }
                jump(p, Exclusive)
            }
            Motion::SentenceForward => {
                let p = motion::sentence_forward(doc, origin, n)?;
                if buf::clamp_normal(doc, p) == origin && !op_pending {
                    return None;
                }
                jump(p, Exclusive)
            }
            Motion::SentenceBackward => {
                // At the start of the document the motion is empty for an
                // operator, not an error.
                let p = motion::sentence_backward(doc, origin, n)?;
                if p == origin && !op_pending {
                    return None;
                }
                jump(p, Exclusive)
            }
            Motion::Find(spec) => {
                let p = motion::find_char(doc, origin, spec.ch, spec.forward, spec.till, n, false)?;
                out(p, if spec.forward { Inclusive } else { Exclusive })
            }
            Motion::RepeatFind { reverse } => {
                let spec = self.last_find?;
                let forward = spec.forward != reverse;
                let p = motion::find_char(doc, origin, spec.ch, forward, spec.till, n, true)?;
                out(p, if forward { Inclusive } else { Exclusive })
            }
            Motion::MatchPair => jump(motion::match_pair(doc, origin)?, Inclusive),
            Motion::ViewTop | Motion::ViewMiddle | Motion::ViewBottom => {
                let vp = cx.host.viewport(cx.ed)?;
                let x = GoalColumn::X(0.0);
                // The rows `scrolloff` keeps free at each edge.
                let (off_top, off_bottom) = self.scroll_margins(cx.ed, cx.host, &vp);
                let row_start = match m {
                    Motion::ViewTop => {
                        let down = (n as isize - 1).max(off_top);
                        let p = cx.host.move_by_rows(cx.ed, vp.top, down, x);
                        // Never below the last row `L` would go to.
                        let limit = cx.host.move_by_rows(cx.ed, vp.bottom, -off_bottom, x);
                        if doc.clamp_position(p) > limit {
                            limit
                        } else {
                            p
                        }
                    }
                    Motion::ViewBottom => {
                        let up = (n as isize - 1).max(off_bottom);
                        let p = cx.host.move_by_rows(cx.ed, vp.bottom, -up, x);
                        let limit = cx.host.move_by_rows(cx.ed, vp.top, off_top, x);
                        if doc.clamp_position(p) < limit {
                            limit
                        } else {
                            p
                        }
                    }
                    _ => cx
                        .host
                        .move_by_rows(cx.ed, vp.top, (vp.rows.max(1) as isize - 1) / 2, x),
                };
                let p = doc.clamp_position(row_start);
                let row = cx.host.visual_row(cx.ed, p);
                let rt = buf::text_of(doc, p.block);
                let rs = row.start.min(rt.len());
                let re = row.end.clamp(rs, rt.len());
                let o = buf::first_non_blank(doc, p.block, rs).min(re);
                jump(Position::new(p.block, o), Linewise)
            }
            Motion::Search { reverse } => {
                let Some(pat) = self.search.pattern.clone() else {
                    self.error("E35: No previous regular expression");
                    return None;
                };
                let forward = self.search.forward != reverse;
                self.search.highlight = true;
                let mut p = origin;
                let mut wrapped = false;
                for _ in 0..n {
                    let Some(found) = search::find(doc, &pat, p, forward) else {
                        self.error(format!("E486: Pattern not found: {}", self.search.raw));
                        return None;
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
                jump(p, Exclusive)
            }
            Motion::MarkLine(c) => {
                let Some(p) = self.mark_position(cx, c) else {
                    self.error("E20: Mark not set");
                    return None;
                };
                let doc = cx.ed.document();
                jump(
                    buf::unit_first_non_blank(doc, buf::unit_at(doc, p)),
                    Linewise,
                )
            }
            Motion::MarkExact(c) => {
                let Some(p) = self.mark_position(cx, c) else {
                    self.error("E20: Mark not set");
                    return None;
                };
                jump(p, Exclusive)
            }
        })
    }

    /// Turns a motion from `origin` into what an operator acts on,
    /// applying Vim's rules for exclusive motions that end at the start of
    /// a line.
    pub(crate) fn motion_target(&self, cx: &Cx, origin: Position, m: &MotionOut) -> Target {
        let doc = cx.ed.document();
        let (s, e) = if origin <= m.pos {
            (origin, m.pos)
        } else {
            (m.pos, origin)
        };
        match m.kind {
            MotionKind::Linewise => Target::Lines(buf::lines_between(doc, s, e)),
            MotionKind::Inclusive => Target::Chars(TextRange::new(s, buf::cell_end(doc, e))),
            MotionKind::Exclusive => {
                let mut e = e;
                if !m.exact && e > s && buf::at_line_start(doc, e) && !buf::same_line(doc, s, e) {
                    let prev_end = buf::prev_cell(doc, e).unwrap_or(e);
                    // "If the motion is exclusive, the end of the motion is
                    // in column 1 and the start is at or before the first
                    // non-blank in the line, the motion becomes linewise."
                    let su = buf::unit_at(doc, s);
                    let eu = buf::unit_at(doc, e);
                    let start_in_indent = buf::line_start(doc, s) == su.start
                        && s.offset <= buf::first_non_blank(doc, s.block, su.start);
                    if start_in_indent && e.offset == eu.start {
                        return Target::Lines(buf::lines_between(doc, s, prev_end));
                    }
                    // Otherwise the end moves to the end of the line before.
                    e = prev_end;
                }
                Target::Chars(TextRange::new(s, e))
            }
        }
    }

    /// Vim's exception for `d`: a characterwise delete over several lines
    /// with only blanks before its start and after its end takes the whole
    /// lines.
    pub(crate) fn widen_delete(&self, cx: &Cx, target: Target) -> Target {
        let Target::Chars(r) = &target else {
            return target;
        };
        let doc = cx.ed.document();
        let (s, e) = (r.start, r.end);
        if buf::same_line(doc, s, e) {
            return target;
        }
        let su = buf::unit_at(doc, s);
        let eu = buf::unit_at(doc, e);
        let blank_before = buf::line_start(doc, s) == su.start
            && s.offset <= buf::first_non_blank(doc, s.block, su.start);
        let le = buf::line_end(doc, e);
        let blank_after = le == eu.end
            && buf::text_of(doc, e.block)[e.offset..le]
                .chars()
                .all(char::is_whitespace);
        if blank_before && blank_after {
            Target::Lines(buf::lines_between(doc, s, e))
        } else {
            target
        }
    }

    /// The linewise range of `count` line units starting with the caret's
    /// (`dd`, `yy`, `>>`, …), clamped to the end of the document.
    ///
    /// `None` when a count asks for more lines while the caret is on the
    /// last one: Vim refuses that, although it takes what is there from any
    /// other line.
    pub(crate) fn lines_from_cursor(
        &self,
        cx: &Cx,
        origin: Position,
        count: usize,
    ) -> Option<Lines> {
        let doc = cx.ed.document();
        let u = buf::unit_at(doc, origin);
        let last = buf::unit_offset(doc, u, count.max(1) as isize - 1);
        if count > 1 && last == u {
            return None;
        }
        Some(buf::lines_between(
            doc,
            Position::new(u.block, u.start),
            Position::new(last.block, last.start),
        ))
    }
}
