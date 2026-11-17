//! Settings: the modal, persistence, live application, view toggles.

use gpui::{AppContext as _, TestAppContext};
use innerword_app::{
    Settings,
    settings::{Paper, keys},
    settings_view::{SettingRow, SettingsView},
};
use innerword_export::SocialNetwork;
use innerword_library::Library;
use innerword_ui::{ThemeMode, theme};

use super::*;

fn settings_view(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Option<Entity<SettingsView>> {
    workspace.read_with(cx, |workspace, cx| {
        workspace.modal_layer().read(cx).active::<SettingsView>()
    })
}

fn focused_row(view: &Entity<SettingsView>, cx: &mut VisualTestContext) -> Option<SettingRow> {
    view.update_in(cx, |view, window, cx| view.focused_row(window, cx))
}

#[gpui::test]
fn settings_modal_is_keyboard_driven_and_applies_live(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(&library, None, &simple_document("Doc", "Text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(vcx.update(|_, cx| theme::theme_mode(cx)), ThemeMode::System);

    vcx.simulate_keystrokes("cmd-,");
    let view = settings_view(&workspace, vcx).expect("settings are open");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::Theme));

    // Theme: right twice goes System -> Paper -> Night, applied at once.
    vcx.simulate_keystrokes("right");
    assert_eq!(vcx.update(|_, cx| theme::theme_mode(cx)), ThemeMode::Light);
    vcx.simulate_keystrokes("right right");
    assert_eq!(vcx.update(|_, cx| theme::theme_mode(cx)), ThemeMode::Dark);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.settings().theme),
        ThemeMode::Dark
    );

    // Down to Vim mode, space turns it on.
    vcx.simulate_keystrokes("down");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::VimMode));
    vcx.simulate_keystrokes("space");
    assert!(workspace.read_with(vcx, |w, _| w.settings().vim_mode));

    // Markdown shortcuts off reaches the editor configuration.
    vcx.simulate_keystrokes("down enter");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::MarkdownShortcuts));
    let config = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |w, cx| {
            let config = w.editor().read(cx).config();
            (config.input_rules, config.smart_punctuation)
        })
    };
    assert_eq!(config(vcx), (false, true));
    // Smart punctuation: left turns a switch off.
    vcx.simulate_keystrokes("down left");
    assert_eq!(config(vcx), (false, false));
    // The format bar over a mouse selection: on unless switched off here.
    vcx.simulate_keystrokes("down");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::FormatBar));
    assert!(workspace.read_with(vcx, |w, cx| w.tools().read(cx).format_bar_enabled()));
    vcx.simulate_keystrokes("space");
    assert!(!workspace.read_with(vcx, |w, _| w.settings().format_bar));
    assert!(!workspace.read_with(vcx, |w, cx| w.tools().read(cx).format_bar_enabled()));
    vcx.simulate_keystrokes("right");
    assert!(workspace.read_with(vcx, |w, cx| w.tools().read(cx).format_bar_enabled()));

    // Spelling: a switch (on unless switched off) and the language.
    vcx.simulate_keystrokes("down");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::CheckSpelling));
    assert!(workspace.read_with(vcx, |w, _| w.settings().check_spelling));
    vcx.simulate_keystrokes("space");
    assert!(!workspace.read_with(vcx, |w, _| w.settings().check_spelling));
    vcx.simulate_keystrokes("space down");
    assert!(workspace.read_with(vcx, |w, _| w.settings().check_spelling));
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::SpellingLanguage));

    // The author name is a text field inside the list.
    vcx.simulate_keystrokes("down");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::Author));
    vcx.simulate_keystrokes("cmd-a");
    vcx.simulate_input("Mara Voss");
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.settings().comment_author.clone()),
        "Mara Voss"
    );
    // Typing a space in the field types a space; enter moves on.
    vcx.simulate_keystrokes("enter");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::Font));

    // Text size: right steps up from 17 to 18.
    vcx.simulate_keystrokes("down right");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::TextSize));
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.settings().new_document_size),
        18.
    );
    // Line height down, page view on, paper to US Letter.
    vcx.simulate_keystrokes("down left down space down right");
    let settings = workspace.read_with(vcx, |w, _| w.settings().clone());
    assert_eq!(settings.new_document_line_height, 1.5);
    assert!(settings.new_document_page_view);
    assert_eq!(settings.new_document_paper, Paper::Letter);
    // Character limit: the first step is X.
    vcx.simulate_keystrokes("down right");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::CharacterLimit));
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.settings().character_limit),
        Some(SocialNetwork::X)
    );
    // What the status bar counts: three switches.
    vcx.simulate_keystrokes("down");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::StatusWords));
    vcx.simulate_keystrokes("down down space");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::StatusReadingTime));
    assert!(!workspace.read_with(vcx, |w, _| w.settings().status_reading_time));
    assert_eq!(workspace.update(vcx, |w, cx| w.status_line(cx)).reading, "");
    vcx.simulate_keystrokes("space");
    assert!(workspace.read_with(vcx, |w, _| w.settings().status_reading_time));
    // The last row about the app leads to the keyboard shortcuts. With a
    // document open the list goes on with that document's own settings;
    // up goes back.
    vcx.simulate_keystrokes("down");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::KeyboardShortcuts));
    // Left and right have nothing to step through on it.
    vcx.simulate_keystrokes("right left");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::KeyboardShortcuts));
    vcx.simulate_keystrokes("down");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::DocumentPageView));
    vcx.simulate_keystrokes("up up up up up up");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::Paper));

    // Escape closes and focus returns to the editor.
    vcx.simulate_keystrokes("escape");
    assert!(settings_view(&workspace, vcx).is_none());
    assert!(editor_focused(&workspace, vcx));

    // A new document picks the defaults up.
    let id = workspace
        .update_in(vcx, |workspace, window, cx| {
            workspace.new_document(window, cx)
        })
        .unwrap();
    let document = workspace.read_with(vcx, |w, cx| w.editor().read(cx).document().clone());
    assert_eq!(document.settings().typography.base_size, 18.);
    assert_eq!(document.settings().typography.line_height, 1.5);
    assert!(document.settings().page_view);
    assert_eq!(
        document.settings().page.paper,
        innerword_core::PaperSize::UsLetter
    );
    // The status bar counts against the limit.
    type_text(&workspace, "Hello", vcx);
    let status = workspace.update(vcx, |w, cx| w.status_line(cx));
    assert_eq!(status.limit.as_deref(), Some("5 / 280 on X"));
    // The indicator is the character count while it is on.
    assert_eq!(status.characters, "");
    assert!(!status.over_limit);
    type_text(&workspace, &"word ".repeat(60), vcx);
    let status = workspace.update(vcx, |w, cx| w.status_line(cx));
    assert!(status.over_limit, "{status:?}");
    let _ = id;
}

#[gpui::test]
fn settings_survive_a_relaunch(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    {
        let library = cx.new(|_| Library::open(&path).unwrap());
        let (workspace, vcx) = open_workspace(&library, cx);
        workspace.update_in(vcx, |workspace, window, cx| {
            workspace.set_theme(ThemeMode::Light, cx);
            workspace.toggle_vim_mode(cx);
            workspace.toggle_comments(cx);
            workspace.set_sidebar_width(333., cx);
            workspace.set_sidebar_visible(false, window, cx);
            workspace.write_setting(keys::NEW_DOCUMENT_FONT, "Georgia", cx);
            workspace.write_setting(keys::CHARACTER_LIMIT, "mastodon", cx);
        });
        // The first run created the welcome document.
        assert!(
            workspace
                .read_with(vcx, |w, _| w.open_document_id())
                .is_some()
        );
    }
    cx.run_until_parked();

    // A different process would open the file again.
    let reopened = Library::open(&path).unwrap();
    let loaded = Settings::load(&reopened);
    assert_eq!(loaded.theme, ThemeMode::Light);
    assert!(loaded.vim_mode);
    assert!(!loaded.comments_visible);
    assert_eq!(loaded.sidebar_width, 333.);
    assert!(!loaded.sidebar_visible);
    assert_eq!(loaded.new_document_font, "Georgia");
    assert_eq!(loaded.character_limit, Some(SocialNetwork::Mastodon));
    // Untouched settings keep their defaults.
    assert!(loaded.input_rules && loaded.smart_punctuation);
    drop(reopened);

    // And a workspace over it applies them.
    let library = cx.new(|_| Library::open(&path).unwrap());
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(vcx.update(|_, cx| theme::theme_mode(cx)), ThemeMode::Light);
    assert!(!workspace.read_with(vcx, |w, cx| w.editor().read(cx).config().comments_visible));
    assert!(!workspace.read_with(vcx, |w, _| w.settings().sidebar_visible));
    // No second welcome document.
    assert_eq!(
        library.read_with(vcx, |library, _| library.all_documents().unwrap().len()),
        1
    );
}

#[gpui::test]
fn view_toggles_flip_settings_and_the_document(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let id = add_document(&library, None, &simple_document("Pages", "Text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    let mut events = Vec::new();
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let _subscription = vcx.update(|_, cx| {
        let seen = seen.clone();
        cx.subscribe(
            &workspace,
            move |_, event: &innerword_app::WorkspaceEvent, _| {
                seen.borrow_mut().push(event.clone());
            },
        )
    });

    // cmd-alt-p: page view, stored in the document.
    assert!(!workspace.read_with(vcx, |w, cx| w.editor().read(cx).page_view()));
    vcx.simulate_keystrokes("cmd-alt-p");
    assert!(workspace.read_with(vcx, |w, cx| w.editor().read(cx).page_view()));
    workspace.update(vcx, |workspace, cx| workspace.flush(cx));
    let stored = library.read_with(vcx, |library, _| library.document(id).unwrap().content);
    assert!(Document::from_json(&stored).unwrap().settings().page_view);

    // cmd-alt-v: Vim mode, a setting.
    vcx.simulate_keystrokes("cmd-alt-v");
    assert!(workspace.read_with(vcx, |w, _| w.settings().vim_mode));
    assert_eq!(
        library.read_with(vcx, |library, _| library
            .setting::<bool>(keys::VIM_MODE)
            .unwrap()),
        Some(true)
    );

    // cmd-alt-c: comments, a setting that reaches the editor.
    vcx.simulate_keystrokes("cmd-alt-c");
    assert!(!workspace.read_with(vcx, |w, _| w.settings().comments_visible));
    assert!(!workspace.read_with(vcx, |w, cx| w.editor().read(cx).config().comments_visible));
    vcx.simulate_keystrokes("cmd-alt-c");
    assert!(workspace.read_with(vcx, |w, cx| w.editor().read(cx).config().comments_visible));

    // A setting written by someone else (another crate, the settings
    // modal) is applied the same way.
    library.update(vcx, |library, cx| {
        library
            .set_setting(keys::SMART_PUNCTUATION, &false, cx)
            .unwrap()
    });
    vcx.run_until_parked();
    assert!(!workspace.read_with(vcx, |w, cx| w.editor().read(cx).config().smart_punctuation));

    events.extend(seen.borrow().iter().cloned());
    let changes = events
        .iter()
        .filter(|event| **event == innerword_app::WorkspaceEvent::SettingsChanged)
        .count();
    assert_eq!(changes, 4, "{events:?}");
}

#[gpui::test]
fn this_document_section_changes_the_open_document(cx: &mut TestAppContext) {
    use innerword_core::{Orientation, PaperSize};

    init(cx);
    let library = library(cx);
    let id = add_document(
        &library,
        None,
        &simple_document("Pages", "A paragraph on a page."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    let document = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |w, cx| {
            w.editor().read(cx).document().settings().clone()
        })
    };
    let before = document(vcx);
    assert_eq!(before.page.paper, PaperSize::A4);

    // "Document settings" in the palette opens the settings at the section
    // for this document.
    vcx.simulate_keystrokes("cmd-shift-p");
    vcx.simulate_input("document settings");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let view = settings_view(&workspace, vcx).expect("the settings are open");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::DocumentPageView));
    let rows = view.read_with(vcx, |view, cx| view.rows(cx));
    assert!(rows.starts_with(&SettingRow::APP));
    assert!(!rows.contains(&SettingRow::DocumentPaperWidth));

    // Page view on; paper to US Letter, then to a custom size, which starts
    // from the sheet it replaces and brings two more rows.
    vcx.simulate_keystrokes("space down right");
    assert!(document(vcx).page_view);
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::DocumentPaper));
    assert_eq!(document(vcx).page.paper, PaperSize::UsLetter);
    let field = |row: SettingRow, cx: &mut VisualTestContext| {
        view.read_with(cx, |view, cx| view.length_field_text(row, cx).unwrap())
    };
    // Lengths of a Letter document are in inches.
    assert_eq!(field(SettingRow::DocumentMarginTop, vcx), "1");
    vcx.simulate_keystrokes("right");
    assert_eq!(
        document(vcx).page.paper,
        PaperSize::Custom {
            width: 612.,
            height: 792.
        }
    );
    assert!(
        view.read_with(vcx, |view, cx| view.rows(cx))
            .contains(&SettingRow::DocumentPaperWidth)
    );
    // ...and of any other in millimetres.
    assert_eq!(field(SettingRow::DocumentPaperWidth, vcx), "215.9");
    assert_eq!(field(SettingRow::DocumentPaperHeight, vcx), "279.4");
    assert_eq!(field(SettingRow::DocumentMarginTop, vcx), "25.4");

    // The size is typed: A5.
    vcx.simulate_keystrokes("down");
    assert_eq!(
        focused_row(&view, vcx),
        Some(SettingRow::DocumentPaperWidth)
    );
    vcx.simulate_keystrokes("cmd-a");
    vcx.simulate_input("148");
    vcx.simulate_keystrokes("enter");
    assert_eq!(
        focused_row(&view, vcx),
        Some(SettingRow::DocumentPaperHeight)
    );
    vcx.simulate_keystrokes("cmd-a");
    vcx.simulate_input("210");
    let (width, height) = document(vcx).page.paper.portrait_size();
    assert!((width - 419.53).abs() < 0.01 && (height - 595.28).abs() < 0.01);

    // Orientation, with the arrow keys.
    vcx.simulate_keystrokes("down right");
    assert_eq!(
        focused_row(&view, vcx),
        Some(SettingRow::DocumentOrientation)
    );
    assert_eq!(document(vcx).page.orientation, Orientation::Landscape);
    let (sheet_width, sheet_height) = document(vcx).page.page_size();
    assert!(sheet_width > sheet_height);

    // Margins: millimetres by default, another unit when it is named.
    vcx.simulate_keystrokes("down cmd-a");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::DocumentMarginTop));
    // The row that is typed into says so while the keyboard is in it.
    let help = |focused: bool, cx: &mut VisualTestContext| {
        view.read_with(cx, |view, cx| {
            view.description(SettingRow::DocumentMarginTop, focused, cx)
                .to_string()
        })
    };
    assert!(
        help(true, vcx).starts_with("In millimetres."),
        "{}",
        help(true, vcx)
    );
    assert_eq!(help(false, vcx), "");
    vcx.simulate_input("20");
    vcx.simulate_keystrokes("down cmd-a");
    vcx.simulate_input("0.5in");
    vcx.simulate_keystrokes("down cmd-a");
    vcx.simulate_input("3cm");
    vcx.simulate_keystrokes("down cmd-a");
    assert_eq!(
        focused_row(&view, vcx),
        Some(SettingRow::DocumentMarginRight)
    );
    vcx.simulate_input("24pt");
    let margins = document(vcx).page.margins;
    assert!((margins.top - 56.69).abs() < 0.01, "{margins:?}");
    assert_eq!(margins.bottom, 36.);
    assert!((margins.left - 85.04).abs() < 0.01, "{margins:?}");
    assert_eq!(margins.right, 24.);
    // A field that was left shows the value in the document's unit again;
    // text that is no length changes nothing.
    assert_eq!(field(SettingRow::DocumentMarginBottom, vcx), "12.7");
    vcx.simulate_keystrokes("cmd-a");
    vcx.simulate_input("wide");
    assert_eq!(document(vcx).page.margins.right, 24.);

    // Typography: font, size, line height, paragraph spacing.
    let font_before = document(vcx).typography.body_font.clone();
    vcx.simulate_keystrokes("down");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::DocumentFont));
    assert_eq!(field(SettingRow::DocumentMarginRight, vcx), "8.5");
    vcx.simulate_keystrokes("right");
    // Right steps to the next family by name, of every family there is.
    let fonts = vcx.update(|_, cx| innerword_app::settings::font_steps(&font_before, cx));
    if let Some(at) = fonts.iter().position(|font| *font == font_before)
        && let Some(next) = fonts.get(at + 1)
    {
        assert_eq!(&document(vcx).typography.body_font, next);
    }
    vcx.simulate_keystrokes("down right right");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::DocumentTextSize));
    assert_eq!(
        document(vcx).typography.base_size,
        before.typography.base_size + 2.
    );
    vcx.simulate_keystrokes("down left");
    assert!((document(vcx).typography.line_height - 1.5).abs() < 0.001);
    vcx.simulate_keystrokes("down right");
    assert_eq!(
        focused_row(&view, vcx),
        Some(SettingRow::DocumentParagraphSpacing)
    );
    assert_eq!(document(vcx).typography.paragraph_spacing, 13.);
    // The language the document's spelling is checked in comes last.
    vcx.simulate_keystrokes("down");
    assert_eq!(
        focused_row(&view, vcx),
        Some(SettingRow::DocumentSpellingLanguage)
    );
    assert_eq!(document(vcx).spelling_language, None);
    // The list ends here.
    vcx.simulate_keystrokes("down");
    assert_eq!(
        focused_row(&view, vcx),
        Some(SettingRow::DocumentSpellingLanguage)
    );

    // The defaults for new documents were not touched.
    let settings = workspace.read_with(vcx, |w, _| w.settings().clone());
    assert_eq!(
        settings,
        Settings {
            comment_author: settings.comment_author.clone(),
            ..Settings::default()
        }
    );

    // Escape closes; the changes are the document's own: saved with it.
    vcx.simulate_keystrokes("escape");
    assert!(settings_view(&workspace, vcx).is_none());
    assert!(editor_focused(&workspace, vcx));
    assert!(workspace.read_with(vcx, |w, cx| w.editor().read(cx).page_view()));
    vcx.executor()
        .advance_clock(innerword_app::workspace::AUTOSAVE_DELAY * 2);
    vcx.run_until_parked();
    let stored = library.read_with(vcx, |library, _| library.document(id).unwrap().content);
    let reread = Document::from_json(&stored).unwrap();
    assert_eq!(reread.settings(), &document(vcx));

    // Back to a standard sheet: the custom rows go away.
    vcx.simulate_keystrokes("cmd-shift-p");
    vcx.simulate_input("document settings");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let view = settings_view(&workspace, vcx).expect("the settings are open");
    vcx.simulate_keystrokes("down left left");
    assert_eq!(document(vcx).page.paper, PaperSize::A4);
    assert!(
        !view
            .read_with(vcx, |view, cx| view.rows(cx))
            .contains(&SettingRow::DocumentPaperWidth)
    );
    vcx.simulate_keystrokes("down");
    assert_eq!(
        focused_row(&view, vcx),
        Some(SettingRow::DocumentOrientation)
    );
    vcx.simulate_keystrokes("escape");

    // Without a document there is no such section, and the command says so.
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.close_document(window, cx)
    });
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.toggle_document_settings(window, cx)
    });
    assert!(settings_view(&workspace, vcx).is_none());
    vcx.simulate_keystrokes("cmd-,");
    let view = settings_view(&workspace, vcx).expect("the settings are open");
    assert_eq!(
        view.read_with(vcx, |view, cx| view.rows(cx)),
        SettingRow::APP.to_vec()
    );
}

#[gpui::test]
fn held_keys_follow_vim_mode_from_the_next_launch(cx: &mut TestAppContext) {
    use innerword_app::press_and_hold;

    init(cx);
    // As in the real app (without writing the user's preferences).
    cx.update(press_and_hold::manage_without_writing);
    let library = library(cx);
    add_document(&library, None, &simple_document("Doc", "Text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    let state = |cx: &mut VisualTestContext| cx.update(|_, cx| press_and_hold::state(cx));
    let toasts = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |w, cx| {
            w.toast_layer()
                .read(cx)
                .toasts()
                .map(|toast| toast.message.to_string())
                .collect::<Vec<_>>()
        })
    };
    let vim_text = |cx: &mut VisualTestContext| {
        cx.simulate_keystrokes("cmd-,");
        let view = settings_view(&workspace, cx).expect("settings are open");
        let text = view.read_with(cx, |view, cx| {
            view.description(SettingRow::VimMode, false, cx).to_string()
        });
        cx.simulate_keystrokes("escape");
        text
    };

    // Launched with Vim mode off: the accent menu is the system's, and
    // nothing has to be said.
    assert_eq!(state(vcx).repeats_now(), Some(false));
    assert!(!state(vcx).relaunch_needed());
    assert!(toasts(vcx).is_empty());
    assert!(!vim_text(vcx).contains("Restart"));

    // Vim mode on: stored for the next launch, and the app says that the
    // rest follows then, once in a toast and in the settings.
    vcx.simulate_keystrokes("cmd-alt-v");
    assert!(workspace.read_with(vcx, |w, _| w.settings().vim_mode));
    assert_eq!(state(vcx).repeats_next(), Some(true));
    assert_eq!(state(vcx).repeats_now(), Some(false));
    assert!(state(vcx).restart_note_due());
    assert_eq!(toasts(vcx).len(), 1);
    assert!(
        toasts(vcx)[0].contains("after a restart"),
        "{:?}",
        toasts(vcx)
    );
    assert!(vim_text(vcx).contains("Restart Inner Word for held keys to repeat"));

    // Off again before any relaunch: back to what this launch does anyway.
    vcx.simulate_keystrokes("cmd-alt-v");
    assert!(!state(vcx).relaunch_needed());
    assert!(!vim_text(vcx).contains("Restart"));
    assert_eq!(toasts(vcx).len(), 1, "nothing new to say");

    // The next launch, with Vim mode stored as on: in effect from the
    // start, nothing to say; turning it off then needs a relaunch too.
    vcx.simulate_keystrokes("cmd-alt-v");
    cx.update(press_and_hold::reset);
    let (second, vcx) = open_workspace(&library, cx);
    let state = |cx: &mut VisualTestContext| cx.update(|_, cx| press_and_hold::state(cx));
    assert!(second.read_with(vcx, |w, _| w.settings().vim_mode));
    assert_eq!(state(vcx).repeats_now(), Some(true));
    assert!(!state(vcx).relaunch_needed());
    second.update(vcx, |workspace, cx| workspace.toggle_vim_mode(cx));
    assert_eq!(state(vcx).repeats_next(), Some(false));
    assert!(state(vcx).restart_note_due());
}

#[gpui::test]
fn tests_and_scripted_runs_leave_the_user_default_alone(cx: &mut TestAppContext) {
    use innerword_app::press_and_hold;

    init(cx);
    let library = library(cx);
    add_document(&library, None, &simple_document("Doc", "Text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    vcx.simulate_keystrokes("cmd-alt-v");
    let state = vcx.update(|_, cx| press_and_hold::state(cx));
    // The bookkeeping is the same...
    assert_eq!(state.repeats_next(), Some(true));
    assert!(state.relaunch_needed());
    // ...but nothing is stored, so nothing is promised for a relaunch.
    assert!(!state.is_managed());
    assert!(!state.restart_note_due());
    let toasts = workspace.read_with(vcx, |w, cx| w.toast_layer().read(cx).toasts().count());
    assert_eq!(toasts, 0);
}

// ----- the fonts ---------------------------------------------------------------

use gpui::{Modifiers, point, px};
use innerword_app::font_choice::{FontChoicePicker, FontTarget, MISSING_LABEL};
use innerword_ui::picker::{PickerDelegate as _, PickerEntry};

/// The families of the Mac the font tests run on, by name.
const FAMILIES: [&str; 7] = [
    "Avenir Next",
    "Charter",
    "Georgia",
    "Menlo",
    "Palatino",
    "Papyrus",
    "Zapfino",
];

fn install_fonts(cx: &mut TestAppContext) {
    cx.update(|cx| innerword_editor_tools::set_font_families(FAMILIES, cx));
}

fn font_list(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Option<Entity<FontChoicePicker>> {
    workspace.read_with(cx, |workspace, cx| {
        workspace
            .modal_layer()
            .read(cx)
            .active::<FontChoicePicker>()
    })
}

fn listed(picker: &Entity<FontChoicePicker>, cx: &mut VisualTestContext) -> Vec<String> {
    picker.read_with(cx, |picker, _| picker.delegate.labels())
}

fn marked(picker: &Entity<FontChoicePicker>, cx: &mut VisualTestContext) -> Option<String> {
    picker.read_with(cx, |picker, _| {
        picker
            .selected_index()
            .and_then(|ix| picker.delegate.font_at(ix))
    })
}

/// Moves the keyboard down the open settings until it is on `row`.
fn walk_to(row: SettingRow, view: &Entity<SettingsView>, cx: &mut VisualTestContext) {
    for _ in 0..60 {
        if focused_row(view, cx) == Some(row) {
            return;
        }
        cx.simulate_keystrokes("down");
    }
    panic!("the keyboard never reached {row:?}");
}

/// Clicks the row `row` of the open settings where it is drawn (brought
/// into view first: the list is longer than the window).
fn click_setting(row: SettingRow, view: &Entity<SettingsView>, cx: &mut VisualTestContext) {
    let ix = view
        .read_with(cx, |view, cx| {
            view.rows(cx).iter().position(|listed| *listed == row)
        })
        .expect("the row is listed");
    view.update(cx, |view, cx| {
        view.scroll_handle().scroll_to_item(ix);
        cx.notify();
    });
    cx.run_until_parked();
    let selector: &'static str = Box::leak(format!("setting-{ix}").into_boxed_str());
    let bounds = cx.debug_bounds(selector).expect("the row is drawn");
    // The name of the row, away from the arrows at its right.
    let at = point(bounds.left() + px(60.), bounds.bottom() - px(24.));
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
}

/// Clicks the row of the open list of fonts that names `font`.
fn click_font(font: &str, picker: &Entity<FontChoicePicker>, cx: &mut VisualTestContext) {
    let ix = listed(picker, cx)
        .iter()
        .position(|label| label == font)
        .unwrap_or_else(|| panic!("{font} is not listed"));
    let selector: &'static str = Box::leak(format!("picker-row-{ix}").into_boxed_str());
    let bounds = cx.debug_bounds(selector).expect("the row is drawn");
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.run_until_parked();
}

fn document_font(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> String {
    workspace.read_with(cx, |w, cx| w.document_font(cx))
}

fn new_document_font(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> String {
    workspace.read_with(cx, |w, _| w.settings().new_document_font.clone())
}

#[gpui::test]
fn the_settings_list_the_fonts_the_font_picker_lists(cx: &mut TestAppContext) {
    init(cx);
    install_fonts(cx);
    let library = library(cx);
    add_document(&library, None, &simple_document("Doc", "Text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    // Two families were used lately: one is a writing font, one is not.
    vcx.update(|_, cx| innerword_editor_tools::set_recent_fonts(["Papyrus", "Georgia"], cx));

    // The font picker of the text, without its own first row ("Document
    // default" is a choice of the selection, not a font).
    let picker_rows = |query: &str, cx: &mut VisualTestContext| {
        cx.simulate_keystrokes("cmd-t");
        cx.simulate_input(query);
        let rows: Vec<String> = workspace.read_with(cx, |w, cx| {
            w.tools()
                .read(cx)
                .menu_rows(cx)
                .into_iter()
                .map(|row| row.label)
                .filter(|label| label != "Document default")
                .collect()
        });
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        rows
    };
    let in_picker = picker_rows("", vcx);
    let in_picker_filtered = picker_rows("a", vcx);
    assert_eq!(
        in_picker,
        [
            "Writing fonts",
            "Charter",
            "Georgia",
            "Palatino",
            "Avenir Next",
            "Menlo",
            "Recent",
            "Papyrus",
            "All fonts",
            "Avenir Next",
            "Charter",
            "Georgia",
            "Menlo",
            "Palatino",
            "Papyrus",
            "Zapfino",
        ]
    );
    assert!(in_picker_filtered.len() > 2, "{in_picker_filtered:?}");

    // Both font rows of the settings: the same rows in the same order,
    // with and without a query.
    for (target, row) in [
        (FontTarget::NewDocuments, SettingRow::Font),
        (FontTarget::Document, SettingRow::DocumentFont),
    ] {
        vcx.simulate_keystrokes("cmd-,");
        let view = settings_view(&workspace, vcx).expect("settings are open");
        view.update(vcx, |view, cx| view.activate(row, cx));
        vcx.run_until_parked();
        let list = font_list(&workspace, vcx).expect("the list of fonts is open");
        assert_eq!(
            list.read_with(vcx, |list, _| list.delegate.target()),
            target
        );
        assert_eq!(listed(&list, vcx), in_picker, "{target:?}");
        vcx.simulate_input("a");
        assert_eq!(listed(&list, vcx), in_picker_filtered, "{target:?}");
        vcx.simulate_keystrokes("escape");
        vcx.run_until_parked();
        vcx.simulate_keystrokes("escape");
        assert!(settings_view(&workspace, vcx).is_none());
    }

    // Left and right on a row step through every family, by name.
    assert_eq!(
        vcx.update(|_, cx| innerword_app::settings::font_steps("Charter", cx)),
        FAMILIES
    );
}

#[gpui::test]
fn the_font_of_new_documents_is_chosen_from_the_list_with_the_keyboard(cx: &mut TestAppContext) {
    init(cx);
    install_fonts(cx);
    let library = library(cx);
    add_document(&library, None, &simple_document("Doc", "Text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(new_document_font(&workspace, vcx), "Charter");

    vcx.simulate_keystrokes("cmd-,");
    let view = settings_view(&workspace, vcx).expect("settings are open");
    walk_to(SettingRow::Font, &view, vcx);
    // The row says which key opens the whole list.
    let says = view.read_with(vcx, |view, cx| {
        view.description(SettingRow::Font, true, cx).to_string()
    });
    assert!(says.ends_with("opens the whole list."), "{says}");
    // The footer under it names the same keys for the same thing (0.6.0
    // said "↩ or Space opens the whole list" in the row and "Space  Whole
    // list" under it).
    let (keys, what) = vcx
        .update(|window, cx| view.read(cx).activate_hint(window, cx))
        .expect("the footer has a hint for the row's keys");
    assert_eq!(what, "Whole list");
    assert_eq!(keys, "↩ or Space");
    assert!(
        says.ends_with(&format!(" {keys} opens the whole list.")),
        "{says}"
    );
    // On a row that switches, the same keys switch.
    walk_to(SettingRow::PageView, &view, vcx);
    assert_eq!(
        vcx.update(|window, cx| view.read(cx).activate_hint(window, cx)),
        Some(("↩ or Space".to_string(), "Switch"))
    );
    vcx.simulate_keystrokes("up up up");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::Font));

    // Enter opens the list in the place of the settings, on the font in
    // use.
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let list = font_list(&workspace, vcx).expect("the list of fonts is open");
    assert!(settings_view(&workspace, vcx).is_none());
    assert_eq!(marked(&list, vcx).as_deref(), Some("Charter"));
    // Down and up move.
    vcx.simulate_keystrokes("down");
    assert_eq!(marked(&list, vcx).as_deref(), Some("Georgia"));
    vcx.simulate_keystrokes("up");
    assert_eq!(marked(&list, vcx).as_deref(), Some("Charter"));
    // Typing narrows; escape closes without a change and goes back to
    // the row.
    vcx.simulate_input("zap");
    assert_eq!(listed(&list, vcx), ["Zapfino"]);
    assert_eq!(marked(&list, vcx).as_deref(), Some("Zapfino"));
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(font_list(&workspace, vcx).is_none());
    let view = settings_view(&workspace, vcx).expect("back in the settings");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::Font));
    assert_eq!(new_document_font(&workspace, vcx), "Charter");

    // Space opens it too; enter chooses.
    vcx.simulate_keystrokes("space");
    vcx.run_until_parked();
    let list = font_list(&workspace, vcx).expect("the list of fonts is open");
    vcx.simulate_input("zap");
    assert_eq!(listed(&list, vcx), ["Zapfino"]);
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(font_list(&workspace, vcx).is_none());
    let view = settings_view(&workspace, vcx).expect("back in the settings");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::Font));
    assert_eq!(new_document_font(&workspace, vcx), "Zapfino");
    assert_eq!(
        view.read_with(vcx, |v, _| v.settings().new_document_font.clone()),
        "Zapfino"
    );
    // The open document keeps its font.
    assert_eq!(document_font(&workspace, vcx), "Charter");
    // A family chosen here is one of the recently used ones, as one
    // chosen in the font picker of the text is.
    assert_eq!(
        vcx.update(|_, cx| innerword_editor_tools::recent_fonts(cx)),
        ["Zapfino"]
    );

    // Left and right still step, through every family by name.
    vcx.simulate_keystrokes("left");
    assert_eq!(new_document_font(&workspace, vcx), "Papyrus");
    vcx.simulate_keystrokes("right right");
    assert_eq!(
        new_document_font(&workspace, vcx),
        "Zapfino",
        "the last one"
    );
    vcx.simulate_keystrokes("escape");
    assert!(editor_focused(&workspace, vcx));

    // A new document gets the font; so does the stored setting.
    workspace
        .update_in(vcx, |workspace, window, cx| {
            workspace.new_document(window, cx)
        })
        .unwrap();
    assert_eq!(document_font(&workspace, vcx), "Zapfino");
    let stored: String = library.read_with(vcx, |library, _| {
        library.setting_or(keys::NEW_DOCUMENT_FONT, String::new())
    });
    assert_eq!(stored, "Zapfino");
}

#[gpui::test]
fn the_font_of_this_document_is_chosen_from_the_list_with_the_keyboard(cx: &mut TestAppContext) {
    init(cx);
    install_fonts(cx);
    let library = library(cx);
    let id = add_document(&library, None, &simple_document("Doc", "Text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);

    vcx.simulate_keystrokes("cmd-shift-p");
    vcx.simulate_input("document settings");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let view = settings_view(&workspace, vcx).expect("the settings are open");
    walk_to(SettingRow::DocumentFont, &view, vcx);
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let list = font_list(&workspace, vcx).expect("the list of fonts is open");
    assert_eq!(
        list.read_with(vcx, |list, _| list.delegate.target()),
        FontTarget::Document
    );
    assert_eq!(marked(&list, vcx).as_deref(), Some("Charter"));
    vcx.simulate_input("men");
    assert_eq!(listed(&list, vcx), ["Menlo"]);
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();

    let view = settings_view(&workspace, vcx).expect("back in the settings");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::DocumentFont));
    assert_eq!(document_font(&workspace, vcx), "Menlo");
    // The font of new documents is another setting.
    assert_eq!(new_document_font(&workspace, vcx), "Charter");
    // Escape in the list changes nothing.
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let list = font_list(&workspace, vcx).expect("the list of fonts is open");
    assert_eq!(marked(&list, vcx).as_deref(), Some("Menlo"));
    vcx.simulate_input("geo");
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert_eq!(document_font(&workspace, vcx), "Menlo");
    vcx.simulate_keystrokes("escape");
    assert!(editor_focused(&workspace, vcx));

    // The change is the document's own: saved with it.
    assert!(workspace.update(vcx, |w, cx| w.flush(cx)));
    let stored = library.read_with(vcx, |library, _| library.document(id).unwrap().content);
    let reread = innerword_core::Document::from_json(&stored).unwrap();
    assert_eq!(reread.settings().typography.body_font, "Menlo");
}

#[gpui::test]
fn the_fonts_are_chosen_from_the_list_with_the_pointer_alone(cx: &mut TestAppContext) {
    init(cx);
    install_fonts(cx);
    let library = library(cx);
    add_document(&library, None, &simple_document("Doc", "Text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.toggle_document_settings(window, cx)
    });
    vcx.run_until_parked();

    // New documents: a click on the row opens the list, a click on a font
    // chooses it and comes back.
    let view = settings_view(&workspace, vcx).expect("the settings are open");
    click_setting(SettingRow::Font, &view, vcx);
    let list = font_list(&workspace, vcx).expect("the list of fonts is open");
    assert!(settings_view(&workspace, vcx).is_none());
    click_font("Palatino", &list, vcx);
    assert!(font_list(&workspace, vcx).is_none());
    assert_eq!(new_document_font(&workspace, vcx), "Palatino");
    assert_eq!(document_font(&workspace, vcx), "Charter");
    let view = settings_view(&workspace, vcx).expect("back in the settings");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::Font));

    // A click outside the list closes it without a change, as it closes
    // the list of spelling languages: all the way, back to the text.
    click_setting(SettingRow::Font, &view, vcx);
    assert!(font_list(&workspace, vcx).is_some());
    vcx.simulate_click(point(px(20.), px(500.)), Modifiers::default());
    vcx.run_until_parked();
    assert!(font_list(&workspace, vcx).is_none());
    assert!(settings_view(&workspace, vcx).is_none());
    assert_eq!(new_document_font(&workspace, vcx), "Palatino");
    assert!(editor_focused(&workspace, vcx));
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.toggle_document_settings(window, cx)
    });
    vcx.run_until_parked();
    let view = settings_view(&workspace, vcx).expect("the settings are open");

    // This document.
    click_setting(SettingRow::DocumentFont, &view, vcx);
    let list = font_list(&workspace, vcx).expect("the list of fonts is open");
    assert_eq!(
        list.read_with(vcx, |list, _| list.delegate.target()),
        FontTarget::Document
    );
    click_font("Georgia", &list, vcx);
    assert_eq!(document_font(&workspace, vcx), "Georgia");
    assert_eq!(new_document_font(&workspace, vcx), "Palatino");
    let view = settings_view(&workspace, vcx).expect("back in the settings");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::DocumentFont));

    // A new document gets the font chosen for new documents.
    vcx.simulate_click(point(px(20.), px(500.)), Modifiers::default());
    vcx.run_until_parked();
    assert!(settings_view(&workspace, vcx).is_none());
    workspace
        .update_in(vcx, |workspace, window, cx| {
            workspace.new_document(window, cx)
        })
        .unwrap();
    assert_eq!(document_font(&workspace, vcx), "Palatino");
}

#[gpui::test]
fn a_font_that_is_not_on_this_mac_stays_set_and_is_listed_as_missing(cx: &mut TestAppContext) {
    init(cx);
    install_fonts(cx);
    let library = library(cx);
    // A library from another Mac: its font of new documents and the body
    // font of a document are families this Mac does not have.
    library.update(cx, |library, cx| {
        library
            .set_setting(keys::NEW_DOCUMENT_FONT, "Lucida Fax", cx)
            .unwrap();
    });
    let mut settings = innerword_core::DocumentSettings::default();
    settings.typography.body_font = "Sabon Next".to_string();
    let document = simple_document("Doc", "Text.").with_settings(settings);
    add_document(&library, None, &document, cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(new_document_font(&workspace, vcx), "Lucida Fax");
    assert_eq!(document_font(&workspace, vcx), "Sabon Next");
    assert!(!vcx.update(|_, cx| innerword_app::settings::font_is_installed("Lucida Fax", cx)));

    vcx.simulate_keystrokes("cmd-,");
    let view = settings_view(&workspace, vcx).expect("settings are open");
    for (row, font) in [
        (SettingRow::Font, "Lucida Fax"),
        (SettingRow::DocumentFont, "Sabon Next"),
    ] {
        // The row shows the font as what it is, and says so.
        assert!(view.read_with(vcx, |view, cx| view.font_is_missing(row, cx)));
        let says = view.read_with(vcx, |view, cx| view.description(row, false, cx).to_string());
        assert!(
            says.contains(&format!("{font} is not on this Mac")),
            "{says}"
        );
        // It is the first of the steps, before every family.
        let steps = vcx.update(|_, cx| innerword_app::settings::font_steps(font, cx));
        assert_eq!(steps[0], font);
        assert_eq!(steps[1..], FAMILIES);
    }
    // Left has nowhere to go and changes nothing.
    walk_to(SettingRow::Font, &view, vcx);
    vcx.simulate_keystrokes("left");
    assert_eq!(new_document_font(&workspace, vcx), "Lucida Fax");

    // In the list it is the first row, marked as missing and as in use,
    // and the row the list opens on.
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let list = font_list(&workspace, vcx).expect("the list of fonts is open");
    let labels = listed(&list, vcx);
    assert_eq!(labels[..2], ["Lucida Fax", "Writing fonts"]);
    assert_eq!(marked(&list, vcx).as_deref(), Some("Lucida Fax"));
    let (detail, sample) = list.read_with(vcx, |list, cx| match list.delegate.entry(0, cx) {
        PickerEntry::Item(item) => (
            item.detail.map(|detail| detail.to_string()),
            item.sample_font,
        ),
        PickerEntry::Header(_) => panic!("the first row is a heading"),
    });
    let detail = detail.expect("the row says what it is");
    assert!(detail.starts_with(MISSING_LABEL), "{detail}");
    assert!(
        detail.ends_with(innerword_app::spelling::IN_USE_LABEL),
        "{detail}"
    );
    assert_eq!(sample, None, "no face to show it in");
    assert_eq!(
        list.read_with(vcx, |list, _| list.delegate.missing().map(str::to_string)),
        Some("Lucida Fax".to_string())
    );
    // An installed family has a sample in its own face.
    let sample = list.read_with(vcx, |list, cx| match list.delegate.entry(2, cx) {
        PickerEntry::Item(item) => item.sample_font.map(|family| family.to_string()),
        PickerEntry::Header(_) => None,
    });
    assert_eq!(sample.as_deref(), Some("Charter"));
    // It is found by its name like the others.
    vcx.simulate_input("lucida");
    assert_eq!(listed(&list, vcx), ["Lucida Fax"]);
    // Choosing it again keeps it, and does not make it a recent font.
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(new_document_font(&workspace, vcx), "Lucida Fax");
    assert!(
        vcx.update(|_, cx| innerword_editor_tools::recent_fonts(cx))
            .is_empty()
    );

    // It stays set: a new document gets it, the open one keeps its own.
    vcx.simulate_keystrokes("escape");
    assert_eq!(document_font(&workspace, vcx), "Sabon Next");
    let before = workspace.read_with(vcx, |w, _| w.open_document_id());
    workspace
        .update_in(vcx, |workspace, window, cx| {
            workspace.new_document(window, cx)
        })
        .unwrap();
    assert_ne!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        before
    );
    assert_eq!(document_font(&workspace, vcx), "Lucida Fax");

    // Until the user chooses another: then it is gone from the list.
    vcx.simulate_keystrokes("cmd-,");
    let view = settings_view(&workspace, vcx).expect("settings are open");
    walk_to(SettingRow::Font, &view, vcx);
    vcx.simulate_keystrokes("right");
    assert_eq!(new_document_font(&workspace, vcx), "Avenir Next");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let list = font_list(&workspace, vcx).expect("the list of fonts is open");
    assert!(!listed(&list, vcx).contains(&"Lucida Fax".to_string()));
    assert_eq!(marked(&list, vcx).as_deref(), Some("Avenir Next"));
}

#[gpui::test]
fn the_state_of_a_scripted_run_shows_the_open_list_of_fonts(cx: &mut TestAppContext) {
    init(cx);
    install_fonts(cx);
    let library = library(cx);
    add_document(&library, None, &simple_document("Doc", "Text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.describe_font_list(cx)),
        None
    );
    vcx.simulate_keystrokes("cmd-,");
    let view = settings_view(&workspace, vcx).expect("settings are open");
    view.update(vcx, |view, cx| view.activate(SettingRow::Font, cx));
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.describe_font_list(cx)),
        Some(
            "target=new-documents query=\"\" rows=12 selected=\"Charter\" in_use=\"Charter\" \
             missing=false"
                .to_string()
        )
    );
    vcx.simulate_input("pal");
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.describe_font_list(cx)),
        Some(
            "target=new-documents query=\"pal\" rows=1 selected=\"Palatino\" in_use=\"Charter\" \
             missing=false"
                .to_string()
        )
    );
}
