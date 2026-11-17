//! Copy, cut and paste.

use crate::block::{Block, BlockId};
use crate::fragment::Fragment;
use crate::history::EditKind;
use crate::position::Position;

use super::blocks::clean_fragment_blocks;
use super::text_cmds::Dir;
use super::{EditorState, inherited_attrs};

/// Joins two neighbouring blocks if possible. A non-void block with text
/// absorbs the other's text (keeping its own id and kind); an empty non-void
/// block gives way to the other block (which takes over its id); void blocks
/// never merge.
fn join(a: Block, b: Block) -> Vec<Block> {
    if a.is_void() || b.is_void() {
        if !a.is_void() && a.is_empty() {
            return vec![b];
        }
        if !b.is_void() && b.is_empty() {
            return vec![a];
        }
        return vec![a, b];
    }
    if a.is_empty() {
        let mut r = b;
        r.id = a.id;
        r.mark_dirty();
        return vec![r];
    }
    let mut r = a;
    r.append(&b);
    vec![r]
}

impl EditorState {
    /// Whether [`EditorState::copy`] would give something: the selection is
    /// a range, or the caret is on an image.
    pub fn has_something_to_copy(&self) -> bool {
        !self.selection.is_collapsed() || self.selected_image().is_some()
    }

    /// The selection as a [`Fragment`] (without comment anchors). With the
    /// caret on an image ([`EditorState::selected_image`]) that is the
    /// image: a fragment of its one block, with its width, its alignment
    /// and its description, which [`EditorState::paste`] puts back as a
    /// block of its own. A caret anywhere else gives an empty fragment.
    pub fn copy(&self) -> Fragment {
        if let Some((index, _)) = self.selected_image() {
            return self.doc.block_fragment(index..index + 1);
        }
        self.doc.fragment(self.selection.range())
    }

    /// Copies the selection and deletes it (one undo step). With the caret
    /// on an image that is the image: its block is removed and the caret
    /// goes where Backspace on it leaves it (the end of the block before).
    pub fn cut(&mut self) -> Fragment {
        let f = self.copy();
        if let Some((index, _)) = self.selected_image() {
            self.edit(EditKind::Other, |ed| ed.remove_void(index, Dir::Backward));
        } else {
            self.delete_selection();
        }
        f
    }

    /// Pastes `fragment` at the selection (replacing it), as one undo step.
    ///
    /// - Into a code block: the fragment's plain text is inserted.
    /// - A single-block fragment is inlined: its text and attributes go into
    ///   the current block, which keeps its kind.
    /// - A multi-block fragment splits the current block at the caret. The
    ///   first fragment block joins the head (the head keeps its kind unless
    ///   it is empty, then it takes the fragment block's kind); the last
    ///   fragment block joins the tail (the joined block takes the fragment
    ///   block's kind unless that block is empty); blocks in between are
    ///   inserted whole. Void blocks never merge.
    ///
    /// Comment anchors in the fragment are dropped. The caret ends after the
    /// pasted content.
    pub fn paste(&mut self, fragment: &Fragment) -> bool {
        self.edit(EditKind::Other, |ed| ed.paste_impl(fragment))
    }

    fn paste_impl(&mut self, fragment: &Fragment) {
        if !self.selection.is_collapsed() {
            self.delete_selection_impl();
        }
        let blocks = clean_fragment_blocks(fragment);
        if blocks.is_empty() {
            return;
        }
        let pos = self.leave_void(self.selection.head);
        self.pending = None;
        let target = &self.doc.blocks[pos.block];
        if target.kind.is_code() {
            let text = fragment.to_plain_text();
            let attrs = inherited_attrs(target, pos.offset);
            let p = self.insert_in_block(pos, &text, &attrs);
            self.caret_to(p);
            return;
        }
        if blocks.len() == 1 && !blocks[0].is_void() {
            let f = &blocks[0];
            let len = f.len();
            self.update_block(pos.block, |b| b.insert_content(pos.offset, f));
            self.caret_to(Position::new(pos.block, pos.offset + len));
            return;
        }
        let mut left = target.clone();
        let right = left.split_off(pos.offset);
        let n = blocks.len();
        let mut it = blocks.into_iter();
        let first = it.next().expect("non-empty");
        let mut out = join(left, first);
        for _ in 1..n.saturating_sub(1) {
            out.push(it.next().expect("middle block"));
        }
        let last = if n > 1 {
            it.next().expect("last block")
        } else {
            out.pop().expect("joined block")
        };
        let caret_rel;
        if last.is_void() {
            out.push(last);
            out.push(right);
            caret_rel = (out.len() - 1, 0);
        } else if last.is_empty() {
            out.push(right);
            caret_rel = (out.len() - 1, 0);
        } else {
            let mut m = last;
            let l = m.len();
            m.append(&right);
            out.push(m);
            caret_rel = (out.len() - 1, l);
        }
        // The pasted-into block's id survives: when its head gave way to a
        // void block, the block holding its tail takes the id.
        let target_id = self.doc.blocks[pos.block].id;
        if !out.iter().any(|b| b.id == target_id)
            && let Some(b) = out
                .iter_mut()
                .rev()
                .find(|b| !b.is_void() && b.id == BlockId::UNASSIGNED)
        {
            b.id = target_id;
        }
        self.splice(pos.block, 1, out);
        self.caret_to(Position::new(pos.block + caret_rel.0, caret_rel.1));
    }
}
