//! Import and export from the keys; the import preview and its choices.

use gpui::{TestAppContext, px};
use writer_keymap::ImportMode;
use writer_keymap_ui::{Choice, ModeKind, Request};

use super::*;

fn mode(view: &Entity<KeymapView>, cx: &mut VisualTestContext) -> ModeKind {
    view.read_with(cx, |view, _| view.mode())
}

fn choices(view: &Entity<KeymapView>, cx: &mut VisualTestContext) -> Vec<(Choice, String)> {
    view.read_with(cx, |view, _| view.choices())
}

fn rows(view: &Entity<KeymapView>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |view, _| view.preview_rows())
}

/// The file of the fixture: Bold and Version history get keys, three
/// entries cannot be applied.
fn offer_edited_file(host: &Entity<Host>, cx: &mut VisualTestContext) {
    host.update(cx, |host, _| {
        host.import_file = Some(("Mine.json".into(), host::edited_file(host)));
    });
}

#[gpui::test]
fn export_and_import_are_asked_for_from_the_keys(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    press(cx, "cmd-e");
    assert_eq!(requests(&host, cx), vec![Request::Export]);
    let file = host.read_with(cx, |host, _| host.exported.clone().unwrap());
    assert!(file.contains("\"title\": \"Bold\""));
    assert_eq!(mode(&view, cx), ModeKind::List);

    press(cx, "cmd-i");
    assert_eq!(
        requests(&host, cx),
        vec![Request::Export, Request::ChooseImportFile]
    );
}

#[gpui::test]
fn the_preview_says_what_changes_what_is_skipped_and_why(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    offer_edited_file(&host, cx);
    press(cx, "cmd-i");
    assert_eq!(mode(&view, cx), ModeKind::Question);
    assert_eq!(
        view.read_with(cx, |view, _| view.question_title().unwrap()),
        "Import “Mine.json”"
    );
    let summary: Vec<String> = view.read_with(cx, |view, _| {
        view.question_lines()
            .into_iter()
            .map(|line| line.text)
            .collect()
    });
    assert_eq!(
        summary,
        vec![
            "2 commands get other keys.",
            "3 entries of the file are skipped."
        ]
    );
    let rows = rows(&view, cx);
    assert_eq!(rows[0], "Keys that change (2)");
    assert_eq!(rows[1], "Bold (in the text): ⌘B → ⌥⌘B");
    assert_eq!(rows[2], "Version history (anywhere): none → ⌥⌘H");
    assert_eq!(rows[3], "Entries that are skipped (3)");
    // Each with its reason: a key that cannot be read, a key a guard rail
    // refuses, a command this version does not have.
    let skipped = rows[4..].join("\n");
    assert!(
        skipped.contains("Italic") && skipped.contains("a key cannot be read"),
        "{skipped}"
    );
    assert!(
        skipped.contains("Underline") && skipped.contains("is a character"),
        "{skipped}"
    );
    assert!(
        skipped.contains("demo::Gone") && skipped.contains("does not have the command"),
        "{skipped}"
    );

    // Nothing of the user's own to weigh against the file: one way in.
    assert_eq!(
        choices(&view, cx),
        vec![
            (Choice::ImportMerge, "Import".to_string()),
            (Choice::Cancel, "Cancel".to_string())
        ]
    );
    // Nothing is applied by looking.
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-b"]);

    // The rows are looked through with the arrows; the selected one is
    // shown in full under the list.
    assert!(cx.debug_bounds("keymap-preview-detail").is_some());
    press(cx, "down down down");
    press(cx, "up");
    assert_eq!(mode(&view, cx), ModeKind::Question);

    press(cx, "enter");
    assert_eq!(mode(&view, cx), ModeKind::List);
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-alt-b"]);
    assert_eq!(keys_of(&host, &version_history(), cx), vec!["cmd-alt-h"]);
    // What was skipped is as it was.
    assert_eq!(keys_of(&host, &italic(), cx), vec!["cmd-i"]);
    match requests(&host, cx).last() {
        Some(Request::Import { mode, plan }) => {
            assert_eq!(*mode, ImportMode::Merge);
            assert_eq!(plan.skipped.len(), 3);
        }
        other => panic!("{other:?}"),
    }
    // The host can undo it: it was handed the changes as they were.
    assert!(host.read_with(cx, |host, _| host.undo.clone().unwrap().is_empty()));
    assert_eq!(view.read_with(cx, |view, _| view.changed_count()), 2);
}

#[gpui::test]
fn with_changes_of_ones_own_there_are_three_choices(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    // The user's own: Italic on another key, and Bold too (the file also
    // changes Bold).
    select(&view, &italic(), cx);
    press(cx, "enter cmd-alt-i enter");
    select(&view, &bold(), cx);
    press(cx, "enter f5 enter");
    offer_edited_file(&host, cx);

    press(cx, "cmd-i");
    assert_eq!(
        choices(&view, cx),
        vec![
            (Choice::ImportMerge, "Add to My Changes".to_string()),
            (Choice::ImportReplace, "Replace My Changes".to_string()),
            (Choice::Cancel, "Cancel".to_string())
        ]
    );
    // Marked first: adding. The file wins for Bold; Italic is not touched.
    assert_eq!(
        view.read_with(cx, |view, _| view.marked_choice()),
        Some(Choice::ImportMerge)
    );
    let adding = rows(&view, cx);
    assert_eq!(adding[0], "Keys that change (2)");
    assert!(
        adding.contains(&"Bold (in the text): F5 → ⌥⌘B".to_string()),
        "{adding:?}"
    );
    assert!(
        !adding.iter().any(|row| row.starts_with("Italic (")),
        "{adding:?}"
    );

    // Marking the other way shows what that one would do: Italic goes back.
    press(cx, "right");
    assert_eq!(
        view.read_with(cx, |view, _| view.marked_choice()),
        Some(Choice::ImportReplace)
    );
    let replacing = rows(&view, cx);
    assert_eq!(replacing[0], "Keys that change (3)");
    assert!(
        replacing.contains(&"Italic (in the text): ⌥⌘I → ⌘I".to_string()),
        "{replacing:?}"
    );

    // 1. Cancel: nothing changes.
    press(cx, "escape");
    assert_eq!(mode(&view, cx), ModeKind::List);
    assert_eq!(keys_of(&host, &bold(), cx), vec!["f5"]);
    assert_eq!(keys_of(&host, &italic(), cx), vec!["cmd-alt-i"]);

    // 2. Add to mine.
    press(cx, "cmd-i enter");
    assert_eq!(
        keys_of(&host, &bold(), cx),
        vec!["cmd-alt-b"],
        "the file wins"
    );
    assert_eq!(
        keys_of(&host, &italic(), cx),
        vec!["cmd-alt-i"],
        "my own change stays"
    );
    assert_eq!(keys_of(&host, &version_history(), cx), vec!["cmd-alt-h"]);
    // Undo, as the host's toast would: the changes as they were.
    host.update(cx, |host, cx| {
        let before = host.undo.take().unwrap();
        host.keymap.set_changes(before);
        host.install(cx);
    });
    assert_eq!(keys_of(&host, &bold(), cx), vec!["f5"]);

    // 3. Replace mine: only what the file changes is left.
    let listing = host.read_with(cx, |host, _| host.listing());
    view.update_in(cx, |view, window, cx| view.set_listing(listing, window, cx));
    press(cx, "cmd-i right enter");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-alt-b"]);
    assert_eq!(
        keys_of(&host, &italic(), cx),
        vec!["cmd-i"],
        "my own change is gone"
    );
    assert!(matches!(
        requests(&host, cx).last(),
        Some(Request::Import {
            mode: ImportMode::Replace,
            ..
        })
    ));
}

#[gpui::test]
fn the_three_choices_work_with_the_pointer(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &italic(), cx);
    press(cx, "enter cmd-alt-i enter");
    offer_edited_file(&host, cx);
    click(cx, "keymap-import");
    assert_eq!(choices(&view, cx).len(), 3);
    // The pointer over a way of importing shows what it would do.
    let replace = bounds(cx, "keymap-choice-1").center();
    cx.simulate_mouse_move(replace, None, gpui::Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |view, _| view.marked_choice()),
        Some(Choice::ImportReplace)
    );
    assert_eq!(rows(&view, cx)[0], "Keys that change (3)");
    // A click on a row of the preview selects it.
    click(cx, "keymap-preview-row-2");
    assert_eq!(mode(&view, cx), ModeKind::Question);
    // Cancel.
    click(cx, "keymap-choice-2");
    assert_eq!(mode(&view, cx), ModeKind::List);
    assert_eq!(keys_of(&host, &italic(), cx), vec!["cmd-alt-i"]);
    // Replace.
    click(cx, "keymap-import");
    click(cx, "keymap-choice-1");
    assert_eq!(keys_of(&host, &italic(), cx), vec!["cmd-i"]);
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-alt-b"]);
    // Add (nothing more to add now: the file's keys are the keys).
    click(cx, "keymap-import");
    assert_eq!(
        choices(&view, cx),
        vec![(Choice::Cancel, "Close".to_string())]
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.question_lines()[0].text.clone()),
        "Nothing would change: you already have the keys of the file."
    );
    click(cx, "keymap-choice-0");
    assert_eq!(mode(&view, cx), ModeKind::List);
}

#[gpui::test]
fn a_collision_an_import_would_leave_is_said(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    // A file that gives Bold the key Italic has.
    host.update(cx, |host, _| {
        let exported = host.keymap.export(&host.listing().catalogue, "0.4.0");
        let mut file = host::serde_json_lite::parse(&exported);
        file.set_keys("demo::Bold", &["cmd-i"]);
        host.import_file = Some(("Clash.json".into(), file.to_text()));
    });
    press(cx, "cmd-i");
    let summary: Vec<String> = view.read_with(cx, |view, _| {
        view.question_lines()
            .into_iter()
            .map(|line| line.text)
            .collect()
    });
    assert_eq!(summary[0], "1 command gets other keys.");
    assert_eq!(
        summary[1],
        "It would leave 1 collision: two commands on one key in the same place."
    );
    let rows = rows(&view, cx);
    assert!(
        rows.contains(&"Collisions it would leave (1)".to_string()),
        "{rows:?}"
    );
    assert!(
        rows.contains(&"⌘I would be the key of “Bold” and of “Italic” in the text.".to_string()),
        "{rows:?}"
    );
}

#[gpui::test]
fn a_file_that_is_not_a_shortcuts_file_says_what_is_wrong_and_where(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    host.update(cx, |host, _| {
        host.import_file = Some((
            "Notes.json".into(),
            "{\n  \"format\": \"the-writer-shortcuts\",\n  oops\n}".into(),
        ));
    });
    press(cx, "cmd-i");
    assert_eq!(mode(&view, cx), ModeKind::Question);
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.question_title().as_deref(),
            Some("“Notes.json” was not imported")
        );
        let text = &view.question_lines()[0].text;
        assert!(
            text.starts_with("This is not a shortcuts file: it is not JSON. Line 3, column"),
            "{text}"
        );
        assert_eq!(
            view.choices(),
            vec![
                (Choice::ChooseFile, "Choose Another File…".to_string()),
                (Choice::Cancel, "Close".to_string())
            ]
        );
    });
    // The list is still there behind the message.
    assert!(cx.debug_bounds("keymap-row-1").is_some());

    // Another file: some other JSON.
    host.update(cx, |host, _| {
        host.import_file = Some((
            "Other.json".into(),
            "{\"format\": \"something-else\"}".into(),
        ));
    });
    press(cx, "enter");
    let text = view.read_with(cx, |view, _| view.question_lines()[0].text.clone());
    assert!(
        text.contains("its \"format\" is \"something-else\""),
        "{text}"
    );
    press(cx, "escape");
    assert_eq!(mode(&view, cx), ModeKind::List);
    assert!(host.read_with(cx, |host, cx| host.is_open(cx)));
}

#[gpui::test]
fn a_file_that_changes_very_many_keys_stays_readable(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    // Every command of the fixture gets another key: far more rows than
    // fit.
    let changed = host.update(cx, |host, _| {
        let exported = host.keymap.export(&host.listing().catalogue, "0.4.0");
        let mut file = host::serde_json_lite::parse(&exported);
        let targets: Vec<String> = host
            .keymap
            .defaults()
            .targets()
            .iter()
            .map(|target| target.command.action.clone())
            .collect();
        let mut seen = Vec::new();
        for (ix, action) in targets.iter().enumerate() {
            // The first entry of an action is the one the helper edits.
            if seen.contains(action) {
                continue;
            }
            seen.push(action.clone());
            let key = format!("ctrl-alt-cmd-f{}", ix + 1);
            file.set_keys(action, &[key.as_str()]);
        }
        host.import_file = Some(("Everything.json".into(), file.to_text()));
        seen.len()
    });
    assert!(changed > 60, "{changed}");
    press(cx, "cmd-i");
    let rows = rows(&view, cx);
    assert_eq!(rows[0], format!("Keys that change ({changed})"));
    assert_eq!(rows.len(), changed + 1);
    // The panel stays inside the window, and the choices stay in it.
    let window = cx.update(|window, _| window.viewport_size());
    let panel = bounds(cx, "keymap-panel");
    assert!(panel.size.height < window.height, "{:?}", panel.size.height);
    let choice = bounds(cx, "keymap-choice-0");
    assert!(choice.bottom() < panel.bottom());
    // Only some rows are drawn; each has the one height.
    let first = bounds(cx, "keymap-preview-row-1");
    assert_eq!(first.size.height, px(28.));
    assert!(
        cx.debug_bounds(selector(format!("keymap-preview-row-{changed}")))
            .is_none()
    );
    // The keys reach the last row.
    press(cx, "end");
    assert!(
        cx.debug_bounds(selector(format!("keymap-preview-row-{changed}")))
            .is_some()
    );
    press(cx, "home pagedown pageup");
    press(cx, "enter");
    assert_eq!(view.read_with(cx, |view, _| view.changed_count()), changed);
}
