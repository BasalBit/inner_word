//! The spelling keys against stand-in hooks: a "spell checker" for which a
//! fixed list of words is misspelled, reading and editing the real editor
//! entity as the application's checker does.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gpui::{App, Entity, TestAppContext, VisualTestContext, Window};
use writer_core::{Block, BlockKind, Document, Position, TextRange};
use writer_editor::Editor;
use writer_vim_mode::{Mode, VimEvent, VimMessage, VimSpelling, VimSuggestions};

use super::{Rig, at, doc, open, setup};

struct FakeSpelling {
    editor: Entity<Editor>,
    enabled: Cell<bool>,
    learned: RefCell<Vec<String>>,
    /// Every call of a hook, in order.
    log: RefCell<Vec<String>>,
}

impl FakeSpelling {
    /// The misspelled words of the document, in order. Reads the editor
    /// entity: a hook called while the editor is being updated would
    /// panic here.
    fn marks(&self, cx: &App) -> Vec<(TextRange, String)> {
        let editor = self.editor.read(cx);
        let mut marks = Vec::new();
        for (index, block) in editor.document().blocks().iter().enumerate() {
            if matches!(block.kind(), BlockKind::CodeBlock { .. }) {
                continue;
            }
            let text = block.text();
            let mut start = None;
            for (offset, ch) in text.char_indices().chain([(text.len(), ' ')]) {
                match (start, ch.is_alphanumeric()) {
                    (None, true) => start = Some(offset),
                    (Some(from), false) => {
                        let word = &text[from..offset];
                        let bad = matches!(word, "teh" | "wrod" | "recieve");
                        if bad && !self.learned.borrow().iter().any(|l| l == word) {
                            marks.push((TextRange::in_block(index, from, offset), word.into()));
                        }
                        start = None;
                    }
                    _ => {}
                }
            }
        }
        marks
    }

    fn mark_at(&self, position: Position, cx: &App) -> Option<(TextRange, String)> {
        self.marks(cx)
            .into_iter()
            .find(|(range, _)| range.start <= position && position <= range.end)
    }

    fn note(&self, line: String) {
        self.log.borrow_mut().push(line);
    }
}

impl VimSpelling for FakeSpelling {
    fn is_enabled(&self, _: &App) -> bool {
        self.enabled.get()
    }

    fn misspelling_from(&self, from: Position, forward: bool, cx: &mut App) -> Option<TextRange> {
        self.note(format!(
            "{} {}:{}",
            if forward { "next" } else { "previous" },
            from.block,
            from.offset
        ));
        let marks = self.marks(cx);
        let found = if forward {
            marks.iter().find(|(r, _)| r.start > from).or(marks.first())
        } else {
            marks
                .iter()
                .rev()
                .find(|(r, _)| r.start < from)
                .or(marks.last())
        };
        found.map(|(range, _)| *range)
    }

    fn suggestions_at(&self, position: Position, cx: &mut App) -> Option<VimSuggestions> {
        self.note(format!(
            "suggestions {}:{}",
            position.block, position.offset
        ));
        let (range, word) = self.mark_at(position, cx)?;
        let suggestions: &[&str] = match word.as_str() {
            "teh" => &["the", "ten"],
            "wrod" => &["word"],
            _ => &[],
        };
        Some(VimSuggestions {
            range,
            suggestions: suggestions.iter().map(|s| s.to_string()).collect(),
        })
    }

    fn replace(&self, range: TextRange, replacement: &str, cx: &mut App) -> bool {
        self.note(format!("replace {replacement}"));
        self.editor.update(cx, |editor, cx| {
            editor.edit(|state| state.replace_range(range, replacement), cx)
        })
    }

    fn learn(&self, position: Position, word: &str, cx: &mut App) -> Option<String> {
        self.note(format!("learn {word}"));
        let (_, marked) = self.mark_at(position, cx)?;
        self.learned.borrow_mut().push(marked.clone());
        Some(marked)
    }

    fn unlearn(&self, _: Position, word: &str, _: &mut App) -> Option<String> {
        self.note(format!("unlearn {word}"));
        let mut learned = self.learned.borrow_mut();
        let index = learned.iter().position(|l| l == word)?;
        Some(learned.remove(index))
    }

    fn open_suggestions(&self, position: Position, window: &mut Window, cx: &mut App) -> bool {
        self.note(format!(
            "menu {}:{} caret {:?}",
            position.block,
            position.offset,
            self.editor.read(cx).selection().head.offset
        ));
        // The window is the editor's.
        assert!(self.editor.read(cx).is_focused(window));
        self.mark_at(position, cx).is_some()
    }
}

fn connect(rig: &Rig, cx: &mut VisualTestContext) -> Rc<FakeSpelling> {
    let fake = Rc::new(FakeSpelling {
        editor: rig.editor.clone(),
        enabled: Cell::new(true),
        learned: RefCell::default(),
        log: RefCell::default(),
    });
    let hooks = fake.clone();
    rig.vim.update(cx, |vim, _| {
        vim.set_spelling(Some(hooks as Rc<dyn VimSpelling>))
    });
    fake
}

fn message(rig: &Rig, cx: &mut VisualTestContext) -> Option<VimMessage> {
    rig.status(cx).message
}

fn bells(rig: &Rig) -> usize {
    rig.take_events()
        .iter()
        .filter(|event| matches!(event, VimEvent::Bell))
        .count()
}

const OFF: &str = "E756: Spell checking is off";

#[gpui::test]
fn without_a_spell_checker_the_keys_say_so(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["a teh b wrod c"]), cx);
    assert!(!rig.vim.read_with(cx, |vim, _| vim.has_spelling()));
    rig.take_events();
    for keys in ["] s", "[ s", "d ] s", "z =", "1 z =", "z g", "z u g"] {
        cx.simulate_keystrokes(keys);
        assert_eq!(message(&rig, cx), Some(VimMessage::error(OFF)), "{keys}");
        assert_eq!(bells(&rig), 1, "{keys}");
        assert_eq!(rig.head(cx), at(0, 0), "{keys}");
        assert_eq!(rig.status(cx).pending_keys, "", "{keys}");
    }
    assert_eq!(rig.text(cx), "a teh b wrod c");

    // Connected but switched off: the same, and nothing else is asked.
    let fake = connect(&rig, cx);
    fake.enabled.set(false);
    for keys in ["] s", "z =", "2 z =", "z g"] {
        cx.simulate_keystrokes(keys);
        assert_eq!(message(&rig, cx), Some(VimMessage::error(OFF)), "{keys}");
        assert_eq!(bells(&rig), 1, "{keys}");
    }
    assert!(fake.log.borrow().is_empty());

    // Switched on, it answers; disconnected again, off again.
    fake.enabled.set(true);
    cx.simulate_keystrokes("] s");
    assert_eq!(rig.head(cx), at(0, 2));
    assert_eq!(message(&rig, cx), None);
    rig.vim.update(cx, |vim, _| vim.set_spelling(None));
    cx.simulate_keystrokes("] s");
    assert_eq!(message(&rig, cx), Some(VimMessage::error(OFF)));
    assert_eq!(rig.head(cx), at(0, 2));
    // The other keys behind `z` are untouched.
    cx.simulate_keystrokes("z z");
    assert_eq!(message(&rig, cx), None);
}

#[gpui::test]
fn bracket_s_moves_between_misspelled_words(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["a teh b wrod c", "fine", "x recieve teh"]), cx);
    let fake = connect(&rig, cx);
    rig.take_events();

    cx.simulate_keystrokes("] s");
    assert_eq!(rig.head(cx), at(0, 2));
    assert_eq!(rig.mode(cx), Mode::Normal);
    cx.simulate_keystrokes("] s");
    assert_eq!(rig.head(cx), at(0, 8));
    cx.simulate_keystrokes("2 ] s");
    assert_eq!(rig.head(cx), at(2, 10));
    assert_eq!(message(&rig, cx), None);
    // Around the end of the document, with Vim's message.
    cx.simulate_keystrokes("] s");
    assert_eq!(rig.head(cx), at(0, 2));
    assert_eq!(
        message(&rig, cx),
        Some(VimMessage::info("search hit BOTTOM, continuing at TOP"))
    );
    cx.simulate_keystrokes("[ s");
    assert_eq!(rig.head(cx), at(2, 10));
    assert_eq!(
        message(&rig, cx),
        Some(VimMessage::info("search hit TOP, continuing at BOTTOM"))
    );
    // Backwards from inside a word: its start first.
    cx.simulate_keystrokes("l [ s");
    assert_eq!(rig.head(cx), at(2, 10));
    cx.simulate_keystrokes("3 [ s");
    assert_eq!(rig.head(cx), at(0, 2));
    assert_eq!(bells(&rig), 0);

    // A count far beyond the number of misspelled words costs one round.
    fake.log.borrow_mut().clear();
    cx.simulate_keystrokes("9 9 9 ] s");
    // 999 steps over 4 words: (999 - 1) % 4 = 2 words on from the first
    // one found (wrod).
    assert_eq!(rig.head(cx), at(2, 10));
    assert_eq!(fake.log.borrow().len(), 5, "{:?}", fake.log.borrow());

    // As the motion of an operator, repeated, and undone.
    cx.simulate_keystrokes("g g 0 d ] s");
    assert_eq!(rig.text(cx), "teh b wrod c\nfine\nx recieve teh");
    cx.simulate_keystrokes(".");
    assert_eq!(rig.text(cx), "wrod c\nfine\nx recieve teh");
    cx.simulate_keystrokes("u u");
    assert_eq!(rig.text(cx), "a teh b wrod c\nfine\nx recieve teh");
    cx.simulate_keystrokes("$ d [ s");
    assert_eq!(rig.text(cx), "a teh b c\nfine\nx recieve teh");
    cx.simulate_keystrokes("u");

    // In Visual mode the selection runs to the word.
    cx.simulate_keystrokes("0 v ] s");
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.cursor(cx), at(0, 2));
    assert_eq!(rig.selected_text(cx), "a t");
    cx.simulate_keystrokes("] s");
    assert_eq!(rig.selected_text(cx), "a teh b w");
    cx.simulate_keystrokes("escape");

    // Every question the engine asked had been asked of the checker
    // before the call, and the layout was there.
    assert_eq!(rig.vim.read_with(cx, |vim, _| vim.spell_misses()), 0);
    assert_eq!(rig.fallbacks(cx), 0);
    assert_eq!(bells(&rig), 0);
}

#[gpui::test]
fn nothing_misspelled_is_said_and_rings(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(
        Document::from_blocks(vec![
            Block::paragraph("all good here"),
            Block::with_text(BlockKind::code(), "teh wrod"),
        ]),
        cx,
    );
    connect(&rig, cx);
    rig.take_events();
    for keys in ["] s", "[ s", "4 ] s", "d ] s"] {
        cx.simulate_keystrokes(keys);
        assert_eq!(
            message(&rig, cx),
            Some(VimMessage::info("No misspelled words")),
            "{keys}"
        );
        assert_eq!(bells(&rig), 1, "{keys}");
        assert_eq!(rig.head(cx), at(0, 0));
    }
    assert_eq!(rig.text(cx), "all good here\nteh wrod");
}

#[gpui::test]
fn a_count_before_z_equals_takes_that_suggestion(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["a teh b wrod c recieve d"]), cx);
    let fake = connect(&rig, cx);
    rig.take_events();

    // On the word, anywhere in it.
    cx.simulate_keystrokes("4 | 1 z =");
    assert_eq!(rig.text(cx), "a the b wrod c recieve d");
    assert_eq!(
        rig.head(cx),
        at(0, 2),
        "the cursor is on the start of the word"
    );
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.caret_override(cx), None);
    assert_eq!(message(&rig, cx), None);
    assert_eq!(
        *fake.log.borrow(),
        ["suggestions 0:3", "replace the"],
        "one question, one replacement"
    );
    // One undo step, by Vim's own undo.
    cx.simulate_keystrokes("u");
    assert_eq!(rig.text(cx), "a teh b wrod c recieve d");
    cx.simulate_keystrokes("ctrl-r");
    assert_eq!(rig.text(cx), "a the b wrod c recieve d");
    cx.simulate_keystrokes("u");
    // The second suggestion.
    cx.simulate_keystrokes("0 ] s 2 z =");
    assert_eq!(rig.text(cx), "a ten b wrod c recieve d");
    cx.simulate_keystrokes("u");
    assert_eq!(bells(&rig), 0);

    // More than there are, none at all, and a word that is not marked.
    cx.simulate_keystrokes("0 ] s 3 z =");
    assert_eq!(
        message(&rig, cx),
        Some(VimMessage::error("Sorry, only 2 suggestions"))
    );
    cx.simulate_keystrokes("] s 2 z =");
    assert_eq!(
        message(&rig, cx),
        Some(VimMessage::error("Sorry, only 1 suggestion"))
    );
    cx.simulate_keystrokes("] s 1 z =");
    assert_eq!(
        message(&rig, cx),
        Some(VimMessage::error("Sorry, no suggestions"))
    );
    cx.simulate_keystrokes("0 1 z =");
    assert_eq!(
        message(&rig, cx),
        Some(VimMessage::error("No misspelled word under the cursor"))
    );
    assert_eq!(bells(&rig), 4);
    assert_eq!(rig.text(cx), "a teh b wrod c recieve d");

    // A read-only editor: nothing is replaced and the checker is not asked.
    fake.log.borrow_mut().clear();
    rig.editor
        .update(cx, |editor, cx| editor.set_read_only(true, cx));
    cx.simulate_keystrokes("] s 1 z =");
    assert!(
        message(&rig, cx).is_some_and(|m| m.is_error && m.text.starts_with("E21")),
        "{:?}",
        message(&rig, cx)
    );
    assert_eq!(*fake.log.borrow(), ["next 0:0"]);
    assert_eq!(rig.text(cx), "a teh b wrod c recieve d");
}

#[gpui::test]
fn z_equals_opens_the_suggestions(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["a teh b wrod c"]), cx);
    let fake = connect(&rig, cx);
    rig.take_events();

    cx.simulate_keystrokes("] s l z =");
    assert_eq!(
        fake.log.borrow().last().map(String::as_str),
        Some("menu 0:3 caret 3")
    );
    assert_eq!(message(&rig, cx), None);
    assert_eq!(bells(&rig), 0);
    assert_eq!(rig.status(cx).pending_keys, "");

    // On a word that is not marked.
    cx.simulate_keystrokes("0 z =");
    assert_eq!(
        message(&rig, cx),
        Some(VimMessage::error("No misspelled word under the cursor"))
    );
    assert_eq!(bells(&rig), 1);

    // From Visual mode (a right-click selects the word): the command is
    // for the word the cursor is on, and Visual mode ends.
    cx.simulate_keystrokes("] s v i w");
    assert_eq!(rig.mode(cx), Mode::Visual);
    cx.simulate_keystrokes("z =");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(
        fake.log.borrow().last().map(String::as_str),
        Some("menu 0:4 caret 4")
    );
    assert_eq!(rig.caret_override(cx), None);

    // Fed without a window there is nowhere to open a menu.
    rig.vim.update(cx, |vim, cx| vim.feed_keys("z=", cx));
    assert_eq!(
        message(&rig, cx),
        Some(VimMessage::error("z= needs the window of the editor"))
    );
    // With a count no window is needed.
    rig.vim.update(cx, |vim, cx| vim.feed_keys("1z=", cx));
    cx.run_until_parked();
    assert_eq!(rig.text(cx), "a the b wrod c");
    assert_eq!(rig.head(cx), at(0, 2));
}

#[gpui::test]
fn zg_learns_and_zug_unlearns(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["a teh b wrod c teh"]), cx);
    let fake = connect(&rig, cx);
    rig.take_events();

    cx.simulate_keystrokes("] s z g");
    assert_eq!(
        message(&rig, cx),
        Some(VimMessage::info("Learned spelling: teh"))
    );
    assert_eq!(*fake.learned.borrow(), ["teh"]);
    assert_eq!(rig.head(cx), at(0, 2), "the cursor stays");
    assert_eq!(rig.text(cx), "a teh b wrod c teh");
    // Learned words are no longer stops.
    cx.simulate_keystrokes("] s");
    assert_eq!(rig.head(cx), at(0, 8));
    cx.simulate_keystrokes("] s");
    assert_eq!(rig.head(cx), at(0, 8), "the only one left");
    assert_eq!(bells(&rig), 0);

    // Nothing to learn on a word that is not marked.
    cx.simulate_keystrokes("0 z g");
    assert_eq!(
        message(&rig, cx),
        Some(VimMessage::error("No misspelled word under the cursor"))
    );
    assert_eq!(bells(&rig), 1);

    // Unlearn: on the word, which carries no mark any more.
    cx.simulate_keystrokes("w z u");
    assert_eq!(rig.status(cx).pending_keys, "zu");
    cx.simulate_keystrokes("g");
    assert_eq!(rig.status(cx).pending_keys, "");
    assert_eq!(
        message(&rig, cx),
        Some(VimMessage::info("Unlearned spelling: teh"))
    );
    assert!(fake.learned.borrow().is_empty());
    cx.simulate_keystrokes("0 ] s");
    assert_eq!(rig.head(cx), at(0, 2));
    // Twice is once too often.
    cx.simulate_keystrokes("z u g");
    assert_eq!(
        message(&rig, cx),
        Some(VimMessage::error("Not a learned word: teh"))
    );
    assert_eq!(bells(&rig), 1);

    // Marking a word as wrong is not offered.
    for keys in ["z w", "z u w"] {
        cx.simulate_keystrokes(keys);
        let shown = message(&rig, cx).expect("a message");
        assert!(
            shown.is_error && shown.text.contains("not supported"),
            "{shown:?}"
        );
        assert_eq!(bells(&rig), 1, "{keys}");
    }
    assert_eq!(rig.vim.read_with(cx, |vim, _| vim.spell_misses()), 0);
}
