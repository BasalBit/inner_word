//! Bringing a document that was changed somewhere else into the open
//! editor.
//!
//! The app holds the open document in the editor and saves all of it. When
//! something else changes the stored copy while the app is open (an agent,
//! through the command line tool: the app carries the change out itself and
//! reads the document back), the editor takes over the result with
//! [`Editor::adopt_document`] instead of opening it anew: the caret, the
//! selection and the view stay, and the undo history stays with one more
//! step on top.

use gpui::Context;
use innerword_core::{Adopted, Document};

use crate::{Autoscroll, Editor};

impl Editor {
    /// Whether text is being composed with an input method right now
    /// (marked text: a dead key waiting for its letter, a candidate that
    /// has not been confirmed). A change from elsewhere ends a composition
    /// where it stands; whoever can wait asks this first.
    pub fn is_composing(&self) -> bool {
        self.marked.is_some()
    }

    /// Brings the document to `document`: a later state of the same
    /// document that was made elsewhere from the document as it is in the
    /// editor now. See `innerword_core::EditorState::adopt` for what is kept:
    /// the blocks that did not change, the undo history (the change is one
    /// more step, which `Undo` takes out again), the selection on its
    /// text.
    ///
    /// The view does not move: the scroll position is anchored to a block
    /// and follows it when blocks are added or removed above it, and the
    /// caret is not scrolled into view.
    ///
    /// Emits what any change emits ([`crate::EditorEvent::Edited`],
    /// `SettingsChanged`, `SelectionChanged`), once, and nothing when the
    /// document is the same already. Works in read-only mode, like
    /// [`Editor::edit`]. A composition in progress ends (see
    /// [`Editor::is_composing`]).
    pub fn adopt_document(&mut self, document: Document, cx: &mut Context<Self>) -> Adopted {
        self.edit_with(Autoscroll::None, |state| state.adopt(document), cx)
    }

    /// Undoes the newest `steps` steps of the undo history in one go (one
    /// batch of events), without scrolling to the caret. Returns how many
    /// steps were undone. For taking out again what
    /// [`Editor::adopt_document`] brought in.
    pub fn undo_steps(&mut self, steps: usize, cx: &mut Context<Self>) -> usize {
        self.edit_with(
            Autoscroll::None,
            |state| (0..steps).take_while(|_| state.undo()).count(),
            cx,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use gpui::{Entity, EntityInputHandler, TestAppContext, VisualTestContext};
    use innerword_core::{Block, Document, EditorState, Position, Selection};
    use innerword_ui::theme::{self, ThemeMode};

    use crate::{Editor, EditorConfig, EditorEvent};

    fn open(
        document: Document,
        cx: &mut TestAppContext,
    ) -> (Entity<Editor>, &mut VisualTestContext) {
        cx.update(|cx| {
            theme::init(ThemeMode::Light, cx);
            crate::init(cx);
        });
        let (editor, cx) = cx.add_window_view(|window, cx| {
            let editor = Editor::new(document, EditorConfig::full(), window, cx);
            editor.focus(window);
            editor
        });
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        (editor, cx)
    }

    fn paragraphs(count: usize) -> Document {
        Document::from_blocks(
            (0..count)
                .map(|ix| Block::paragraph(&format!("Paragraph number {ix}")))
                .collect(),
        )
    }

    /// What somebody else makes of the document as the editor has it.
    fn elsewhere(
        editor: &Entity<Editor>,
        cx: &mut VisualTestContext,
        change: impl FnOnce(&mut EditorState),
    ) -> Document {
        let stored = editor.read_with(cx, |editor, _| editor.document().to_json());
        let mut other = EditorState::new(Document::from_json(&stored).unwrap());
        change(&mut other);
        Document::from_json(&other.document().to_json()).unwrap()
    }

    #[gpui::test]
    fn an_adopted_change_is_one_event_and_one_step_of_undo(cx: &mut TestAppContext) {
        let (editor, cx) = open(paragraphs(3), cx);
        cx.simulate_keystrokes("cmd-down");
        cx.simulate_input("typed");
        let events = Rc::new(RefCell::new(Vec::new()));
        cx.update(|_, cx| {
            let events = events.clone();
            cx.subscribe(&editor, move |_, event: &EditorEvent, _| {
                events.borrow_mut().push(event.clone());
            })
            .detach();
        });

        let target = elsewhere(&editor, cx, |other| {
            other.set_caret(Position::new(0, 0));
            other.insert_text("Changed: ");
        });
        let caret = editor.read_with(cx, |editor, _| editor.selection());
        let adopted = editor.update(cx, |editor, cx| editor.adopt_document(target.clone(), cx));
        cx.run_until_parked();

        assert!(adopted.undo_step);
        editor.read_with(cx, |editor, _| {
            assert_eq!(editor.document(), &target);
            // The caret is where it was: at the end of the last paragraph.
            assert_eq!(editor.selection(), caret);
        });
        let edited = events
            .borrow()
            .iter()
            .filter(|event| matches!(event, EditorEvent::Edited { .. }))
            .count();
        assert_eq!(edited, 1, "{:?}", events.borrow());

        // The undo key takes the adopted change out, then the typing.
        cx.simulate_keystrokes("cmd-z");
        editor.read_with(cx, |editor, _| {
            assert_eq!(
                editor.document().plain_text(),
                "Paragraph number 0\nParagraph number 1\nParagraph number 2typed"
            );
        });
        cx.simulate_keystrokes("cmd-z");
        editor.read_with(cx, |editor, _| {
            assert_eq!(editor.document(), &paragraphs(3));
        });
    }

    #[gpui::test]
    fn the_view_stays_on_its_text_when_paragraphs_are_added_above(cx: &mut TestAppContext) {
        let (editor, cx) = open(paragraphs(400), cx);
        // Somewhere in the middle, with the caret there.
        editor.update(cx, |editor, cx| {
            editor.edit(|state| state.set_caret(Position::new(200, 4)), cx);
        });
        cx.run_until_parked();
        let (block, offset) = editor.read_with(cx, |editor, _| editor.scroll_anchor());
        assert!(block > 100, "scrolled to the caret: {block}");

        let target = elsewhere(&editor, cx, |other| {
            other.set_caret(Position::new(3, 0));
            other.insert_text("One more.");
            other.insert_paragraph_break();
            other.insert_text("And another.");
            other.insert_paragraph_break();
        });
        editor.update(cx, |editor, cx| editor.adopt_document(target, cx));
        cx.run_until_parked();

        editor.read_with(cx, |editor, _| {
            assert_eq!(editor.document().block_count(), 402);
            // The same paragraph is at the top of the view, and the caret
            // is in the paragraph it was in.
            assert_eq!(editor.scroll_anchor(), (block + 2, offset));
            assert_eq!(editor.selection(), Selection::caret(Position::new(202, 4)));
            assert_eq!(
                editor.document().blocks()[202].text(),
                "Paragraph number 200"
            );
        });
    }

    #[gpui::test]
    fn several_steps_are_undone_in_one_go(cx: &mut TestAppContext) {
        let (editor, cx) = open(paragraphs(2), cx);
        for round in 0..3 {
            let target = elsewhere(&editor, cx, |other| {
                other.set_caret(Position::new(1, 0));
                other.insert_text(&format!("{round} "));
            });
            editor.update(cx, |editor, cx| editor.adopt_document(target, cx));
        }
        editor.read_with(cx, |editor, _| {
            assert_eq!(
                editor.document().blocks()[1].text(),
                "2 1 0 Paragraph number 1"
            );
        });
        let undone = editor.update(cx, |editor, cx| editor.undo_steps(3, cx));
        assert_eq!(undone, 3);
        editor.read_with(cx, |editor, _| {
            assert_eq!(editor.document(), &paragraphs(2))
        });
        // No more than there are.
        let undone = editor.update(cx, |editor, cx| editor.undo_steps(3, cx));
        assert_eq!(undone, 0);
    }

    #[gpui::test]
    fn a_composition_is_reported_while_it_lasts(cx: &mut TestAppContext) {
        let (editor, cx) = open(Document::from_blocks(vec![Block::paragraph("caf")]), cx);
        cx.simulate_keystrokes("cmd-down");
        assert!(!editor.read_with(cx, |editor, _| editor.is_composing()));
        editor.update_in(cx, |editor, window, cx| {
            editor.replace_and_mark_text_in_range(None, "\u{b4}", Some(1..1), window, cx);
        });
        assert!(editor.read_with(cx, |editor, _| editor.is_composing()));
        editor.update_in(cx, |editor, window, cx| {
            editor.replace_text_in_range(None, "\u{e9}", window, cx);
        });
        assert!(!editor.read_with(cx, |editor, _| editor.is_composing()));
    }
}
