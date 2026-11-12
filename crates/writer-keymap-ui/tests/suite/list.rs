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

#[gpui::test]
fn the_view_fits_the_smallest_window(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    // The smallest size the app's window can have.
    cx.simulate_resize(gpui::size(px(640.), px(480.)));
    cx.run_until_parked();
    let window = cx.update(|window, _| window.viewport_size());
    assert_eq!(window.height, px(480.));
    let panel = bounds(cx, "keymap-panel");
    assert!(panel.size.width <= window.width, "{:?}", panel.size.width);
    assert!(
        panel.bottom() <= window.height,
        "the panel ends at {:?}",
        panel.bottom()
    );
    // Some rows are still shown, each at its height.
    let rows = (0..12)
        .filter(|row| {
            cx.debug_bounds(selector(format!("keymap-row-{row}")))
                .is_some()
        })
        .count();
    assert!(rows >= 4, "{rows}");
    // The controls are all inside.
    for control in [
        "keymap-import",
        "keymap-export",
        "keymap-reset-all",
        "keymap-close",
    ] {
        let at = bounds(cx, control);
        assert!(
            at.right() <= panel.right() && at.left() >= panel.left(),
            "{control}: {at:?}"
        );
    }
    // With a question open the panel still ends inside the window.
    select(&view, &bold(), cx);
    press(cx, "enter cmd-i");
    let panel = bounds(cx, "keymap-panel");
    assert!(
        panel.bottom() <= window.height,
        "with the band: {:?}",
        panel.bottom()
    );
    let choice = bounds(cx, "keymap-choice-0");
    assert!(choice.bottom() <= panel.bottom());
    press(cx, "escape");
    // And the import preview.
    host.update(cx, |host, _| {
        host.import_file = Some(("Mine.json".into(), host::edited_file(host)));
    });
    press(cx, "cmd-i");
    let panel = bounds(cx, "keymap-panel");
    assert!(
        panel.bottom() <= window.height,
        "the preview: {:?}",
        panel.bottom()
    );
    assert!(bounds(cx, "keymap-choice-0").bottom() <= panel.bottom());
}

#[gpui::test]
fn changes_kept_from_another_version_are_said_and_can_be_removed(cx: &mut TestAppContext) {
    use writer_keymap::{Changes, DefaultBinding, Defaults, Keymap, Places};
    let (host, view, cx) = open(cx);
    // A change stored by a version that had a command this one has not.
    let stored = {
        let defaults =
            Defaults::from_bindings([
                DefaultBinding::new("old::Gone", None, None, "cmd-g").unwrap()
            ]);
        let mut other = Keymap::new(defaults, Places::new());
        other.replace_key(
            &Target::new("old::Gone", None),
            &key("cmd-g"),
            key("cmd-alt-g"),
        );
        other.stored().unwrap()
    };
    let listing = host.update(cx, |host, _| {
        host.keymap
            .set_changes(Changes::from_stored(&stored).unwrap());
        host.listing()
    });
    view.update_in(cx, |view, window, cx| view.set_listing(listing, window, cx));
    view.read_with(cx, |view, _| {
        let headings = view.headings();
        assert_eq!(headings[0], host::NOTE);
        assert_eq!(
            headings[1],
            "1 change made in another version of The Writer is kept; it does nothing in this one"
        );
        assert_eq!(view.changed_count(), 0);
        // It is something Reset All removes.
        assert!(view.can_reset_all());
    });
    press(cx, "cmd-shift-d");
    assert!(
        view.read_with(cx, |view, _| view.question_lines()[0].text.clone())
            .starts_with("No key differs from its default in this version.")
    );
    press(cx, "enter");
    assert!(host.read_with(cx, |host, _| host.keymap.changes().is_empty()));
    view.read_with(cx, |view, _| {
        assert!(!view.can_reset_all());
        assert_eq!(view.headings()[1], "Anywhere \u{b7} View");
    });
}

/// The line under the query counts commands, not keys: a command with two
/// keys of the user's is one, a command left without a key is one. (The
/// released list said "4 keys changed" for four commands with five keys
/// between them.)
#[gpui::test]
fn the_header_counts_the_commands_that_were_changed(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    let header = |cx: &mut VisualTestContext| view.read_with(cx, |view, _| view.changed_text());
    assert_eq!(header(cx), "No key changed");
    assert!(cx.debug_bounds("keymap-changed").is_some());

    select(&view, &bold(), cx);
    press(cx, "enter cmd-alt-b enter");
    assert_eq!(header(cx), "1 command changed");
    // A second key for the same command: still one command.
    press(cx, "cmd-enter f5 enter");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-alt-b", "f5"]);
    assert_eq!(header(cx), "1 command changed");
    // A command left without a key is one more.
    select(&view, &italic(), cx);
    press(cx, "cmd-backspace");
    assert_eq!(header(cx), "2 commands changed");
    // A command that had no key and got one.
    select(&view, &version_history(), cx);
    press(cx, "enter f6 enter");
    assert_eq!(header(cx), "3 commands changed");
    // The state line of a scripted run keeps its word.
    assert!(describe(&view, cx).contains(" changed=3 "));

    // A changed command the list has no row for right now (the commands
    // of a part of the app that is not there) is said beside the number,
    // as "Reset All" counts it too.
    let mut listing = host.read_with(cx, |host, _| host.listing());
    listing.rows.retain(|row| row.target != italic());
    view.update_in(cx, |view, window, cx| view.set_listing(listing, window, cx));
    cx.run_until_parked();
    assert_eq!(header(cx), "2 commands changed, 1 more not shown now");
    press(cx, "cmd-shift-d");
    assert!(
        view.read_with(cx, |view, _| view.question_lines()[0].text.clone())
            .starts_with("3 commands go back to the default keys."),
    );
}

/// A name is found by the words of the query standing in it, letters
/// together. (The released list found "Insert menu" for "italic", by an i
/// here and a t there, and marked those letters.)
#[gpui::test]
fn a_search_by_name_finds_words_not_scattered_letters(cx: &mut TestAppContext) {
    let (_, view, cx) = open(cx);
    let search = |query: &str, cx: &mut VisualTestContext| -> Vec<(String, String)> {
        view.update(cx, |view, cx| view.set_query(query, cx));
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            view.listed()
                .into_iter()
                .map(|(_, title)| title)
                .zip(view.marked_letters())
                .collect()
        })
    };
    let pair = |title: &str, marked: &str| (title.to_string(), marked.to_string());

    assert_eq!(search("italic", cx), vec![pair("Italic", "Italic")]);
    assert_eq!(search("ital", cx), vec![pair("Italic", "Ital")]);
    // "Insert menu" is there for its own words, and only those are marked.
    assert_eq!(
        search("insert menu", cx),
        vec![pair("Insert menu", "Insertmenu")]
    );
    // Titles first; after them the rows of a place with the word in its
    // name ("In a list or menu"), with nothing of their titles marked.
    let found = search("menu", cx);
    assert_eq!(
        found[..2],
        [pair("Context menu", "menu"), pair("Insert menu", "menu")]
    );
    for (title, marked) in &found {
        let in_title = title.to_lowercase().contains("menu");
        assert_eq!(marked, if in_title { "menu" } else { "" }, "{title}");
    }
    let first_by_place = found.iter().position(|(_, marked)| marked.is_empty());
    let last_by_title = found.iter().rposition(|(_, marked)| !marked.is_empty());
    assert!(first_by_place > last_by_title, "{found:?}");
    // The start of a word before the inside of one; and "or" inside
    // "Format" or "shortcuts" is no reason to list a whole place.
    let found = search("or", cx);
    assert_eq!(found[0], pair("Show or hide comments", "or"));
    let inside: Vec<&(String, String)> = found[1..]
        .iter()
        .take_while(|(_, marked)| marked == "or")
        .collect();
    assert!(inside.len() >= 4, "{found:?}");
    for (title, _) in &inside {
        assert!(title.to_lowercase().contains("or"), "{title}");
    }
    for (title, marked) in &found[1 + inside.len()..] {
        // What is left is found by a word of its place ("In a list or
        // menu").
        assert_eq!(marked, "", "{title}");
    }
    assert!(
        !found.iter().any(|(title, _)| title == "Bold"),
        "Bold is under Format, which only has \"or\" inside: {found:?}"
    );
    assert!(
        !found.iter().any(|(title, _)| title == "Change key"),
        "nor do the rows of keyboard shortcuts: {found:?}"
    );
    // Letters that are all there, in order, but not together: nothing.
    assert!(search("itlc", cx).is_empty());
    assert!(search("nmbl", cx).is_empty());
    // Found by its place: nothing of the title is marked.
    assert_eq!(search("find bar", cx), vec![pair("Case sensitive", "")]);
    // Found by its key: nothing marked either.
    assert_eq!(search("cmd-b", cx), vec![pair("Bold", "")]);

    // One typing mistake, when nothing matches as written: found, and the
    // list says that it is a near miss.
    assert_eq!(search("itlaic", cx), vec![pair("Italic", "Italic")]);
    assert_eq!(
        view.read_with(cx, |view, _| view.headings()),
        vec!["Nothing matches “itlaic” as written. One letter away:"]
    );
    assert_eq!(search("undrline", cx), vec![pair("Underline", "Undrline")]);
    // As written, no such line.
    search("italic", cx);
    assert!(view.read_with(cx, |view, _| view.headings()).is_empty());
    // Two mistakes are not forgiven.
    assert!(search("itlakc", cx).is_empty());
    assert!(view.read_with(cx, |view, _| view.headings()).is_empty());
}

/// Somebody who writes a whole key means that key. (The released list
/// showed six ⌘⌫ rows after Bold for "cmd-b", because "backspace" begins
/// with a b.)
#[gpui::test]
fn a_whole_key_lists_the_rows_that_have_exactly_that_key(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    let search = |query: &str, cx: &mut VisualTestContext| -> Vec<String> {
        view.update(cx, |view, cx| view.set_query(query, cx));
        cx.run_until_parked();
        let listed = view.read_with(cx, |view, _| view.listed());
        listed
            .into_iter()
            .map(|(target, title)| {
                let keys = keys_of(&host, &target, cx);
                format!("{title} [{}]", keys.join(", "))
            })
            .collect()
    };
    // The host has ⌘⌫ twice (the view's "Remove key", the text field's
    // "Delete to line start" or the like): none of them for "cmd-b".
    let with_backspace = search("cmd-backspace", cx);
    assert!(with_backspace.len() >= 2, "{with_backspace:?}");
    for query in ["cmd-b", "⌘B", "command+b"] {
        assert_eq!(search(query, cx), vec!["Bold [cmd-b]"], "{query}");
    }
    // A key nobody has: now the keys that begin with it are of interest.
    let found = search("cmd-ba", cx);
    assert_eq!(found, with_backspace);
    // ⌘E is the view's Export; ⌘↩ ("enter" begins with an e) is not
    // listed with it.
    let found = search("cmd-e", cx);
    assert_eq!(found, vec!["Export [cmd-e]"]);
    // No row has ⌘H: then what begins with it (⌘↖, "home").
    let found = search("cmd-h", cx);
    assert!(
        found.iter().all(|row| row.contains("cmd-home")),
        "{found:?}"
    );
}

/// A question with much to say still ends inside the smallest window,
/// with its choices in view: the list gives up rows for it, and what does
/// not fit even then scrolls above the choices. (With Esc for a command of
/// the text the choices of the released list stood over the hints at the
/// bottom in a window of 640 by 480.)
#[gpui::test]
fn a_long_question_fits_the_smallest_window(cx: &mut TestAppContext) {
    fn leaves(_: &KeySequence) -> bool {
        false
    }
    let (host, view, cx) = open(cx);
    cx.simulate_resize(gpui::size(px(640.), px(480.)));
    cx.run_until_parked();
    let check = |cx: &mut VisualTestContext| {
        assert_eq!(
            view.read_with(cx, |view, _| view.mode()),
            ModeKind::Question
        );
        let window = cx.update(|window, _| window.viewport_size());
        let panel = bounds(cx, "keymap-panel");
        assert!(
            panel.top() >= px(0.) && panel.bottom() <= window.height,
            "the panel reaches from {:?} to {:?} in a window of {:?}",
            panel.top(),
            panel.bottom(),
            window.height
        );
        let band = bounds(cx, "keymap-band");
        let footer = bounds(cx, "keymap-footer");
        assert_eq!(footer.size.height, px(32.));
        assert!(band.bottom() <= footer.top() && footer.bottom() <= panel.bottom());
        // Every choice whole, inside the band, under what the band says.
        let text = bounds(cx, "keymap-question-text");
        assert!(text.top() >= band.top() && text.bottom() <= band.bottom());
        let count = view.read_with(cx, |view, _| view.choices().len());
        for ix in 0..count {
            let choice = bounds(cx, selector(format!("keymap-choice-{ix}")));
            assert!(
                choice.top() >= text.bottom() && choice.bottom() <= band.bottom(),
                "choice {ix}: {choice:?} under {text:?} in {band:?}"
            );
            assert!(choice.left() >= panel.left() && choice.right() <= panel.right());
        }
        // The row the question is about is shown above it.
        let row = view.read_with(cx, |view, _| view.row_of(&bold()).unwrap());
        let row = bounds(cx, selector(format!("keymap-row-{row}")));
        let toolbar = bounds(cx, "keymap-changed");
        assert!(
            row.top() >= toolbar.bottom() && row.bottom() <= band.top(),
            "{row:?} between {toolbar:?} and {band:?}"
        );
        text
    };

    // Esc is a key of many places: several sentences.
    select(&view, &bold(), cx);
    press(cx, "enter escape");
    let lines = view.read_with(cx, |view, _| view.question_lines());
    assert!(lines.len() >= 2, "{lines:?}");
    let text = check(cx);
    // All of it is in view here, nothing has to scroll: the band is as
    // tall as what it says at the width it has (two rows of the list made
    // room for it).
    let title = bounds(cx, "keymap-question-title");
    let sentences = bounds(cx, "keymap-question-lines");
    assert!(title.top() >= text.top());
    assert!(
        sentences.bottom() <= text.bottom(),
        "the sentences end at {:?}, what shows them at {:?}",
        sentences.bottom(),
        text.bottom()
    );
    press(cx, "escape");

    // And with far more to say than any window of that size holds: the
    // sentences scroll, the choices stay.
    let listing = host
        .read_with(cx, |host, _| host.listing())
        .sees_keys_first(
            "Something",
            ["Editor"],
            leaves,
            "It sees this key before any shortcut does. ".repeat(24),
        );
    view.update_in(cx, |view, window, cx| view.set_listing(listing, window, cx));
    cx.run_until_parked();
    select(&view, &bold(), cx);
    press(cx, "enter escape");
    let text = check(cx);
    assert!(
        text.size.height < px(260.),
        "what the band says is as tall as the window lets it be: {:?}",
        text.size
    );
    assert!(text.size.height >= px(40.), "{:?}", text.size);
}
