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
    assert_eq!(rows[0], "Commands that get other keys (2)");
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
    assert_eq!(adding[0], "Commands that get other keys (2)");
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
    assert_eq!(replacing[0], "Commands that get other keys (3)");
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
    assert_eq!(rows(&view, cx)[0], "Commands that get other keys (3)");
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

    // Imported all the same: both rows say what is wrong with them, by a
    // sign beside the key and in words while the row is selected.
    press(cx, "enter");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-i"]);
    assert_eq!(keys_of(&host, &italic(), cx), vec!["cmd-i"]);
    select(&view, &bold(), cx);
    let line = describe(&view, cx);
    assert!(
        line.contains("clash=\"⌘I is also the key of “Italic” in the text: one of the two cannot run with it.\""),
        "{line}"
    );
    let bold_row = view.read_with(cx, |view, _| view.row_of(&bold()).unwrap());
    assert!(
        cx.debug_bounds(selector(format!("keymap-clash-{bold_row}")))
            .is_some()
    );
    select(&view, &italic(), cx);
    assert!(describe(&view, cx).contains("clash=\"⌘I is also the key of “Bold”"));
    // Settled by giving one of them another key.
    press(cx, "enter cmd-alt-i enter");
    assert!(!describe(&view, cx).contains("clash="));
    select(&view, &bold(), cx);
    assert!(!describe(&view, cx).contains("clash="));
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
    assert_eq!(rows[0], format!("Commands that get other keys ({changed})"));
    assert_eq!(rows.len(), changed + 1);
    // The panel stays inside the window, and the choices stay in it.
    let window = cx.update(|window, _| window.viewport_size());
    let panel = bounds(cx, "keymap-panel");
    assert!(panel.size.height < window.height, "{:?}", panel.size.height);
    let choice = bounds(cx, "keymap-choice-0");
    assert!(choice.bottom() < panel.bottom());
    // A row of one line has the one height; the rows scroll inside the
    // panel, and the last one is out of sight.
    let first = bounds(cx, "keymap-preview-row-1");
    assert_eq!(first.size.height, px(28.));
    let last = selector(format!("keymap-preview-row-{changed}"));
    assert!(in_view(cx, "keymap-preview-row-1"));
    assert!(!in_view(cx, last));
    // The keys reach the last row.
    press(cx, "end");
    assert!(in_view(cx, last));
    assert!(!in_view(cx, "keymap-preview-row-1"));
    press(cx, "home");
    assert!(in_view(cx, "keymap-preview-row-1"));
    press(cx, "pagedown pageup");
    press(cx, "enter");
    assert_eq!(view.read_with(cx, |view, _| view.changed_count()), changed);
}

/// Whether what is drawn under `selector` lies inside the part of the
/// preview that shows its rows (and not above or below it, scrolled away).
fn in_view(cx: &mut VisualTestContext, selector: &'static str) -> bool {
    let rows = bounds(cx, "keymap-preview-rows");
    let row = bounds(cx, selector);
    row.top() >= rows.top() && row.bottom() <= rows.bottom()
}

/// A skipped entry is read in its row, without selecting it: what the
/// command is called first, then the reason, which wraps instead of being
/// cut off at the edge. The name the file has for the action is not in the
/// row (it is under the rows, for whoever edits the file), except for a
/// command this version does not have: there it is all there is to go by.
/// (The picture of 0.5.0: "Entry 51 (Inline code, editor::ToggleInlineCode):
/// a key cannot be read: "hyper-e" has more than one key (…".)
#[gpui::test]
fn a_skipped_entry_is_read_in_its_row(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    host.update(cx, |host, _| {
        let exported = host.keymap.export(&host.listing().catalogue, "0.5.0");
        let mut file = host::serde_json_lite::parse(&exported);
        file.set_keys("demo::Bold", &["cmd-alt-b"]);
        file.set_keys("demo::Italic", &["hyper-i"]);
        file.set_keys("demo::Underline", &["u"]);
        file.push_entry("demo::SmallCaps", "Small caps", &["cmd-alt-k"]);
        host.import_file = Some(("Mine.json".into(), file.to_text()));
    });
    press(cx, "cmd-i");
    let rows = rows(&view, cx);
    assert_eq!(rows[0], "Commands that get other keys (1)");
    assert_eq!(rows[2], "Entries that are skipped (3)");
    // The command's name first, then the reason in the keymap's words;
    // keys as the list writes them.
    assert!(
        rows[3].starts_with("Italic: a key cannot be read: \"hyper-i\""),
        "{}",
        rows[3]
    );
    assert_eq!(
        rows[4],
        "Underline: U is a character, and text is typed where the command applies"
    );
    assert_eq!(
        rows[5],
        "Small caps (demo::SmallCaps): this version does not have the command"
    );
    // No number of an entry and no name of an action in the rows of the
    // commands this version has.
    for row in &rows[3..5] {
        assert!(!row.contains("demo::") && !row.contains("Entry"), "{row}");
    }

    // How the file names it stands under the rows while the row is
    // selected, for a change and for a skipped entry alike.
    assert_eq!(
        view.read_with(cx, |view, _| view.preview_detail())
            .as_deref(),
        Some("In the file: \"demo::Bold\" in \"Editor\"")
    );
    press(cx, "down");
    let detail = view.read_with(cx, |view, _| view.preview_detail().unwrap());
    assert!(
        detail.starts_with("In the file: entry ") && detail.ends_with(", \"demo::Italic\""),
        "{detail}"
    );
    assert!(cx.debug_bounds("keymap-preview-detail").is_some());

    // On screen: the name at the left, the reason beside it. A long
    // reason takes more than one line and ends inside the row; a short one
    // leaves the row at its one-line height.
    let panel = bounds(cx, "keymap-panel");
    let long = bounds(cx, "keymap-preview-row-3");
    let name = bounds(cx, "keymap-preview-name-3");
    let reason = bounds(cx, "keymap-preview-reason-3");
    assert!(name.left() >= long.left() && name.right() <= reason.left());
    assert!(reason.right() <= long.right() && long.right() <= panel.right());
    assert!(
        reason.size.height >= px(36.),
        "the reason wraps: {:?}",
        reason.size
    );
    assert!(reason.top() >= long.top() && reason.bottom() <= long.bottom());
    assert!(long.size.height > px(28.), "{:?}", long.size.height);
    let unknown = bounds(cx, "keymap-preview-row-5");
    assert_eq!(unknown.size.height, px(28.));
    assert_eq!(bounds(cx, "keymap-preview-reason-5").size.height, px(18.));
    // Rows one under the other, none over another.
    let short = bounds(cx, "keymap-preview-row-4");
    assert_eq!(short.top(), long.bottom());
    assert_eq!(unknown.top(), short.bottom());
}

/// A file with many rows of each kind: 20 commands that change, 10
/// collisions, 15 skipped entries. The rows scroll, the choices stay where
/// they are and in view, nothing reaches past the panel; also in the
/// smallest window the app has.
#[gpui::test]
fn a_preview_with_many_rows_of_each_kind_scrolls_and_keeps_its_choices(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    host.update(cx, |host, _| {
        let exported = host.keymap.export(&host.listing().catalogue, "0.5.0");
        let mut file = host::serde_json_lite::parse(&exported);
        // 9 commands on keys of their own.
        for (ix, command) in [
            "demo::Bold",
            "demo::NumberedList",
            "demo::NewDocument",
            "demo::QuickOpen",
            "demo::Quit",
            "demo::ToggleComments",
            "demo::OpenContextMenu",
            "demo::CaseSensitive",
            "demo::VersionHistory",
        ]
        .into_iter()
        .enumerate()
        {
            file.set_keys(command, &[format!("ctrl-alt-cmd-f{}", ix + 1).as_str()]);
        }
        // 11 more that share keys in one place: three keys with three
        // commands each (three pairs a key), one with two.
        for (key, commands) in [
            (
                "ctrl-alt-cmd-f10",
                &["SelectPageUp", "SelectPageDown", "SelectFirst"][..],
            ),
            ("ctrl-alt-cmd-f11", &["SelectLast", "ChangeKey", "AddKey"]),
            (
                "ctrl-alt-cmd-f12",
                &["RemoveKey", "UseDefault", "RunCommand"],
            ),
            ("ctrl-alt-cmd-f13", &["NextKey", "PreviousKey"]),
        ] {
            for command in commands {
                file.set_keys(&format!("keymap_view::{command}"), &[key]);
            }
        }
        // 15 entries that cannot be applied, for every kind of reason.
        file.set_keys("demo::Italic", &["hyper-i"]);
        file.set_keys("demo::Underline", &["u"]);
        file.set_keys("demo::Find", &["cmd-"]);
        file.set_keys("demo::MoveBlockUp", &["cmd-tab"]);
        for ix in 1..=11 {
            file.push_entry(
                &format!("demo::Gone{ix}"),
                &format!("A command of another version, number {ix}"),
                &["cmd-g"],
            );
        }
        host.import_file = Some(("Everything.json".into(), file.to_text()));
    });
    press(cx, "cmd-i");
    let rows = rows(&view, cx);
    let headings: Vec<&String> = rows
        .iter()
        .filter(|row| {
            row.starts_with("Commands that")
                || row.starts_with("Collisions")
                || row.starts_with("Entries")
        })
        .collect();
    assert_eq!(
        headings,
        [
            "Commands that get other keys (20)",
            "Collisions it would leave (10)",
            "Entries that are skipped (15)"
        ]
    );
    assert_eq!(rows.len(), 3 + 20 + 10 + 15);
    let last = rows.len() - 1;
    let first_skipped = 2 + 20 + 10 + 1;
    assert!(rows[first_skipped].starts_with("Italic: "), "{rows:?}");

    let check = |cx: &mut VisualTestContext| {
        let window = cx.update(|window, _| window.viewport_size());
        let panel = bounds(cx, "keymap-panel");
        assert!(panel.left() >= px(0.) && panel.right() <= window.width);
        assert!(
            panel.top() >= px(0.) && panel.bottom() <= window.height,
            "the panel ends at {:?} in a window of {:?}",
            panel.bottom(),
            window.height
        );
        // The part with the rows is inside the panel and scrolls.
        let list = bounds(cx, "keymap-preview-rows");
        assert!(list.top() > panel.top() && list.bottom() < panel.bottom());
        assert!(list.left() >= panel.left() && list.right() <= panel.right());
        // The choices are under it, whole, inside the panel.
        let choices: Vec<_> = (0..2)
            .map(|ix| bounds(cx, selector(format!("keymap-choice-{ix}"))))
            .collect();
        for choice in &choices {
            assert!(choice.top() >= list.bottom(), "{choice:?} under {list:?}");
            assert!(choice.bottom() <= panel.bottom() && choice.right() <= panel.right());
            assert!(choice.left() >= panel.left());
        }
        // From the top: the first rows are in view, the last is not.
        press(cx, "home");
        assert!(in_view(cx, "keymap-preview-row-1"));
        assert!(!in_view(cx, selector(format!("keymap-preview-row-{last}"))));
        // No row is wider than the part that shows it, and a skipped
        // entry's reason ends inside its row: nothing is cut off at the
        // right edge.
        for ix in 0..=last {
            let row = bounds(cx, selector(format!("keymap-preview-row-{ix}")));
            assert!(
                row.left() >= list.left() && row.right() <= list.right(),
                "row {ix}: {row:?} in {list:?}"
            );
            if ix >= first_skipped {
                let reason = bounds(cx, selector(format!("keymap-preview-reason-{ix}")));
                let name = bounds(cx, selector(format!("keymap-preview-name-{ix}")));
                assert!(reason.right() <= row.right(), "row {ix}: {reason:?}");
                assert!(name.right() <= reason.left(), "row {ix}: {name:?}");
                assert!(reason.size.width >= px(200.), "row {ix}: {reason:?}");
                assert!(
                    reason.top() >= row.top() && reason.bottom() <= row.bottom(),
                    "row {ix}: {reason:?} in {row:?}"
                );
            }
        }
        // Row by row to the end: the selected row is in view each time
        // (the first of the collisions, the first skipped entry, the
        // last), and the choices have not moved.
        for ix in 1..=last {
            let selected_is_shown = in_view(cx, selector(format!("keymap-preview-row-{ix}")));
            let heading = ix == 21 || ix == 32;
            if !heading {
                assert!(selected_is_shown, "row {ix} is selected and in view");
                press(cx, "down");
            }
        }
        assert!(in_view(cx, selector(format!("keymap-preview-row-{last}"))));
        assert!(!in_view(cx, "keymap-preview-row-1"));
        for (ix, before) in choices.iter().enumerate() {
            let now = bounds(cx, selector(format!("keymap-choice-{ix}")));
            assert_eq!(now, *before, "choice {ix} stayed where it was");
        }
        // And back up by pages.
        press(cx, "pageup pageup pageup pageup pageup pageup pageup");
        assert!(in_view(cx, "keymap-preview-row-1"));
    };
    check(cx);
    // The smallest window the app has.
    cx.simulate_resize(gpui::size(px(640.), px(480.)));
    cx.run_until_parked();
    check(cx);
    // Nothing was applied by looking.
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-b"]);
    press(cx, "escape");
    assert_eq!(mode(&view, cx), ModeKind::List);
}
