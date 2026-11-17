//! Block-level commands.

use std::ops::Range;

use crate::block::{
    Alignment, Block, BlockId, BlockKind, Image, ImageWidth, MAX_INDENT, TEXT_SIZE_SCALE,
    sanitize_text_size,
};
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
    let steps = steps.clamp(
        -(TEXT_SIZE_SCALE.len() as i32),
        TEXT_SIZE_SCALE.len() as i32,
    );
    let mut v = current;
    if steps > 0 {
        for _ in 0..steps {
            v = TEXT_SIZE_SCALE
                .iter()
                .copied()
                .find(|s| *s > v + 0.01)
                .unwrap_or(v);
        }
    } else {
        for _ in 0..steps.unsigned_abs() {
            v = TEXT_SIZE_SCALE
                .iter()
                .rev()
                .copied()
                .find(|s| *s < v - 0.01)
                .unwrap_or(v);
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
        let lang = language
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string);
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

    /// Sets the checked flag of the to-do item at block `index` without
    /// moving the selection (a click on its checkbox). Returns false when
    /// that block is not a to-do item or already has that state. Undoable.
    pub fn set_todo_checked(&mut self, index: usize, checked: bool) -> bool {
        match self.doc.blocks.get(index).map(|b| &b.kind) {
            Some(BlockKind::TodoItem { checked: c }) if *c != checked => {}
            _ => return false,
        }
        self.edit(EditKind::Other, |ed| {
            ed.update_block(index, |b| b.set_kind(BlockKind::TodoItem { checked }));
        })
    }

    /// Sets the alignment of the touched blocks: every block with text,
    /// and every image (an image stands left, centred or right on its
    /// line; asked to be justified it stays as it is). Dividers and page
    /// breaks have no alignment to set.
    pub fn set_alignment(&mut self, alignment: Alignment) -> bool {
        let targets: Vec<usize> = self
            .touched_blocks()
            .filter(|&i| {
                let b = &self.doc.blocks[i];
                !b.is_void() || (b.kind.is_image() && alignment != Alignment::Justify)
            })
            .collect();
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
                let new = if (next - default).abs() < 0.01 {
                    None
                } else {
                    Some(next)
                };
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

    // ----- images ----------------------------------------------------------------------

    /// Puts `image` where an insertion goes, as a block of its own with
    /// `alignment` (justified is left): when the caret is in an empty
    /// paragraph, that paragraph becomes the image and keeps its id;
    /// otherwise the image goes after the last block the selection touches
    /// ([`EditorState::touched_blocks`]: the caret's block, the block a
    /// range ends in). Nothing is deleted, a selected range stays as it is
    /// in the text. The caret ends on the image, which is what "the image
    /// is selected" means ([`EditorState::selected_image`]). One undo step.
    pub fn insert_image(&mut self, image: Image, alignment: Alignment) -> bool {
        let block = Block::image(image).with_alignment(alignment);
        self.edit(EditKind::Other, |ed| {
            let caret = ed.selection.head;
            let here = &ed.doc.blocks[caret.block];
            let index = if ed.selection.is_collapsed()
                && here.kind == BlockKind::Paragraph
                && here.is_empty()
            {
                let (kind, alignment) = (block.kind.clone(), block.alignment);
                ed.update_block(caret.block, |b| {
                    b.set_kind(kind);
                    b.set_alignment(alignment);
                });
                caret.block
            } else {
                let after = ed.touched_blocks().end;
                ed.splice(after, 0, vec![block]);
                after
            };
            ed.caret_to(Position::new(index, 0));
            // Formatting chosen for the next typed character belonged to
            // the line that is now a picture.
            ed.pending = None;
        })
    }

    /// Inserts `images` as whole blocks before the block that is now at
    /// `index` (clamped to `0..=block_count`; at `block_count` they go to
    /// the end), in order, each with its alignment (justified is left).
    /// Nothing is merged or replaced. The caret ends on the last of them.
    /// One undo step; `false` when there is nothing to insert.
    pub fn insert_images_at(&mut self, index: usize, images: Vec<(Image, Alignment)>) -> bool {
        if images.is_empty() {
            return false;
        }
        let index = index.min(self.doc.blocks.len());
        let blocks: Vec<Block> = images
            .into_iter()
            .map(|(image, alignment)| Block::image(image).with_alignment(alignment))
            .collect();
        self.edit(EditKind::Other, |ed| {
            let last = index + blocks.len() - 1;
            ed.splice(index, 0, blocks);
            ed.caret_to(Position::new(last, 0));
        })
    }

    /// The image the caret is on, with the index of its block: the
    /// selection is collapsed on an image block. This is the selected
    /// image; a range that covers an image selects text and blocks, not
    /// that image.
    pub fn selected_image(&self) -> Option<(usize, &Image)> {
        if !self.selection.is_collapsed() {
            return None;
        }
        let index = self.selection.head.block;
        self.doc.blocks[index]
            .as_image()
            .map(|image| (index, image))
    }

    /// The widest `image` is drawn in this document: the whole column, or
    /// less where it would be taller than a page of the document's page
    /// setup ([`Image::max_width`]). It holds in both views: a page is what
    /// a document is made for, whether or not it is shown paginated.
    pub fn max_image_width(&self, image: &Image) -> ImageWidth {
        image.max_width(&self.doc.settings.page)
    }

    /// The width at which `image` has its own size on a page of this
    /// document ([`Image::original_width`]): what an inserted image
    /// starts with, and what "original size" sets.
    pub fn original_image_width(&self, image: &Image) -> ImageWidth {
        image.original_width(&self.doc.settings.page)
    }

    /// Sets the width of the image at block `index`, without moving the
    /// selection. What is stored is `width` or, where that is wider than
    /// the page allows, [`EditorState::max_image_width`]: the stored
    /// width is the width that is drawn. Returns `false` when that block
    /// is not an image or already has that width. One undo step.
    pub fn set_image_width(&mut self, index: usize, width: ImageWidth) -> bool {
        let Some(image) = self.doc.blocks.get(index).and_then(Block::as_image) else {
            return false;
        };
        let width = width.min(self.max_image_width(image));
        if width == image.width {
            return false;
        }
        let mut image = image.clone();
        image.width = width;
        self.edit(EditKind::Other, |ed| {
            ed.update_block(index, |b| b.set_kind(BlockKind::Image(image)));
        })
    }

    /// Steps the width of the image at block `index` along
    /// [`ImageWidth::STEPS`] (positive is wider), not beyond
    /// [`EditorState::max_image_width`]. The steps are counted from the
    /// width that is drawn: a stored width the page no longer allows (the
    /// paper or the margins changed since) counts as that limit. Returns
    /// `false` when that block is not an image or the stored width does
    /// not change, as for a step up at the limit.
    pub fn step_image_width(&mut self, index: usize, steps: i32) -> bool {
        let Some(image) = self.doc.blocks.get(index).and_then(Block::as_image) else {
            return false;
        };
        let drawn = image.width.min(self.max_image_width(image));
        self.set_image_width(index, drawn.stepped(steps))
    }

    /// Gives the image at block `index` its own size on the page
    /// ([`EditorState::original_image_width`]). Returns `false` when that
    /// block is not an image or already has that width.
    pub fn set_image_original_size(&mut self, index: usize) -> bool {
        let Some(image) = self.doc.blocks.get(index).and_then(Block::as_image) else {
            return false;
        };
        self.set_image_width(index, self.original_image_width(image))
    }

    /// Sets what the image at block `index` shows, without moving the
    /// selection. The text is kept as an image keeps a description (one
    /// line, trimmed, no control characters, at most
    /// [`Image::MAX_DESCRIPTION_CHARS`] characters); an empty one removes
    /// it. Returns `false` when that block is not an image or the
    /// description stays what it is. One undo step.
    pub fn set_image_description(&mut self, index: usize, description: &str) -> bool {
        let Some(image) = self.doc.blocks.get(index).and_then(Block::as_image) else {
            return false;
        };
        let mut image = image.clone();
        image.description = description.to_string();
        self.edit(EditKind::Other, |ed| {
            ed.update_block(index, |b| b.set_kind(BlockKind::Image(image)));
        })
    }

    /// With the caret on a void block (a divider, a page break, an image):
    /// inserts an empty paragraph before it (`above`) or after it and puts
    /// the caret there. The void block keeps its id. One undo step.
    /// Returns `false`, and does nothing, when the selection is not a caret
    /// on a void block.
    ///
    /// "Below" is what Enter and typing on a void block do; "above" is
    /// what shift-enter does ([`EditorState::insert_soft_break`]), and the
    /// only way to a line above a void block that is the first block.
    pub fn insert_paragraph_beside_void(&mut self, above: bool) -> bool {
        let index = self.selection.head.block;
        if !self.selection.is_collapsed() || !self.doc.blocks[index].is_void() {
            return false;
        }
        self.edit(EditKind::Other, |ed| {
            let at = if above { index } else { index + 1 };
            ed.splice(at, 0, vec![Block::new(BlockKind::Paragraph)]);
            ed.caret_to(Position::new(at, 0));
        })
    }

    /// Takes the block at `from` out and puts it before the block that was
    /// at `to` (both are indices of the document before the move; `to` is
    /// clamped to `0..=block_count`, and `block_count` is the end). The
    /// block keeps its id and its content, and so do the blocks it passes.
    /// The selection goes with the text it is in: a caret on the moved
    /// block stays on it. One undo step.
    ///
    /// Returns `false`, and does nothing, when `from` is no block or the
    /// block would stay where it is (`to == from` and `to == from + 1`).
    pub fn move_block_to(&mut self, from: usize, to: usize) -> bool {
        let n = self.doc.blocks.len();
        let to = to.min(n);
        if from >= n || to == from || to == from + 1 {
            return false;
        }
        self.edit(EditKind::Other, |ed| {
            // One splice over the blocks between the two places, so undo
            // and the reported change cover exactly what moved.
            let (start, end, landed) = if to > from {
                (from, to, to - 1)
            } else {
                (to, from + 1, to)
            };
            let mut blocks: Vec<Block> = ed.doc.blocks[start..end].to_vec();
            if to > from {
                blocks.rotate_left(1);
            } else {
                blocks.rotate_right(1);
            }
            ed.splice(start, end - start, blocks);
            let shift = |p: Position| {
                let block = if p.block == from {
                    landed
                } else if to > from && (from + 1..to).contains(&p.block) {
                    p.block - 1
                } else if to < from && (to..from).contains(&p.block) {
                    p.block + 1
                } else {
                    p.block
                };
                Position::new(block, p.offset)
            };
            let s = ed.selection;
            ed.set_sel(Selection::new(shift(s.anchor), shift(s.head)));
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
