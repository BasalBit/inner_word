//! The assistant in the assembled app.
//!
//! **No test here ever runs Claude Code.** Every test that has an assistant
//! installs `writer_assistant::FakeBackend`, which starts no program and
//! answers from a script; a workspace opened without one has no assistant
//! at all (the tests of the rest of the suite).

use std::sync::Arc;

use gpui::TestAppContext;
use writer_app::{
    actions::CATEGORY_ASSISTANT,
    assistant::{self, apply, choices, conversations},
    commands, menus,
    settings::keys,
    settings_view::{ASSISTANT_SECTION, SettingRow, SettingsView},
};
use writer_assistant::{
    AssistantError, Attachment, Backend as _, EffortChoice, FakeBackend, Mode, ModelChoice,
    Selection, Transcript, TurnId, fake_catalog,
};
use writer_core::{Position, TextRange};

use super::*;

/// Installs `fake` as the engine of the assistant for the workspaces this
/// test opens.
fn install(fake: &FakeBackend, cx: &mut TestAppContext) {
    let backend = Arc::new(fake.clone());
    cx.update(|cx| assistant::set_backend(backend, cx));
}

/// A library with one document, opened in a workspace with `fake` as the
/// engine. The check at launch has answered when this returns.
fn workspace_with<'a>(
    fake: &FakeBackend,
    cx: &'a mut TestAppContext,
) -> (
    Entity<Library>,
    DocumentId,
    Entity<Workspace>,
    &'a mut VisualTestContext,
) {
    init(cx);
    install(fake, cx);
    let library = library(cx);
    let id = add_document(
        &library,
        None,
        &simple_document("The Lighthouse", "The keeper climbed the stairs at dusk."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    vcx.run_until_parked();
    (library, id, workspace, vcx)
}

fn settings_view(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Option<Entity<SettingsView>> {
    workspace.read_with(cx, |workspace, cx| {
        workspace.modal_layer().read(cx).active::<SettingsView>()
    })
}

fn palette_titles(cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        commands::all(cx)
            .into_iter()
            .filter(|command| command.category.as_ref() == CATEGORY_ASSISTANT)
            .map(|command| command.title.to_string())
            .collect()
    })
}

fn menu_bar_names(cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        menus::menu_actions(&menus::app_menus_with(menus::assistant_shown(cx)))
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    })
}

// ----- availability ---------------------------------------------------------------

#[gpui::test]
fn without_an_engine_there_is_no_assistant(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(&library, None, &simple_document("Doc", "Text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    assert!(!workspace.read_with(vcx, |w, _| w.assistant_shown()));
    assert!(workspace.read_with(vcx, |w, _| w.assistant_host().is_none()));
    assert!(palette_titles(vcx).is_empty());
    assert!(
        !menu_bar_names(vcx)
            .iter()
            .any(|name| name.contains("Assistant"))
    );

    // The settings have no section and no line about it.
    vcx.simulate_keystrokes("cmd-,");
    let view = settings_view(&workspace, vcx).expect("settings are open");
    view.read_with(vcx, |view, cx| {
        assert!(!view.rows(cx).contains(&SettingRow::Assistant));
        assert!(view.assistant_missing_note(cx).is_none());
        assert!(view.assistant_privacy_note(cx).is_none());
    });
}

#[gpui::test]
fn an_engine_that_is_not_installed_shows_nothing_but_a_line_in_the_settings(
    cx: &mut TestAppContext,
) {
    let fake = FakeBackend::not_installed();
    let (_library, _id, workspace, vcx) = workspace_with(&fake, cx);
    // The launch looked once, in the background.
    assert_eq!(fake.availability_checks(), 1);
    assert!(!workspace.read_with(vcx, |w, _| w.assistant_shown()));
    assert!(palette_titles(vcx).is_empty());
    assert!(
        !menu_bar_names(vcx)
            .iter()
            .any(|name| name.contains("Assistant"))
    );
    assert!(!vcx.update(|_, cx| menus::assistant_shown(cx)));

    vcx.simulate_keystrokes("cmd-,");
    vcx.run_until_parked();
    // Opening the settings looks again.
    assert_eq!(fake.availability_checks(), 2);
    let view = settings_view(&workspace, vcx).expect("settings are open");
    view.read_with(vcx, |view, cx| {
        assert!(!view.rows(cx).contains(&SettingRow::Assistant));
        let note = view
            .assistant_missing_note(cx)
            .expect("a line says what is missing");
        assert_eq!(
            note,
            "The assistant needs Fake Agent, which is not installed on this Mac. \
             Install Fake Agent, then open the settings again."
        );
        assert!(view.assistant_privacy_note(cx).is_none());
    });
    assert!(vcx.debug_bounds("assistant-missing-note").is_some());
    assert!(vcx.debug_bounds("assistant-privacy-note").is_none());
}

#[gpui::test]
fn installing_the_engine_brings_the_assistant_when_the_settings_are_opened(
    cx: &mut TestAppContext,
) {
    let fake = FakeBackend::not_installed();
    let (_library, _id, workspace, vcx) = workspace_with(&fake, cx);
    assert!(!workspace.read_with(vcx, |w, _| w.assistant_shown()));

    // The user installs the program and opens the settings.
    fake.set_availability(FakeBackend::new().availability());
    vcx.simulate_keystrokes("cmd-,");
    vcx.run_until_parked();
    assert!(workspace.read_with(vcx, |w, _| w.assistant_shown()));
    let view = settings_view(&workspace, vcx).expect("settings are open");
    view.read_with(vcx, |view, cx| {
        assert!(view.rows(cx).contains(&SettingRow::Assistant));
        assert!(view.assistant_missing_note(cx).is_none());
    });
    assert_eq!(palette_titles(vcx).len(), commands_of_the_assistant());
    assert!(vcx.update(|_, cx| menus::assistant_shown(cx)));
}

/// How many commands the assistant has in the palette while it is shown:
/// the app's own and the side chat's.
fn commands_of_the_assistant() -> usize {
    writer_app::actions::assistant_commands().len() + writer_assistant_ui::commands().len()
}

#[gpui::test]
fn an_installed_engine_shows_the_assistant_and_the_switch_hides_it(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    let (library, _id, workspace, vcx) = workspace_with(&fake, cx);
    assert!(workspace.read_with(vcx, |w, _| w.assistant_shown()));
    let titles = palette_titles(vcx);
    assert!(
        titles.contains(&"Ask the assistant".to_string()),
        "{titles:?}"
    );
    assert!(
        titles.contains(&"Go to assistant".to_string()),
        "{titles:?}"
    );
    let names = menu_bar_names(vcx);
    for item in [
        "Ask the Assistant…",
        "Show or Hide Assistant",
        "Go to Assistant",
    ] {
        assert!(names.contains(&item.to_string()), "{item} in {names:?}");
    }

    // The settings: which engine, the switch, the four choices, the note.
    vcx.simulate_keystrokes("cmd-,");
    vcx.run_until_parked();
    let view = settings_view(&workspace, vcx).expect("settings are open");
    view.read_with(vcx, |view, cx| {
        let rows = view.rows(cx);
        let at = rows
            .iter()
            .position(|row| *row == SettingRow::Assistant)
            .expect("the section is listed");
        assert_eq!(&rows[at..at + 5], &SettingRow::ASSISTANT);
        assert_eq!(
            view.description(SettingRow::Assistant, false, cx).as_ref(),
            "Runs on Fake Agent 1.0.0, Fake plan."
        );
        let note = view.assistant_privacy_note(cx).expect("the note is shown");
        assert!(note.contains("sent to Fake Vendor through your own Fake Agent sign-in"));
        assert!(note.contains("Nothing is sent before you send a message"));
    });
    assert!(vcx.debug_bounds("assistant-privacy-note").is_some());
    assert!(vcx.debug_bounds("assistant-missing-note").is_none());

    // Off: the choices go, the assistant goes, live; the switch stays.
    view.update(vcx, |view, cx| view.choose(SettingRow::Assistant, 0, cx));
    vcx.run_until_parked();
    assert!(!library.read_with(vcx, |library, _| {
        library.setting_or(keys::ASSISTANT_ENABLED, true)
    }));
    assert!(!workspace.read_with(vcx, |w, _| w.assistant_shown()));
    assert!(palette_titles(vcx).is_empty());
    assert!(
        !menu_bar_names(vcx)
            .iter()
            .any(|name| name.contains("Assistant"))
    );
    view.read_with(vcx, |view, cx| {
        let rows = view.rows(cx);
        assert!(rows.contains(&SettingRow::Assistant));
        assert!(!rows.contains(&SettingRow::AssistantQuickModel));
        assert!(view.assistant_privacy_note(cx).is_none());
    });

    // And on again.
    view.update(vcx, |view, cx| view.activate(SettingRow::Assistant, cx));
    vcx.run_until_parked();
    assert!(workspace.read_with(vcx, |w, _| w.assistant_shown()));
    assert_eq!(palette_titles(vcx).len(), commands_of_the_assistant());
}

#[gpui::test]
fn signed_out_the_assistant_is_there_and_the_settings_say_how_to_sign_in(cx: &mut TestAppContext) {
    let fake = FakeBackend::signed_out();
    let (_library, _id, workspace, vcx) = workspace_with(&fake, cx);
    assert!(workspace.read_with(vcx, |w, _| w.assistant_shown()));
    vcx.simulate_keystrokes("cmd-,");
    vcx.run_until_parked();
    let view = settings_view(&workspace, vcx).expect("settings are open");
    view.read_with(vcx, |view, cx| {
        assert_eq!(
            view.description(SettingRow::Assistant, false, cx).as_ref(),
            "Fake Agent 1.0.0, not signed in. Open Terminal, run fake-agent and sign in."
        );
    });
}

// ----- the settings ---------------------------------------------------------------

#[gpui::test]
fn the_model_and_effort_of_each_chat_are_chosen_and_kept(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    let (library, _id, workspace, vcx) = workspace_with(&fake, cx);
    let selection = |mode: Mode, cx: &mut VisualTestContext| {
        workspace.read_with(cx, |w, _| {
            choices::selection_for(mode, &w.settings().assistant, &fake_catalog())
        })
    };
    // Nothing chosen: the engine's suggestion for each chat.
    assert_eq!(
        selection(Mode::Quick, vcx),
        Selection::named("swift", "low")
    );
    assert_eq!(
        selection(Mode::Side, vcx),
        Selection::named("deep", "medium")
    );

    vcx.simulate_keystrokes("cmd-,");
    vcx.run_until_parked();
    let view = settings_view(&workspace, vcx).expect("settings are open");

    // The side chat's effort: one step up, from the keyboard.
    view.update_in(vcx, |view, window, cx| {
        let rows = view.rows(cx);
        assert!(rows.contains(&SettingRow::AssistantSideEffort));
        // Step as `right` does on the focused row.
        let _ = window;
        view.step(SettingRow::AssistantSideEffort, 1, cx);
    });
    assert_eq!(selection(Mode::Side, vcx), Selection::named("deep", "high"));
    assert_eq!(
        library.read_with(vcx, |library, _| library
            .setting_or(keys::ASSISTANT_SIDE_EFFORT, String::new())),
        "high"
    );
    // The quick chat is not touched by that.
    assert_eq!(
        selection(Mode::Quick, vcx),
        Selection::named("swift", "low")
    );

    // The side chat's model to "Swift", which does not take "high": the
    // effort gives way to the engine's default, and is stored so.
    view.update(vcx, |view, cx| {
        view.step(SettingRow::AssistantSideModel, -1, cx)
    });
    assert_eq!(
        selection(Mode::Side, vcx),
        Selection {
            model: ModelChoice::named("swift"),
            effort: EffortChoice::AgentDefault,
        }
    );
    assert_eq!(
        library.read_with(vcx, |library, _| library
            .setting_or(keys::ASSISTANT_SIDE_EFFORT, String::new())),
        choices::AGENT_DEFAULT
    );

    // The quick chat's model: the first option is the engine's own default.
    view.update(vcx, |view, cx| {
        view.choose(SettingRow::AssistantQuickModel, 0, cx)
    });
    assert_eq!(selection(Mode::Quick, vcx).model, ModelChoice::AgentDefault);
    assert_eq!(
        library.read_with(vcx, |library, _| library
            .setting_or(keys::ASSISTANT_QUICK_MODEL, String::new())),
        choices::AGENT_DEFAULT
    );

    // A reopened library has what was chosen.
    let settings = library.read_with(vcx, |library, _| writer_app::Settings::load(library));
    assert_eq!(
        choices::selection_for(Mode::Quick, &settings.assistant, &fake_catalog()).model,
        ModelChoice::AgentDefault
    );
    assert_eq!(
        choices::selection_for(Mode::Side, &settings.assistant, &fake_catalog()).model,
        ModelChoice::named("swift")
    );
}

#[gpui::test]
fn the_assistant_rows_are_reached_with_the_keyboard(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    let (_library, _id, workspace, vcx) = workspace_with(&fake, cx);
    vcx.simulate_keystrokes("cmd-,");
    vcx.run_until_parked();
    let view = settings_view(&workspace, vcx).expect("settings are open");
    let rows = view.read_with(vcx, |view, cx| view.rows(cx));
    let at = rows
        .iter()
        .position(|row| *row == SettingRow::Assistant)
        .unwrap();
    for _ in 0..at {
        vcx.simulate_keystrokes("down");
    }
    let focused = |cx: &mut VisualTestContext| {
        view.update_in(cx, |view, window, cx| view.focused_row(window, cx))
    };
    assert_eq!(focused(vcx), Some(SettingRow::Assistant));
    // Space switches the assistant off and on again.
    vcx.simulate_keystrokes("space");
    assert!(!workspace.read_with(vcx, |w, _| w.settings().assistant.enabled));
    vcx.simulate_keystrokes("space");
    assert!(workspace.read_with(vcx, |w, _| w.settings().assistant.enabled));
    // Down to the quick chat's model; right steps to the next model.
    vcx.simulate_keystrokes("down");
    assert_eq!(focused(vcx), Some(SettingRow::AssistantQuickModel));
    vcx.simulate_keystrokes("right");
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.settings().assistant.quick_model.clone()),
        Some("deep".to_string())
    );
    assert_eq!(ASSISTANT_SECTION, "Assistant");
}

// ----- what the user applies -------------------------------------------------------

#[gpui::test]
fn an_answer_goes_into_the_text_as_formatted_text_in_one_undo_step(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(
        &library,
        None,
        &simple_document("Notes", "The keeper climbed the stairs."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    let editor = workspace.read_with(vcx, |w, _| w.editor().clone());
    let before = editor_text(&workspace, vcx);

    // The caret at the end of the paragraph; an answer of three blocks.
    editor.update(vcx, |editor, cx| {
        editor.edit(
            |state| state.select_range(TextRange::empty(Position::new(1, 30))),
            cx,
        );
    });
    let answer = "## Next\n\nA **bold** start.\n\n- one\n- two";
    assert!(vcx.update(|_, cx| apply::insert_at_caret(&editor, answer, cx)));
    editor.read_with(vcx, |editor, _| {
        let blocks = editor.document().blocks();
        let kinds: Vec<&BlockKind> = blocks.iter().map(|block| block.kind()).collect();
        assert!(kinds.contains(&&BlockKind::heading(2)), "{kinds:?}");
        assert!(kinds.contains(&&BlockKind::BulletItem), "{kinds:?}");
        let bold = blocks
            .iter()
            .find(|block| block.text().contains("bold"))
            .expect("the paragraph is there");
        let at = bold.text().find("bold").unwrap();
        assert!(bold.attrs_at(at).unwrap().bold);
        // No Markdown characters are left in the text.
        assert!(!editor.document().plain_text().contains("**"));
        assert!(!editor.document().plain_text().contains("##"));
    });
    // One undo takes all of it back.
    editor.update(vcx, |editor, cx| editor.edit(|state| state.undo(), cx));
    assert_eq!(editor_text(&workspace, vcx), before);

    // Replace Selection: only with a selection.
    assert!(!vcx.update(|_, cx| apply::replace_selection(&editor, "x", cx)));
    editor.update(vcx, |editor, cx| {
        editor.edit(
            |state| state.select_range(TextRange::in_block(1, 4, 10)),
            cx,
        );
    });
    assert_eq!(
        editor.read_with(vcx, |editor, _| apply::selection_markdown(editor)),
        Some("keeper".to_string())
    );
    assert!(vcx.update(|_, cx| apply::replace_selection(&editor, "*warden*", cx)));
    editor.read_with(vcx, |editor, _| {
        let block = &editor.document().blocks()[1];
        assert_eq!(block.text(), "The warden climbed the stairs.");
        assert!(block.attrs_at(5).unwrap().italic);
        assert!(!block.attrs_at(12).unwrap().italic);
    });
    editor.update(vcx, |editor, cx| editor.edit(|state| state.undo(), cx));
    assert_eq!(editor_text(&workspace, vcx), before);

    // Insert Below: whole blocks under the selection, which stays.
    editor.update(vcx, |editor, cx| {
        editor.edit(|state| state.select_range(TextRange::in_block(0, 0, 5)), cx);
    });
    assert!(vcx.update(|_, cx| apply::insert_below(&editor, "> A quote.", cx)));
    editor.read_with(vcx, |editor, _| {
        let blocks = editor.document().blocks();
        assert_eq!(blocks[0].text(), "Notes");
        assert_eq!(blocks[1].kind(), &BlockKind::Quote);
        assert_eq!(blocks[1].text(), "A quote.");
        assert_eq!(blocks[2].text(), "The keeper climbed the stairs.");
    });
    editor.update(vcx, |editor, cx| editor.edit(|state| state.undo(), cx));
    assert_eq!(editor_text(&workspace, vcx), before);

    // Nothing in the answer: nothing happens.
    assert!(!vcx.update(|_, cx| apply::insert_at_caret(&editor, "  \n", cx)));
    assert_eq!(editor_text(&workspace, vcx), before);
}

#[gpui::test]
fn an_answer_of_several_blocks_is_never_merged_into_the_text_around_it(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let document = Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), "Notes"),
        Block::paragraph("The keeper climbed the stairs."),
        Block::paragraph(""),
        Block::paragraph("The lamp was cold."),
    ]);
    add_document(&library, None, &document, cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    let editor = workspace.read_with(vcx, |w, _| w.editor().clone());
    let before = editor_text(&workspace, vcx);
    let texts = |cx: &mut VisualTestContext| -> Vec<(BlockKind, String)> {
        editor.read_with(cx, |editor, _| {
            editor
                .document()
                .blocks()
                .iter()
                .map(|block| (block.kind().clone(), block.text().to_string()))
                .collect()
        })
    };
    let caret_at = |block: usize, offset: usize, cx: &mut VisualTestContext| {
        editor.update(cx, |editor, cx| {
            editor.edit(
                |state| state.select_range(TextRange::empty(Position::new(block, offset))),
                cx,
            );
        });
    };
    let undo = |cx: &mut VisualTestContext| {
        editor.update(cx, |editor, cx| editor.edit(|state| state.undo(), cx));
    };
    let paragraph = |text: &str| (BlockKind::Paragraph, text.to_string());
    let answer = "## A heading\n\nAnd a paragraph.";

    // In the middle of a paragraph: the paragraph is split around the answer.
    caret_at(1, 10, vcx);
    assert!(vcx.update(|_, cx| apply::insert_at_caret(&editor, answer, cx)));
    assert_eq!(
        texts(vcx)[1..5],
        [
            paragraph("The keeper"),
            (BlockKind::heading(2), "A heading".to_string()),
            paragraph("And a paragraph."),
            paragraph(" climbed the stairs."),
        ]
    );
    // The caret is at the end of what went in, and one undo takes it out.
    assert_eq!(
        editor.read_with(vcx, |editor, _| editor.selection().head),
        Position::new(3, "And a paragraph.".len())
    );
    undo(vcx);
    assert_eq!(editor_text(&workspace, vcx), before);

    // In an empty paragraph: the answer takes its place.
    caret_at(2, 0, vcx);
    assert!(vcx.update(|_, cx| apply::insert_at_caret(&editor, answer, cx)));
    assert_eq!(
        texts(vcx)[1..],
        [
            paragraph("The keeper climbed the stairs."),
            (BlockKind::heading(2), "A heading".to_string()),
            paragraph("And a paragraph."),
            paragraph("The lamp was cold."),
        ]
    );
    undo(vcx);
    assert_eq!(editor_text(&workspace, vcx), before);

    // At the start of a block: above it. At its end: below it.
    caret_at(3, 0, vcx);
    assert!(vcx.update(|_, cx| apply::insert_at_caret(&editor, answer, cx)));
    assert_eq!(texts(vcx)[3].1, "A heading");
    assert_eq!(texts(vcx)[5], paragraph("The lamp was cold."));
    undo(vcx);
    caret_at(0, 5, vcx);
    assert!(vcx.update(|_, cx| apply::insert_at_caret(&editor, answer, cx)));
    assert_eq!(texts(vcx)[0], (BlockKind::heading(1), "Notes".to_string()));
    assert_eq!(texts(vcx)[1].1, "A heading");
    undo(vcx);
    assert_eq!(editor_text(&workspace, vcx), before);

    // A selection over two paragraphs is replaced by the blocks.
    editor.update(vcx, |editor, cx| {
        editor.edit(
            |state| state.select_range(TextRange::new(Position::new(1, 0), Position::new(3, 18))),
            cx,
        );
    });
    assert!(vcx.update(|_, cx| apply::replace_selection(&editor, answer, cx)));
    assert_eq!(
        texts(vcx),
        [
            (BlockKind::heading(1), "Notes".to_string()),
            (BlockKind::heading(2), "A heading".to_string()),
            paragraph("And a paragraph."),
        ]
    );
    undo(vcx);
    assert_eq!(editor_text(&workspace, vcx), before);

    // One paragraph is running text: it joins the block and takes the
    // formatting typed text would take there.
    editor.update(vcx, |editor, cx| {
        editor.edit(
            |state| {
                state.select_range(TextRange::in_block(1, 4, 10));
                state.toggle_bold();
            },
            cx,
        );
    });
    assert!(vcx.update(|_, cx| apply::replace_selection(&editor, "*warden*", cx)));
    editor.read_with(vcx, |editor, _| {
        let block = &editor.document().blocks()[1];
        assert_eq!(block.text(), "The warden climbed the stairs.");
        let attrs = block.attrs_at(5).unwrap();
        assert!(attrs.bold && attrs.italic);
        assert!(!block.attrs_at(12).unwrap().bold);
    });
}

#[gpui::test]
fn the_document_and_the_selection_are_read_from_the_live_text(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let id = add_document(
        &library,
        None,
        &simple_document("Notes", "The keeper climbed the stairs."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    let editor = workspace.read_with(vcx, |w, _| w.editor().clone());
    // Typed a moment ago and not saved yet.
    editor.update(vcx, |editor, cx| {
        editor.edit(
            |state| {
                state.select_range(TextRange::empty(Position::new(1, 30)));
                state.insert_text(" It was dusk.");
            },
            cx,
        );
    });
    assert!(!stored_text(&library, id, vcx).contains("dusk"));
    let markdown = editor.read_with(vcx, |editor, _| apply::document_markdown(editor));
    assert_eq!(
        markdown.trim_end(),
        "# Notes\n\nThe keeper climbed the stairs. It was dusk."
    );

    // Nothing selected: no selection. Words of a heading: words, no heading.
    assert_eq!(
        editor.read_with(vcx, |editor, _| apply::selection_markdown(editor)),
        None
    );
    editor.update(vcx, |editor, cx| {
        editor.edit(|state| state.select_range(TextRange::in_block(0, 0, 5)), cx);
    });
    assert_eq!(
        editor.read_with(vcx, |editor, _| apply::selection_markdown(editor)),
        Some("Notes".to_string())
    );
    // Over two blocks the kinds are kept.
    editor.update(vcx, |editor, cx| {
        editor.edit(
            |state| state.select_range(TextRange::new(Position::new(0, 0), Position::new(1, 10))),
            cx,
        );
    });
    assert_eq!(
        editor.read_with(vcx, |editor, _| apply::selection_markdown(editor)),
        Some("# Notes\n\nThe keeper".to_string())
    );
}

// ----- the choices, the conversations and the fake engine's script -----------------

#[test]
fn a_stored_choice_reads_back_as_it_was_made() {
    let catalog = fake_catalog();
    let mut settings = writer_app::settings::AssistantSettings::default();
    assert_eq!(
        choices::selection_for(Mode::Quick, &settings, &catalog),
        catalog.quick_default
    );
    settings.quick_model = Some(choices::stored_model(&ModelChoice::AgentDefault));
    settings.quick_effort = Some(choices::stored_effort(&EffortChoice::named("medium")));
    assert_eq!(
        choices::selection_for(Mode::Quick, &settings, &catalog),
        Selection {
            model: ModelChoice::AgentDefault,
            effort: EffortChoice::named("medium"),
        }
    );
    // A model the engine no longer lists is kept and can be stepped away from.
    settings.side_model = Some("retired".to_string());
    let selection = choices::selection_for(Mode::Side, &settings, &catalog);
    assert_eq!(selection.model, ModelChoice::named("retired"));
    let options = choices::model_options(&catalog, &selection.model);
    assert_eq!(
        options,
        vec![
            ModelChoice::AgentDefault,
            ModelChoice::named("retired"),
            ModelChoice::named("swift"),
            ModelChoice::named("deep"),
        ]
    );
    // "swift" takes two of the three levels.
    assert_eq!(
        choices::effort_options(&catalog, &ModelChoice::named("swift")),
        vec![
            EffortChoice::AgentDefault,
            EffortChoice::named("low"),
            EffortChoice::named("medium"),
        ]
    );
    assert_eq!(choices::keys_for(Mode::Side).0, keys::ASSISTANT_SIDE_MODEL);
}

/// A transcript of `exchanges` questions and answers, each answer `chars`
/// characters long.
fn transcript_of(exchanges: usize, chars: usize) -> Transcript {
    let mut transcript = Transcript::new();
    for ix in 0..exchanges {
        transcript.push_user(format!("Question {ix}"), None);
        let turn = TurnId::next();
        transcript.begin_answer(turn, Selection::default());
        transcript.apply(&writer_assistant::AssistantEvent::Text {
            turn,
            text: "a".repeat(chars),
        });
        transcript.apply(&writer_assistant::AssistantEvent::Finished {
            turn,
            text: None,
            end: writer_assistant::TurnEnd::Complete,
            usage: None,
        });
    }
    transcript
}

#[test]
fn a_stored_conversation_is_the_recent_end_in_whole_exchanges() {
    // Few and short: stored as it is.
    let small = transcript_of(3, 10);
    assert_eq!(conversations::bounded(&small), small);

    // Too many messages: the oldest exchanges go, questions stay with
    // their answers.
    let many = transcript_of(60, 10);
    let bounded = conversations::bounded(&many);
    assert_eq!(bounded.messages.len(), conversations::STORED_MESSAGES);
    assert_eq!(bounded.dropped, 120 - conversations::STORED_MESSAGES);
    assert_eq!(bounded.messages[0].role, writer_assistant::Role::User);
    assert_eq!(bounded.messages.last(), many.messages.last());

    // Too much text: the same, by characters. The last exchange always
    // stays, however long it is.
    let long = transcript_of(5, 50_000);
    let bounded = conversations::bounded(&long);
    assert!(bounded.char_count() <= conversations::STORED_CHARS);
    assert_eq!(bounded.messages[0].role, writer_assistant::Role::User);
    let huge = transcript_of(1, conversations::STORED_CHARS + 10);
    assert_eq!(conversations::bounded(&huge).messages.len(), 2);
}

#[gpui::test]
fn conversations_are_stored_per_document_and_go_with_it(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let first = add_document(&library, None, &simple_document("One", "First."), cx);
    let second = add_document(&library, None, &simple_document("Two", "Second."), cx);

    let mut about_first = transcript_of(2, 20);
    about_first.attach(Attachment::document(second.0, "Two", "Two"));
    let about_second = transcript_of(1, 5);
    cx.update(|cx| {
        assert!(conversations::store(&library, first, &about_first, cx));
        assert!(conversations::store(&library, second, &about_second, cx));
    });
    library.read_with(cx, |library, _| {
        let loaded = conversations::load(library, first);
        assert_eq!(loaded.messages.len(), 4);
        assert_eq!(loaded.messages[1].text, "a".repeat(20));
        assert_eq!(loaded.attachments, about_first.attachments);
        assert_eq!(conversations::load(library, second).messages.len(), 2);
    });

    // An empty conversation is not kept.
    cx.update(|cx| {
        assert!(!conversations::store(
            &library,
            second,
            &Transcript::new(),
            cx
        ))
    });
    library.read_with(cx, |library, _| {
        assert!(!conversations::is_stored(library, second));
        assert!(conversations::load(library, second).is_empty());
    });

    // The document is deleted for good with no workspace to hear of it
    // (the command line tool does that while the app is closed): at the
    // next launch its conversation is dropped.
    library.update(cx, |library, cx| {
        library.trash_document(first, cx).unwrap();
        // Still there while the document is only in the trash.
        assert!(conversations::is_stored(library, first));
        library.purge_document(first, cx).unwrap();
        assert!(conversations::is_stored(library, first));
    });
    cx.update(|cx| conversations::forget_missing(&library, cx));
    library.read_with(cx, |library, _| {
        assert!(!conversations::is_stored(library, first));
    });
}

#[test]
fn a_replies_file_scripts_the_fake_engine() {
    let replies = assistant::fake::parse_replies(
        "First answer,\nwith two lines.\n====\n\
         !tool read_document Reading Chapter 2\n!tool search_library Searching for dusk\n\
         An answer after two tools.\n=====\n\
         !error usage-limit\n====\n\n====\n\
         !error network\nHalf an answer",
    );
    assert_eq!(replies.len(), 4);
    assert_eq!(replies[0].full_text(), "First answer,\nwith two lines.");
    assert_eq!(replies[1].full_text(), "An answer after two tools.");

    // What they do, through the fake engine itself.
    let fake = FakeBackend::new();
    for reply in replies {
        fake.push(reply);
    }
    let mut chat = writer_assistant::ChatSession::quick(Arc::new(fake.clone()));
    let events = chat.events();
    for question in ["one", "two", "three", "four"] {
        chat.ask_quick(question, None);
        for event in events.drain() {
            chat.apply(event);
        }
    }
    let messages = &chat.transcript().messages;
    assert_eq!(messages[1].text, "First answer,\nwith two lines.");
    let tools: Vec<&str> = messages[3]
        .activity
        .iter()
        .map(|activity| activity.description.as_str())
        .collect();
    assert_eq!(tools, ["Reading Chapter 2", "Searching for dusk"]);
    assert!(matches!(
        messages[5].error(),
        Some(AssistantError::UsageLimit {
            resets_at: Some(_),
            ..
        })
    ));
    assert_eq!(messages[7].text, "Half an answer");
    assert!(matches!(
        messages[7].error(),
        Some(AssistantError::Network { .. })
    ));
}
