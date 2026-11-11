//! Replace commands.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::history::EditKind;
use crate::position::{Position, Selection, TextRange};
use crate::search::SearchOptions;
use crate::text;

use super::{EditorState, inherited_attrs};

impl EditorState {
    /// Replaces `range` (clamped) with `replacement`, which takes the
    /// attributes of the first replaced character (comment anchors and link
    /// included). A `\n` in the replacement is a soft line break. The caret
    /// goes to the end of the replacement. One undo step.
    pub fn replace_range(&mut self, range: TextRange, replacement: &str) -> bool {
        let s = self.doc.clamp_position(range.start);
        let e = self.doc.clamp_position(range.end);
        let (s, e) = if s <= e { (s, e) } else { (e, s) };
        let rep = text::normalize_input(replacement).into_owned();
        if s == e && rep.is_empty() {
            return false;
        }
        let sb = &self.doc.blocks[s.block];
        let attrs = if s < e {
            sb.attrs_at(s.offset)
                .cloned()
                .unwrap_or_else(|| inherited_attrs(sb, s.offset))
        } else {
            inherited_attrs(sb, s.offset)
        };
        self.edit(EditKind::Other, |ed| {
            let p = if s < e { ed.delete_range_impl(s, e) } else { s };
            let p = ed.leave_void(p);
            let end = ed.insert_in_block(p, &rep, &attrs);
            ed.caret_to(end);
            ed.pending = None;
        })
    }

    /// Replaces every match of `query` with `replacement` as one undo step;
    /// returns the number of replacements. Each replacement takes the
    /// attributes of the first character it replaces. The selection is kept
    /// in place (moved to the start of a match it was inside).
    pub fn replace_all(&mut self, query: &str, opts: SearchOptions, replacement: &str) -> usize {
        let matches = self.doc.find_all(query, opts);
        if matches.is_empty() {
            return 0;
        }
        let rep = text::normalize_input(replacement).into_owned();
        let mut by_block: BTreeMap<usize, Vec<Range<usize>>> = BTreeMap::new();
        for m in &matches {
            by_block
                .entry(m.start.block)
                .or_default()
                .push(m.start.offset..m.end.offset);
        }
        let map_pos = |p: Position, by_block: &BTreeMap<usize, Vec<Range<usize>>>| -> Position {
            let Some(rs) = by_block.get(&p.block) else {
                return p;
            };
            let mut off = p.offset as isize;
            for r in rs.iter().rev() {
                if p.offset >= r.end {
                    off += rep.len() as isize - r.len() as isize;
                } else if p.offset > r.start {
                    off -= (p.offset - r.start) as isize;
                }
            }
            Position::new(p.block, off.max(0) as usize)
        };
        let sel = self.selection;
        let new_sel = Selection::new(map_pos(sel.anchor, &by_block), map_pos(sel.head, &by_block));
        self.edit(EditKind::Other, |ed| {
            for (i, rs) in &by_block {
                ed.update_block(*i, |b| {
                    for r in rs.iter().rev() {
                        let attrs = b.attrs_at(r.start).cloned().unwrap_or_default();
                        b.remove(r.clone());
                        b.insert(r.start, &rep, &attrs);
                    }
                });
            }
            ed.set_sel(new_sel);
        });
        matches.len()
    }
}
