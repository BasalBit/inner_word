//! The list: its headings, the search, the changed-only switch, moving in
//! it, and how tall its rows are.

use gpui::{TestAppContext, px};
use writer_keymap::Target;
use writer_keymap_ui::{ModeKind, Request};

use super::*;

#[gpui::test]
fn the_list_is_grouped_by_place_under_its_headings(cx: &mut TestAppContext) {
    let (_, view, cx) = open(cx);
    let headings = view.read_with(cx, |view, _| view.headings());
    assert_eq!(headings[0], host::NOTE, "the host's note stands first");
    assert_eq!(headings[1], "Anywhere \u{b7} View");
    assert!(headings.contains(&"In the text \u{b7} Format".to_string()));
    assert!(headings.contains(&"In keyboard shortcuts".to_string()));
    assert!(headings.contains(&"In a text field".to_string()));
    // Places in the host's order, each heading once.
    let anywhere = headings
        .iter()
        .position(|h| h.starts_with("Anywhere"))
        .unwrap();
    let text = headings
        .iter()
        .position(|h| h.starts_with("In the text"))
        .unwrap();
    let own = headings
        .iter()
        .position(|h| h == "In keyboard shortcuts")
        .unwrap();
    assert!(anywhere < text && text < own);
    let mut unique = headings.clone();
    unique.dedup();
    assert_eq!(unique, headings);

    view.read_with(cx, |view, cx| {
        assert_eq!(view.mode(), ModeKind::List);
        assert_eq!(view.changed_count(), 0);
        // The first command is selected, not the note or a heading.
        assert_eq!(
            view.selected_target(),
            Some(&Target::new("demo::OpenShortcuts", None))
        );
        // The view lists its own keys like every other key.
        let titles: Vec<String> = view.listed().into_iter().map(|(_, title)| title).collect();
        for own in [
            "Change key",
            "Add key",
            "Remove key",
            "Use default",
            "Import",
            "Export",
            "Reset all",
            "Close",
        ] {
            assert!(
                titles.contains(&own.to_string()),
                "{own} is listed: {titles:?}"
            );
        }
        assert!(view.describe(cx).starts_with("mode=list rows="));
    });
}

#[gpui::test]
fn typing_searches_by_title_by_place_and_by_key(cx: &mut TestAppContext) {
    let (_, view, cx) = open(cx);
    let titles = |cx: &mut VisualTestContext| -> Vec<String> {
        view.read_with(cx, |view, _| {
            view.listed().into_iter().map(|(_, title)| title).collect()
        })
    };
    let search = |query: &str, cx: &mut VisualTestContext| {
        view.update(cx, |view, cx| view.set_query(query, cx));
        cx.run_until_parked();
    };

    // Typed, as a person does: the characters go into the query field.
    cx.simulate_input("bold");
    assert_eq!(view.read_with(cx, |view, cx| view.query(cx)), "bold");
    assert_eq!(titles(cx), vec!["Bold"]);
    // A search shows no headings; the row says where it belongs.
    assert!(view.read_with(cx, |view, _| view.headings()).is_empty());

    search("find bar", cx);
    assert_eq!(titles(cx), vec!["Case sensitive"]);
    search("format", cx);
    assert_eq!(
        titles(cx),
        vec!["Bold", "Italic", "Underline", "Numbered list"]
    );

    // By key: as text, with the glyphs, in any order of the modifiers.
    for query in ["cmd-i", "⌘I", "command+i"] {
        search(query, cx);
        let found = titles(cx);
        assert_eq!(&found[..2], ["Italic", "Import"], "{query}: {found:?}");
    }
    search("cmd-shift-7", cx);
    assert_eq!(titles(cx), vec!["Numbered list"]);
    search("⇧⌘7", cx);
    assert_eq!(titles(cx), vec!["Numbered list"]);
    search("⌥⌘C", cx);
    assert_eq!(titles(cx), vec!["Show or hide comments", "Case sensitive"]);
    // The keys of the context menu: both are found.
    search("shift-f10", cx);
    assert_eq!(titles(cx)[0], "Context menu");
    search("ctrl-enter", cx);
    assert_eq!(titles(cx)[0], "Context menu");

    search("no such thing anywhere", cx);
    assert!(titles(cx).is_empty());
    assert_eq!(
        view.read_with(cx, |view, _| view.selected_target().cloned()),
        None
    );

    // Emptying the query brings the grouped list back.
    search("", cx);
    assert!(!view.read_with(cx, |view, _| view.headings()).is_empty());
}

#[gpui::test]
fn a_changed_key_is_found_by_the_key_it_had(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &bold(), cx);
    press(cx, "enter cmd-alt-b enter");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-alt-b"]);
    view.update(cx, |view, cx| view.set_query("cmd-alt-b", cx));
    assert_eq!(view.read_with(cx, |view, _| view.listed())[0].1, "Bold");
    // What it was by default still finds it.
    view.update(cx, |view, cx| view.set_query("cmd-b", cx));
    let listed = view.read_with(cx, |view, _| view.listed());
    assert!(
        listed.iter().any(|(target, _)| *target == bold()),
        "{listed:?}"
    );
}

#[gpui::test]
fn the_changed_only_switch_shows_what_was_changed_and_counts_it(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    let all = view.read_with(cx, |view, _| view.listed().len());

    // Nothing changed yet: the switch empties the list and says why.
    press(cx, "cmd-shift-c");
    view.read_with(cx, |view, _| {
        assert!(view.changed_only());
        assert!(view.listed().is_empty());
    });
    press(cx, "cmd-shift-c");
    assert_eq!(view.read_with(cx, |view, _| view.listed().len()), all);

    select(&view, &bold(), cx);
    press(cx, "enter cmd-alt-b enter");
    select(&view, &italic(), cx);
    press(cx, "cmd-backspace");
    assert_eq!(view.read_with(cx, |view, _| view.changed_count()), 2);
    assert!(describe(&view, cx).contains("changed=2"));

    // By the pointer this time.
    click(cx, "keymap-changed-only");
    view.read_with(cx, |view, _| {
        assert!(view.changed_only());
        let titles: Vec<String> = view.listed().into_iter().map(|(_, title)| title).collect();
        assert_eq!(titles, vec!["Bold", "Italic"]);
        // Still under their heading, without the host's note.
        assert_eq!(view.headings(), vec!["In the text \u{b7} Format"]);
    });
    // A row that is put back leaves the changed list.
    select(&view, &italic(), cx);
    press(cx, "cmd-d");
    assert_eq!(keys_of(&host, &italic(), cx), vec!["cmd-i"]);
    view.read_with(cx, |view, _| {
        assert_eq!(view.listed().len(), 1);
        assert_eq!(view.changed_count(), 1);
    });
    click(cx, "keymap-changed-only");
    assert_eq!(view.read_with(cx, |view, _| view.listed().len()), all);
    assert!(matches!(
        requests(&host, cx).last(),
        Some(Request::UseDefault { .. })
    ));
}

#[gpui::test]
fn the_keys_move_over_the_headings(cx: &mut TestAppContext) {
    let (_, view, cx) = open(cx);
    let selected = |cx: &mut VisualTestContext| {
        view.read_with(cx, |view, _| {
            let target = view.selected_target().cloned().unwrap();
            view.listed()
                .into_iter()
                .position(|(listed, _)| listed == target)
                .unwrap()
        })
    };
    let count = view.read_with(cx, |view, _| view.listed().len());
    assert_eq!(selected(cx), 0);
    press(cx, "down");
    assert_eq!(selected(cx), 1);
    // The next command is under another heading: the heading is skipped.
    press(cx, "down");
    assert_eq!(selected(cx), 2);
    press(cx, "up up up");
    assert_eq!(selected(cx), 0, "up at the first row stays there");
    press(cx, "ctrl-n ctrl-n ctrl-p");
    assert_eq!(selected(cx), 1);
    press(cx, "end");
    assert_eq!(selected(cx), count - 1);
    press(cx, "down");
    assert_eq!(selected(cx), count - 1);
    press(cx, "home");
    assert_eq!(selected(cx), 0);
    press(cx, "pagedown");
    let page = selected(cx);
    assert!(page > 3, "a page is more than a few rows: {page}");
    press(cx, "pagedown pageup pageup");
    assert_eq!(selected(cx), 0);
    press(cx, "cmd-down");
    assert_eq!(selected(cx), count - 1);
    press(cx, "cmd-up");
    assert_eq!(selected(cx), 0);
    // The selected row is drawn (the list scrolled to it).
    press(cx, "end");
    let last = view.read_with(cx, |view, _| view.selected_target().cloned().unwrap());
    let row = row_selector(&view, &last, cx);
    assert!(cx.debug_bounds(row).is_some());
}

#[gpui::test]
fn no_row_is_taller_than_its_text(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    // A changed row, a row with two keys, a row without a key, the locked
    // row: none may grow.
    select(&view, &bold(), cx);
    press(cx, "enter cmd-alt-b enter");
    press(cx, "home");
    let panel = bounds(cx, "keymap-panel");
    let mut seen = 0;
    for row in 0..40 {
        let Some(row) = cx.debug_bounds(selector(format!("keymap-row-{row}"))) else {
            continue;
        };
        seen += 1;
        assert_eq!(row.size.height, px(32.), "every row has the one height");
        assert!(row.size.width <= panel.size.width);
    }
    assert!(seen >= 8, "rows are drawn: {seen}");
    // The panel fits the window with room to spare, whatever is in it.
    let window = cx.update(|window, _| window.viewport_size());
    assert!(panel.size.height < window.height);
    assert_eq!(panel.size.width, px(680.));

    // The sentences of a question wrap inside the band: it stays a few
    // lines tall and as wide as the panel.
    select(&view, &bold(), cx);
    press(cx, "enter cmd-i");
    assert_eq!(
        view.read_with(cx, |view, _| view.mode()),
        ModeKind::Question
    );
    let band = bounds(cx, "keymap-band");
    assert!(
        band.size.height < px(220.),
        "the band is {:?} tall",
        band.size.height
    );
    assert!(band.size.height > px(60.));
    assert_eq!(
        band.size.width,
        panel.size.width - px(2.),
        "inside the panel's border"
    );
    let panel = bounds(cx, "keymap-panel");
    assert!(panel.size.height < window.height);
    drop(host);
}
