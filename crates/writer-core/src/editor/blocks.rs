//! Block-level commands.

use std::ops::Range;

use crate::block::{Alignment, Block, BlockId, BlockKind, MAX_INDENT, TEXT_SIZE_SCALE, sanitize_text_size};
use crate::fragment::Fragment;
use crate::history::EditKind;
use crate::position::{Position, Selection};

use super::EditorState;

/// Maps positions in a rebuilt block range to the new blocks.
struct PosMap {
    range: Range<usize>,
    new_len: usize,
    /// (old block, old offset start, new block, new offset start), sorted.
    segs: Vec<(usize, usize, usize, usize)>,
}

impl PosMap {
    fn map(&self, p: Position) -> Position {
        if p.block < self.range.start {
            return p;
        }
        if p.block >= self.range.end {
            return Position::new(p.block + self.new_len - self.range.len(), p.offset);
        }
        let seg = self
            .segs
            .iter()
            .rev()
            .find(|s| s.0 == p.block && s.1 <= p.offset)
            .copied();
        match seg {
            Some((_, os, nb, ns)) => Position::new(nb, ns + (p.offset - os)),
            None => Position::new(self.range.start, 0),
        }
    }
}

/// The block that "open line" style insertions create next to `b`: list
/// items and quotes continue (a to-do unchecked), anything else gets a paragraph.
fn sibling_of(b: &Block) -> Block {
    let kind = match &b.kind {
        BlockKind::BulletItem => BlockKind::BulletItem,
        BlockKind::NumberedItem => BlockKind::NumberedItem,
        BlockKind::TodoItem { .. } => BlockKind::todo(),
        BlockKind::Quote => BlockKind::Quote,
        _ => BlockKind::Paragraph,
    };
    let keep_indent = kind != BlockKind::Paragraph;
    let mut n = Block::new(kind);
    if keep_indent {
        n.indent = b.indent;
    }
    n
}

/// The next size on [`TEXT_SIZE_SCALE`] in the direction of `steps`.
fn stepped_size(current: f32, steps: i32) -> f32 {
    let steps = steps.clamp(-(TEXT_SIZE_SCALE.len() as i32), TEXT_SIZE_SCALE.len() as i32);
    let mut v = current;
    if steps > 0 {
        for _ in 0..steps {
            v = TEXT_SIZE_SCALE.iter().copied().find(|s| *s > v + 0.01).unwrap_or(v);
        }
    } else {
        for _ in 0..steps.unsigned_abs() {
            v = TEXT_SIZE_SCALE.iter().rev().copied().find(|s| *s < v - 0.01).unwrap_or(v);
        }
    }
    v
}

impl EditorState {
    /// Indices of non-void touched blocks.
    fn touched_text_blocks(&self) -> Vec<usize> {
        self.touched_blocks()
            .filter(|&i| !self.doc.blocks[i].is_void())
            .collect()
    }

    /// Sets the kind of every touched (non-void) block. If they all already
    /// have that kind (same heading level), they become paragraphs instead,
    /// so shortcuts toggle. Converting several blocks to a code block merges
    /// each run of adjacent blocks into one code block (lines joined by
    /// `\n`, formatting dropped); converting a code block to another kind
    /// makes one block per line. Void kinds are rejected (use
    /// [`EditorState::insert_divider`] / [`EditorState::insert_page_break`]).
    pub fn set_block_kind(&mut self, kind: BlockKind) -> bool {
        let kind = kind.sanitized();
        if kind.is_void() {
            return false;
        }
        let targets = self.touched_text_blocks();
        if targets.is_empty() {
            return false;
        }
        let all_same = targets
            .iter()
            .all(|&i| self.doc.blocks[i].kind.same_type(&kind));
        let target = if all_same { BlockKind::Paragraph } else { kind };
        let range = self.touched_blocks();
        self.edit(EditKind::Other, |ed| ed.convert_blocks(range, &target))
    }

    /// Sets the language of every touched code block.
    pub fn set_code_language(&mut self, language: Option<&str>) -> bool {
        let lang = language.map(str::trim).filter(|l| !l.is_empty()).map(str::to_string);
        let targets: Vec<usize> = self
            .touched_blocks()
            .filter(|&i| self.doc.blocks[i].kind.is_code())
            .collect();
        self.edit(EditKind::Other, |ed| {
            for i in targets {
                ed.update_block(i, |b| {
                    b.set_kind(BlockKind::CodeBlock {
                        language: lang.clone(),
                    })
                });
            }
        })
    }

    /// Rebuilds `range` with every non-void block converted to `target`,
    /// mapping the selection.
    pub(crate) fn convert_blocks(&mut self, range: Range<usize>, target: &BlockKind) {
        let mut out: Vec<Block> = Vec::with_capacity(range.len());
        let mut segs = Vec::new();
        let mut changed = false;
        let mut code_run_open = false;
        for j in range.clone() {
            let b = &self.doc.blocks[j];
            if b.is_void() {
                segs.push((j, 0, range.start + out.len(), 0));
                out.push(b.clone());
                code_run_open = false;
                continue;
            }
            if target.is_code() {
                if code_run_open {
                    let last = out.last_mut().expect("open code run");
                    let base = last.len() + 1;
                    let nl = Block::paragraph("\n");
                    last.append(&nl);
                    last.append(b);
                    segs.push((j, 0, range.start + out.len() - 1, base));
                    changed = true;
                } else {
                    let mut nb = b.clone();
                    if !b.kind.is_code() {
                        nb.set_kind(target.clone());
                        changed = true;
                    }
                    segs.push((j, 0, range.start + out.len(), 0));
                    out.push(nb);
                    code_run_open = true;
                }
                continue;
            }
            code_run_open = false;
            if b.kind.is_code() {
                // One block per line.
                let text = b.text.clone();
                let mut start = 0;
                let mut first = true;
                for line in text.split('\n') {
                    let end = start + line.len();
                    let mut nb = b.slice(start..end);
                    if first {
                        nb.id = b.id;
                    } else {
                        nb.id = BlockId::UNASSIGNED;
                    }
                    nb.text_size = None;
                    nb.set_kind(target.clone());
                    segs.push((j, start, range.start + out.len(), 0));
                    out.push(nb);
                    start = end + 1;
                    first = false;
                }
                changed = true;
            } else {
                let mut nb = b.clone();
                if nb.kind != *target {
                    nb.set_kind(target.clone());
                    changed = true;
                }
                segs.push((j, 0, range.start + out.len(), 0));
                out.push(nb);
            }
        }
        if !changed {
            return;
        }
        let map = PosMap {
            range: range.clone(),
            new_len: out.len(),
            segs,
        };
        let sel = Selection::new(map.map(self.selection.anchor), map.map(self.selection.head));
        // Only replace the blocks that changed position or content.
        self.splice(range.start, range.len(), out);
        self.set_sel(sel);
    }

    /// Toggles the checked flag of the touched to-do items: checks them all
    /// unless all are checked, in which case unchecks them all.
    pub fn toggle_todo_checked(&mut self) -> bool {
        let todos: Vec<usize> = self
            .touched_blocks()
            .filter(|&i| matches!(self.doc.blocks[i].kind, BlockKind::TodoItem { .. }))
            .collect();
        if todos.is_empty() {
            return false;
        }
        let all_checked = todos
            .iter()
            .all(|&i| self.doc.blocks[i].kind == BlockKind::TodoItem { checked: true });
        self.edit(EditKind::Other, |ed| {
            for i in todos {
                ed.update_block(i, |b| {
                    b.set_kind(BlockKind::TodoItem {
                        checked: !all_checked,
                    })
                });
            }
        })
    }

    /// Sets the alignment of the touched blocks.
    pub fn set_alignment(&mut self, alignment: Alignment) -> bool {
        let targets = self.touched_text_blocks();
        self.edit(EditKind::Other, |ed| {
            for i in targets {
                ed.update_block(i, |b| b.set_alignment(alignment));
            }
        })
    }

    /// Sets (or with `None` resets) the text size override of the touched
    /// blocks, clamped to `MIN_TEXT_SIZE..=MAX_TEXT_SIZE`.
    pub fn set_text_size(&mut self, size: Option<f32>) -> bool {
        let size = sanitize_text_size(size);
        let targets = self.touched_text_blocks();
        self.edit(EditKind::Other, |ed| {
            for i in targets {
                ed.update_block(i, |b| b.set_text_size(size));
            }
        })
    }

    /// Removes the text size override of the touched blocks.
    pub fn reset_text_size(&mut self) -> bool {
        self.set_text_size(None)
    }

    /// Steps the text size of each touched block along
    /// [`TEXT_SIZE_SCALE`] (positive = larger), starting from its override or
    /// the default size of its kind ([`Typography::size_for`](crate::Typography::size_for)).
    /// Landing on the default size removes the override.
    pub fn step_text_size(&mut self, steps: i32) -> bool {
        if steps == 0 {
            return false;
        }
        let typo = self.doc.settings.typography.clone();
        let targets = self.touched_text_blocks();
        self.edit(EditKind::Other, |ed| {
            for i in targets {
                let b = &ed.doc.blocks[i];
                let default = typo.size_for(&b.kind);
                let cur = b.text_size.unwrap_or(default);
                let next = stepped_size(cur, steps);
                let new = if (next - default).abs() < 0.01 { None } else { Some(next) };
                ed.update_block(i, |b| b.set_text_size(new));
            }
        })
    }

    /// Indents the touched blocks one level (up to [`MAX_INDENT`]).
    pub fn indent(&mut self) -> bool {
        self.shift_indent(1)
    }

    /// Outdents the touched blocks one level.
    pub fn outdent(&mut self) -> bool {
        self.shift_indent(-1)
    }

    fn shift_indent(&mut self, delta: i32) -> bool {
        let targets = self.touched_text_blocks();
        self.edit(EditKind::Other, |ed| {
            for i in targets {
                let cur = ed.doc.blocks[i].indent as i32;
                let n = (cur + delta).clamp(0, MAX_INDENT as i32) as u8;
                ed.update_block(i, |b| b.set_indent(n));
            }
        })
    }

    /// Moves the touched blocks up past the previous block.
    pub fn move_blocks_up(&mut self) -> bool {
        let r = self.touched_blocks();
        if r.start == 0 {
            return false;
        }
        self.edit(EditKind::Other, |ed| {
            let mut moved: Vec<Block> = ed.doc.blocks[r.clone()].to_vec();
            moved.push(ed.doc.blocks[r.start - 1].clone());
            ed.splice(r.start - 1, r.len() + 1, moved);
            let s = ed.selection;
            // A selection end at offset 0 of the block after the range (not
            // touched) stays right after the moved blocks.
            let shift = |p: Position| {
                if r.contains(&p.block) || p.block == r.end {
                    Position::new(p.block - 1, p.offset)
                } else {
                    p
                }
            };
            ed.set_sel(Selection::new(shift(s.anchor), shift(s.head)));
        })
    }

    /// Moves the touched blocks down past the next block.
    pub fn move_blocks_down(&mut self) -> bool {
        let r = self.touched_blocks();
        if r.end >= self.doc.blocks.len() {
            return false;
        }
        self.edit(EditKind::Other, |ed| {
            let mut moved = vec![ed.doc.blocks[r.end].clone()];
            moved.extend(ed.doc.blocks[r.clone()].iter().cloned());
            ed.splice(r.start, r.len() + 1, moved);
            let s = ed.selection;
            let n = ed.doc.blocks.len();
            let last_len = ed.doc.blocks[r.end].len();
            let shift = |p: Position| {
                if r.contains(&p.block) {
                    Position::new(p.block + 1, p.offset)
                } else if p.block == r.end {
                    // Selection end at offset 0 of the following block (not
                    // touched): keep it right after the moved blocks.
                    if r.end + 1 < n {
                        Position::new(r.end + 1, 0)
                    } else {
                        Position::new(r.end, last_len)
                    }
                } else {
                    p
                }
            };
            ed.set_sel(Selection::new(shift(s.anchor), shift(s.head)));
        })
    }

    /// Duplicates the touched blocks below themselves (new ids, without
    /// comment anchors) and moves the selection into the copies.
    pub fn duplicate_blocks(&mut self) -> bool {
        let r = self.touched_blocks();
        self.edit(EditKind::Other, |ed| {
            let copies: Vec<Block> = ed.doc.blocks[r.clone()]
                .iter()
                .map(|b| {
                    let mut c = b.clone();
                    c.id = BlockId::UNASSIGNED;
                    c.strip_comments();
                    c.mark_dirty();
                    c
                })
                .collect();
            ed.splice(r.end, 0, copies);
            let s = ed.selection;
            let n = r.len();
            // A selection end at offset 0 of the block after the range (not
            // touched) moves along so it stays right after the copies.
            let shift = |p: Position| {
                if r.contains(&p.block) || p.block == r.end {
                    Position::new(p.block + n, p.offset)
                } else {
                    p
                }
            };
            ed.set_sel(Selection::new(shift(s.anchor), shift(s.head)));
        })
    }

    /// Deletes the touched blocks. The caret goes to the start of the block
    /// that follows (or the end of the one before). Deleting every block
    /// leaves one empty paragraph.
    pub fn delete_blocks(&mut self) -> bool {
        let r = self.touched_blocks();
        self.edit(EditKind::Other, |ed| {
            if r.len() == ed.doc.blocks.len() {
                ed.splice(0, r.len(), vec![Block::new(BlockKind::Paragraph)]);
                ed.caret_to(Position::ZERO);
                return;
            }
            ed.splice(r.start, r.len(), Vec::new());
            let n = ed.doc.blocks.len();
            let p = if r.start < n {
                Position::new(r.start, 0)
            } else {
                Position::new(n - 1, ed.doc.blocks[n - 1].len())
            };
            ed.caret_to(p);
        })
    }

    /// Inserts a divider after the current block (an empty paragraph is
    /// turned into the divider instead); the caret lands in the paragraph
    /// after it, which is created if needed.
    pub fn insert_divider(&mut self) -> bool {
        self.insert_void(BlockKind::Divider)
    }

    /// Inserts a page break after the current block (see
    /// [`EditorState::insert_divider`]).
    pub fn insert_page_break(&mut self) -> bool {
        self.insert_void(BlockKind::PageBreak)
    }

    fn insert_void(&mut self, kind: BlockKind) -> bool {
        let i = self.selection.head.block;
        self.edit(EditKind::Other, |ed| {
            let b = &ed.doc.blocks[i];
            let void_at = if b.kind == BlockKind::Paragraph && b.is_empty() {
                ed.update_block(i, |b| b.set_kind(kind.clone()));
                i
            } else {
                ed.splice(i + 1, 0, vec![Block::new(kind.clone())]);
                i + 1
            };
            let p = ed.paragraph_after(void_at);
            ed.caret_to(p);
        })
    }

    /// The start of an empty paragraph right after `index`, inserting one
    /// unless the next block already is an empty paragraph.
    pub(crate) fn paragraph_after(&mut self, index: usize) -> Position {
        let next = index + 1;
        let reuse = self
            .doc
            .blocks
            .get(next)
            .is_some_and(|b| b.kind == BlockKind::Paragraph && b.is_empty());
        if !reuse {
            self.splice(next, 0, vec![Block::new(BlockKind::Paragraph)]);
        }
        Position::new(next, 0)
    }

    /// Inserts an empty block before the first touched block and moves the
    /// caret into it. List items and quotes get a sibling of the same kind,
    /// everything else a paragraph.
    pub fn insert_block_before(&mut self) -> bool {
        let i = self.selection.start().block;
        self.edit(EditKind::Other, |ed| {
            let nb = sibling_of(&ed.doc.blocks[i]);
            ed.splice(i, 0, vec![nb]);
            ed.caret_to(Position::new(i, 0));
        })
    }

    /// Inserts an empty block after the block at the selection end and moves
    /// the caret into it (see [`EditorState::insert_block_before`]).
    pub fn insert_block_after(&mut self) -> bool {
        let i = self.selection.end().block;
        self.edit(EditKind::Other, |ed| {
            let nb = sibling_of(&ed.doc.blocks[i]);
            ed.splice(i + 1, 0, vec![nb]);
            ed.caret_to(Position::new(i + 1, 0));
        })
    }

    /// Inserts the blocks of `fragment` as whole blocks at block `index`
    /// (clamped to `0..=block_count`), without merging into neighbours (Vim
    /// linewise put). The caret goes to the start of the first inserted
    /// block. Comment anchors are stripped.
    pub fn insert_blocks_at(&mut self, index: usize, fragment: &Fragment) -> bool {
        let index = index.min(self.doc.blocks.len());
        let blocks = clean_fragment_blocks(fragment);
        if blocks.is_empty() {
            return false;
        }
        self.edit(EditKind::Other, |ed| {
            ed.splice(index, 0, blocks);
            ed.caret_to(Position::new(index, 0));
        })
    }
}

/// Fragment blocks ready to insert: no ids, no anchors, fresh stamps.
pub(crate) fn clean_fragment_blocks(fragment: &Fragment) -> Vec<Block> {
    fragment
        .blocks
        .iter()
        .map(|b| {
            let mut c = b.clone();
            c.id = BlockId::UNASSIGNED;
            c.strip_comments();
            c.mark_dirty();
            c
        })
        .collect()
}
