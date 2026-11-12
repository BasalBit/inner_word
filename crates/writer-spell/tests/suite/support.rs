//! Shared helpers: a focused editor with a checker over the fake service,
//! in an active test window that draws context menus.

use std::rc::Rc;
use std::time::Duration;

use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};
use writer_core::{Block, Document, Position};
use writer_editor::{Editor, EditorConfig};
use writer_spell::{
    FakeSpellService, IDLE_PAUSE, SLICE_PAUSE, SpellChecker, SpellLanguage, SpellService,
};
use writer_ui::theme::{self, ThemeMode};

pub fn setup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        theme::init(ThemeMode::Light, cx);
        writer_ui::init(cx);
        writer_editor::init(cx);
        writer_spell::init(cx);
    });
}

/// The words every test text is made of.
pub const WORDS: &[&str] = &[
    "a",
    "an",
    "and",
    "the",
    "this",
    "that",
    "is",
    "was",
    "it",
    "in",
    "of",
    "to",
    "with",
    "here",
    "there",
    "word",
    "words",
    "sentence",
    "ship",
    "came",
    "morning",
    "nobody",
    "expected",
    "harbor",
    "quiet",
    "boats",
    "before",
    "weather",
    "turned",
    "went",
    "paragraph",
    "one",
    "two",
    "three",
    "chapter",
    "text",
    "code",
    "some",
    "more",
    "good",
    "all",
    "at",
    "anything",
    "town",
    "owed",
    "plain",
    "fact",
    "let",
    "be",
    "light",
];

/// English from [`WORDS`], with a few scripted suggestions.
pub fn english() -> FakeSpellService {
    FakeSpellService::with_words(WORDS)
        .suggest("teh", ["the", "ten", "tea"])
        .suggest("sentense", ["sentence", "sentences"])
        .suggest(
            "wrod",
            ["word", "wood", "rod", "trod", "prod", "wold", "ward"],
        )
        .suggest("mornin", ["morning"])
        .suggest("dont", ["don't"])
}

/// What mounts the checker the way the app does: the handlers of the
/// spelling actions around the editor, and the layer that draws menus.
pub struct Host {
    pub editor: Entity<Editor>,
    pub checker: Entity<SpellChecker>,
}

impl gpui::Render for Host {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement, Styled};
        writer_spell::handle_actions(gpui::div().size_full(), &self.checker)
            .child(self.editor.clone())
            .child(writer_ui::ContextMenuLayer)
    }
}

/// Opens `document` in a focused editor with a checker in the fixed fake
/// language. Nothing has been checked yet when it returns only if the
/// document is long: the first slice has run.
pub fn open<'a>(
    document: Document,
    service: &Rc<FakeSpellService>,
    cx: &'a mut TestAppContext,
) -> (
    Entity<SpellChecker>,
    Entity<Editor>,
    &'a mut VisualTestContext,
) {
    open_in(
        document,
        service,
        SpellLanguage::fixed(writer_spell::FAKE_LANGUAGE),
        EditorConfig::full(),
        cx,
    )
}

pub fn open_in<'a>(
    document: Document,
    service: &Rc<FakeSpellService>,
    language: SpellLanguage,
    config: EditorConfig,
    cx: &'a mut TestAppContext,
) -> (
    Entity<SpellChecker>,
    Entity<Editor>,
    &'a mut VisualTestContext,
) {
    let service: Rc<dyn SpellService> = service.clone();
    let (host, cx) = cx.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(document, config, window, cx));
        editor.read(cx).focus(window);
        let checker = cx.new(|cx| {
            let mut checker = SpellChecker::new(editor.clone(), service, cx);
            checker.set_language(language, cx);
            checker
        });
        Host { editor, checker }
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let (checker, editor) =
        host.read_with(cx, |host, _| (host.checker.clone(), host.editor.clone()));
    (checker, editor, cx)
}

pub fn paragraphs(lines: &[&str]) -> Document {
    Document::from_blocks(lines.iter().map(|line| Block::paragraph(line)).collect())
}

/// Lets `duration` pass on the test clock.
pub fn wait(duration: Duration, cx: &mut VisualTestContext) {
    cx.executor().advance_clock(duration);
    cx.run_until_parked();
}

/// Runs slices until every block is checked. Returns how many rounds it
/// took.
pub fn settle(checker: &Entity<SpellChecker>, cx: &mut VisualTestContext) -> usize {
    wait(IDLE_PAUSE, cx);
    let mut rounds = 0;
    while !checker.read_with(cx, |checker, cx| checker.is_settled(cx)) {
        rounds += 1;
        assert!(rounds < 100_000, "the checker never settles");
        wait(SLICE_PAUSE, cx);
    }
    rounds
}

pub fn marked(checker: &Entity<SpellChecker>, cx: &mut VisualTestContext) -> Vec<String> {
    checker.read_with(cx, |checker, cx| checker.misspelled_words(cx))
}

pub fn caret(editor: &Entity<Editor>, block: usize, offset: usize, cx: &mut VisualTestContext) {
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(block, offset)), cx)
    });
    cx.run_until_parked();
}

pub fn text(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |editor, _| editor.document().plain_text())
}

pub fn selected_text(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |editor, _| {
        editor.document().text_in(editor.selection().range())
    })
}

/// The labels of the open context menu ("-" for a separator).
pub fn menu_labels(cx: &mut VisualTestContext) -> Option<Vec<String>> {
    cx.update(|window, cx| {
        writer_ui::ContextMenu::current(window, cx).map(|menu| menu.read(cx).labels())
    })
}

pub fn choose(title: &str, cx: &mut VisualTestContext) -> bool {
    let chosen = cx.update(|window, cx| writer_ui::ContextMenu::choose(title, window, cx));
    cx.run_until_parked();
    chosen
}

/// The last thing the checker told the user.
pub fn notice(checker: &Entity<SpellChecker>, cx: &mut VisualTestContext) -> Option<String> {
    checker.read_with(cx, |checker, _| {
        checker.last_notice().map(|notice| notice.to_string())
    })
}
