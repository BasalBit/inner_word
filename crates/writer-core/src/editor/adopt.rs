//! Taking over a later state of the same document that was made somewhere
//! else.
//!
//! The app holds a document in an [`EditorState`]; an agent changes the
//! stored copy of it through the command line tool while the app is open.
//! [`EditorState::adopt`] brings the state to that later copy without
//! throwing anything away: the blocks that did not change stay the blocks
//! they are, the change is one step of undo on top of the steps that are
//! there, and the selection stays on the text it was on.

use crate::attrs::CommentId;
use crate::block::{Block, BlockId};
use crate::document::Document;
use crate::history::EditKind;
use crate::position::{Position, Selection};

use super::EditorState;

/// What [`EditorState::adopt`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Adopted {
    /// Blocks or comment threads changed, and the change is one step of
    /// undo (the newest).
    pub undo_step: bool,
    /// The document settings changed. They are not part of the undo
    /// history.
    pub settings: bool,
}

impl Adopted {
    /// Whether anything changed at all.
    pub fn changed(&self) -> bool {
        self.undo_step || self.settings
    }
}

/// Where a selection end is before a document is adopted, in terms that
/// still mean something afterwards.
struct Held {
    /// The id of the block it is in.
    id: BlockId,
    /// The text of that block.
    text: String,
    /// Where it is.
    position: Position,
}

impl Held {
    fn of(document: &Document, position: Position) -> Held {
        let block = &document.blocks[position.block];
        Held {
            id: block.id,
            text: block.text.clone(),
            position,
        }
    }

    /// The same place in `document`: in the block with the same id, moved
    /// with the text around it when the text of the block changed; at the
    /// same block index and offset when that block is gone. Clamped.
    fn in_document(&self, document: &Document) -> Position {
        let position = match document.block_index(self.id) {
            Some(index) => Position::new(
                index,
                moved_offset(
                    &self.text,
                    &document.blocks[index].text,
                    self.position.offset,
                ),
            ),
            None => self.position,
        };
        document.clamp_position(position)
    }
}

/// Where byte `offset` of `old` is in `new`: unchanged in what the two
/// texts share at the start, moved by the difference in length in what
/// they share at the end, and at the end of what was put in between
/// otherwise.
fn moved_offset(old: &str, new: &str, offset: usize) -> usize {
    if old == new {
        return offset;
    }
    let prefix = old
        .char_indices()
        .zip(new.chars())
        .find(|((_, a), b)| a != b)
        .map(|((index, _), _)| index)
        .unwrap_or(old.len().min(new.len()));
    let suffix = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum::<usize>();
    if offset <= prefix {
        offset
    } else if offset >= old.len() - suffix {
        (offset + new.len()).saturating_sub(old.len())
    } else {
        new.len() - suffix
    }
}

impl EditorState {
    /// Brings the document to `target`: a later state of the same
    /// document, made elsewhere from the document as it is here (the app
    /// saved it, something changed the stored copy, the app read it back).
    ///
    /// - **Blocks that did not change stay.** The blocks at the start and
    ///   at the end that are the same in both (same id, same content) are
    ///   not touched; what lies between is replaced, block by block where
    ///   the blocks there are the same ones with other content.
    /// - **One step of undo.** The change of the blocks and of the comment
    ///   threads is one transaction on top of the history that is there:
    ///   undo takes it out again and the steps before it are as they
    ///   were. An undo group that is open (an insert session of Vim) is
    ///   cut in two around it, so the adopted change never becomes part of
    ///   somebody's typing.
    /// - **The selection stays on its text.** Each end stays in its block
    ///   (found by id), and moves with the text when text before it in
    ///   that block was changed; when its block is gone it stays at the
    ///   same block index and offset, clamped.
    /// - **Ids.** The blocks and threads of `target` come in with the ids
    ///   they have, and ids handed out later are above them.
    /// - **Settings** are replaced when they differ, outside the undo
    ///   history like every change of settings.
    ///
    /// Does nothing inside a transaction. Reports the change like any
    /// other ([`EditorState::take_changes`]).
    pub fn adopt(&mut self, target: Document) -> Adopted {
        if self.tx.is_some() {
            return Adopted::default();
        }
        let mut adopted = Adopted::default();

        // Ids handed out from here on must not be ones `target` has used.
        self.doc.next_block_id = self.doc.next_block_id.max(target.next_block_id);
        self.doc.next_comment_id = self.doc.next_comment_id.max(target.next_comment_id);

        let Document {
            blocks: new_blocks,
            comments: mut new_threads,
            settings,
            ..
        } = target;

        // The blocks that are the same at both ends.
        let old_len = self.doc.blocks.len();
        let new_len = new_blocks.len();
        let prefix = self
            .doc
            .blocks
            .iter()
            .zip(&new_blocks)
            .take_while(|(old, new)| old == new)
            .count();
        let suffix = self.doc.blocks[prefix..]
            .iter()
            .rev()
            .zip(new_blocks[prefix..].iter().rev())
            .take_while(|(old, new)| old == new)
            .count();
        let blocks_differ = prefix < old_len || prefix < new_len;

        // Threads to set (new, or not what they were) and threads that are
        // gone.
        let gone: Vec<CommentId> = self
            .doc
            .comments
            .keys()
            .filter(|id| !new_threads.contains_key(id))
            .copied()
            .collect();
        new_threads.retain(|id, thread| self.doc.comments.get(id) != Some(&*thread));

        if blocks_differ || !gone.is_empty() || !new_threads.is_empty() {
            let anchor = Held::of(&self.doc, self.selection.anchor);
            let head = Held::of(&self.doc, self.selection.head);

            // Not part of an undo group that happens to be open.
            let depth = self.history.group_depth;
            self.history.force_end_group();
            let changed = self.edit(EditKind::Other, |ed| {
                // Threads that are new or changed first, threads that are
                // gone last: an anchor in the text always has its thread,
                // here and when the step is undone.
                for (id, thread) in new_threads {
                    ed.set_thread(id, Some(thread));
                }
                if blocks_differ {
                    ed.replace_middle(prefix, old_len - prefix - suffix, new_blocks, suffix);
                }
                for id in gone {
                    ed.set_thread(id, None);
                }
                let selection =
                    Selection::new(anchor.in_document(&ed.doc), head.in_document(&ed.doc));
                ed.set_sel(selection);
            });
            self.history.close();
            self.history.group_depth = depth;
            self.history.group_started = false;
            adopted.undo_step = changed;
        }

        adopted.settings = self.set_settings(settings);
        adopted
    }

    /// Replaces the `remove` blocks at `start` with the blocks of `new`
    /// that lie between its first `start` and its last `suffix` blocks.
    /// Where the two runs are the same blocks one for one (same ids, same
    /// order), only the blocks whose content differs are replaced, so the
    /// step of undo holds those and nothing else.
    fn replace_middle(&mut self, start: usize, remove: usize, new: Vec<Block>, suffix: usize) {
        let end = new.len() - suffix;
        let mut insert: Vec<Block> = new
            .into_iter()
            .skip(start)
            .take(end - start)
            .map(|mut block| {
                // A stamp of this document, not of the copy it came from.
                block.mark_dirty();
                block
            })
            .collect();
        let same_blocks = insert.len() == remove
            && self.doc.blocks[start..start + remove]
                .iter()
                .zip(&insert)
                .all(|(old, new)| old.id == new.id);
        if !same_blocks {
            self.splice(start, remove, insert);
            return;
        }
        for (offset, block) in insert.drain(..).enumerate() {
            if self.doc.blocks[start + offset] != block {
                self.splice(start + offset, 1, vec![block]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockKind;

    fn state(texts: &[&str]) -> EditorState {
        EditorState::new(Document::from_blocks(
            texts.iter().map(|text| Block::paragraph(text)).collect(),
        ))
    }

    fn texts(state: &EditorState) -> Vec<String> {
        state
            .document()
            .blocks()
            .iter()
            .map(|block| block.text().to_string())
            .collect()
    }

    /// What somebody else makes of the document as it is in `state`: the
    /// stored form goes out, is changed there and comes back.
    fn elsewhere(state: &EditorState, change: impl FnOnce(&mut EditorState)) -> Document {
        let stored = state.document().to_json();
        let mut other = EditorState::new(Document::from_json(&stored).unwrap());
        change(&mut other);
        Document::from_json(&other.document().to_json()).unwrap()
    }

    fn caret(state: &mut EditorState, block: usize, offset: usize) {
        state.set_caret(Position::new(block, offset));
    }

    #[test]
    fn a_changed_paragraph_comes_in_as_one_step_of_undo() {
        let mut state = state(&["One", "Two", "Three"]);
        caret(&mut state, 0, 3);
        state.insert_text(" more");
        let target = elsewhere(&state, |other| {
            caret(other, 1, 3);
            other.insert_text(" and a half");
        });
        let ids: Vec<_> = state.document().blocks().iter().map(|b| b.id()).collect();
        let depth = state.undo_depth();
        state.take_changes();

        let adopted = state.adopt(target.clone());

        assert_eq!(
            adopted,
            Adopted {
                undo_step: true,
                settings: false
            }
        );
        assert_eq!(state.document(), &target);
        assert_eq!(texts(&state), ["One more", "Two and a half", "Three"]);
        assert_eq!(state.undo_depth(), depth + 1);
        // Only the block that changed was replaced.
        let change = state.take_changes().unwrap();
        let splice = change.blocks.unwrap();
        assert_eq!((splice.start, splice.old_len, splice.new_len), (1, 1, 1));
        let after: Vec<_> = state.document().blocks().iter().map(|b| b.id()).collect();
        assert_eq!(ids, after);
        state.check_invariants().unwrap();

        // Undo takes the adopted change out, and then the typing before it.
        assert!(state.undo());
        assert_eq!(texts(&state), ["One more", "Two", "Three"]);
        assert!(state.undo());
        assert_eq!(texts(&state), ["One", "Two", "Three"]);
        assert!(state.redo());
        assert!(state.redo());
        assert_eq!(state.document(), &target);
        state.check_invariants().unwrap();
    }

    #[test]
    fn the_same_document_changes_nothing() {
        let mut state = state(&["One", "Two"]);
        let target = elsewhere(&state, |_| {});
        let depth = state.undo_depth();
        let revision = state.revision();
        assert_eq!(state.adopt(target), Adopted::default());
        assert_eq!(state.undo_depth(), depth);
        assert_eq!(state.revision(), revision);
        assert!(state.take_changes().is_none());
    }

    #[test]
    fn the_caret_stays_on_its_text() {
        // Text put in before the caret, in its own paragraph.
        let mut state = state(&["Alpha", "The harbour was quiet.", "Omega"]);
        state.set_selection(Selection::new(Position::new(1, 12), Position::new(1, 15)));
        let target = elsewhere(&state, |other| {
            caret(other, 1, 4);
            other.insert_text("old ");
        });
        state.adopt(target);
        assert_eq!(texts(&state)[1], "The old harbour was quiet.");
        assert_eq!(
            state.selection(),
            Selection::new(Position::new(1, 16), Position::new(1, 19))
        );
        assert_eq!(state.selected_text(), "was");

        // Text put in after the caret: it does not move.
        let target = elsewhere(&state, |other| {
            caret(other, 1, 26);
            other.insert_text(" Very.");
        });
        state.adopt(target);
        assert_eq!(state.selected_text(), "was");

        // A paragraph put in above: the caret follows its block.
        let target = elsewhere(&state, |other| {
            caret(other, 0, 5);
            other.insert_paragraph_break();
            other.insert_text("Beta");
        });
        state.adopt(target);
        assert_eq!(texts(&state)[1], "Beta");
        assert_eq!(state.selection().head.block, 2);
        assert_eq!(state.selected_text(), "was");
        state.check_invariants().unwrap();
    }

    #[test]
    fn a_caret_inside_what_was_replaced_goes_to_its_end() {
        let mut state = state(&["The quick brown fox"]);
        caret(&mut state, 0, 7);
        let target = elsewhere(&state, |other| {
            other.set_selection(Selection::new(Position::new(0, 4), Position::new(0, 15)));
            other.insert_text("slow");
        });
        state.adopt(target);
        assert_eq!(texts(&state), ["The slow fox"]);
        assert_eq!(state.selection(), Selection::caret(Position::new(0, 8)));
    }

    #[test]
    fn a_whole_new_text_keeps_the_caret_where_the_text_allows() {
        let mut state = state(&["One", "Two", "Three"]);
        caret(&mut state, 2, 4);
        let target = Document::from_blocks(vec![
            Block::with_text(BlockKind::heading(1), "Other"),
            Block::paragraph("Text"),
        ]);
        let adopted = state.adopt(target.clone());
        assert!(adopted.undo_step);
        assert_eq!(texts(&state), ["Other", "Text"]);
        // The last block there is, and as far into it as it goes.
        assert_eq!(state.selection(), Selection::caret(Position::new(1, 4)));
        state.check_invariants().unwrap();
        assert!(state.undo());
        assert_eq!(texts(&state), ["One", "Two", "Three"]);
        assert_eq!(state.selection(), Selection::caret(Position::new(2, 4)));
        state.check_invariants().unwrap();
    }

    #[test]
    fn blocks_changed_far_apart_are_replaced_one_by_one() {
        let mut state = state(&["a cat", "b", "c", "d", "a cat again"]);
        let target = elsewhere(&state, |other| {
            other.replace_all("cat", Default::default(), "dog");
        });
        let untouched: Vec<u64> = state.document().blocks()[1..4]
            .iter()
            .map(|block| block.revision())
            .collect();
        state.adopt(target.clone());
        assert_eq!(state.document(), &target);
        // The blocks between the two that changed are the blocks they were.
        let after: Vec<u64> = state.document().blocks()[1..4]
            .iter()
            .map(|block| block.revision())
            .collect();
        assert_eq!(untouched, after);
        assert!(state.undo());
        assert_eq!(texts(&state), ["a cat", "b", "c", "d", "a cat again"]);
    }

    #[test]
    fn comments_come_and_go_with_their_anchors() {
        let mut state = state(&["The Maren left at dawn.", "Nobody waved."]);
        // A thread of the writer's own, to be deleted elsewhere.
        state.set_selection(Selection::new(Position::new(1, 0), Position::new(1, 6)));
        let own = state.add_comment("Who?", "Me", 1).unwrap();
        let depth = state.undo_depth();

        let mut added = None;
        let target = elsewhere(&state, |other| {
            other.set_selection(Selection::new(Position::new(0, 4), Position::new(0, 9)));
            added = other.add_comment("The same ship?", "Claude", 2);
            other.delete_comment(own);
        });
        let added = added.unwrap();
        let adopted = state.adopt(target.clone());
        assert!(adopted.undo_step);
        assert_eq!(state.document(), &target);
        assert!(state.document().comment(own).is_none());
        assert_eq!(state.document().comment(added).unwrap().author, "Claude");
        assert_eq!(state.document().comment_ranges(added).len(), 1);
        assert!(state.take_changes().unwrap().comments);
        assert_eq!(state.undo_depth(), depth + 1);
        state.check_invariants().unwrap();

        // A thread started here afterwards gets an id of its own.
        state.set_selection(Selection::new(Position::new(1, 7), Position::new(1, 12)));
        let next = state.add_comment("Why not?", "Me", 3).unwrap();
        assert_ne!(next, added);
        assert!(state.undo());

        assert!(state.undo());
        assert!(state.document().comment(added).is_none());
        assert_eq!(state.document().comment(own).unwrap().body, "Who?");
        assert_eq!(state.document().comment_ranges(own).len(), 1);
        state.check_invariants().unwrap();
    }

    #[test]
    fn a_reply_alone_is_a_step_of_undo() {
        let mut state = state(&["The Maren left at dawn."]);
        state.set_selection(Selection::new(Position::new(0, 4), Position::new(0, 9)));
        let id = state.add_comment("Which ship?", "Me", 1).unwrap();
        let target = elsewhere(&state, |other| {
            other.add_reply(id, "The one from chapter 3.", "Claude", 2);
            other.resolve_comment(id);
        });
        state.take_changes();
        let adopted = state.adopt(target);
        assert!(adopted.undo_step);
        let thread = state.document().comment(id).unwrap();
        assert_eq!(thread.replies.len(), 1);
        assert!(thread.resolved);
        let change = state.take_changes().unwrap();
        assert!(change.comments);
        assert!(change.blocks.is_none());
        assert!(state.undo());
        let thread = state.document().comment(id).unwrap();
        assert!(thread.replies.is_empty());
        assert!(!thread.resolved);
    }

    #[test]
    fn settings_come_in_outside_the_undo_history() {
        let mut state = state(&["One"]);
        let target = elsewhere(&state, |other| {
            other.set_page_view(true);
        });
        let depth = state.undo_depth();
        let adopted = state.adopt(target);
        assert_eq!(
            adopted,
            Adopted {
                undo_step: false,
                settings: true
            }
        );
        assert!(state.document().settings().page_view);
        assert_eq!(state.undo_depth(), depth);
        assert!(state.take_changes().unwrap().settings);
    }

    #[test]
    fn an_open_undo_group_is_cut_in_two_around_the_adopted_change() {
        let mut state = state(&["One", "Two"]);
        caret(&mut state, 0, 3);
        // An insert session of Vim: everything typed is one step.
        state.begin_undo_group();
        state.insert_text(" a");
        state.insert_text(" b");
        let target = elsewhere(&state, |other| {
            caret(other, 1, 3);
            other.insert_text("!");
        });
        state.adopt(target);
        state.insert_text(" c");
        state.insert_text(" d");
        state.end_undo_group();
        assert_eq!(texts(&state), ["One a b c d", "Two!"]);

        assert!(state.undo());
        assert_eq!(texts(&state), ["One a b", "Two!"]);
        assert!(state.undo());
        assert_eq!(texts(&state), ["One a b", "Two"]);
        assert!(state.undo());
        assert_eq!(texts(&state), ["One", "Two"]);
        assert!(!state.undo());
    }

    #[test]
    fn typing_after_an_adopted_change_is_a_step_of_its_own() {
        let mut state = state(&["One", "Two"]);
        caret(&mut state, 0, 3);
        state.insert_text("x");
        let target = elsewhere(&state, |other| {
            caret(other, 1, 3);
            other.insert_text("!");
        });
        state.adopt(target);
        state.insert_text("y");
        assert_eq!(texts(&state), ["Onexy", "Two!"]);
        assert!(state.undo());
        assert_eq!(texts(&state), ["Onex", "Two!"]);
        assert!(state.undo());
        assert_eq!(texts(&state), ["Onex", "Two"]);
        assert!(state.undo());
        assert_eq!(texts(&state), ["One", "Two"]);
    }

    #[test]
    fn ids_handed_out_afterwards_are_new() {
        let mut state = state(&["One"]);
        let target = elsewhere(&state, |other| {
            caret(other, 0, 3);
            other.insert_paragraph_break();
            other.insert_text("Two");
            other.insert_paragraph_break();
            other.insert_text("Three");
        });
        state.adopt(target);
        caret(&mut state, 2, 5);
        state.insert_paragraph_break();
        state.insert_text("Four");
        state.check_invariants().unwrap();
        let mut ids: Vec<_> = state.document().blocks().iter().map(|b| b.id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 4);
    }

    #[test]
    fn offsets_move_with_the_text_around_them() {
        assert_eq!(moved_offset("abc", "abc", 2), 2);
        // Put in before.
        assert_eq!(moved_offset("abc", "xxabc", 2), 4);
        // Put in after.
        assert_eq!(moved_offset("abc", "abcxx", 2), 2);
        // At the very place: the caret stays before what was put in.
        assert_eq!(moved_offset("abc", "abxxc", 2), 2);
        // Taken out around it.
        assert_eq!(moved_offset("abcdef", "af", 3), 1);
        // Characters of several bytes.
        assert_eq!(moved_offset("é à", "é ü à", 3), 3);
        assert_eq!(moved_offset("é à", "é ü à", 5), 8);
        assert_eq!(moved_offset("", "abc", 0), 0);
        assert_eq!(moved_offset("abc", "", 3), 0);
    }
}
