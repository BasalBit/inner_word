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
    // The list ends here; up goes back.
    vcx.simulate_keystrokes("down down up");
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
