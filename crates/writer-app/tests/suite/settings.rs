//! Settings: the modal, persistence, live application, view toggles.

use gpui::{AppContext as _, TestAppContext};
use writer_app::{
    Settings,
    settings::{Paper, keys},
    settings_view::{SettingRow, SettingsView},
};
use writer_export::SocialNetwork;
use writer_library::Library;
use writer_ui::{ThemeMode, theme};

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
    // With a document open the list goes on with that document's own
    // settings; up goes back.
    vcx.simulate_keystrokes("down");
    assert_eq!(focused_row(&view, vcx), Some(SettingRow::DocumentPageView));
    vcx.simulate_keystrokes("up up");
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
        writer_core::PaperSize::UsLetter
    );
    // The status bar counts against the limit.
    type_text(&workspace, "Hello", vcx);
    let status = workspace.update(vcx, |w, cx| w.status_line(cx));
    assert_eq!(status.limit.as_deref(), Some("X 5 / 280"));
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
            move |_, event: &writer_app::WorkspaceEvent, _| {
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
        .filter(|event| **event == writer_app::WorkspaceEvent::SettingsChanged)
        .count();
    assert_eq!(changes, 4, "{events:?}");
}

#[gpui::test]
fn this_document_section_changes_the_open_document(cx: &mut TestAppContext) {
    use writer_core::{Orientation, PaperSize};

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
    let fonts = vcx.update(|_, cx| writer_app::settings::available_fonts(&font_before, cx));
    if fonts.len() > 1 {
        assert_ne!(document(vcx).typography.body_font, font_before);
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
    // The list ends here.
    vcx.simulate_keystrokes("down");
    assert_eq!(
        focused_row(&view, vcx),
        Some(SettingRow::DocumentParagraphSpacing)
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
        .advance_clock(writer_app::workspace::AUTOSAVE_DELAY * 2);
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
    use writer_app::press_and_hold;

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
    assert!(vim_text(vcx).contains("Restart The Writer for held keys to repeat"));

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
    use writer_app::press_and_hold;

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
