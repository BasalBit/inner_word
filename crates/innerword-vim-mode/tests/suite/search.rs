//! The command line, search highlights and requests to the application.

use gpui::{ClipboardItem, TestAppContext};
use innerword_vim_mode::{CommandLine, HostRequest, Mode, SEARCH_LAYER, VimEvent};

use super::{at, doc, open, setup};

fn requests(events: Vec<VimEvent>) -> Vec<HostRequest> {
    events
        .into_iter()
        .filter_map(|event| match event {
            VimEvent::Request(request) => Some(request),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn search_moves_highlights_and_clears(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["alpha beta", "gamma beta", "beta"]), cx);

    cx.simulate_keystrokes("/");
    let status = rig.status(cx);
    assert_eq!(status.mode, Mode::CommandLine);
    assert_eq!(status.mode_label(), "COMMAND");
    assert_eq!(
        status.command_line,
        Some(CommandLine {
            prefix: '/',
            text: String::new(),
            cursor: 0
        })
    );
    assert!(rig.command_line.read_with(cx, |line, cx| line.is_open(cx)));
    assert_eq!(rig.context_mode(cx).as_deref(), Some("command"));
    assert!(!rig.text_input(cx));

    // Typing goes to the command line, not the document; the search is
    // incremental.
    cx.simulate_keystrokes("b e t");
    assert_eq!(rig.text(cx), "alpha beta\ngamma beta\nbeta");
    let line = rig.status(cx).command_line.unwrap();
    assert_eq!((line.text.as_str(), line.cursor), ("bet", 3));
    assert_eq!(rig.head(cx), at(0, 6));
    assert_eq!(
        rig.layer(SEARCH_LAYER, cx),
        vec![(0, 6, 9), (1, 6, 9), (2, 0, 3)]
    );
    cx.simulate_keystrokes("a left backspace");
    let line = rig.status(cx).command_line.unwrap();
    assert_eq!((line.text.as_str(), line.cursor), ("bea", 2));
    cx.simulate_keystrokes("t enter");
    let status = rig.status(cx);
    assert_eq!(status.mode, Mode::Normal);
    assert_eq!(status.command_line, None);
    assert!(status.search_highlighted);
    assert!(!rig.command_line.read_with(cx, |line, cx| line.is_open(cx)));
    assert_eq!(rig.head(cx), at(0, 6));
    assert_eq!(
        rig.layer(SEARCH_LAYER, cx),
        vec![(0, 6, 10), (1, 6, 10), (2, 0, 4)]
    );

    cx.simulate_keystrokes("n");
    assert_eq!(rig.head(cx), at(1, 6));
    cx.simulate_keystrokes("n n");
    assert_eq!(rig.head(cx), at(0, 6), "wraps around");
    cx.simulate_keystrokes("shift-n");
    assert_eq!(rig.head(cx), at(2, 0));

    // The highlights follow edits: text typed before a match, a line
    // deleted, a line added.
    cx.simulate_keystrokes("g g i");
    cx.simulate_input("beta x ");
    cx.simulate_keystrokes("escape");
    assert_eq!(
        rig.layer(SEARCH_LAYER, cx),
        vec![(0, 0, 4), (0, 13, 17), (1, 6, 10), (2, 0, 4)]
    );
    cx.simulate_keystrokes("j d d");
    assert_eq!(rig.text(cx), "beta x alpha beta\nbeta");
    assert_eq!(
        rig.layer(SEARCH_LAYER, cx),
        vec![(0, 0, 4), (0, 13, 17), (1, 0, 4)]
    );
    cx.simulate_keystrokes("shift-p");
    assert_eq!(
        rig.layer(SEARCH_LAYER, cx),
        vec![(0, 0, 4), (0, 13, 17), (1, 6, 10), (2, 0, 4)]
    );
    // An editor command from outside the engine as well.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(
        rig.layer(SEARCH_LAYER, cx),
        vec![(0, 0, 4), (0, 13, 17), (1, 0, 4)]
    );

    // :noh clears them, the next search command brings them back.
    cx.simulate_keystrokes(": n o h enter");
    assert!(rig.layer(SEARCH_LAYER, cx).is_empty());
    assert!(!rig.status(cx).search_highlighted);
    cx.simulate_keystrokes("n");
    assert_eq!(rig.layer(SEARCH_LAYER, cx).len(), 3);
}

#[gpui::test]
fn a_failed_search_reports_an_error(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["alpha beta"]), cx);
    cx.simulate_keystrokes("/ z z z enter");
    let status = rig.status(cx);
    assert_eq!(status.mode, Mode::Normal);
    let message = status.message.expect("an error");
    assert!(message.is_error);
    assert!(message.text.contains("zzz"), "{}", message.text);
    assert_eq!(rig.head(cx), at(0, 0));
    // The next command clears it.
    cx.simulate_keystrokes("l");
    assert_eq!(rig.status(cx).message, None);

    // escape abandons a search and returns the caret.
    cx.simulate_keystrokes("/ b e t");
    assert_eq!(rig.head(cx), at(0, 6));
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.head(cx), at(0, 1));
    assert_eq!(rig.status(cx).command_line, None);
}

#[gpui::test]
fn command_line_history_and_paste(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["one", "two", "three"]), cx);
    cx.simulate_keystrokes(": 3 enter");
    assert_eq!(rig.head(cx).block, 2);
    cx.simulate_keystrokes(": 1 enter");
    assert_eq!(rig.head(cx).block, 0);

    // up / down walk the history.
    cx.simulate_keystrokes(": up");
    assert_eq!(rig.status(cx).command_line.unwrap().text, "1");
    cx.simulate_keystrokes("up");
    assert_eq!(rig.status(cx).command_line.unwrap().text, "3");
    cx.simulate_keystrokes("down");
    assert_eq!(rig.status(cx).command_line.unwrap().text, "1");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.mode(cx), Mode::Normal);

    // cmd-v pastes into the command line, not into the document.
    cx.write_to_clipboard(ClipboardItem::new_string("thr\nignored".into()));
    cx.simulate_keystrokes("/ cmd-v");
    assert_eq!(rig.status(cx).command_line.unwrap().text, "thr");
    assert_eq!(rig.text(cx), "one\ntwo\nthree");
    cx.simulate_keystrokes("enter");
    assert_eq!(rig.head(cx), at(2, 0));

    // An ex command that edits: one undo step.
    cx.simulate_keystrokes(": % s / t / T / g enter");
    assert_eq!(rig.text(cx), "one\nTwo\nThree");
    cx.simulate_keystrokes("u");
    assert_eq!(rig.text(cx), "one\ntwo\nthree");

    // From Visual mode the range is filled in.
    cx.simulate_keystrokes("g g shift-v j :");
    let line = rig.status(cx).command_line.unwrap();
    assert_eq!(line.prefix, ':');
    assert_eq!(line.text, "'<,'>");
    cx.simulate_keystrokes("d enter");
    assert_eq!(rig.text(cx), "three");
}

#[gpui::test]
fn requests_are_emitted_for_the_application(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["one", "two", "three"]), cx);
    rig.take_events();

    cx.simulate_keystrokes(": w enter");
    assert_eq!(requests(rig.take_events()), vec![HostRequest::Save]);
    cx.simulate_keystrokes(": q ! enter");
    assert_eq!(
        requests(rig.take_events()),
        vec![HostRequest::Close { force: true }]
    );
    cx.simulate_keystrokes(": x enter");
    assert_eq!(requests(rig.take_events()), vec![HostRequest::SaveAndClose]);
    cx.simulate_keystrokes("shift-z shift-z");
    assert_eq!(requests(rig.take_events()), vec![HostRequest::SaveAndClose]);

    // A command the engine does not know, with a range and arguments.
    cx.simulate_keystrokes(": 2 , 3 e x p o r t space m d enter");
    assert_eq!(
        requests(rig.take_events()),
        vec![HostRequest::Command {
            name: "export".into(),
            args: "md".into(),
            bang: false,
            range: Some(1..3),
        }]
    );
    assert_eq!(rig.mode(cx), Mode::Normal);
    // The application reports names it does not know.
    rig.vim.update(cx, |vim, cx| {
        vim.show_error("E492: Not an editor command: export", cx)
    });
    let message = rig.status(cx).message.unwrap();
    assert!(message.is_error);
    assert!(rig.take_events().contains(&VimEvent::StatusChanged));
    cx.simulate_keystrokes("j");
    assert_eq!(rig.status(cx).message, None);

    // Mode changes are events too.
    cx.simulate_keystrokes("i escape");
    let events = rig.take_events();
    assert!(events.contains(&VimEvent::ModeChanged(Mode::Insert)));
    assert!(events.contains(&VimEvent::ModeChanged(Mode::Normal)));

    // A command run for the user (the palette).
    rig.vim.update(cx, |vim, cx| vim.execute("2d", cx));
    cx.run_until_parked();
    assert_eq!(rig.text(cx), "one\nthree");
    rig.vim.update(cx, |vim, cx| vim.feed_keys("ggdw", cx));
    cx.run_until_parked();
    assert_eq!(rig.text(cx), "\nthree");
}
