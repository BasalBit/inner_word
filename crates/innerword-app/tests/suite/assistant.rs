//! The assistant in the assembled app.
//!
//! **No test here ever runs Claude Code.** Every test that has an assistant
//! installs `innerword_assistant::FakeBackend`, which starts no program and
//! answers from a script; a workspace opened without one has no assistant
//! at all (the tests of the rest of the suite).

use std::sync::Arc;

use gpui::TestAppContext;
use innerword_app::{
    actions::CATEGORY_ASSISTANT,
    assistant::{self, apply, choices, conversations},
    commands, menus,
    settings::keys,
    settings_view::{ASSISTANT_SECTION, SettingRow, SettingsView},
};
use innerword_assistant::{
    AssistantError, Attachment, Backend as _, EffortChoice, FakeBackend, Mode, ModelChoice,
    Selection, Transcript, TurnId, fake_catalog,
};
use innerword_core::{Position, TextRange};

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
    innerword_app::actions::assistant_commands().len() + innerword_assistant_ui::commands().len()
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
    let settings = library.read_with(vcx, |library, _| innerword_app::Settings::load(library));
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
    assert!(vcx.update(|_, cx| apply::replace_selection(&editor, "*warden*", cx)));
    editor.read_with(vcx, |editor, _| {
        let block = &editor.document().blocks()[1];
        assert_eq!(block.text(), "The warden climbed the stairs.");
        assert!(block.attrs_at(5).unwrap().italic);
        assert!(!block.attrs_at(12).unwrap().italic);
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
fn the_document_is_read_from_the_live_text(cx: &mut TestAppContext) {
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
}

// ----- the choices, the conversations and the fake engine's script -----------------

#[test]
fn a_stored_choice_reads_back_as_it_was_made() {
    let catalog = fake_catalog();
    let mut settings = innerword_app::settings::AssistantSettings::default();
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
        transcript.apply(&innerword_assistant::AssistantEvent::Text {
            turn,
            text: "a".repeat(chars),
        });
        transcript.apply(&innerword_assistant::AssistantEvent::Finished {
            turn,
            text: None,
            end: innerword_assistant::TurnEnd::Complete,
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
    assert_eq!(bounded.messages[0].role, innerword_assistant::Role::User);
    assert_eq!(bounded.messages.last(), many.messages.last());

    // Too much text: the same, by characters. The last exchange always
    // stays, however long it is.
    let long = transcript_of(5, 50_000);
    let bounded = conversations::bounded(&long);
    assert!(bounded.char_count() <= conversations::STORED_CHARS);
    assert_eq!(bounded.messages[0].role, innerword_assistant::Role::User);
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
    let mut chat = innerword_assistant::ChatSession::quick(Arc::new(fake.clone()));
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

// ----- the two chats in the workspace ----------------------------------------------

use innerword_app::workspace::{ASK_THE_ASSISTANT, RightPanel};
use innerword_assistant::{FakeReply, MessageState, Role};
use innerword_assistant_quick::{QuickAction, QuickChat};
use innerword_assistant_ui::{ApplyKind, AssistantPanel};

fn quick(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<QuickChat> {
    workspace.read_with(cx, |w, _| w.quick_chat().expect("an engine").clone())
}

fn side(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<AssistantPanel> {
    workspace.read_with(cx, |w, _| w.side_chat().expect("an engine").clone())
}

fn quick_open(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    quick(workspace, cx).read_with(cx, |quick, _| quick.is_open())
}

fn right_panel(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<RightPanel> {
    workspace.read_with(cx, |w, _| w.right_panel_shown())
}

fn side_has_keyboard(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    workspace.update_in(cx, |w, window, cx| w.assistant_panel_has_focus(window, cx))
}

/// Selects `range` in the text, as the keyboard would.
fn select(workspace: &Entity<Workspace>, range: TextRange, cx: &mut VisualTestContext) {
    let editor = workspace.read_with(cx, |w, _| w.editor().clone());
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.select_range(range), cx);
    });
    cx.run_until_parked();
}

/// Runs the palette command titled `title`, as choosing it there does.
fn run_palette_command(workspace: &Entity<Workspace>, title: &str, cx: &mut VisualTestContext) {
    let command = cx.update(|_, cx| {
        commands::all(cx)
            .into_iter()
            .find(|command| command.title.as_ref() == title)
            .unwrap_or_else(|| panic!("no command {title:?} in the palette"))
    });
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.run_command(&command, window, cx)
    });
    cx.run_until_parked();
}

/// The text of the answers of `transcript`, in order.
fn answers(transcript: &Transcript) -> Vec<String> {
    transcript
        .messages
        .iter()
        .filter(|message| message.role == Role::Assistant)
        .map(|message| message.text.clone())
        .collect()
}

#[gpui::test]
fn the_quick_chat_opens_in_every_way_and_sends_only_the_selection(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    let (_library, _id, workspace, vcx) = workspace_with(&fake, cx);
    assert!(editor_focused(&workspace, vcx));
    assert!(!quick_open(&workspace, vcx));

    // The key, on a selection: "keeper" in the paragraph.
    select(&workspace, TextRange::in_block(1, 4, 10), vcx);
    vcx.simulate_keystrokes("cmd-j");
    vcx.run_until_parked();
    assert!(quick_open(&workspace, vcx));
    let chat = quick(&workspace, vcx);
    assert_eq!(
        chat.read_with(vcx, |chat, _| chat.quote()),
        Some("keeper".to_string())
    );
    // The keyboard is in its composer: what is typed is the question.
    fake.reply("warden, *guardian*");
    vcx.simulate_input("Another word?");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let sent = fake.last_sent().expect("a message was sent");
    assert!(sent.contains("keeper"), "{sent}");
    assert!(sent.contains("Another word?"), "{sent}");
    // Nothing else of the document, its title or the library went along.
    assert!(!sent.contains("climbed"), "{sent}");
    assert!(!sent.contains("Lighthouse"), "{sent}");
    // No tools, and the quick chat's own model and effort.
    let started = fake.last_started().expect("a conversation was started");
    assert!(started.tools.is_none());
    assert_eq!(started.selection, Selection::named("swift", "low"));
    assert_eq!(
        chat.read_with(vcx, |chat, _| answers(chat.transcript().unwrap())),
        ["warden, *guardian*"]
    );
    // A second question goes on in the same conversation.
    fake.reply("sentinel");
    vcx.simulate_input("One more");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(fake.started().len(), 1);
    assert_eq!(
        chat.read_with(vcx, |chat, _| chat.transcript().unwrap().len()),
        4
    );

    // The key again closes it, and the keyboard is back in the text.
    vcx.simulate_keystrokes("cmd-j");
    vcx.run_until_parked();
    assert!(!quick_open(&workspace, vcx));
    assert!(editor_focused(&workspace, vcx));

    // With nothing selected it is the question alone.
    select(&workspace, TextRange::empty(Position::new(1, 3)), vcx);
    run_palette_command(&workspace, "Ask the assistant", vcx);
    assert!(quick_open(&workspace, vcx));
    assert_eq!(chat.read_with(vcx, |chat, _| chat.quote()), None);
    fake.reply("Forty-two.");
    vcx.simulate_input("What is six times seven?");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(
        fake.last_sent().as_deref(),
        Some("What is six times seven?")
    );
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!quick_open(&workspace, vcx));

    // The text menu, from the keyboard: "Ask the Assistant" with its key.
    select(&workspace, TextRange::in_block(1, 4, 10), vcx);
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    let menu = vcx
        .update(|window, cx| innerword_ui::ContextMenu::current(window, cx))
        .expect("the text menu is open");
    assert!(
        menu.read_with(vcx, |menu, _| menu.labels())
            .contains(&ASK_THE_ASSISTANT.to_string())
    );
    assert!(
        vcx.update(|window, cx| {
            innerword_ui::ContextMenu::choose(ASK_THE_ASSISTANT, window, cx)
        })
    );
    vcx.run_until_parked();
    assert!(quick_open(&workspace, vcx));
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();

    // The format bar over a selection: a button beside "Add Comment".
    select(&workspace, TextRange::in_block(1, 4, 10), vcx);
    let tools = workspace.read_with(vcx, |w, _| w.tools().clone());
    assert!(tools.update_in(vcx, |tools, window, cx| tools.show_format_bar(window, cx)));
    vcx.run_until_parked();
    let button = tools
        .read_with(vcx, |tools, cx| tools.format_bar_items(cx))
        .into_iter()
        .find(|item| item.label == ASK_THE_ASSISTANT)
        .expect("the format bar has the button");
    assert!(tools.update_in(vcx, |tools, window, cx| {
        tools.press_format_bar_button(&button.id, window, cx)
    }));
    vcx.run_until_parked();
    assert!(quick_open(&workspace, vcx));
    assert_eq!(
        chat.read_with(vcx, |chat, _| chat.quote()),
        Some("keeper".to_string())
    );
    // The bar has made way for the chat, which has the keyboard.
    assert!(!tools.read_with(vcx, |tools, _| tools.format_bar_visible()));
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();

    // The menu bar's item is the same action.
    vcx.dispatch_action(innerword_app::actions::AskAssistant);
    vcx.run_until_parked();
    assert!(quick_open(&workspace, vcx));
}

#[gpui::test]
fn a_quick_answer_replaces_the_selection_in_one_undo_step(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    let (_library, _id, workspace, vcx) = workspace_with(&fake, cx);
    let before = editor_text(&workspace, vcx);
    select(&workspace, TextRange::in_block(1, 4, 10), vcx);
    vcx.simulate_keystrokes("cmd-j");
    vcx.run_until_parked();
    fake.reply("**warden**");
    vcx.simulate_input("Another word?");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();

    let chat = quick(&workspace, vcx);
    assert!(chat.update_in(vcx, |chat, window, cx| {
        chat.apply_answer(QuickAction::Replace, window, cx)
    }));
    vcx.run_until_parked();
    let editor = workspace.read_with(vcx, |w, _| w.editor().clone());
    editor.read_with(vcx, |editor, _| {
        let block = &editor.document().blocks()[1];
        assert_eq!(block.text(), "The warden climbed the stairs at dusk.");
        assert!(block.attrs_at(5).unwrap().bold);
    });
    // The popover is gone, the text has the keyboard, the document is
    // marked as edited.
    assert!(!quick_open(&workspace, vcx));
    assert!(editor_focused(&workspace, vcx));
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.save_state()),
        Some(innerword_app::SaveState::Edited)
    );
    vcx.simulate_keystrokes("cmd-z");
    vcx.run_until_parked();
    assert_eq!(editor_text(&workspace, vcx), before);
}

#[gpui::test]
fn the_side_chat_opens_closes_and_takes_the_keyboard(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    let (library, _id, workspace, vcx) = workspace_with(&fake, cx);
    assert_eq!(right_panel(&workspace, vcx), None);
    assert!(vcx.debug_bounds("assistant-panel").is_none());

    // cmd-3 opens it with the keyboard in its composer, and remembers.
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Assistant));
    assert!(vcx.debug_bounds("assistant-panel").is_some());
    assert!(side_has_keyboard(&workspace, vcx));
    assert!(library.read_with(vcx, |library, _| {
        library.setting_or(keys::ASSISTANT_PANEL_OPEN, false)
    }));
    assert!(side(&workspace, vcx).read_with(vcx, |side, _| side.is_shown()));

    // Escape hands the keyboard back to the text; the panel stays.
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(editor_focused(&workspace, vcx));
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Assistant));
    // cmd-3 goes there again.
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    assert!(side_has_keyboard(&workspace, vcx));

    // "Show or hide assistant" hides it; the keyboard returns to the text.
    run_palette_command(&workspace, "Show or hide assistant", vcx);
    assert_eq!(right_panel(&workspace, vcx), None);
    assert!(editor_focused(&workspace, vcx));
    assert!(!library.read_with(vcx, |library, _| {
        library.setting_or(keys::ASSISTANT_PANEL_OPEN, true)
    }));
    assert!(!side(&workspace, vcx).read_with(vcx, |side, _| side.is_shown()));
    // And shows it again, ready to type.
    vcx.dispatch_action(innerword_app::actions::ToggleAssistant);
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Assistant));
    assert!(side_has_keyboard(&workspace, vcx));

    // Its own close control closes it too.
    side(&workspace, vcx).update_in(vcx, |side, window, cx| {
        side.press_header(
            innerword_assistant_ui::HeaderControl::Close,
            false,
            window,
            cx,
        )
    });
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), None);
    assert!(editor_focused(&workspace, vcx));

    // A command of the side chat chosen while it is closed brings it.
    run_palette_command(&workspace, "New chat", vcx);
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Assistant));

    // Without a document there is nothing to talk about: the panel goes
    // with the document and comes back with the next one.
    workspace.update_in(vcx, |w, window, cx| w.close_document(window, cx));
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), None);
    workspace.update_in(vcx, |w, window, cx| {
        let id = w.new_document(window, cx);
        assert!(id.is_some());
    });
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Assistant));
}

#[gpui::test]
fn the_side_chat_sends_the_live_document_and_what_was_added(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    init(cx);
    install(&fake, cx);
    let library = library(cx);
    let (folder, id, chapter) = library.update(cx, |library, cx| {
        let folder = library.create_folder(None, "The Salt Road", cx).unwrap().id;
        let form = StoredForm::of(
            &simple_document("Chapter 1", "The road began at the sea."),
            None,
        );
        let id = library
            .create_document(Some(folder), &form.content(), cx)
            .unwrap()
            .id;
        let form = StoredForm::of(&simple_document("Chapter 2", "Salt on every stone."), None);
        let chapter = library
            .create_document(Some(folder), &form.content(), cx)
            .unwrap()
            .id;
        (folder, id, chapter)
    });
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |w, window, cx| w.open_document(id, true, window, cx));
    vcx.run_until_parked();

    // Typed a moment ago at the end of the paragraph, not saved yet.
    select(&workspace, TextRange::empty(Position::new(1, 26)), vcx);
    type_text(&workspace, " Nobody was on the quay.", vcx);
    assert!(!stored_text(&library, id, vcx).contains("quay"));

    // What the workspace hands over: the text as it is in the editor, the
    // title and the path in the library.
    let open = workspace
        .read_with(vcx, |w, cx| w.assistant_open_document(cx))
        .expect("a document is open");
    assert_eq!(open.id, Some(id.0));
    assert_eq!(open.title, "Chapter 1");
    assert_eq!(open.path, "The Salt Road/Chapter 1");
    assert!(open.markdown.contains("Nobody was on the quay."));

    // The picker's list: the folder and the documents of the library.
    let offered = library.read_with(vcx, |library, _| {
        innerword_app::workspace::library_attachments(library)
    });
    assert!(offered.contains(&Attachment::folder(
        folder.0,
        "The Salt Road",
        "The Salt Road"
    )));
    assert!(offered.contains(&Attachment::document(
        chapter.0,
        "Chapter 2",
        "The Salt Road/Chapter 2"
    )));

    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    let panel = side(&workspace, vcx);
    // The open document itself is not offered as something to add.
    let candidates = panel.read_with(vcx, |panel, cx| panel.context_candidates(cx));
    assert!(!candidates.iter().any(|candidate| candidate.id == id.0
        && candidate.kind == innerword_assistant::AttachmentKind::Document));
    assert!(panel.update(vcx, |panel, cx| {
        panel.attach(
            Attachment::document(chapter.0, "Chapter 2", "The Salt Road/Chapter 2"),
            cx,
        )
    }));

    // A message, from the keyboard.
    fake.reply("It opens well.");
    vcx.simulate_input("How does it open?");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let sent = fake.last_sent().expect("a message was sent");
    assert!(sent.contains("How does it open?"), "{sent}");
    assert!(
        sent.contains("The road began at the sea. Nobody was on the quay."),
        "{sent}"
    );
    assert!(sent.contains("Chapter 1"), "{sent}");
    assert!(sent.contains("The Salt Road/Chapter 1"), "{sent}");
    // The added item is named; its text is for the tools to fetch.
    assert!(sent.contains("Chapter 2"), "{sent}");
    let started = fake.last_started().expect("a conversation was started");
    assert_eq!(started.selection, Selection::named("deep", "medium"));
    assert_eq!(
        panel.read_with(vcx, |panel, _| answers(panel.transcript())),
        ["It opens well."]
    );

    // The next message has the text as it is then.
    type_text(&workspace, " A gull watched.", vcx);
    fake.reply("Better.");
    panel.update_in(vcx, |panel, window, cx| panel.send("And now?", window, cx));
    vcx.run_until_parked();
    let sent = fake.last_sent().unwrap();
    assert!(sent.contains("A gull watched."), "{sent}");
}

#[gpui::test]
fn the_side_chat_is_given_the_library_as_read_only_tools(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    init(cx);
    install(&fake, cx);
    // A library on disk, and the `innerword` tool of this copy of the app.
    let directory = tempfile::tempdir().unwrap();
    let data_dir = directory.path().canonicalize().unwrap().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let tool = directory
        .path()
        .canonicalize()
        .unwrap()
        .join("Inner Word.app/Contents/MacOS/innerword");
    std::fs::create_dir_all(tool.parent().unwrap()).unwrap();
    std::fs::write(&tool, "#!/bin/sh\n").unwrap();
    let home = directory.path().canonicalize().unwrap().join("home");
    std::fs::create_dir_all(&home).unwrap();
    cx.update(|cx| {
        innerword_app::agents::set_environment(
            innerword_app::agents::Environment::isolated(home, tool.clone()),
            cx,
        )
    });
    let library = cx.new(|_| Library::open(data_dir.join("library.db")).unwrap());
    library.update(cx, |library, cx| {
        library
            .set_setting(keys::WELCOME_CREATED, &true, cx)
            .unwrap();
    });
    add_document(&library, None, &simple_document("Doc", "Text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    vcx.run_until_parked();

    let server = library
        .read_with(vcx, |library, cx| {
            innerword_app::workspace::library_tool_server(library, cx)
        })
        .expect("the tool and the library are there");
    assert_eq!(server.command, tool);
    assert_eq!(
        server.args,
        ["--library", data_dir.to_str().unwrap(), "mcp"]
    );
    // Reading only: nothing that writes is allowed, everything that
    // writes is withheld.
    assert!(server.allowed.iter().any(|tool| tool == "read_document"));
    assert!(server.allowed.iter().any(|tool| tool == "search_library"));
    for writes in ["create_document", "edit_document", "trash_document"] {
        assert!(
            !server.allowed.iter().any(|tool| tool == writes),
            "{writes}"
        );
        assert!(server.denied.iter().any(|tool| tool == writes), "{writes}");
    }

    // A message of the side chat starts the engine with exactly that.
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    side(&workspace, vcx).update_in(vcx, |panel, window, cx| panel.send("Hello", window, cx));
    vcx.run_until_parked();
    let started = fake.last_started().expect("a conversation was started");
    assert_eq!(started.tools.as_ref(), Some(&server));
    // The quick chat never has tools.
    select(&workspace, TextRange::in_block(0, 0, 3), vcx);
    vcx.simulate_keystrokes("cmd-j");
    vcx.run_until_parked();
    quick(&workspace, vcx).update_in(vcx, |quick, window, cx| quick.ask("Hm?", window, cx));
    vcx.run_until_parked();
    assert!(fake.last_started().unwrap().tools.is_none());
}

#[gpui::test]
fn an_answer_of_the_side_chat_is_applied_to_the_text_and_undone(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    let (_library, _id, workspace, vcx) = workspace_with(&fake, cx);
    let before = editor_text(&workspace, vcx);
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    let panel = side(&workspace, vcx);
    fake.reply("the *old* keeper");
    panel.update_in(vcx, |panel, window, cx| panel.send("Who?", window, cx));
    vcx.run_until_parked();
    let answer = panel.read_with(vcx, |panel, _| {
        panel.transcript().last_answer().expect("an answer").id
    });

    // Nothing is selected in the text: Replace Selection is not offered,
    // and asking for it does nothing.
    select(&workspace, TextRange::empty(Position::new(1, 0)), vcx);
    assert!(!panel.update(vcx, |panel, cx| panel.apply(
        ApplyKind::ReplaceSelection,
        answer,
        cx
    )));
    assert_eq!(editor_text(&workspace, vcx), before);

    // With a selection it replaces it, formatted, in one undo step.
    select(&workspace, TextRange::in_block(1, 0, 10), vcx);
    assert!(panel.update(vcx, |panel, cx| panel.apply(
        ApplyKind::ReplaceSelection,
        answer,
        cx
    )));
    vcx.run_until_parked();
    let editor = workspace.read_with(vcx, |w, _| w.editor().clone());
    editor.read_with(vcx, |editor, _| {
        let block = &editor.document().blocks()[1];
        assert_eq!(block.text(), "the old keeper climbed the stairs at dusk.");
        assert!(block.attrs_at(4).unwrap().italic);
        assert!(!block.attrs_at(9).unwrap().italic);
    });
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.save_state()),
        Some(innerword_app::SaveState::Edited)
    );
    editor.update(vcx, |editor, cx| editor.edit(|state| state.undo(), cx));
    vcx.run_until_parked();
    assert_eq!(editor_text(&workspace, vcx), before);

    // Insert at Caret, from the keyboard in the side chat: cmd-enter acts
    // on the latest answer.
    select(&workspace, TextRange::empty(Position::new(1, 3)), vcx);
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    assert!(side_has_keyboard(&workspace, vcx));
    vcx.simulate_keystrokes("cmd-enter");
    vcx.run_until_parked();
    assert_eq!(
        editor_text(&workspace, vcx).lines().nth(1),
        Some("Thethe old keeper keeper climbed the stairs at dusk.")
    );
    editor.update(vcx, |editor, cx| editor.edit(|state| state.undo(), cx));
    vcx.run_until_parked();
    assert_eq!(editor_text(&workspace, vcx), before);

    // The assistant never changes the text by itself: two answers later
    // the document is what it was.
    fake.reply("# A heading\n\nAnd more.");
    panel.update_in(vcx, |panel, window, cx| panel.send("More", window, cx));
    vcx.run_until_parked();
    assert_eq!(editor_text(&workspace, vcx), before);
}

#[gpui::test]
fn each_document_keeps_its_conversation_across_switches_and_a_relaunch(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    init(cx);
    install(&fake, cx);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("library.db");
    let open_library = |cx: &mut TestAppContext| {
        let library = cx.new(|_| Library::open(&path).unwrap());
        library.update(cx, |library, cx| {
            library
                .set_setting(keys::WELCOME_CREATED, &true, cx)
                .unwrap();
        });
        library
    };
    let library = open_library(cx);
    let first = add_document(&library, None, &simple_document("One", "First text."), cx);
    let second = add_document(&library, None, &simple_document("Two", "Second text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(first, true, window, cx)
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    let panel = side(&workspace, vcx);
    let conversation = |cx: &mut VisualTestContext| -> Vec<String> {
        panel.read_with(cx, |panel, _| {
            panel
                .transcript()
                .messages
                .iter()
                .map(|message| message.text.clone())
                .collect()
        })
    };

    fake.reply("About one.");
    panel.update_in(vcx, |panel, window, cx| {
        panel.send("First question", window, cx)
    });
    vcx.run_until_parked();
    assert_eq!(conversation(vcx), ["First question", "About one."]);
    assert!(panel.update(vcx, |panel, cx| {
        panel.attach(Attachment::document(second.0, "Two", "Two"), cx)
    }));

    // Another document: its own, empty conversation.
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(second, true, window, cx)
    });
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Assistant));
    assert!(conversation(vcx).is_empty());
    assert!(panel.read_with(vcx, |panel, _| panel.attachments().is_empty()));
    fake.reply("About two.");
    panel.update_in(vcx, |panel, window, cx| {
        panel.send("Second question", window, cx)
    });
    vcx.run_until_parked();
    assert_eq!(conversation(vcx), ["Second question", "About two."]);

    // Back: the first conversation, with what was added to it.
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(first, true, window, cx)
    });
    vcx.run_until_parked();
    assert_eq!(conversation(vcx), ["First question", "About one."]);
    assert_eq!(
        panel.read_with(vcx, |panel, _| panel.attachments().len()),
        1
    );

    // Clear removes the messages and keeps what was added; it is stored so.
    panel.update_in(vcx, |panel, window, cx| panel.clear(window, cx));
    vcx.run_until_parked();
    assert!(conversation(vcx).is_empty());
    library.read_with(vcx, |library, _| {
        let stored = conversations::load(library, first);
        assert!(stored.messages.is_empty());
        assert_eq!(stored.attachments.len(), 1);
        assert_eq!(conversations::load(library, second).messages.len(), 2);
    });
    // New Chat removes both, and then nothing is stored for the document.
    fake.reply("Again.");
    panel.update_in(vcx, |panel, window, cx| panel.send("Once more", window, cx));
    vcx.run_until_parked();
    panel.update_in(vcx, |panel, window, cx| panel.new_chat(window, cx));
    vcx.run_until_parked();
    assert!(conversation(vcx).is_empty());
    assert!(panel.read_with(vcx, |panel, _| panel.attachments().is_empty()));
    library.read_with(vcx, |library, _| {
        assert!(!conversations::is_stored(library, first))
    });

    // Quit and launch again: the second document's conversation is there,
    // the side chat is open as it was left.
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(second, true, window, cx)
    });
    vcx.run_until_parked();
    workspace.update(vcx, |w, cx| {
        w.flush(cx);
    });
    drop(panel);
    drop(workspace);
    drop(library);
    cx.run_until_parked();

    let library = open_library(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(second)
    );
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Assistant));
    let panel = side(&workspace, vcx);
    assert_eq!(
        panel.read_with(vcx, |panel, _| panel
            .transcript()
            .messages
            .iter()
            .map(|message| (message.text.clone(), message.state.clone()))
            .collect::<Vec<_>>()),
        [
            ("Second question".to_string(), MessageState::Done),
            ("About two.".to_string(), MessageState::Done),
        ]
    );
    // The next message tells the engine what was said before.
    fake.reply("Still two.");
    panel.update_in(vcx, |panel, window, cx| {
        panel.send("And after the relaunch?", window, cx)
    });
    vcx.run_until_parked();
    let started = fake.last_started().unwrap();
    assert_eq!(started.history.len(), 2);
    assert_eq!(started.history[0].text, "Second question");

    // A document that is deleted for good takes its conversation along.
    library.update(vcx, |library, cx| {
        library.trash_document(second, cx).unwrap();
    });
    vcx.run_until_parked();
    library.read_with(vcx, |library, _| {
        assert!(conversations::is_stored(library, second))
    });
    library.update(vcx, |library, cx| {
        library.purge_document(second, cx).unwrap();
    });
    vcx.run_until_parked();
    library.read_with(vcx, |library, _| {
        assert!(!conversations::is_stored(library, second))
    });
}

#[gpui::test]
fn the_comments_and_the_side_chat_share_the_slot_one_at_a_time(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    let (library, _id, workspace, vcx) = workspace_with(&fake, cx);
    // (Whether something is drawn is asked of the workspace: the test
    // window keeps the place of an element that is no longer drawn.)
    let tabs =
        |cx: &mut VisualTestContext| workspace.read_with(cx, |w, _| w.right_panel_has_tabs());

    // Only the side chat: it has the slot to itself, without tabs.
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Assistant));
    assert!(!tabs(vcx));

    // A comment is added: the comments come forward (its composer has the
    // keyboard), and the slot has two tabs.
    vcx.simulate_keystrokes("escape");
    select(&workspace, TextRange::in_block(1, 4, 10), vcx);
    vcx.simulate_keystrokes("cmd-alt-m");
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Comments));
    assert!(tabs(vcx));
    assert!(vcx.debug_bounds("right-panel-tabs").is_some());
    vcx.simulate_input("Check this");
    vcx.simulate_keystrokes("cmd-enter");
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Comments));
    assert!(!side(&workspace, vcx).read_with(vcx, |side, _| side.is_shown()));

    // cmd-3 brings the side chat forward, cmd-2 the comments; the choice
    // is remembered.
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Assistant));
    assert!(side_has_keyboard(&workspace, vcx));
    assert_eq!(
        library.read_with(vcx, |library, _| library
            .setting_or(keys::RIGHT_PANEL, String::new())),
        "assistant"
    );
    vcx.simulate_keystrokes("cmd-2");
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Comments));
    assert!(workspace.update_in(vcx, |w, window, cx| {
        w.comments().read(cx).has_focus(window, cx)
    }));
    assert_eq!(
        library.read_with(vcx, |library, _| library
            .setting_or(keys::RIGHT_PANEL, String::new())),
        "comments"
    );

    // The tabs, with the mouse: a click brings the other panel forward,
    // and the keyboard, which was in the slot, goes with it.
    let tab = vcx
        .debug_bounds(RightPanel::Assistant.tab_selector())
        .expect("the tab is drawn");
    vcx.simulate_click(tab.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Assistant));
    assert!(side_has_keyboard(&workspace, vcx));
    let tab = vcx
        .debug_bounds(RightPanel::Comments.tab_selector())
        .expect("the tab is drawn");
    vcx.simulate_click(tab.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Comments));

    // Closing the side chat leaves the comments alone, without tabs.
    vcx.dispatch_action(innerword_app::actions::ToggleAssistant);
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Assistant));
    vcx.dispatch_action(innerword_app::actions::ToggleAssistant);
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Comments));
    assert!(!tabs(vcx));

    // Hiding the comments (cmd-alt-c) with the side chat open leaves the
    // side chat.
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    vcx.simulate_keystrokes("cmd-alt-c");
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Assistant));
    assert!(!tabs(vcx));
}

#[gpui::test]
fn each_panel_at_the_right_keeps_a_width_of_its_own(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    let (library, _id, workspace, vcx) = workspace_with(&fake, cx);
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    let width = |cx: &mut VisualTestContext| {
        f32::from(
            cx.debug_bounds("right-panel")
                .expect("the slot is drawn")
                .size
                .width,
        )
    };
    assert_eq!(
        width(vcx),
        innerword_app::workspace::DEFAULT_ASSISTANT_PANEL_WIDTH
    );

    // Dragging the edge of the slot to the left makes the panel wider.
    let handle = vcx
        .debug_bounds("right-panel-resize")
        .expect("the edge can be dragged");
    let from = handle.center();
    let to = gpui::point(from.x - gpui::px(60.), from.y);
    vcx.simulate_mouse_down(from, gpui::MouseButton::Left, gpui::Modifiers::none());
    vcx.simulate_mouse_move(to, Some(gpui::MouseButton::Left), gpui::Modifiers::none());
    vcx.simulate_mouse_up(to, gpui::MouseButton::Left, gpui::Modifiers::none());
    vcx.run_until_parked();
    let dragged = width(vcx);
    assert!((dragged - 440.).abs() < 2., "{dragged}");
    assert_eq!(
        library.read_with(vcx, |library, _| library
            .setting_or(keys::ASSISTANT_PANEL_WIDTH, 0.0_f32)),
        workspace.read_with(vcx, |w, _| w.right_panel_width_of(RightPanel::Assistant))
    );
    // The comments keep theirs.
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.right_panel_width_of(RightPanel::Comments)),
        innerword_app::workspace::DEFAULT_RIGHT_PANEL_WIDTH
    );
    // A double click on the edge brings the default back.
    let handle = vcx.debug_bounds("right-panel-resize").unwrap();
    vcx.simulate_event(gpui::MouseDownEvent {
        position: handle.center(),
        modifiers: gpui::Modifiers::none(),
        button: gpui::MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    vcx.simulate_mouse_up(
        handle.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    vcx.run_until_parked();
    assert_eq!(
        width(vcx),
        innerword_app::workspace::DEFAULT_ASSISTANT_PANEL_WIDTH
    );
}

#[gpui::test]
fn the_note_about_where_the_text_goes_is_shown_once(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    let (library, _id, workspace, vcx) = workspace_with(&fake, cx);
    assert!(workspace.read_with(vcx, |w, cx| w.assistant_notice_pending(cx)));
    let panel = side(&workspace, vcx);
    let chat = quick(&workspace, vcx);
    // Both chats show it before anything was sent.
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    assert!(panel.read_with(vcx, |panel, _| panel.shows_privacy_notice()));
    assert!(chat.read_with(vcx, |chat, _| chat.shows_privacy_notice()));
    assert!(fake.sent().is_empty());

    // The first message acknowledges it: it is gone from both, for good.
    fake.reply("Yes.");
    vcx.simulate_input("Is this a good opening?");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(fake.sent().len(), 1);
    assert!(!panel.read_with(vcx, |panel, _| panel.shows_privacy_notice()));
    assert!(!chat.read_with(vcx, |chat, _| chat.shows_privacy_notice()));
    assert!(!workspace.read_with(vcx, |w, cx| w.assistant_notice_pending(cx)));
    assert!(library.read_with(vcx, |library, _| {
        library.setting_or(keys::ASSISTANT_NOTICE_ACCEPTED, false)
    }));

    // A workspace opened on the same library later does not show it.
    drop(panel);
    drop(chat);
    let (workspace, vcx) = open_workspace(&library, vcx);
    vcx.run_until_parked();
    assert!(!side(&workspace, vcx).read_with(vcx, |panel, _| panel.shows_privacy_notice()));
    assert!(!quick(&workspace, vcx).read_with(vcx, |chat, _| chat.shows_privacy_notice()));
}

#[gpui::test]
fn what_is_chosen_in_a_chat_is_what_the_settings_show(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    let (library, _id, workspace, vcx) = workspace_with(&fake, cx);
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    let panel = side(&workspace, vcx);
    // The model picker of the side chat: "Swift" instead of "Deep".
    panel.update_in(vcx, |panel, window, cx| {
        panel.open_picker(innerword_assistant_ui::PickerKind::Model, true, window, cx);
    });
    vcx.run_until_parked();
    assert!(
        vcx.update(|window, cx| innerword_ui::ContextMenu::is_open(window, cx)),
        "the model picker is a menu"
    );
    assert!(vcx.update(|window, cx| innerword_ui::ContextMenu::choose("Swift", window, cx)));
    vcx.run_until_parked();
    assert_eq!(
        library.read_with(vcx, |library, _| library
            .setting_or(keys::ASSISTANT_SIDE_MODEL, String::new())),
        "swift"
    );
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.settings().assistant.side_model.clone()),
        Some("swift".to_string())
    );
    // The quick chat's choice is its own.
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.settings().assistant.quick_model.clone()),
        None
    );

    // A change in the settings reaches the chats at once.
    library.update(vcx, |library, cx| {
        library
            .set_setting(keys::ASSISTANT_QUICK_MODEL, "deep", cx)
            .unwrap();
        library
            .set_setting(keys::ASSISTANT_QUICK_EFFORT, "high", cx)
            .unwrap();
    });
    vcx.run_until_parked();
    assert_eq!(
        quick(&workspace, vcx).read_with(vcx, |chat, _| chat.selection().clone()),
        Selection::named("deep", "high")
    );
    select(&workspace, TextRange::in_block(1, 4, 10), vcx);
    vcx.simulate_keystrokes("cmd-j");
    vcx.run_until_parked();
    quick(&workspace, vcx).update_in(vcx, |quick, window, cx| quick.ask("Hm?", window, cx));
    vcx.run_until_parked();
    assert_eq!(
        fake.last_started().unwrap().selection,
        Selection::named("deep", "high")
    );
}

#[gpui::test]
fn errors_of_the_engine_are_said_in_plain_words(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    let (_library, _id, workspace, vcx) = workspace_with(&fake, cx);
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    let panel = side(&workspace, vcx);
    let info = innerword_assistant::fake_info();

    // The usage limit, with the time it resets.
    let resets_at = innerword_assistant::time::now_seconds() + 2 * 60 * 60;
    fake.fail(AssistantError::UsageLimit {
        resets_at: Some(resets_at),
        detail: String::new(),
    });
    panel.update_in(vcx, |panel, window, cx| panel.send("One", window, cx));
    vcx.run_until_parked();
    let error = panel.read_with(vcx, |panel, _| {
        panel.transcript().last_answer().unwrap().error().cloned()
    });
    let text = error.expect("the answer failed").text(&info);
    assert_eq!(
        text.headline,
        "You have reached the usage limit of your Fake Agent plan."
    );
    assert!(text.advice.starts_with("It resets "), "{}", text.advice);

    // The program crashed: the conversation is kept and the next message
    // starts it again.
    fake.push(
        FakeReply::text("Half an").then_fail(AssistantError::Exited {
            code: Some(1),
            detail: "boom".into(),
        }),
    );
    panel.update_in(vcx, |panel, window, cx| panel.send("Two", window, cx));
    vcx.run_until_parked();
    let last = panel.read_with(vcx, |panel, _| {
        panel.transcript().last_answer().cloned().unwrap()
    });
    assert_eq!(last.text, "Half an");
    assert_eq!(
        last.error().unwrap().text(&info).headline,
        "Fake Agent stopped unexpectedly."
    );
    assert_eq!(panel.read_with(vcx, |panel, _| panel.transcript().len()), 4);

    // Not signed in: the engine says so, the host learns it, and the
    // settings say how to sign in.
    fake.set_sign_in(innerword_assistant::SignIn::SignedOut);
    fake.fail(AssistantError::NotSignedIn);
    panel.update_in(vcx, |panel, window, cx| panel.send("Three", window, cx));
    vcx.run_until_parked();
    vcx.simulate_keystrokes("cmd-,");
    vcx.run_until_parked();
    let host = workspace.read_with(vcx, |w, _| w.assistant_host().unwrap().clone());
    assert!(host.read_with(vcx, |host, _| host.is_signed_out()));
    // Still shown: the feature is there and says what to do.
    assert!(workspace.read_with(vcx, |w, _| w.assistant_shown()));
    assert!(
        panel
            .read_with(vcx, |panel, _| panel.availability().cloned())
            .is_some_and(|availability| availability.is_signed_out())
    );
}

#[gpui::test]
fn the_keys_of_the_chats_and_of_the_window_work_in_both_chats(cx: &mut TestAppContext) {
    use super::keymap::{GLOBAL, assert_table};
    let fake = FakeBackend::new();
    let (_library, _id, workspace, vcx) = workspace_with(&fake, cx);

    // The composer of the side chat.
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    assert!(side_has_keyboard(&workspace, vcx));
    assert_table(GLOBAL, "the side chat", vcx);
    assert_table(
        &[
            ("escape", "assistant::DismissChat"),
            ("cmd-.", "assistant::StopAnswer"),
            ("cmd-enter", "assistant::InsertAtCaret"),
            ("cmd-shift-enter", "assistant::ReplaceSelection"),
            ("cmd-shift-c", "assistant::CopyAnswer"),
            ("cmd-alt-n", "assistant::NewChat"),
            ("cmd-alt-k", "assistant::ClearChat"),
            ("cmd-shift-m", "assistant::ChooseModel"),
            ("cmd-alt-e", "assistant::ChooseEffort"),
            ("cmd-shift-a", "assistant::AddContext"),
            ("tab", "assistant::FocusNextArea"),
        ],
        "the side chat",
        vcx,
    );

    // The composer of the quick chat.
    vcx.simulate_keystrokes("escape");
    select(&workspace, TextRange::in_block(1, 4, 10), vcx);
    vcx.simulate_keystrokes("cmd-j");
    vcx.run_until_parked();
    assert!(
        quick(&workspace, vcx).update_in(vcx, |quick, window, cx| quick.has_keyboard(window, cx))
    );
    assert_table(GLOBAL, "the quick chat", vcx);
    assert_table(
        &[
            ("escape", "assistant::DismissChat"),
            ("cmd-.", "assistant::StopAnswer"),
            ("cmd-enter", "quick_chat::InsertBelow"),
            ("cmd-shift-enter", "assistant::ReplaceSelection"),
            ("cmd-shift-c", "assistant::CopyAnswer"),
            ("cmd-shift-m", "assistant::ChooseModel"),
            ("cmd-alt-e", "assistant::ChooseEffort"),
        ],
        "the quick chat",
        vcx,
    );
    // cmd-3 from the quick chat goes to the side chat; the popover, which
    // lost the keyboard, closes.
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    assert!(side_has_keyboard(&workspace, vcx));
    assert!(!quick_open(&workspace, vcx));

    // Without the assistant the two keys do nothing at all.
    workspace.update(vcx, |w, cx| {
        w.write_setting(keys::ASSISTANT_ENABLED, &false, cx)
    });
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), None);
    workspace.update_in(vcx, |w, window, cx| w.focus_editor(window, cx));
    vcx.simulate_keystrokes("cmd-j cmd-3");
    vcx.run_until_parked();
    assert!(!quick_open(&workspace, vcx));
    assert_eq!(right_panel(&workspace, vcx), None);
    assert!(editor_focused(&workspace, vcx));
}

#[gpui::test]
fn the_shortcuts_overlay_lists_the_assistant_only_while_it_is_shown(cx: &mut TestAppContext) {
    use innerword_app::shortcuts::{Place, collect_shortcuts};
    let fake = FakeBackend::new();
    let (_library, _id, workspace, vcx) = workspace_with(&fake, cx);
    let listed = |cx: &mut VisualTestContext| -> Vec<(String, Place, String)> {
        cx.update(|_, cx| {
            collect_shortcuts(cx)
                .into_iter()
                .map(|shortcut| (shortcut.title.to_string(), shortcut.place, shortcut.keys))
                .collect()
        })
    };
    let rows = listed(vcx);
    let row = |title: &str, place: Place, keys: &str| (title.to_string(), place, keys.to_string());
    // The two keys of the window.
    assert!(
        rows.contains(&row("Ask the assistant", Place::Anywhere, "⌘J")),
        "{rows:?}"
    );
    assert!(
        rows.contains(&row("Go to assistant", Place::Anywhere, "⌘3")),
        "{rows:?}"
    );
    // The keys inside the chats, under a place of their own.
    for (title, keys) in [
        ("New chat", "⌥⌘N"),
        ("Clear chat", "⌥⌘K"),
        ("Choose model", "⇧⌘M"),
        ("Choose effort", "⌥⌘E"),
        ("Add context", "⇧⌘A"),
        ("Insert answer at caret", "⌘↩"),
        ("Stop the answer", "⌘."),
        ("Insert below", "⌘↩"),
    ] {
        assert!(
            rows.contains(&row(title, Place::AssistantChat, keys)),
            "{title} {keys} in {rows:?}"
        );
    }
    // Nothing of the chats is filed under "Elsewhere" or "In the text".
    assert!(
        !rows
            .iter()
            .any(|(title, place, _)| *place == Place::Elsewhere && title.contains("chat"))
    );
    assert_eq!(
        Place::AssistantChat.heading("Assistant"),
        "In the assistant's chats"
    );

    // Switched off: none of it is listed.
    workspace.update(vcx, |w, cx| {
        w.write_setting(keys::ASSISTANT_ENABLED, &false, cx)
    });
    vcx.run_until_parked();
    let rows = listed(vcx);
    assert!(
        !rows
            .iter()
            .any(|(_, place, _)| *place == Place::AssistantChat),
        "{rows:?}"
    );
    assert!(
        !rows.iter().any(|(title, _, _)| title.contains("assistant")),
        "{rows:?}"
    );
}

/// Opens a workspace over `library` and says what it was like the moment
/// it was made, before the look for the engine could answer: whether the
/// engine had been asked, whether the assistant was shown, and what was
/// in the slot at the right.
fn launch<'a>(
    library: &Entity<Library>,
    cx: &'a mut VisualTestContext,
) -> (
    Entity<Workspace>,
    (bool, bool, Option<RightPanel>),
    &'a mut VisualTestContext,
) {
    let at_once = std::rc::Rc::new(std::cell::Cell::new((false, false, None)));
    let (library, seen) = (library.clone(), at_once.clone());
    let (workspace, cx) = cx.add_window_view(move |window, cx| {
        window.activate_window();
        let workspace = Workspace::new(library, window, cx);
        let asked = workspace
            .assistant_host()
            .is_some_and(|host| host.read(cx).availability().is_some());
        seen.set((
            asked,
            workspace.assistant_shown(),
            workspace.right_panel_shown(),
        ));
        workspace
    });
    cx.run_until_parked();
    (workspace, at_once.get(), cx)
}

#[gpui::test]
fn a_launch_shows_the_assistant_at_once_where_it_was_found_before(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    let (library, _id, workspace, vcx) = workspace_with(&fake, cx);
    assert!(workspace.read_with(vcx, |w, _| w.assistant_shown()));
    assert!(library.read_with(vcx, |library, _| {
        library.setting_or(keys::ASSISTANT_ENGINE_SEEN, false)
    }));
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    drop(workspace);

    // The next launch, with an engine that has not been asked anything
    // yet. Before the look of this launch has answered: shown, and the
    // side chat open as it was left.
    let fresh = FakeBackend::new();
    install(&fresh, vcx);
    let (workspace, at_once, vcx) = launch(&library, vcx);
    assert_eq!(at_once, (false, true, Some(RightPanel::Assistant)));
    // The look has answered: still there.
    assert_eq!(fresh.availability_checks(), 1);
    assert!(workspace.read_with(vcx, |w, _| w.assistant_shown()));
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Assistant));
    drop(workspace);

    // The program was removed since: shown for the moment of the look,
    // then gone, and the launch after that does not show it at all.
    let gone = FakeBackend::not_installed();
    install(&gone, vcx);
    let (workspace, at_once, vcx) = launch(&library, vcx);
    assert_eq!(at_once, (false, true, Some(RightPanel::Assistant)));
    assert!(!workspace.read_with(vcx, |w, _| w.assistant_shown()));
    assert_eq!(right_panel(&workspace, vcx), None);
    assert!(!library.read_with(vcx, |library, _| {
        library.setting_or(keys::ASSISTANT_ENGINE_SEEN, true)
    }));
    drop(workspace);
    let again = FakeBackend::not_installed();
    install(&again, vcx);
    let (workspace, at_once, vcx) = launch(&library, vcx);
    assert_eq!(at_once, (false, false, None));
    assert!(!workspace.read_with(vcx, |w, _| w.assistant_shown()));
}

#[gpui::test]
fn the_side_chat_is_reached_with_the_mouse_alone(cx: &mut TestAppContext) {
    use innerword_app::workspace::MenuKind;
    let fake = FakeBackend::new();
    let (_library, _id, workspace, vcx) = workspace_with(&fake, cx);

    // A small control at the end of the status bar.
    let control = vcx
        .debug_bounds("status-assistant")
        .expect("the status bar has the control");
    vcx.simulate_click(control.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Assistant));
    assert!(side_has_keyboard(&workspace, vcx));
    // The same control puts it away again.
    let control = vcx.debug_bounds("status-assistant").unwrap();
    vcx.simulate_click(control.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), None);
    assert!(editor_focused(&workspace, vcx));

    // The menu of the title has it too, with a check mark while it is open.
    let labels = |cx: &mut VisualTestContext| -> Vec<(String, Option<bool>)> {
        workspace.update(cx, |w, cx| {
            w.menu_items(MenuKind::Title, cx)
                .iter()
                .map(|item| (item.title().to_string(), item.is_checked()))
                .collect()
        })
    };
    assert!(labels(vcx).contains(&("Assistant".to_string(), Some(false))));
    assert!(workspace.update_in(vcx, |w, window, cx| {
        w.open_menu(MenuKind::Title, None, window, cx)
    }));
    vcx.run_until_parked();
    assert!(vcx.update(|window, cx| innerword_ui::ContextMenu::choose("Assistant", window, cx)));
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Assistant));
    assert!(labels(vcx).contains(&("Assistant".to_string(), Some(true))));

    // Switched off, neither is there.
    workspace.update(vcx, |w, cx| {
        w.write_setting(keys::ASSISTANT_ENABLED, &false, cx)
    });
    vcx.run_until_parked();
    assert!(!labels(vcx).iter().any(|(label, _)| label == "Assistant"));
    assert!(!workspace.read_with(vcx, |w, _| w.assistant_shown()));
}

#[gpui::test]
fn an_answer_cut_short_by_quitting_is_there_after_the_relaunch(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    let (library, id, workspace, vcx) = workspace_with(&fake, cx);
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    let panel = side(&workspace, vcx);
    // The answer arrives piece by piece; the app is closed after three.
    fake.hold();
    fake.push(FakeReply::pieces(["One ", "two ", "three ", "four."]));
    panel.update_in(vcx, |panel, window, cx| panel.send("Count", window, cx));
    vcx.run_until_parked();
    // Ready, Started, and three pieces of text.
    fake.advance(5);
    vcx.run_until_parked();
    assert!(panel.read_with(vcx, |panel, _| panel.is_answering()));
    assert_eq!(
        panel.read_with(vcx, |panel, _| answers(panel.transcript())),
        ["One two three "]
    );
    // cmd-w closes the window as quitting does.
    vcx.simulate_keystrokes("cmd-w");
    vcx.run_until_parked();

    let stored = library.read_with(vcx, |library, _| conversations::load(library, id));
    assert_eq!(stored.messages.len(), 2);
    assert_eq!(stored.messages[1].text, "One two three ");
    assert_eq!(stored.messages[1].state, MessageState::Stopped);
}

// ----- what the final check of 0.4.0 added ----------------------------------------
//
// The brief of the assistant, point by point, where the assembled app had no test of
// its own yet: stopping an answer, the pointer on the chats' own controls, a change
// of model in the middle of a conversation, and every kind of failure in both chats.

/// Clicks the control drawn under `selector`.
fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("nothing is drawn as {selector:?}"));
    cx.simulate_click(bounds.center(), gpui::Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn an_answer_is_stopped_with_the_key_and_with_the_pointer(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    let (_library, _id, workspace, vcx) = workspace_with(&fake, cx);

    // The side chat: an answer that arrives piece by piece.
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    let panel = side(&workspace, vcx);
    fake.hold();
    fake.push(FakeReply::pieces(["One ", "two ", "three ", "four."]));
    vcx.simulate_input("Count");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    fake.advance(4);
    vcx.run_until_parked();
    assert!(panel.read_with(vcx, |panel, _| panel.is_answering()));
    // cmd-. stops it: what was written stays, as a stopped answer.
    vcx.simulate_keystrokes("cmd-.");
    vcx.run_until_parked();
    assert_eq!(fake.stops(), 1);
    assert!(!panel.read_with(vcx, |panel, _| panel.is_answering()));
    let last = panel.read_with(vcx, |panel, _| {
        panel.transcript().last_answer().cloned().unwrap()
    });
    assert_eq!(last.text, "One two ");
    assert_eq!(last.state, MessageState::Stopped);
    // The keyboard is still in the composer and the conversation goes on.
    assert!(side_has_keyboard(&workspace, vcx));

    // The same with the pointer: the Stop control stands where Send was.
    fake.push(FakeReply::pieces(["Five ", "six ", "seven."]));
    vcx.simulate_input("Go on");
    click("assistant-send", vcx);
    // Started and one piece (the conversation is the one that was started).
    fake.advance(2);
    vcx.run_until_parked();
    assert!(panel.read_with(vcx, |panel, _| panel.is_answering()));
    click("assistant-stop", vcx);
    assert_eq!(fake.stops(), 2);
    assert!(!panel.read_with(vcx, |panel, _| panel.is_answering()));
    assert_eq!(
        panel.read_with(vcx, |panel, _| answers(panel.transcript())),
        ["One two ", "Five "]
    );
    // A third message is answered to its end, in the conversation that was
    // stopped twice.
    fake.release();
    fake.reply("Eight.");
    vcx.simulate_input("And then?");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(
        panel.read_with(vcx, |panel, _| answers(panel.transcript())),
        ["One two ", "Five ", "Eight."]
    );
    assert_eq!(fake.started().len(), 1);
}

#[gpui::test]
fn escape_in_the_quick_chat_stops_the_answer_first_and_closes_then(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    let (_library, _id, workspace, vcx) = workspace_with(&fake, cx);
    select(&workspace, TextRange::in_block(1, 4, 10), vcx);
    vcx.simulate_keystrokes("cmd-j");
    vcx.run_until_parked();
    let chat = quick(&workspace, vcx);
    fake.hold();
    fake.push(FakeReply::pieces(["warden, ", "guardian, ", "sentinel"]));
    vcx.simulate_input("Another word?");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    fake.advance(3);
    vcx.run_until_parked();
    assert!(chat.read_with(vcx, |chat, _| chat.is_answering()));

    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert_eq!(fake.stops(), 1);
    assert!(quick_open(&workspace, vcx), "the first escape only stops");
    assert!(!chat.read_with(vcx, |chat, _| chat.is_answering()));
    let last = chat.read_with(vcx, |chat, _| {
        chat.transcript().unwrap().last_answer().cloned().unwrap()
    });
    assert_eq!(last.text, "warden, ");
    assert_eq!(last.state, MessageState::Stopped);

    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!quick_open(&workspace, vcx));
    assert!(editor_focused(&workspace, vcx));
    // Closing ended the conversation: nothing of it is alive.
    assert_eq!(fake.alive(), 0);
}

#[gpui::test]
fn the_controls_of_the_side_chat_take_the_pointer(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    let (_library, _id, workspace, vcx) = workspace_with(&fake, cx);
    let before = editor_text(&workspace, vcx);
    // The caret at the end of the paragraph, then the side chat from the
    // status bar.
    let end = "The keeper climbed the stairs at dusk.".len();
    select(&workspace, TextRange::empty(Position::new(1, end)), vcx);
    click("status-assistant", vcx);
    let panel = side(&workspace, vcx);
    assert!(side_has_keyboard(&workspace, vcx));

    // Typed, and sent with the Send control.
    fake.reply("He was *late*.");
    vcx.simulate_input("What comes next?");
    click("assistant-send", vcx);
    assert_eq!(fake.sent().len(), 1);
    let answer = panel.read_with(vcx, |panel, _| panel.transcript().last_answer().unwrap().id);

    // Insert at Caret under the answer: one paragraph joins the text.
    let insert: &'static str =
        Box::leak(format!("assistant-action-insert-{}", answer.0).into_boxed_str());
    click(insert, vcx);
    let after = editor_text(&workspace, vcx);
    assert!(after.contains("at dusk.He was late."), "{after}");
    // The text has the keyboard now, so the next key undoes the answer.
    assert!(editor_focused(&workspace, vcx));
    vcx.simulate_keystrokes("cmd-z");
    vcx.run_until_parked();
    assert_eq!(editor_text(&workspace, vcx), before);
    // The same from the keyboard in the chat: cmd-enter, then cmd-z.
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    assert!(side_has_keyboard(&workspace, vcx));
    vcx.simulate_keystrokes("cmd-enter");
    vcx.run_until_parked();
    assert_ne!(editor_text(&workspace, vcx), before);
    vcx.simulate_keystrokes("cmd-z");
    vcx.run_until_parked();
    assert_eq!(editor_text(&workspace, vcx), before);
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();

    // Clear in the header empties the conversation; Close puts the panel away
    // and the text has the keyboard again.
    click("assistant-clear", vcx);
    assert_eq!(panel.read_with(vcx, |panel, _| panel.transcript().len()), 0);
    click("assistant-close", vcx);
    assert_eq!(right_panel(&workspace, vcx), None);
    assert!(editor_focused(&workspace, vcx));
}

#[gpui::test]
fn a_model_chosen_in_the_middle_of_a_conversation_answers_the_next_message(
    cx: &mut TestAppContext,
) {
    let fake = FakeBackend::new();
    let (_library, _id, workspace, vcx) = workspace_with(&fake, cx);
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    let panel = side(&workspace, vcx);
    fake.reply("First.");
    vcx.simulate_input("One");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(
        fake.last_started().unwrap().selection,
        Selection::named("deep", "medium")
    );

    // The model from the picker's key, the effort from its own.
    vcx.simulate_keystrokes("cmd-shift-m");
    vcx.run_until_parked();
    assert!(vcx.update(|window, cx| innerword_ui::ContextMenu::choose("Swift", window, cx)));
    vcx.run_until_parked();
    vcx.simulate_keystrokes("cmd-alt-e");
    vcx.run_until_parked();
    assert!(vcx.update(|window, cx| innerword_ui::ContextMenu::choose("Low", window, cx)));
    vcx.run_until_parked();
    assert_eq!(
        panel.read_with(vcx, |panel, _| panel.selection().clone()),
        Selection::named("swift", "low")
    );

    // The next message is answered by that model, and the conversation is
    // the same one for the user (the fake engine starts anew and is handed
    // what was said).
    fake.reply("Second.");
    vcx.simulate_input("Two");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let started = fake.last_started().unwrap();
    assert_eq!(started.selection, Selection::named("swift", "low"));
    assert!(
        started
            .history
            .iter()
            .any(|message| message.text.contains("First.")),
        "{:?}",
        started.history
    );
    assert_eq!(
        panel.read_with(vcx, |panel, _| answers(panel.transcript())),
        ["First.", "Second."]
    );

    // With an engine that changes it in the running conversation (as Claude
    // Code does) nothing is started again: the conversation that begins after
    // this takes the next change as it runs.
    fake.set_live_selection(true);
    vcx.simulate_keystrokes("cmd-shift-m");
    vcx.run_until_parked();
    assert!(vcx.update(|window, cx| innerword_ui::ContextMenu::choose("Deep", window, cx)));
    vcx.run_until_parked();
    fake.reply("Third.");
    vcx.simulate_input("Three");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let conversations = fake.started().len();
    vcx.simulate_keystrokes("cmd-shift-m");
    vcx.run_until_parked();
    assert!(vcx.update(|window, cx| innerword_ui::ContextMenu::choose("Swift", window, cx)));
    vcx.run_until_parked();
    fake.reply("Fourth.");
    vcx.simulate_input("Four");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(fake.started().len(), conversations);
    assert_eq!(
        fake.selections()
            .last()
            .map(|(_, selection)| selection.model.clone()),
        Some(ModelChoice::Named("swift".to_string()))
    );
    assert_eq!(
        panel.read_with(vcx, |panel, _| answers(panel.transcript())),
        ["First.", "Second.", "Third.", "Fourth."]
    );
}

#[gpui::test]
fn every_failure_is_said_in_both_chats_and_the_next_message_goes_through(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    let (_library, _id, workspace, vcx) = workspace_with(&fake, cx);
    let info = innerword_assistant::fake_info();
    let failures = || {
        vec![
            AssistantError::NotSignedIn,
            AssistantError::UsageLimit {
                resets_at: None,
                detail: String::new(),
            },
            AssistantError::Network {
                detail: "no route".into(),
            },
            AssistantError::Unavailable {
                detail: "overloaded".into(),
            },
            AssistantError::Exited {
                code: Some(1),
                detail: "boom".into(),
            },
            AssistantError::Outdated {
                found: "1.0.0".into(),
                needed: "2.0.0".into(),
            },
            AssistantError::NotInstalled,
        ]
    };

    // The side chat.
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    let panel = side(&workspace, vcx);
    for error in failures() {
        fake.set_sign_in(innerword_assistant::SignIn::SignedIn { account: None });
        fake.fail(error.clone());
        panel.update_in(vcx, |panel, window, cx| panel.send("Hm?", window, cx));
        vcx.run_until_parked();
        let last = panel.read_with(vcx, |panel, _| {
            panel.transcript().last_answer().cloned().unwrap()
        });
        assert_eq!(last.error(), Some(&error), "the side chat: {error:?}");
        let text = error.text(&info);
        assert!(
            !text.headline.is_empty() && !text.advice.is_empty(),
            "{error:?}: {text:?}"
        );
        // The row of the failed answer is drawn.
        let row: &'static str =
            Box::leak(format!("assistant-failed-{}", last.id.0).into_boxed_str());
        assert!(vcx.debug_bounds(row).is_some(), "no row for {error:?}");
    }
    fake.set_sign_in(innerword_assistant::SignIn::SignedIn { account: None });
    panel.update_in(vcx, |panel, window, cx| panel.recheck(window, cx));
    vcx.run_until_parked();
    fake.reply("Here again.");
    panel.update_in(vcx, |panel, window, cx| panel.send("Now?", window, cx));
    vcx.run_until_parked();
    assert_eq!(
        panel.read_with(vcx, |panel, _| panel
            .transcript()
            .last_answer()
            .unwrap()
            .text
            .clone()),
        "Here again."
    );

    // The quick chat.
    vcx.simulate_keystrokes("cmd-1");
    select(&workspace, TextRange::in_block(1, 4, 10), vcx);
    vcx.simulate_keystrokes("cmd-j");
    vcx.run_until_parked();
    let chat = quick(&workspace, vcx);
    assert!(quick_open(&workspace, vcx));
    for error in failures() {
        fake.set_sign_in(innerword_assistant::SignIn::SignedIn { account: None });
        chat.update_in(vcx, |chat, window, cx| chat.recheck(window, cx));
        vcx.run_until_parked();
        fake.fail(error.clone());
        chat.update_in(vcx, |chat, window, cx| chat.ask("Hm?", window, cx));
        vcx.run_until_parked();
        let last = chat.read_with(vcx, |chat, _| {
            chat.transcript().unwrap().last_answer().cloned().unwrap()
        });
        assert_eq!(last.error(), Some(&error), "the quick chat: {error:?}");
        assert!(
            quick_open(&workspace, vcx),
            "a failure does not close the chat"
        );
    }
    fake.set_sign_in(innerword_assistant::SignIn::SignedIn { account: None });
    chat.update_in(vcx, |chat, window, cx| chat.recheck(window, cx));
    vcx.run_until_parked();
    fake.reply("warden");
    chat.update_in(vcx, |chat, window, cx| chat.ask("Now?", window, cx));
    vcx.run_until_parked();
    assert_eq!(
        chat.read_with(vcx, |chat, _| {
            chat.transcript()
                .unwrap()
                .last_answer()
                .unwrap()
                .text
                .clone()
        }),
        "warden"
    );
}

#[gpui::test]
fn the_composer_and_the_notices_fill_the_side_chat_from_the_first_moment(cx: &mut TestAppContext) {
    // Seen in a capture of the real app: while the conversation was empty
    // the composer was 290 points wide in a panel of 380, and a notice with
    // a short text as wide as its text.
    for signed_in in [true, false] {
        let fake = if signed_in {
            FakeBackend::new()
        } else {
            FakeBackend::signed_out()
        };
        let (_library, _id, workspace, vcx) = workspace_with(&fake, cx);
        vcx.simulate_keystrokes("cmd-3");
        vcx.run_until_parked();
        assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Assistant));
        let panel = vcx.debug_bounds("assistant-panel").expect("the panel");
        let example = vcx.debug_bounds("assistant-example-0").expect("an example");
        let composer = vcx
            .debug_bounds("assistant-composer")
            .expect("the composer");
        let notice = vcx
            .debug_bounds(if signed_in {
                "assistant-notice-privacy"
            } else {
                "assistant-notice-availability"
            })
            .expect("a notice");
        assert!(example.size.width > panel.size.width - gpui::px(40.));
        assert_eq!(composer.size.width, example.size.width, "the composer");
        assert_eq!(notice.size.width, example.size.width, "the notice");
        assert_eq!(composer.left(), example.left());
        // The Send control stands at the end of the composer, not in the
        // middle of it.
        let send = vcx
            .debug_bounds("assistant-send")
            .expect("the Send control");
        assert!(
            composer.right() - send.right() < gpui::px(12.),
            "{composer:?} {send:?}"
        );
        // And the notice is as tall as its lines, not more.
        assert!(notice.size.height < gpui::px(220.), "{:?}", notice.size);
    }
}

// ----- images ------------------------------------------------------------------------------

/// The side chat reads an image as a line of Markdown and never sees the
/// picture. An answer that has the line gets the image back when it is
/// applied, as the document has it; an image of another document comes in
/// by its id; and a path in an answer is never read.
#[gpui::test]
fn the_side_chat_reads_an_image_as_a_line_and_an_answer_that_names_it_brings_it_back(
    cx: &mut TestAppContext,
) {
    use super::images::{images_in, picture, stored_png};
    use innerword_core::{Alignment, Document};

    let fake = FakeBackend::new();
    init(cx);
    install(&fake, cx);
    let library = library(cx);
    let image = stored_png(&library, 400, 300, [50, 80, 190], cx);
    let elsewhere = stored_png(&library, 300, 300, [200, 60, 40], cx);
    let first = "The keeper climbed the stairs at dusk.";
    let last = "The lamp was already lit.";
    let id = add_document(
        &library,
        None,
        &Document::from_blocks(vec![
            Block::with_text(BlockKind::heading(1), "The Lighthouse"),
            Block::paragraph(first),
            picture(&image, 50, Alignment::Right, "The lamp room"),
            Block::paragraph(last),
        ]),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |w, window, cx| w.open_document(id, true, window, cx));
    vcx.run_until_parked();

    // What the assistant is sent: the image is one line, with its
    // description and its id. No bytes, no path.
    let line = format!("![The lamp room](image:{})", image.id);
    let open = workspace
        .read_with(vcx, |w, cx| w.assistant_open_document(cx))
        .expect("a document is open");
    assert!(open.markdown.contains(&line), "{}", open.markdown);

    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    let panel = side(&workspace, vcx);
    let ask = |reply: &str, cx: &mut VisualTestContext| {
        fake.reply(reply);
        panel.update_in(cx, |panel, window, cx| panel.send("Go on.", window, cx));
        cx.run_until_parked();
        panel.read_with(cx, |panel, _| {
            panel.transcript().last_answer().expect("an answer").id
        })
    };
    let state = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |w, cx| {
            let document = w.editor().read(cx).document();
            document
                .blocks()
                .iter()
                .map(|block| match block.as_image() {
                    Some(image) => format!(
                        "[{} {}% {:?} {:?}]",
                        image.id,
                        image.width.percent(),
                        block.alignment(),
                        image.description
                    ),
                    None => block.text().to_string(),
                })
                .collect::<Vec<_>>()
        })
    };
    let picture_line = format!("[{} 50% Right \"The lamp room\"]", image.id);
    assert_eq!(
        state(vcx),
        ["The Lighthouse", first, picture_line.as_str(), last]
    );

    // An answer that turns the three blocks round: the image line stands
    // first now. Replace Selection over all three.
    let answer = ask(&format!("{line}\n\n{last}\n\n{first}"), vcx);
    select(
        &workspace,
        TextRange::new(Position::new(1, 0), Position::new(3, last.len())),
        vcx,
    );
    assert!(panel.update(vcx, |panel, cx| panel.apply(
        ApplyKind::ReplaceSelection,
        answer,
        cx
    )));
    vcx.run_until_parked();
    // The image moved with its line, as wide as it was and on its side.
    assert_eq!(
        state(vcx),
        ["The Lighthouse", picture_line.as_str(), last, first]
    );
    // One step of undo.
    let editor = workspace.read_with(vcx, |w, _| w.editor().clone());
    editor.update(vcx, |editor, cx| editor.edit(|state| state.undo(), cx));
    vcx.run_until_parked();
    assert_eq!(
        state(vcx),
        ["The Lighthouse", first, picture_line.as_str(), last]
    );

    // An image of another document, named by its id (the assistant read
    // it there with its tools): it comes in at its own size.
    let answer = ask(&format!("![A red boat](image:{})", elsewhere.id), vcx);
    select(
        &workspace,
        TextRange::empty(Position::new(3, last.len())),
        vcx,
    );
    assert!(panel.update(vcx, |panel, cx| panel.apply(
        ApplyKind::InsertAtCaret,
        answer,
        cx
    )));
    vcx.run_until_parked();
    let found = images_in(&workspace, vcx);
    assert_eq!(found.len(), 2);
    assert_eq!(found[1].1.id, elsewhere.id);
    assert_eq!(found[1].1.description, "A red boat");
    // 300 pixels on a column of 451 points.
    assert_eq!(found[1].1.width.percent(), 67);

    // A path, an address and an id the library does not have: none of
    // them becomes an image, and no file is read. Each stays a link.
    let pictures = library.read_with(vcx, |library, _| library.store().image_count().unwrap());
    for destination in [
        "/etc/hosts",
        "~/Pictures/secret.png",
        "file:///etc/hosts",
        "https://example.com/harbour.png",
        "image:00112233445566778899aabbccddeeff",
    ] {
        let answer = ask(&format!("![x]({destination})"), vcx);
        let end = editor.read_with(vcx, |editor, _| {
            let document = editor.document();
            let last = document.block_count() - 1;
            Position::new(last, document.block(last).unwrap().len())
        });
        select(&workspace, TextRange::empty(end), vcx);
        assert!(
            panel.update(vcx, |panel, cx| panel.apply(
                ApplyKind::InsertAtCaret,
                answer,
                cx
            )),
            "{destination}"
        );
        vcx.run_until_parked();
        assert_eq!(
            images_in(&workspace, vcx).len(),
            2,
            "{destination} became an image"
        );
        let linked = editor.read_with(vcx, |editor, _| {
            editor.document().blocks().iter().any(|block| {
                block
                    .text_spans()
                    .any(|(_, _, attrs)| attrs.link.as_deref() == Some(destination))
            })
        });
        assert!(linked, "{destination} stays a link");
    }
    assert_eq!(
        library.read_with(vcx, |library, _| library.store().image_count().unwrap()),
        pictures,
        "nothing was stored"
    );
}

/// Over an open page the side chat opens (`cmd-3`), is sent the text the
/// page showed with the sentence that says what a page is, offers nothing
/// to put into the page, and keeps a conversation of the page's own.
#[gpui::test]
fn the_side_chat_over_a_page_sends_its_text(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    init(cx);
    install(&fake, cx);
    cx.update(innerword_app::pages::use_fake);
    let library = library(cx);
    let text = add_document(
        &library,
        None,
        &simple_document("The Lighthouse", "The keeper climbed the stairs at dusk."),
        cx,
    );
    let bundle = innerword_app::pages::offline_bundle(
        "harbour",
        "index.html",
        vec![(
            "index.html".to_string(),
            b"<!doctype html><html><head><title>Harbour Guide</title></head>\
              <body><p>The boats come by road every spring.</p></body></html>"
                .to_vec(),
        )],
    );
    let page = {
        let library = library.clone();
        cx.update(|cx| {
            innerword_app::pages::store_offline_bundle(
                &library,
                bundle,
                None,
                Some("https://example.com/harbour"),
                cx,
            )
            .unwrap()
            .id
        })
    };
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |w, window, cx| {
        assert!(w.open_document(page, true, window, cx));
    });
    vcx.run_until_parked();

    // What the side chat is handed: the page's text, as a page.
    let open = workspace
        .read_with(vcx, |w, cx| w.assistant_open_document(cx))
        .expect("the page is the open item");
    assert_eq!(open.id, Some(page.0));
    assert_eq!(open.title, "Harbour Guide");
    assert_eq!(
        open.kind,
        innerword_assistant::OpenKind::Page {
            source_url: Some("https://example.com/harbour".into())
        }
    );
    assert!(
        open.markdown
            .contains("The boats come by road every spring.")
    );

    // cmd-3 from the page opens the side chat (it does not say "open a
    // document").
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Assistant));
    assert!(side_has_keyboard(&workspace, vcx));
    let panel = side(&workspace, vcx);
    assert_eq!(
        panel.read_with(vcx, |panel, _| panel.document().and_then(|d| d.id)),
        Some(page.0)
    );
    fake.reply("It is a guide to the harbour.");
    panel.update_in(vcx, |panel, window, cx| {
        panel.send("What is this page about?", window, cx)
    });
    vcx.run_until_parked();
    let sent = fake.last_sent().expect("a message was sent");
    assert!(
        sent.starts_with(innerword_assistant::PAGE_PREFACE),
        "{sent}"
    );
    assert!(sent.contains("kind=\"page\""), "{sent}");
    assert!(
        sent.contains("The boats come by road every spring."),
        "{sent}"
    );
    assert!(sent.ends_with("What is this page about?"), "{sent}");

    // The conversation is the page's own, and stays with it.
    let stored = library.read_with(vcx, |library, _| conversations::load(library, page));
    assert_eq!(answers(&stored), ["It is a guide to the harbour."]);
    assert!(
        library
            .read_with(vcx, |library, _| conversations::load(library, text))
            .is_empty()
    );
}

/// A text element as Excalidraw keeps it.
fn board_text(id: &str, text: &str, y: f64) -> serde_json::Value {
    serde_json::json!({
        "id": id, "type": "text", "x": 0.0, "y": y, "width": 120.0, "height": 25.0,
        "angle": 0, "strokeColor": "#1e1e1e", "backgroundColor": "transparent",
        "fillStyle": "solid", "strokeWidth": 2, "strokeStyle": "solid", "roughness": 1,
        "opacity": 100, "groupIds": [], "frameId": null, "roundness": null,
        "seed": 1, "version": 1, "versionNonce": 1, "isDeleted": false,
        "boundElements": null, "updated": 1, "link": null, "locked": false,
        "index": format!("a{}", id.len()),
        "text": text, "originalText": text, "fontSize": 20, "fontFamily": 5,
        "textAlign": "left", "verticalAlign": "top", "containerId": null,
        "autoResize": true, "lineHeight": 1.25,
    })
}

/// The scene of `elements` as the page posts it.
fn board_scene(elements: &[serde_json::Value]) -> String {
    serde_json::json!({
        "type": "excalidraw", "version": 2, "source": "inner-word",
        "elements": elements, "appState": { "viewBackgroundColor": "#ffffff" }, "files": {},
    })
    .to_string()
}

/// Over an open whiteboard the side chat is sent the listing of its
/// elements as the app holds them when the message is sent (what the writer
/// drew since it opened included, and what the page had not posted when
/// `cmd-3` asked it), after the sentence that says what a whiteboard is;
/// the conversation is its own.
#[gpui::test]
fn the_side_chat_over_a_whiteboard_sends_its_listing(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    init(cx);
    install(&fake, cx);
    let pages = cx.update(innerword_app::pages::use_fake);
    let library = library(cx);
    let text = add_document(
        &library,
        None,
        &simple_document("The Lighthouse", "The keeper climbed the stairs at dusk."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    vcx.run_until_parked();
    let board = workspace
        .update_in(vcx, |w, window, cx| w.new_board(Some("Plan"), window, cx))
        .expect("a whiteboard is made");
    vcx.run_until_parked();
    pages.board_ready();
    pages.draw(&board_scene(&[board_text("title", "System overview", 0.0)]));
    vcx.run_until_parked();

    let open = workspace
        .read_with(vcx, |w, cx| w.assistant_open_document(cx))
        .expect("the whiteboard is the open item");
    assert_eq!(open.id, Some(board.0));
    assert_eq!(open.kind, innerword_assistant::OpenKind::Board);
    assert!(
        open.markdown.contains("System overview"),
        "{}",
        open.markdown
    );
    assert!(
        open.markdown.starts_with(&format!("board:{}", board.0)),
        "{}",
        open.markdown
    );

    // A stroke the page has not posted: cmd-3 asks the page for it.
    pages.draw_unposted(&board_scene(&[
        board_text("title", "System overview", 0.0),
        board_text("late", "Written just now", 60.0),
    ]));
    vcx.run_until_parked();
    let asked = pages.collects_asked();
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    assert_eq!(right_panel(&workspace, vcx), Some(RightPanel::Assistant));
    assert!(pages.collects_asked() > asked, "the page was asked");
    let panel = side(&workspace, vcx);
    fake.reply("It shows the system.");
    panel.update_in(vcx, |panel, window, cx| {
        panel.send("What does this whiteboard show?", window, cx)
    });
    vcx.run_until_parked();
    let sent = fake.last_sent().expect("a message was sent");
    assert!(
        sent.starts_with(innerword_assistant::BOARD_PREFACE),
        "{sent}"
    );
    assert!(sent.contains("kind=\"board\""), "{sent}");
    assert!(sent.contains("System overview"), "{sent}");
    assert!(sent.contains("Written just now"), "{sent}");
    assert!(sent.ends_with("What does this whiteboard show?"), "{sent}");

    // The conversation is the whiteboard's own.
    let stored = library.read_with(vcx, |library, _| conversations::load(library, board));
    assert_eq!(answers(&stored), ["It shows the system."]);
    assert!(
        library
            .read_with(vcx, |library, _| conversations::load(library, text))
            .is_empty()
    );

    // The keyboard goes back to the whiteboard, the writer draws, and comes
    // back into the composer the way a click in it does (not by cmd-3): the
    // page is asked as well, and the next message has the new text.
    workspace.update_in(vcx, |w, window, cx| w.focus_editor(window, cx));
    vcx.run_until_parked();
    assert!(!side_has_keyboard(&workspace, vcx));
    pages.draw_unposted(&board_scene(&[
        board_text("title", "System overview", 0.0),
        board_text("late", "Written just now", 60.0),
        board_text("later", "Drawn after the answer", 120.0),
    ]));
    vcx.run_until_parked();
    let asked = pages.collects_asked();
    panel.update_in(vcx, |panel, window, cx| panel.focus(window, cx));
    vcx.run_until_parked();
    assert!(side_has_keyboard(&workspace, vcx));
    assert_eq!(pages.collects_asked(), asked + 1, "the page was asked once");
    fake.reply("Three texts now.");
    panel.update_in(vcx, |panel, window, cx| panel.send("And now?", window, cx));
    vcx.run_until_parked();
    let sent = fake.last_sent().expect("a message was sent");
    assert!(sent.contains("Drawn after the answer"), "{sent}");
}

/// A message sent over a whiteboard waits for what its page holds and has
/// not posted (spec 6.6): the page is asked as the message is sent, and the
/// message carries what it handed over. A page that does not answer holds
/// the message back for at most a second; then it goes with what the app
/// holds.
#[gpui::test]
fn a_message_over_a_whiteboard_waits_for_what_its_page_holds(cx: &mut TestAppContext) {
    let fake = FakeBackend::new();
    init(cx);
    install(&fake, cx);
    let pages = cx.update(innerword_app::pages::use_fake);
    let library = library(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    vcx.run_until_parked();
    workspace
        .update_in(vcx, |w, window, cx| w.new_board(Some("Plan"), window, cx))
        .expect("a whiteboard is made");
    vcx.run_until_parked();
    pages.board_ready();
    pages.draw(&board_scene(&[board_text("title", "System overview", 0.0)]));
    vcx.run_until_parked();

    // The side chat has the keyboard before anything is left unposted, so
    // only sending can ask the page.
    vcx.simulate_keystrokes("cmd-3");
    vcx.run_until_parked();
    assert!(side_has_keyboard(&workspace, vcx));
    let panel = side(&workspace, vcx);

    // A text the page has not posted, and a message sent at once.
    pages.draw_unposted(&board_scene(&[
        board_text("title", "System overview", 0.0),
        board_text("late", "Written just now", 60.0),
    ]));
    let asked = pages.collects_asked();
    fake.reply("Two texts.");
    panel.update_in(vcx, |panel, window, cx| {
        panel.send("What does it say?", window, cx)
    });
    vcx.run_until_parked();
    assert_eq!(pages.collects_asked(), asked + 1, "the page was asked");
    let sent = fake.last_sent().expect("a message was sent");
    assert!(sent.contains("System overview"), "{sent}");
    assert!(sent.contains("Written just now"), "{sent}");
    assert!(sent.ends_with("What does it say?"), "{sent}");

    // Nothing is held back: the page is not asked, the message goes.
    let asked = pages.collects_asked();
    let count = fake.sent().len();
    fake.reply("Still two.");
    panel.update_in(vcx, |panel, window, cx| panel.send("Sure?", window, cx));
    vcx.run_until_parked();
    assert_eq!(pages.collects_asked(), asked, "nothing to ask");
    assert_eq!(fake.sent().len(), count + 1);

    // A page that does not answer: the message waits a second, then goes
    // with what the app holds.
    pages.hold_collects();
    pages.draw_unposted(&board_scene(&[
        board_text("title", "System overview", 0.0),
        board_text("late", "Written just now", 60.0),
        board_text("lost", "Never handed over", 120.0),
    ]));
    let count = fake.sent().len();
    fake.reply("Two, still.");
    panel.update_in(vcx, |panel, window, cx| panel.send("And now?", window, cx));
    vcx.run_until_parked();
    assert_eq!(fake.sent().len(), count, "the message waits for the page");
    assert!(panel.read_with(vcx, |panel, _| panel.is_holding_a_message()));
    vcx.executor()
        .advance_clock(innerword_app::boards::VIEW_COLLECT_PATIENCE);
    vcx.run_until_parked();
    assert_eq!(fake.sent().len(), count + 1, "it went after a second");
    // What the app holds is what the last message had: the whiteboard is
    // said to be unchanged, and the text the page kept is not in it.
    let sent = fake.last_sent().expect("a message was sent");
    assert!(
        sent.contains("has not changed since my last message"),
        "{sent}"
    );
    assert!(!sent.contains("Never handed over"), "{sent}");
    assert!(sent.ends_with("And now?"), "{sent}");
    pages.release_collects();
    vcx.run_until_parked();
}

// ----- an answer's drawing over a document (docs/assistant-draws-design.md, 6) ----------

mod drawings {
    use std::{rc::Rc, sync::Arc};

    use gpui::TestAppContext;
    use innerword_app::{
        assistant::conversations,
        commands,
        pages::{self, FakePages},
        workspace::{made_sentence, new_board_names_missing, origin_note},
    };
    use innerword_assistant::{DrawingTarget, FakeBackend, MessageId};
    use innerword_assistant_ui::{ACTION_DRAW, ActionState, AssistantPanel};
    use innerword_library::{DocumentKind, FolderId};

    use crate::suite::*;

    const WHO_SERVES_WHOM: &str = r#"{"title": "Who serves whom", "add": [
  {"type": "ellipse", "id": "keeper", "x": 0, "y": 0, "width": 180, "height": 80, "label": {"text": "The keeper"}},
  {"type": "ellipse", "id": "ada", "x": 320, "y": 0, "width": 180, "height": 80, "label": {"text": "Ada"}},
  {"type": "arrow", "id": "serves", "x": 180, "y": 40, "start": {"id": "keeper"}, "end": {"id": "ada"}, "label": {"text": "serves"}}]}"#;

    fn answer_with(drawing: &str) -> String {
        format!("Here is who serves whom in the chapter.\n\n```whiteboard\n{drawing}\n```\n")
    }

    struct Desk<'a> {
        fake: FakeBackend,
        pages: Option<Rc<FakePages>>,
        library: Entity<Library>,
        workspace: Entity<Workspace>,
        folder: FolderId,
        chapter: DocumentId,
        cx: &'a mut VisualTestContext,
    }

    /// The assistant on its fake engine, the fake page engine when `boards`,
    /// and "Chapter 2" in the folder "Part one", open.
    fn desk(cx: &mut TestAppContext, boards: bool) -> Desk<'_> {
        init(cx);
        let fake = FakeBackend::new();
        cx.update(|cx| innerword_app::assistant::set_backend(Arc::new(fake.clone()), cx));
        let pages = boards.then(|| cx.update(pages::use_fake));
        let library = library(cx);
        let folder = library.update(cx, |l, cx| {
            l.create_folder(None, "Part one", cx).unwrap().id
        });
        let chapter = add_document(
            &library,
            Some(folder),
            &simple_document("Chapter 2", "The keeper served Ada."),
            cx,
        );
        let (workspace, vcx) = open_workspace(&library, cx);
        workspace.update_in(vcx, |w, window, cx| {
            assert!(w.open_document(chapter, true, window, cx));
        });
        vcx.run_until_parked();
        Desk {
            fake,
            pages,
            library,
            workspace,
            folder,
            chapter,
            cx: vcx,
        }
    }

    impl Desk<'_> {
        fn side(&mut self) -> Entity<AssistantPanel> {
            self.workspace
                .read_with(self.cx, |w, _| w.side_chat().expect("an engine").clone())
        }

        fn ask(&mut self, answer: &str) -> MessageId {
            self.cx.simulate_keystrokes("cmd-3");
            self.cx.run_until_parked();
            let panel = self.side();
            self.fake.reply(answer);
            panel.update_in(self.cx, |panel, window, cx| {
                panel.send("Draw who serves whom.", window, cx)
            });
            self.cx.run_until_parked();
            panel.read_with(self.cx, |panel, _| {
                panel.transcript().last_answer().expect("an answer").id
            })
        }

        fn toasts(&mut self) -> Vec<String> {
            self.workspace.read_with(self.cx, |w, cx| {
                w.toast_layer()
                    .read(cx)
                    .toasts()
                    .map(|toast| toast.message.to_string())
                    .collect()
            })
        }

        fn shown(&mut self, id: MessageId) -> String {
            let panel = self.side();
            panel.read_with(self.cx, |panel, cx| {
                panel
                    .messages()
                    .read(cx)
                    .answer_editor(id)
                    .expect("the answer is shown")
                    .read(cx)
                    .document()
                    .plain_text()
            })
        }

        /// A document at the root of the library.
        fn add(&mut self, document: &innerword_core::Document) -> DocumentId {
            let form = innerword_app::document_io::StoredForm::of(document, None);
            self.library.update(self.cx, |library, cx| {
                library
                    .create_document(None, &form.content(), cx)
                    .unwrap()
                    .id
            })
        }

        fn open_board(&mut self) -> Option<DocumentId> {
            self.workspace.read_with(self.cx, |w, _| w.open_board_id())
        }

        fn press_toast(&mut self, start: &str) {
            let layer = self
                .workspace
                .read_with(self.cx, |workspace, _| workspace.toast_layer().clone());
            let id = layer.read_with(self.cx, |layer, _| {
                layer
                    .ids()
                    .into_iter()
                    .zip(layer.toasts())
                    .find(|(_, toast)| toast.message.starts_with(start))
                    .map(|(id, _)| id)
                    .unwrap_or_else(|| panic!("no toast says {start:?}"))
            });
            let ran = layer.update_in(self.cx, |layer, window, cx| {
                layer.run_action(id, window, cx)
            });
            assert!(ran, "the toast {start:?} has no button");
            self.cx.run_until_parked();
        }
    }

    #[gpui::test]
    fn an_answer_over_a_document_makes_a_new_whiteboard_in_its_folder(cx: &mut TestAppContext) {
        let mut desk = desk(cx, true);
        let answer = desk.ask(&answer_with(WHO_SERVES_WHOM));
        let shown = desk.shown(answer);
        assert!(
            shown.contains(
                "Drawing: adds 2 ellipses and an arrow (new whiteboard \u{201c}Who serves whom\u{201d})."
            ),
            "{shown}"
        );
        let panel = desk.side();
        assert_eq!(
            panel.read_with(desk.cx, |panel, cx| {
                panel.messages().read(cx).action_state(ACTION_DRAW, answer)
            }),
            ActionState::Enabled
        );
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();

        // Made in the chapter's folder, titled, opened with the keyboard on it.
        let board = desk.open_board().expect("the new whiteboard is open");
        let meta = desk
            .library
            .read_with(desk.cx, |l, _| l.document_meta(board).unwrap());
        assert_eq!(meta.kind, DocumentKind::Board);
        assert_eq!(meta.title, "Who serves whom");
        assert_eq!(meta.folder_id, Some(desk.folder));
        assert!(
            desk.workspace
                .update_in(desk.cx, |w, window, _| w.page_has_focus(window))
        );
        let content = desk
            .library
            .read_with(desk.cx, |l, _| l.document(board).unwrap().content);
        let scene = innerword_board::Scene::from_stored(&content).unwrap().scene;
        for id in ["keeper", "ada", "serves"] {
            assert!(scene.element(id).is_some(), "{id} is drawn");
        }
        // The toast, and its way back.
        let said = made_sentence("Who serves whom", Some("Part one"));
        assert!(desk.toasts().contains(&said), "{:?}", desk.toasts());
        let report = desk
            .workspace
            .read_with(desk.cx, |w, _| w.last_drawing_report().map(str::to_string))
            .unwrap();
        assert!(
            report.starts_with(&format!(
                "draw: new board:{} \u{201c}Who serves whom\u{201d} in Part one, 3 added",
                board.0
            )),
            "{report}"
        );
        // The whiteboard's own chat says where it came from.
        let note = panel.read_with(desk.cx, |panel, cx| {
            panel
                .messages()
                .read(cx)
                .empty_state()
                .map(|state| state.note.to_string())
        });
        assert_eq!(note, Some(origin_note("Chapter 2")));
        assert_eq!(
            panel.read_with(desk.cx, |panel, _| panel.drawing_target()),
            DrawingTarget::OpenWhiteboard
        );
        // The chapter's conversation is stored with the answer marked.
        let chapter = desk.chapter;
        let stored = desk
            .library
            .read_with(desk.cx, |l, _| conversations::load(l, chapter));
        assert!(stored.last_answer().unwrap().drawn_at.is_some());

        desk.press_toast(&said);
        assert_eq!(
            desk.workspace
                .read_with(desk.cx, |w, _| w.open_document_id()),
            Some(chapter)
        );
        let back = panel.read_with(desk.cx, |panel, _| {
            panel.transcript().last_answer().cloned().unwrap()
        });
        assert_eq!(back.id, answer);
        assert!(back.drawn_at.is_some(), "drawn, after the way back too");

        // Without a title: after the document; for an untitled document,
        // "Drawing".
        let untitled = WHO_SERVES_WHOM.replace(r#""title": "Who serves whom", "#, "");
        desk.ask(&answer_with(&untitled));
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        let second = desk.open_board().unwrap();
        let title = desk
            .library
            .read_with(desk.cx, |l, _| l.document_meta(second).unwrap().title);
        assert_eq!(title, "Chapter 2 (drawing)");
        let nameless = desk.add(&innerword_core::Document::new());
        desk.workspace.update_in(desk.cx, |w, window, cx| {
            assert!(w.open_document(nameless, true, window, cx));
        });
        desk.cx.run_until_parked();
        desk.ask(&answer_with(&untitled));
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        let third = desk.open_board().unwrap();
        let title = desk
            .library
            .read_with(desk.cx, |l, _| l.document_meta(third).unwrap().title);
        assert_eq!(title, "Drawing");
    }

    #[gpui::test]
    fn a_document_in_the_trash_makes_its_whiteboard_at_the_root(cx: &mut TestAppContext) {
        let mut desk = desk(cx, true);
        let (folder, chapter) = (desk.folder, desk.chapter);
        // Its folder in the trash, the document opened from there.
        let text = desk.add(&simple_document("Elsewhere", "Words."));
        desk.workspace.update_in(desk.cx, |w, window, cx| {
            assert!(w.open_document(text, true, window, cx));
        });
        desk.library
            .update(desk.cx, |l, cx| l.trash_folder(folder, cx).unwrap());
        desk.workspace.update_in(desk.cx, |w, window, cx| {
            assert!(w.open_document(chapter, true, window, cx));
        });
        desk.cx.run_until_parked();
        desk.ask(&answer_with(WHO_SERVES_WHOM));
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        let board = desk.open_board().expect("made");
        let meta = desk
            .library
            .read_with(desk.cx, |l, _| l.document_meta(board).unwrap());
        assert_eq!(meta.folder_id, None, "at the root, not in the trash");
        assert!(meta.trashed_at.is_none());
        assert!(
            desk.toasts()
                .contains(&made_sentence("Who serves whom", None)),
            "{:?}",
            desk.toasts()
        );
    }

    #[gpui::test]
    fn an_answer_that_names_what_it_does_not_add_says_so_over_a_document(cx: &mut TestAppContext) {
        let mut desk = desk(cx, true);
        desk.ask(&answer_with(
            r#"{"add": [{"type": "arrow", "id": "to", "x": 0, "y": 0, "start": {"id": "keeper"}, "end": {"id": "cache"}},
                        {"type": "ellipse", "id": "keeper", "x": 0, "y": 200}]}"#,
        ));
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        assert!(desk.open_board().is_none(), "no whiteboard was made");
        assert!(
            desk.toasts()
                .contains(&new_board_names_missing(&["cache".to_string()])),
            "{:?}",
            desk.toasts()
        );
    }

    #[gpui::test]
    fn a_drawing_that_changes_a_whiteboard_is_not_offered_over_a_document(cx: &mut TestAppContext) {
        let mut desk = desk(cx, true);
        let answer = desk.ask(&answer_with(r#"{"update": [{"id": "api", "x": 40}]}"#));
        let panel = desk.side();
        let state = panel.read_with(desk.cx, |panel, cx| {
            panel.messages().read(cx).action_state(ACTION_DRAW, answer)
        });
        assert!(matches!(state, ActionState::Disabled(_)), "{state:?}");
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        assert!(desk.open_board().is_none());
        assert!(
            desk.toasts().iter().any(|toast| toast
                == "Nothing was drawn: it changes elements of a whiteboard: open that whiteboard and ask there."),
            "{:?}",
            desk.toasts()
        );
    }

    #[gpui::test]
    fn insert_at_caret_leaves_the_drawing_out(cx: &mut TestAppContext) {
        let mut desk = desk(cx, true);
        desk.ask(&answer_with(WHO_SERVES_WHOM));
        desk.cx.simulate_keystrokes("cmd-enter");
        desk.cx.run_until_parked();
        let text = editor_text(&desk.workspace, desk.cx);
        assert!(
            text.contains("Here is who serves whom in the chapter."),
            "{text}"
        );
        assert!(!text.contains("whiteboard"), "{text}");
        assert!(!text.contains("keeper\""), "{text}");
    }

    #[gpui::test]
    fn without_a_page_engine_no_drawing_is_offered(cx: &mut TestAppContext) {
        let mut desk = desk(cx, false);
        let answer = desk.ask(&answer_with(WHO_SERVES_WHOM));
        let panel = desk.side();
        assert_eq!(
            panel.read_with(desk.cx, |panel, _| panel.drawing_target()),
            DrawingTarget::None
        );
        assert_eq!(
            panel.read_with(desk.cx, |panel, cx| {
                panel.messages().read(cx).action_state(ACTION_DRAW, answer)
            }),
            ActionState::Hidden
        );
        let prompt = desk.fake.last_started().unwrap().system_prompt;
        assert!(!prompt.contains("Drawing a new whiteboard."), "{prompt}");
        assert!(!prompt.contains("Drawing on the open whiteboard."));
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        let sentence = desk
            .cx
            .update(|_, cx| innerword_app::boards::not_shown_sentence(cx));
        assert!(desk.toasts().contains(&sentence), "{:?}", desk.toasts());
    }

    /// Each item's conversation starts with the prompt of what it offers: a
    /// document's tells how to draw a new whiteboard, a whiteboard's how to
    /// draw on it, a page's neither; each starts once.
    #[gpui::test]
    fn each_item_starts_its_chat_with_its_own_prompt(cx: &mut TestAppContext) {
        let mut desk = desk(cx, true);
        desk.ask("About the chapter.");
        let started = desk.fake.started();
        assert_eq!(started.len(), 1);
        assert!(
            started[0]
                .system_prompt
                .contains("Drawing a new whiteboard.")
        );
        assert!(
            !started[0]
                .system_prompt
                .contains("Drawing on the open whiteboard.")
        );

        let board = desk
            .workspace
            .update_in(desk.cx, |w, window, cx| {
                w.new_board(Some("Plan"), window, cx)
            })
            .unwrap();
        desk.cx.run_until_parked();
        desk.pages.as_ref().unwrap().board_ready();
        desk.cx.run_until_parked();
        desk.ask("About the whiteboard.");
        let started = desk.fake.started();
        assert_eq!(started.len(), 2, "one start for the whiteboard");
        assert!(
            started[1]
                .system_prompt
                .contains("Drawing on the open whiteboard.")
        );
        assert!(
            !started[1]
                .system_prompt
                .contains("Drawing a new whiteboard.")
        );

        let bundle = innerword_app::pages::offline_bundle(
            "harbour",
            "index.html",
            vec![(
                "index.html".to_string(),
                b"<!doctype html><title>Harbour</title><p>Boats.</p>".to_vec(),
            )],
        );
        let library = desk.library.clone();
        let page = desk.cx.update(|_, cx| {
            innerword_app::pages::store_offline_bundle(&library, bundle, None, None, cx)
                .unwrap()
                .id
        });
        desk.workspace.update_in(desk.cx, |w, window, cx| {
            assert!(w.open_document(page, true, window, cx));
        });
        desk.cx.run_until_parked();
        desk.ask("About the page.");
        let started = desk.fake.started();
        assert_eq!(started.len(), 3, "one start for the page");
        assert!(!started[2].system_prompt.contains("Drawing"));
        // Back on the whiteboard: its chat is where it was, nothing restarts.
        desk.workspace.update_in(desk.cx, |w, window, cx| {
            assert!(w.open_document(board, true, window, cx));
        });
        desk.cx.run_until_parked();
        assert_eq!(desk.fake.started().len(), 3);
    }

    #[gpui::test]
    fn the_palette_names_the_command_by_the_open_item(cx: &mut TestAppContext) {
        let mut desk = desk(cx, true);
        let listed = |desk: &mut Desk<'_>| {
            let workspace = desk.workspace.clone();
            workspace.update_in(desk.cx, |w, window, cx| {
                w.toggle_command_palette(window, cx)
            });
            desk.cx.run_until_parked();
            let palette = workspace
                .read_with(desk.cx, |w, cx| {
                    w.modal_layer()
                        .read(cx)
                        .active::<innerword_app::palette::CommandPalette>()
                })
                .expect("the palette is open");
            let titles = palette.read_with(desk.cx, |palette, _| palette.delegate.listed_titles());
            workspace.update_in(desk.cx, |w, window, cx| w.dismiss_modal(window, cx));
            desk.cx.run_until_parked();
            titles
                .into_iter()
                .filter(|title| title.contains("answer") && title.contains("whiteboard"))
                .collect::<Vec<_>>()
        };
        assert_eq!(listed(&mut desk), ["New whiteboard from answer"]);
        desk.workspace
            .update_in(desk.cx, |w, window, cx| {
                w.new_board(Some("Plan"), window, cx)
            })
            .unwrap();
        desk.cx.run_until_parked();
        desk.pages.as_ref().unwrap().board_ready();
        desk.cx.run_until_parked();
        assert_eq!(listed(&mut desk), ["Draw answer on whiteboard"]);
        // The command is one of the assistant's, by itself.
        let all = desk.cx.update(|_, cx| commands::all(cx));
        assert!(
            all.iter()
                .any(|command| command.title.as_ref() == "New whiteboard from answer")
        );
    }
}
