//! Spell checking in the assembled workspace.
//!
//! Every test here uses `writer_spell::FakeSpellService`, built from word
//! lists: the Mac's own dictionary answers differently from one macOS
//! version to the next, and its learned words belong to the person at the
//! machine. A workspace gets a checker only when the app was given a spell
//! service (`writer_app::spelling::set_service`); the tests of the other
//! modules give none and so check nothing.

use std::{rc::Rc, time::Duration};

use gpui::{Modifiers, MouseButton, Pixels, Point, TestAppContext, point, px};
use writer_app::{
    Settings, commands, menus,
    settings::keys,
    settings_view::{SettingRow, SettingsView},
    shortcuts::{Place, collect_shortcuts},
    spelling::{self, LanguagePicker},
    workspace::CHECK_SPELLING_MENU_ITEM,
};
use writer_core::{InlineAttrs, InlineStyle, Position, Selection, SpellingLanguage, TextRange};
use writer_spell::{
    FakeSpellService, IGNORE_SPELLING, LEARN_SPELLING, NO_GUESSES, SpellChecker, SpellLanguage,
    SpellService, actions as spell,
};
use writer_ui::ContextMenu;
use writer_vim_mode::Mode;

use super::*;

/// What the fake's English knows ("en", the first and so the preferred
/// language).
const ENGLISH: [&str; 29] = [
    "the", "harbor", "was", "a", "plain", "grey", "fact", "boats", "ropes", "and", "gulls", "she",
    "bought", "bread", "salt", "came", "by", "road", "every", "spring", "one", "two", "three",
    "more", "text", "here", "is", "it", "ten",
];

/// British English ("en_GB"): the same with "harbour" for "harbor".
fn british() -> Vec<&'static str> {
    ENGLISH
        .into_iter()
        .map(|word| if word == "harbor" { "harbour" } else { word })
        .collect()
}

/// German ("de").
const GERMAN: [&str; 8] = [
    "das", "schiff", "kam", "heute", "morgen", "im", "hafen", "an",
];

fn service() -> Rc<FakeSpellService> {
    Rc::new(
        FakeSpellService::with_words(ENGLISH)
            .language("en_GB", "British English", british())
            .language("de", "German", GERMAN)
            // Six, of which a menu shows five.
            .suggest("teh", ["the", "ten", "tea", "tech", "teak", "term"])
            .suggest("grye", ["grey"])
            .suggest("harbour", ["harbor"]),
    )
}

struct Spelling {
    service: Rc<FakeSpellService>,
    library: Entity<Library>,
    workspace: Entity<Workspace>,
    document: DocumentId,
}

/// A workspace with the fake checker over `library`, with `document` open
/// and checked.
fn open_in<'a>(
    library: &Entity<Library>,
    document: &Document,
    cx: &'a mut TestAppContext,
) -> (Spelling, &'a mut VisualTestContext) {
    let service = service();
    cx.update(|cx| spelling::set_service(service.clone(), cx));
    let id = add_document(library, None, document, cx);
    let (workspace, vcx) = open_workspace(library, cx);
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(id, true, window, cx);
    });
    settle(vcx);
    let spelling = Spelling {
        service,
        library: library.clone(),
        workspace,
        document: id,
    };
    (spelling, vcx)
}

/// The same over a fresh in-memory library, with the app's language set
/// to the fake's English (Automatic would guess per paragraph).
fn open<'a>(
    paragraphs: &[&str],
    cx: &'a mut TestAppContext,
) -> (Spelling, &'a mut VisualTestContext) {
    init(cx);
    let library = library(cx);
    library.update(cx, |library, cx| {
        library
            .set_setting(keys::SPELLING_LANGUAGE, "en", cx)
            .unwrap();
    });
    let document = Document::from_blocks(
        paragraphs
            .iter()
            .map(|text| Block::paragraph(text))
            .collect(),
    );
    open_in(&library, &document, cx)
}

fn checker(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<SpellChecker> {
    workspace.read_with(cx, |w, _| w.spell().cloned().expect("a spell checker"))
}

/// Lets `millis` pass, as a pause at the keyboard does.
fn pause(millis: u64, cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(millis));
    cx.run_until_parked();
}

/// Lets the checker finish everything it has to do.
fn settle(cx: &mut VisualTestContext) {
    for _ in 0..40 {
        pause(250, cx);
    }
}

/// The words that are marked, in document order.
fn marked(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<String> {
    let spell = checker(workspace, cx);
    workspace.read_with(cx, |w, cx| {
        let document = w.editor().read(cx).document().clone();
        spell
            .read(cx)
            .misspellings(cx)
            .into_iter()
            .map(|range| {
                let block = document.block(range.start.block).expect("a block");
                block.text()[range.start.offset..range.end.offset].to_string()
            })
            .collect()
    })
}

fn text_of(workspace: &Entity<Workspace>, block: usize, cx: &mut VisualTestContext) -> String {
    workspace.read_with(cx, |w, cx| {
        w.editor()
            .read(cx)
            .document()
            .block(block)
            .expect("a block")
            .text()
            .to_string()
    })
}

fn selection(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Selection {
    workspace.read_with(cx, |w, cx| w.editor().read(cx).state().selection())
}

fn selected_text(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> String {
    workspace.read_with(cx, |w, cx| w.editor().read(cx).state().selected_text())
}

fn caret_to(
    workspace: &Entity<Workspace>,
    block: usize,
    offset: usize,
    cx: &mut VisualTestContext,
) {
    let editor = workspace.read_with(cx, |w, _| w.editor().clone());
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(block, offset)), cx);
    });
    cx.run_until_parked();
}

/// A point inside the character at `position` of the text.
fn in_text(
    workspace: &Entity<Workspace>,
    block: usize,
    offset: usize,
    cx: &mut VisualTestContext,
) -> Point<Pixels> {
    let position = Position::new(block, offset);
    let bounds = workspace.read_with(cx, |w, cx| {
        w.editor()
            .read(cx)
            .position_bounds(position)
            .unwrap_or_else(|| panic!("{position:?} is not on screen"))
    });
    point(bounds.left() + px(2.), bounds.center().y)
}

fn right_click(
    workspace: &Entity<Workspace>,
    block: usize,
    offset: usize,
    cx: &mut VisualTestContext,
) {
    let at = in_text(workspace, block, offset, cx);
    cx.simulate_mouse_move(at, None, Modifiers::default());
    cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
}

/// The labels of the open context menu ("-" for a separator).
fn menu(cx: &mut VisualTestContext) -> Option<Vec<String>> {
    cx.update(|window, cx| ContextMenu::current(window, cx).map(|menu| menu.read(cx).labels()))
}

#[track_caller]
fn open_menu(cx: &mut VisualTestContext) -> Vec<String> {
    menu(cx).expect("a context menu is open")
}

fn dimmed(label: &str, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).expect("a menu is open");
        menu.read(cx)
            .items()
            .iter()
            .find(|item| item.title().as_ref() == label)
            .unwrap_or_else(|| panic!("no item {label:?}"))
            .is_disabled()
    })
}

/// Clicks the row of the item called `label` of the open menu.
fn click_item(label: &str, cx: &mut VisualTestContext) {
    let at = cx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).expect("a menu is open");
        let menu = menu.read(cx);
        let ix = menu
            .labels()
            .iter()
            .position(|item| item == label)
            .unwrap_or_else(|| panic!("no item {label:?} in {:?}", menu.labels()));
        menu.item_bounds(ix).expect("the menu was drawn").center()
    });
    cx.simulate_mouse_move(at, None, Modifiers::default());
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
}

fn close_menu(cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(menu(cx), None, "escape closes the menu");
}

/// Runs a command from the palette by typing its name.
fn palette(name: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("cmd-shift-p");
    cx.simulate_input(name);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
}

fn settings_view(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Option<Entity<SettingsView>> {
    workspace.read_with(cx, |workspace, cx| {
        workspace.modal_layer().read(cx).active::<SettingsView>()
    })
}

fn language_picker(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Option<Entity<LanguagePicker>> {
    workspace.read_with(cx, |workspace, cx| {
        workspace.modal_layer().read(cx).active::<LanguagePicker>()
    })
}

fn language_of(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> SpellLanguage {
    let spell = checker(workspace, cx);
    spell.read_with(cx, |spell, _| spell.language().clone())
}

fn fixed(id: &str) -> SpellingLanguage {
    SpellingLanguage::language(id).expect("an identifier")
}

// ----- marks while writing -------------------------------------------------------

#[gpui::test]
fn a_misspelled_word_is_marked_once_the_caret_has_left_it(cx: &mut TestAppContext) {
    let (app, cx) = open(&[""], cx);
    let workspace = &app.workspace;
    assert!(checker(workspace, cx).read_with(cx, |spell, cx| spell.is_active(cx)));
    assert_eq!(marked(workspace, cx), Vec::<String>::new());

    // The word that is being typed is left alone, however wrong it looks
    // so far.
    cx.simulate_input("The harbor was teh");
    pause(300, cx);
    assert_eq!(marked(workspace, cx), Vec::<String>::new());

    // The caret moves on: the word is checked and marked.
    cx.simulate_input(" plain ");
    pause(600, cx);
    assert_eq!(marked(workspace, cx), ["teh"]);

    // A word the caret rests in is checked after a pause.
    cx.simulate_input("grye");
    pause(300, cx);
    assert_eq!(marked(workspace, cx), ["teh"]);
    pause(1500, cx);
    assert_eq!(marked(workspace, cx), ["teh", "grye"]);

    // Correcting a word takes its mark away; the other one stays where
    // its word now is.
    caret_to(workspace, 0, 0, cx);
    cx.simulate_input("One more ");
    settle(cx);
    assert_eq!(
        text_of(workspace, 0, cx),
        "One more The harbor was teh plain grye"
    );
    assert_eq!(marked(workspace, cx), ["teh", "grye"]);
    let end = text_of(workspace, 0, cx).len();
    caret_to(workspace, 0, end, cx);
    cx.simulate_keystrokes("backspace backspace");
    cx.simulate_input("ey fact");
    settle(cx);
    assert_eq!(marked(workspace, cx), ["teh"]);
}

#[gpui::test]
fn code_is_not_checked(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    library.update(cx, |library, cx| {
        library
            .set_setting(keys::SPELLING_LANGUAGE, "en", cx)
            .unwrap();
    });
    let code = InlineAttrs::default().with(InlineStyle::Code);
    let document = Document::from_blocks(vec![
        Block::paragraph("The harbor was teh fact"),
        Block::with_text(BlockKind::code(), "let teh = grye;"),
        Block::new(BlockKind::Paragraph)
            .with_span("She bought ", InlineAttrs::default())
            .with_span("teh_bread", code)
            .with_span(" and grye salt", InlineAttrs::default()),
    ]);
    let (app, cx) = open_in(&library, &document, cx);
    assert_eq!(marked(&app.workspace, cx), ["teh", "grye"]);
    let marks = checker(&app.workspace, cx).read_with(cx, |spell, cx| spell.misspellings(cx));
    assert_eq!(
        marks
            .iter()
            .map(|range| range.start.block)
            .collect::<Vec<_>>(),
        [0, 2],
        "nothing in the code block is marked"
    );
}

// ----- the context menu ----------------------------------------------------------

#[gpui::test]
fn a_right_click_on_a_marked_word_leads_with_the_suggestions(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    library.update(cx, |library, cx| {
        library
            .set_setting(keys::SPELLING_LANGUAGE, "en", cx)
            .unwrap();
    });
    let bold = InlineAttrs::default().with(InlineStyle::Bold);
    let document = Document::from_blocks(vec![
        Block::new(BlockKind::Paragraph)
            .with_span("The harbor was ", InlineAttrs::default())
            .with_span("teh", bold)
            .with_span(" plain fact", InlineAttrs::default()),
        Block::paragraph("Boats and xqzzy ropes"),
    ]);
    let (app, cx) = open_in(&library, &document, cx);
    let workspace = &app.workspace;
    assert_eq!(marked(workspace, cx), ["teh", "xqzzy"]);

    // On the marked word: at most five suggestions lead the menu, then
    // Learn Spelling and Ignore Spelling, a separator, and the usual menu.
    right_click(workspace, 0, 16, cx);
    let labels = open_menu(cx);
    assert_eq!(labels[..5], ["the", "ten", "tea", "tech", "teak"]);
    assert!(!labels.contains(&"term".to_string()), "{labels:?}");
    let learn = labels.iter().position(|l| l == LEARN_SPELLING).unwrap();
    let ignore = labels.iter().position(|l| l == IGNORE_SPELLING).unwrap();
    let cut = labels.iter().position(|l| l == "Cut").unwrap();
    assert!(5 <= learn && learn < ignore && ignore < cut, "{labels:?}");
    assert_eq!(labels[cut - 1], "-", "{labels:?}");
    assert!(labels.contains(&"Add Comment".to_string()), "{labels:?}");

    // Choosing one replaces the word, keeps its formatting, and is one
    // step to undo.
    click_item("the", cx);
    assert_eq!(menu(cx), None);
    assert_eq!(text_of(workspace, 0, cx), "The harbor was the plain fact");
    let is_bold = |offset: usize, cx: &mut VisualTestContext| {
        workspace.read_with(cx, |w, cx| {
            let editor = w.editor().read(cx);
            let block = editor.document().block(0).unwrap();
            block
                .attrs_at(offset)
                .is_some_and(|attrs| attrs.has_style(InlineStyle::Bold))
        })
    };
    assert!(is_bold(16, cx) && !is_bold(13, cx) && !is_bold(19, cx));
    settle(cx);
    assert_eq!(marked(workspace, cx), ["xqzzy"]);
    assert!(editor_focused(workspace, cx));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text_of(workspace, 0, cx), "The harbor was teh plain fact");
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text_of(workspace, 0, cx), "The harbor was the plain fact");

    // A word nobody has a guess for says so, in a row that cannot be
    // chosen; it can still be learned or ignored.
    right_click(workspace, 1, 12, cx);
    let labels = open_menu(cx);
    assert_eq!(labels[0], NO_GUESSES);
    assert!(dimmed(NO_GUESSES, cx));
    assert_eq!(
        labels[1..3],
        [LEARN_SPELLING, IGNORE_SPELLING],
        "{labels:?}"
    );
    close_menu(cx);

    // A word that is not marked gets the usual menu and nothing before it.
    right_click(workspace, 1, 2, cx);
    assert_eq!(open_menu(cx)[0], "Cut");
    close_menu(cx);

    // From the keyboard, with the caret in the marked word: the same rows
    // first, on either key, and enter takes the first suggestion.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text_of(workspace, 0, cx), "The harbor was teh plain fact");
    settle(cx);
    caret_to(workspace, 0, 16, cx);
    cx.simulate_keystrokes("shift-f10");
    cx.run_until_parked();
    assert_eq!(open_menu(cx)[..2], ["the", "ten"]);
    close_menu(cx);
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    assert_eq!(open_menu(cx)[..2], ["the", "ten"]);
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    assert_eq!(text_of(workspace, 0, cx), "The harbor was ten plain fact");
}

#[gpui::test]
fn learn_and_ignore(cx: &mut TestAppContext) {
    let (app, cx) = open(
        &[
            "Mara came by the harbor",
            "Mara bought bread and zorblat",
            "zorblat",
        ],
        cx,
    );
    let workspace = &app.workspace;
    assert_eq!(
        marked(workspace, cx),
        ["Mara", "Mara", "zorblat", "zorblat"]
    );

    // Ignore Spelling: every occurrence in this document loses its mark at
    // once; nothing is learned.
    right_click(workspace, 0, 1, cx);
    click_item(IGNORE_SPELLING, cx);
    assert_eq!(marked(workspace, cx), ["zorblat", "zorblat"]);
    let tag = checker(workspace, cx).read_with(cx, |spell, _| spell.document());
    // (The fake keeps its words in lower case.)
    assert_eq!(app.service.ignored_words(tag), ["mara"]);
    assert!(app.service.learned_words().is_empty());

    // Learn Spelling: the word goes into the user's dictionary.
    right_click(workspace, 2, 3, cx);
    click_item(LEARN_SPELLING, cx);
    assert_eq!(marked(workspace, cx), Vec::<String>::new());
    assert_eq!(app.service.learned_words(), ["zorblat"]);
    assert!(app.service.has_learned("zorblat"));

    // And out of it again, from the palette, with the caret in the word.
    caret_to(workspace, 2, 3, cx);
    palette("unlearn spelling", cx);
    settle(cx);
    assert!(!app.service.has_learned("zorblat"));
    assert_eq!(marked(workspace, cx), ["zorblat", "zorblat"]);

    // Learning from the keyboard: the command, with the caret in the
    // word. (Dispatched as the menu bar does; the palette would offer the
    // command that was run last first, and "unlearn" contains "learn".)
    caret_to(workspace, 1, 26, cx);
    cx.dispatch_action(spell::LearnSpelling);
    settle(cx);
    assert!(app.service.has_learned("zorblat"));
    assert_eq!(marked(workspace, cx), Vec::<String>::new());

    // Ignoring lasts until the document is closed.
    let other = add_document(
        &app.library,
        None,
        &simple_document("More text", "Mara is here"),
        cx,
    );
    workspace.update_in(cx, |w, window, cx| {
        w.open_document(other, true, window, cx);
    });
    settle(cx);
    assert_eq!(marked(workspace, cx), ["Mara"]);
    workspace.update_in(cx, |w, window, cx| {
        w.open_document(app.document, true, window, cx);
    });
    settle(cx);
    assert_eq!(marked(workspace, cx), ["Mara", "Mara"]);
}

// ----- moving between misspellings -------------------------------------------------

#[gpui::test]
fn the_keys_step_through_the_misspellings_and_wrap(cx: &mut TestAppContext) {
    let (app, cx) = open(&["teh harbor", "plain fact", "grey xqzzy gulls"], cx);
    let workspace = &app.workspace;
    assert_eq!(marked(workspace, cx), ["teh", "xqzzy"]);

    caret_to(workspace, 1, 0, cx);
    cx.simulate_keystrokes("cmd-;");
    assert_eq!(selected_text(workspace, cx), "xqzzy");
    assert_eq!(
        selection(workspace, cx).range(),
        TextRange::in_block(2, 5, 10)
    );
    // Past the last one: around to the first.
    cx.simulate_keystrokes("cmd-;");
    assert_eq!(selected_text(workspace, cx), "teh");
    // And back the other way, on both spellings of the key.
    cx.simulate_keystrokes("cmd-shift-;");
    assert_eq!(selected_text(workspace, cx), "xqzzy");
    cx.simulate_keystrokes("cmd-:");
    assert_eq!(selected_text(workspace, cx), "teh");

    // From the palette while the keyboard is in the library: the text
    // takes the keyboard and the next word is selected.
    cx.simulate_keystrokes("cmd-0");
    assert!(!editor_focused(workspace, cx));
    palette("next misspelling", cx);
    assert!(editor_focused(workspace, cx));
    assert_eq!(selected_text(workspace, cx), "xqzzy");

    // The suggestions of the selected word, as a menu at the caret.
    cx.simulate_keystrokes("cmd-shift-;");
    assert_eq!(selected_text(workspace, cx), "teh");
    palette("show spelling suggestions", cx);
    assert_eq!(open_menu(cx)[..2], ["the", "ten"]);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(text_of(workspace, 0, cx), "the harbor");

    // With nothing left to find the checker says so and the caret stays.
    caret_to(workspace, 2, 7, cx);
    palette("ignore spelling", cx);
    settle(cx);
    assert_eq!(marked(workspace, cx), Vec::<String>::new());
    let before = selection(workspace, cx);
    cx.simulate_keystrokes("cmd-;");
    assert_eq!(selection(workspace, cx), before);
    assert!(
        checker(workspace, cx)
            .read_with(cx, |spell, _| spell.last_notice())
            .is_some()
    );
}

// ----- the switch ----------------------------------------------------------------

#[gpui::test]
fn the_switch_removes_the_marks_and_stops_checking(cx: &mut TestAppContext) {
    let (app, cx) = open(&["teh harbor was grye", ""], cx);
    let workspace = &app.workspace;
    assert_eq!(marked(workspace, cx), ["teh", "grye"]);
    assert_eq!(
        cx.update(|_, cx| menus::check_mark(CHECK_SPELLING_MENU_ITEM, cx)),
        Some(true)
    );

    // Off, in the settings: the marks go at once.
    cx.simulate_keystrokes("cmd-,");
    let view = settings_view(workspace, cx).expect("settings are open");
    let row = view
        .read_with(cx, |view, cx| view.rows(cx))
        .iter()
        .position(|row| *row == SettingRow::CheckSpelling)
        .unwrap();
    for _ in 0..row {
        cx.simulate_keystrokes("down");
    }
    cx.simulate_keystrokes("space");
    cx.run_until_parked();
    assert!(!workspace.read_with(cx, |w, _| w.settings().check_spelling));
    assert_eq!(marked(workspace, cx), Vec::<String>::new());
    assert!(!checker(workspace, cx).read_with(cx, |spell, _| spell.is_enabled()));
    assert_eq!(
        cx.update(|_, cx| menus::check_mark(CHECK_SPELLING_MENU_ITEM, cx)),
        Some(false)
    );
    cx.simulate_keystrokes("escape");

    // Nothing is checked from now on, whatever is written.
    app.service.reset_calls();
    caret_to(workspace, 1, 0, cx);
    cx.simulate_input("xqzzy and more teh ");
    settle(cx);
    assert_eq!(app.service.check_count(), 0);
    assert_eq!(marked(workspace, cx), Vec::<String>::new());

    // The commands that need the checker say why they do nothing.
    let before = selection(workspace, cx);
    cx.simulate_keystrokes("cmd-;");
    assert_eq!(selection(workspace, cx), before);

    // On again, from the palette: the setting is stored and everything is
    // checked.
    palette("check spelling while typing", cx);
    settle(cx);
    assert!(workspace.read_with(cx, |w, _| w.settings().check_spelling));
    assert!(app.library.read_with(cx, |library, _| {
        library.setting_or(keys::CHECK_SPELLING, false)
    }));
    assert_eq!(marked(workspace, cx), ["teh", "grye", "xqzzy", "teh"]);
    assert_eq!(
        cx.update(|_, cx| menus::check_mark(CHECK_SPELLING_MENU_ITEM, cx)),
        Some(true)
    );

    // The menu bar item is the same switch, also with the keyboard
    // elsewhere and with no document open.
    cx.simulate_keystrokes("cmd-0");
    cx.dispatch_action(spell::ToggleSpellChecking);
    cx.run_until_parked();
    assert!(!workspace.read_with(cx, |w, _| w.settings().check_spelling));
    assert_eq!(marked(workspace, cx), Vec::<String>::new());
}

// ----- the language --------------------------------------------------------------

#[gpui::test]
fn the_document_language_overrides_the_app_language_and_both_are_kept(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let document = Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), "The harbour"),
        Block::paragraph("The harbour was grey"),
    ]);
    let id;
    {
        let library = cx.new(|_| Library::open(&path).unwrap());
        library.update(cx, |library, cx| {
            library
                .set_setting(keys::WELCOME_CREATED, &true, cx)
                .unwrap();
        });
        let (app, cx) = open_in(&library, &document, cx);
        let workspace = &app.workspace;
        id = app.document;

        // A new library checks in Automatic: the fake takes each paragraph
        // for the language that knows most of its words, British English
        // here.
        assert_eq!(
            workspace.read_with(cx, |w, _| w.settings().spelling_language.clone()),
            SpellingLanguage::Automatic
        );
        assert_eq!(language_of(workspace, cx), SpellLanguage::Automatic);
        assert_eq!(marked(workspace, cx), Vec::<String>::new());
        assert_eq!(
            checker(workspace, cx).read_with(cx, |spell, cx| spell.detected_language(1, cx)),
            Some("en_GB".to_string())
        );

        // The app's language, in the settings: right steps from Automatic
        // to the first of the user's languages.
        cx.simulate_keystrokes("cmd-,");
        let view = settings_view(workspace, cx).expect("settings are open");
        view.update(cx, |view, cx| {
            view.step(SettingRow::SpellingLanguage, 1, cx)
        });
        settle(cx);
        assert_eq!(
            workspace.read_with(cx, |w, _| w.settings().spelling_language.clone()),
            fixed("en")
        );
        assert_eq!(language_of(workspace, cx), SpellLanguage::fixed("en"));
        assert_eq!(marked(workspace, cx), ["harbour", "harbour"]);
        cx.simulate_keystrokes("escape");

        // The document's own language, through the palette's picker: it
        // lists "follow the app", Automatic and the languages by name.
        palette("spelling language", cx);
        let picker = language_picker(workspace, cx).expect("the language picker is open");
        assert_eq!(
            picker.read_with(cx, |picker, _| picker.delegate.labels()),
            [
                spelling::FOLLOW_APP_LABEL,
                spelling::AUTOMATIC_LABEL,
                "English",
                "British English",
                "German"
            ]
        );
        // It opens on the choice that applies now: this document follows
        // the app.
        assert_eq!(
            picker.read_with(cx, |picker, _| (
                picker.delegate.current_label(),
                picker.selected_index()
            )),
            (Some(spelling::FOLLOW_APP_LABEL.to_string()), Some(0))
        );
        cx.simulate_input("brit");
        cx.simulate_keystrokes("enter");
        settle(cx);
        assert!(language_picker(workspace, cx).is_none());
        assert!(editor_focused(workspace, cx));
        assert_eq!(
            workspace.read_with(cx, |w, cx| w.document_spelling_language(cx)),
            Some(fixed("en_GB"))
        );
        assert_eq!(language_of(workspace, cx), SpellLanguage::fixed("en_GB"));
        assert_eq!(marked(workspace, cx), Vec::<String>::new());

        // Changing the app's language does not touch a document that has
        // its own.
        workspace.update(cx, |w, cx| w.set_app_spelling_language(&fixed("de"), cx));
        settle(cx);
        assert_eq!(language_of(workspace, cx), SpellLanguage::fixed("en_GB"));
        assert_eq!(marked(workspace, cx), Vec::<String>::new());

        // In the "This document" settings: left from the document's
        // language goes towards "follow the app setting", and then the
        // app's German applies.
        palette("document settings", cx);
        let view = settings_view(workspace, cx).expect("settings are open");
        let row = SettingRow::DocumentSpellingLanguage;
        assert!(view.read_with(cx, |view, cx| view.rows(cx)).contains(&row));
        for _ in 0..4 {
            view.update(cx, |view, cx| view.step(row, -1, cx));
        }
        settle(cx);
        assert_eq!(
            workspace.read_with(cx, |w, cx| w.document_spelling_language(cx)),
            None
        );
        assert_eq!(language_of(workspace, cx), SpellLanguage::fixed("de"));
        assert_eq!(
            marked(workspace, cx),
            ["The", "harbour", "The", "harbour", "was", "grey"]
        );

        // Enter on the row opens the whole list; a choice comes back to
        // the row.
        let handle = view.read_with(cx, |view, cx| {
            view.rows(cx).iter().position(|listed| *listed == row)
        });
        assert!(handle.is_some());
        view.update(cx, |view, cx| view.activate(row, cx));
        cx.run_until_parked();
        let picker = language_picker(workspace, cx).expect("the language picker is open");
        assert!(settings_view(workspace, cx).is_none());
        cx.simulate_input("en_GB");
        assert_eq!(
            picker.read_with(cx, |picker, _| picker.delegate.labels()),
            ["British English"]
        );
        cx.simulate_keystrokes("enter");
        settle(cx);
        let view = settings_view(workspace, cx).expect("back in the settings");
        assert_eq!(
            view.update_in(cx, |view, window, cx| view.focused_row(window, cx)),
            Some(row)
        );
        assert_eq!(
            workspace.read_with(cx, |w, cx| w.document_spelling_language(cx)),
            Some(fixed("en_GB"))
        );
        // Escape in the picker also comes back, with nothing changed.
        view.update(cx, |view, cx| {
            view.activate(SettingRow::SpellingLanguage, cx)
        });
        cx.run_until_parked();
        assert!(language_picker(workspace, cx).is_some());
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        let view = settings_view(workspace, cx).expect("back in the settings");
        assert_eq!(
            view.update_in(cx, |view, window, cx| view.focused_row(window, cx)),
            Some(SettingRow::SpellingLanguage)
        );
        assert_eq!(
            workspace.read_with(cx, |w, _| w.settings().spelling_language.clone()),
            fixed("de")
        );
        cx.simulate_keystrokes("escape");
        assert!(workspace.update(cx, |w, cx| w.flush(cx)));
    }
    cx.run_until_parked();

    // Another launch: both are still there.
    let reopened = Library::open(&path).unwrap();
    let loaded = Settings::load(&reopened);
    assert_eq!(loaded.spelling_language, fixed("de"));
    assert!(loaded.check_spelling);
    let stored = Document::from_json(&reopened.document(id).unwrap().content).unwrap();
    assert_eq!(stored.settings().spelling_language, Some(fixed("en_GB")));
    drop(reopened);

    let library = cx.new(|_| Library::open(&path).unwrap());
    let (workspace, cx) = open_workspace(&library, cx);
    settle(cx);
    assert_eq!(
        workspace.read_with(cx, |w, _| w.open_document_id()),
        Some(id)
    );
    assert_eq!(
        workspace.read_with(cx, |w, cx| w.spelling_language(cx)),
        fixed("en_GB")
    );
    assert_eq!(language_of(&workspace, cx), SpellLanguage::fixed("en_GB"));
    assert_eq!(marked(&workspace, cx), Vec::<String>::new());
    // Without its own language the document would be checked in German.
    workspace.update(cx, |w, cx| w.set_document_spelling_language(None, cx));
    settle(cx);
    assert_eq!(language_of(&workspace, cx), SpellLanguage::fixed("de"));
    assert!(!marked(&workspace, cx).is_empty());
}

#[gpui::test]
fn the_checker_follows_the_open_document(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    library.update(cx, |library, cx| {
        library
            .set_setting(keys::SPELLING_LANGUAGE, "en", cx)
            .unwrap();
    });
    let mut german = Document::from_blocks(vec![Block::paragraph("Das Schiff kam heute teh")]);
    let mut settings = german.settings().clone();
    settings.spelling_language = Some(fixed("de"));
    german = german.with_settings(settings);
    let german = add_document(&library, None, &german, cx);
    let english = Document::from_blocks(vec![Block::paragraph("teh harbor was plain")]);
    let (app, cx) = open_in(&library, &english, cx);
    let workspace = &app.workspace;
    let spell = checker(workspace, cx);

    assert_eq!(language_of(workspace, cx), SpellLanguage::fixed("en"));
    assert_eq!(marked(workspace, cx), ["teh"]);

    // Another document: its own language, its own marks, the same checker.
    workspace.update_in(cx, |w, window, cx| {
        w.open_document(german, true, window, cx);
    });
    settle(cx);
    assert_eq!(checker(workspace, cx), spell);
    assert_eq!(language_of(workspace, cx), SpellLanguage::fixed("de"));
    assert_eq!(marked(workspace, cx), ["teh"]);
    assert_eq!(text_of(workspace, 0, cx), "Das Schiff kam heute teh");

    // And back: the app's language again.
    workspace.update_in(cx, |w, window, cx| {
        w.open_document(app.document, true, window, cx);
    });
    settle(cx);
    assert_eq!(language_of(workspace, cx), SpellLanguage::fixed("en"));
    assert_eq!(marked(workspace, cx), ["teh"]);
    assert_eq!(text_of(workspace, 0, cx), "teh harbor was plain");

    // With no document open nothing is checked and nothing is marked.
    workspace.update_in(cx, |w, window, cx| w.close_document(window, cx));
    settle(cx);
    assert_eq!(marked(workspace, cx), Vec::<String>::new());
    assert!(!spell.read_with(cx, |spell, cx| spell.is_active(cx)));
    let available = cx.update(|window, cx| {
        (
            window.is_action_available(&spell::NextMisspelling, cx),
            window.is_action_available(&spell::ToggleSpellChecking, cx),
        )
    });
    assert_eq!(available, (false, true));
}

// ----- Vim -----------------------------------------------------------------------

#[gpui::test]
fn the_vim_keys_reach_the_checker(cx: &mut TestAppContext) {
    let (app, cx) = open(&["teh harbor was grye", "plain xqzzy"], cx);
    let workspace = &app.workspace;
    workspace.update(cx, |w, cx| w.toggle_vim_mode(cx));
    caret_to(workspace, 0, 0, cx);
    settle(cx);
    let mode =
        |cx: &mut VisualTestContext| workspace.read_with(cx, |w, cx| w.vim().read(cx).mode());
    let head = |cx: &mut VisualTestContext| selection(workspace, cx).head;
    assert_eq!(mode(cx), Mode::Normal);
    assert!(workspace.read_with(cx, |w, cx| w.vim().read(cx).has_spelling()));
    assert_eq!(marked(workspace, cx), ["teh", "grye", "xqzzy"]);

    // ]s and [s, wrapping.
    cx.simulate_keystrokes("] s");
    assert_eq!(head(cx), Position::new(0, 15));
    cx.simulate_keystrokes("] s");
    assert_eq!(head(cx), Position::new(1, 6));
    cx.simulate_keystrokes("] s");
    assert_eq!(head(cx), Position::new(0, 0));
    cx.simulate_keystrokes("[ s");
    assert_eq!(head(cx), Position::new(1, 6));
    cx.simulate_keystrokes("2 [ s");
    assert_eq!(head(cx), Position::new(0, 0));

    // z= opens the suggestions under the caret; enter takes the first.
    cx.simulate_keystrokes("z =");
    cx.run_until_parked();
    assert_eq!(open_menu(cx)[..2], ["the", "ten"]);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(menu(cx), None);
    assert_eq!(text_of(workspace, 0, cx), "the harbor was grye");
    assert_eq!(mode(cx), Mode::Normal);
    // One step to undo.
    cx.simulate_keystrokes("u");
    assert_eq!(text_of(workspace, 0, cx), "teh harbor was grye");
    settle(cx);

    // A count takes that suggestion without asking.
    caret_to(workspace, 0, 1, cx);
    cx.simulate_keystrokes("2 z =");
    cx.run_until_parked();
    assert_eq!(menu(cx), None);
    assert_eq!(text_of(workspace, 0, cx), "ten harbor was grye");
    settle(cx);

    // zg learns the word under the cursor, zug takes it back.
    cx.simulate_keystrokes("] s");
    assert_eq!(head(cx), Position::new(0, 15));
    cx.simulate_keystrokes("z g");
    settle(cx);
    assert_eq!(app.service.learned_words(), ["grye"]);
    assert_eq!(marked(workspace, cx), ["xqzzy"]);
    cx.simulate_keystrokes("z u g");
    settle(cx);
    assert!(app.service.learned_words().is_empty());
    assert_eq!(marked(workspace, cx), ["grye", "xqzzy"]);

    // With checking off the keys say so, as Vim does.
    workspace.update(cx, |w, cx| w.toggle_spell_checking(cx));
    settle(cx);
    let before = head(cx);
    cx.simulate_keystrokes("] s");
    assert_eq!(head(cx), before);
    let message = workspace.read_with(cx, |w, cx| {
        w.vim()
            .read(cx)
            .status()
            .message
            .as_ref()
            .map(|message| message.text.to_string())
    });
    assert!(
        message.as_deref().is_some_and(|text| text.contains("E756")),
        "{message:?}"
    );
}

// ----- where it is offered -------------------------------------------------------

#[gpui::test]
fn spelling_is_in_the_menu_bar_the_palette_and_the_overlay(cx: &mut TestAppContext) {
    init(cx);

    // Edit > Spelling.
    let all = menus::app_menus();
    let edit = all.iter().find(|menu| menu.name == "Edit").unwrap();
    let spelling_menu = edit
        .items
        .iter()
        .find_map(|item| match item {
            gpui::MenuItem::Submenu(menu) if menu.name == "Spelling" => Some(menu),
            _ => None,
        })
        .expect("Edit has a Spelling submenu");
    let names: Vec<String> = spelling_menu
        .items
        .iter()
        .filter_map(|item| match item {
            gpui::MenuItem::Action { name, .. } => Some(name.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(
        names,
        [
            CHECK_SPELLING_MENU_ITEM,
            "Next Misspelling",
            "Previous Misspelling",
            "Show Suggestions",
            "Learn Spelling",
            "Unlearn Spelling",
            "Ignore Spelling",
            "Spelling Language…",
        ]
    );
    // The key beside "Previous Misspelling" is the readable one.
    cx.update(|cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let key = |action: &dyn gpui::Action| {
            menus::native_menu_binding(&keymap, action).map(|binding| {
                binding
                    .keystrokes()
                    .iter()
                    .map(|keystroke| keystroke.unparse())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
        };
        assert_eq!(key(&spell::NextMisspelling), Some("cmd-;".to_string()));
        assert_eq!(
            key(&spell::PreviousMisspelling),
            Some("cmd-shift-;".to_string())
        );
    });

    // The palette: one category, in sentence case, the picker last.
    let titles: Vec<String> = cx.update(|cx| {
        commands::all(cx)
            .iter()
            .filter(|command| command.category.as_ref() == writer_spell::CATEGORY_SPELLING)
            .map(|command| command.title.to_string())
            .collect()
    });
    assert_eq!(
        titles,
        [
            "Next misspelling",
            "Previous misspelling",
            "Show spelling suggestions",
            "Learn spelling",
            "Unlearn spelling",
            "Ignore spelling",
            "Check spelling while typing",
            "Spelling language",
        ]
    );

    // The shortcuts overlay lists the two keys among those of the text.
    let shortcuts = cx.update(|cx| collect_shortcuts(cx));
    for title in ["Next misspelling", "Previous misspelling"] {
        let shortcut = shortcuts
            .iter()
            .find(|shortcut| shortcut.title.as_ref() == title)
            .unwrap_or_else(|| panic!("the overlay does not list {title:?}"));
        assert_eq!(shortcut.place, Place::Text, "{title}");
    }

    // The welcome document says how to correct a word and how to change
    // the language.
    let welcome = writer_app::welcome::welcome_document().plain_text();
    for phrase in [
        "Learn Spelling",
        "Ignore Spelling",
        "Spelling language",
        "⌘;",
    ] {
        assert!(
            welcome.contains(phrase),
            "the welcome document lacks {phrase:?}"
        );
    }
}

#[gpui::test]
fn without_a_spell_service_nothing_is_checked(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let id = add_document(&library, None, &simple_document("Doc", "teh xqzzy"), cx);
    let (workspace, cx) = open_workspace(&library, cx);
    workspace.update_in(cx, |w, window, cx| {
        w.open_document(id, true, window, cx);
    });
    cx.run_until_parked();
    assert!(workspace.read_with(cx, |w, _| w.spell().is_none()));
    assert_eq!(
        workspace.read_with(cx, |w, cx| w.describe_spelling(cx)),
        "none"
    );
    // The settings still list the rows; the language row has only
    // "Automatic" to offer.
    cx.simulate_keystrokes("cmd-,");
    let view = settings_view(&workspace, cx).expect("settings are open");
    let rows = view.read_with(cx, |view, cx| view.rows(cx));
    assert!(rows.contains(&SettingRow::CheckSpelling));
    assert!(rows.contains(&SettingRow::SpellingLanguage));
    assert_eq!(
        cx.update(|_, cx| spelling::app_language_options(&SpellingLanguage::Automatic, cx)),
        [SpellingLanguage::Automatic]
    );
    // And the Vim keys have no checker to ask.
    assert!(!workspace.read_with(cx, |w, cx| w.vim().read(cx).has_spelling()));
    // The word commands are not on offer; the switch is.
    cx.simulate_keystrokes("escape");
    let available = cx.update(|window, cx| {
        (
            window.is_action_available(&spell::NextMisspelling, cx),
            window.is_action_available(&spell::ToggleSpellChecking, cx),
        )
    });
    assert_eq!(available, (false, true));
}
