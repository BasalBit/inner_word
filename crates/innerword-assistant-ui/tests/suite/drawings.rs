//! Drawings in the side chat (`support::Workspace`, the fake backend): the
//! line an answer's drawing is shown as, its button, its menu item, its key
//! and its command, what reaches the host, an answer drawn before, the
//! target each conversation starts with.

use std::{sync::Arc, time::Duration};

use gpui::{App, TestAppContext, VisualTestContext, point, px};
use innerword_assistant::{
    DrawingTarget, ElementName, ElementNames, FakeReply, MessageId, answer_drawing,
    side_system_prompt, without_drawings,
};
use innerword_assistant_ui::{
    ACTION_COPY, ACTION_DRAW, ACTION_INSERT, ActionState, ApplyKind, AssistantPanelEvent,
    DRAWING_UNAVAILABLE, DRAWN_ALREADY, DocumentRef, DrawAnswer, ItemKind, MADE_ALREADY,
    NO_ANSWER_YET, NO_DRAWING, ONLY_A_DRAWING, PanelArea, PanelNotice, STILL_WRITTEN,
};

use super::support::{
    Focus, PanelFixture, bounds, choose, click, menu_labels, open_panel, right_click_at, try_bounds,
};

/// Adds an ellipse and an arrow from `api` to it, and colours `api`.
const ON_THE_BOARD: &str = r##"{"add": [{"type": "ellipse", "id": "db", "x": 300, "y": 80, "label": {"text": "DB"}}, {"type": "arrow", "start": {"id": "api"}, "end": {"id": "db"}}], "update": [{"id": "api", "backgroundColor": "#a5d8ff"}]}"##;

/// Adds two ellipses and an arrow between them, with a title.
const NEW_BOARD: &str = r#"{"title": "Who serves whom", "add": [{"type": "ellipse", "id": "a", "x": 0, "y": 0}, {"type": "ellipse", "id": "b", "x": 200, "y": 0}, {"type": "arrow", "start": {"id": "a"}, "end": {"id": "b"}}]}"#;

fn answer(prose: &str, json: &str) -> String {
    format!("{prose}\n\n```whiteboard\n{json}\n```\n")
}

/// The whiteboard "Plan" (7) is open: drawings go on it.
fn open_board(ws: &PanelFixture, cx: &mut VisualTestContext) {
    open_item(
        ws,
        DocumentRef::new(7, "Plan").with_drawing(DrawingTarget::OpenWhiteboard, None),
        cx,
    );
}

fn open_item(ws: &PanelFixture, document: DocumentRef, cx: &mut VisualTestContext) {
    let stored = ws.workspace.read_with(cx, |workspace, _| {
        workspace.stored.get(&document.id).cloned()
    });
    ws.panel.update_in(cx, |panel, window, cx| {
        panel.set_document(Some(document), stored, window, cx)
    });
    cx.run_until_parked();
}

/// Chapter 1 stays open and offers new whiteboards.
fn offer_new_boards(ws: &PanelFixture, cx: &mut VisualTestContext) {
    ws.panel.update(cx, |panel, cx| {
        panel.set_drawing_target(DrawingTarget::NewWhiteboard, None, cx)
    });
    cx.run_until_parked();
}

fn toasts(ws: &PanelFixture, cx: &mut VisualTestContext) -> Vec<String> {
    ws.workspace.read_with(cx, |workspace, cx| {
        workspace
            .toasts
            .read(cx)
            .toasts()
            .map(|toast| toast.message.to_string())
            .collect()
    })
}

fn last_toast(ws: &PanelFixture, cx: &mut VisualTestContext) -> Option<String> {
    toasts(ws, cx).pop()
}

fn answer_text(ws: &PanelFixture, id: MessageId, cx: &mut VisualTestContext) -> String {
    ws.panel.read_with(cx, |panel, cx| {
        panel
            .messages()
            .read(cx)
            .answer_editor(id)
            .map(|editor| editor.read(cx).plain_text())
            .unwrap_or_default()
    })
}

fn draws(events: &[AssistantPanelEvent]) -> Vec<(MessageId, DrawingTarget)> {
    events
        .iter()
        .filter_map(|event| match event {
            AssistantPanelEvent::Draw {
                message, target, ..
            } => Some((*message, *target)),
            _ => None,
        })
        .collect()
}

fn state(
    ws: &PanelFixture,
    action: &str,
    id: MessageId,
    cx: &mut VisualTestContext,
) -> ActionState {
    ws.panel.read_with(cx, |panel, cx| {
        panel.messages().read(cx).action_state(action, id)
    })
}

fn words(ws: &PanelFixture, id: MessageId, cx: &mut VisualTestContext) -> (String, String) {
    ws.panel.read_with(cx, |panel, cx| {
        let words = panel
            .messages()
            .read(cx)
            .action_words(ACTION_DRAW, id)
            .unwrap();
        (words.label.to_string(), words.tooltip.to_string())
    })
}

fn debug_state(ws: &PanelFixture, cx: &mut VisualTestContext) -> String {
    cx.update(|window, cx| ws.panel.read(cx).debug_state(window, cx))
}

// ----- on a whiteboard ---------------------------------------------------------------

#[gpui::test]
fn an_answer_with_a_drawing_shows_its_line_and_draws_on_the_whiteboard(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    open_board(&ws, cx);
    let text = answer("I put the database to the right of the API.", ON_THE_BOARD);
    ws.fake.reply(&text);
    ws.send("Add the database", cx);
    let id = *ws.message_ids(cx).last().unwrap();
    let drawing = answer_drawing(&text).unwrap().unwrap();

    // The line, never the JSON.
    let shown = answer_text(&ws, id, cx);
    assert!(shown.contains("I put the database"), "{shown}");
    assert!(shown.contains("Drawing: adds"), "{shown}");
    assert!(!shown.contains("\"add\""), "{shown}");
    assert!(!shown.contains("```"), "{shown}");

    // The button: first in the row, with its words.
    let draw = bounds(format!("assistant-action-draw-{}", id.0), cx);
    let insert = bounds(format!("assistant-action-insert-{}", id.0), cx);
    assert!(draw.left() < insert.left());
    assert_eq!(state(&ws, ACTION_DRAW, id, cx), ActionState::Enabled);
    assert_eq!(
        words(&ws, id, cx),
        ("Draw".into(), "Draw on Whiteboard".into())
    );
    assert!(debug_state(&ws, cx).contains("draw=whiteboard drawn=no"));

    ws.take_events(cx);
    click(format!("assistant-action-draw-{}", id.0), cx);
    assert_eq!(
        ws.take_events(cx),
        [AssistantPanelEvent::Draw {
            message: id,
            target: DrawingTarget::OpenWhiteboard,
            drawing: drawing.clone(),
        }]
    );
    // The keyboard did not move.
    assert_eq!(ws.focus(cx), Focus::Panel(PanelArea::Composer));

    // The key, from the composer.
    cx.simulate_keystrokes("cmd-shift-d");
    assert_eq!(
        draws(&ws.take_events(cx)),
        [(id, DrawingTarget::OpenWhiteboard)]
    );

    // Copy takes the answer without its drawing.
    click(format!("assistant-action-copy-{}", id.0), cx);
    let events = ws.take_events(cx);
    assert_eq!(
        events,
        [AssistantPanelEvent::Apply {
            kind: ApplyKind::Copy,
            message: id,
            markdown: without_drawings(&text),
        }]
    );
    assert!(!without_drawings(&text).contains("whiteboard"));

    // The answer's menu: the drawing comes last.
    let row = bounds(format!("assistant-message-{}", id.0), cx);
    right_click_at(point(row.right() - px(8.), row.bottom() - px(8.)), cx);
    assert_eq!(
        menu_labels(cx),
        [
            "Insert at Caret",
            "Replace Selection",
            "Copy Answer",
            "Draw on Whiteboard"
        ]
    );
    choose("Draw on Whiteboard", cx);
    assert_eq!(
        draws(&ws.take_events(cx)),
        [(id, DrawingTarget::OpenWhiteboard)]
    );
}

#[gpui::test]
fn the_marked_answer_is_drawn_and_enter_enter_is_still_insert(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    open_board(&ws, cx);
    ws.fake.reply(answer("First.", ON_THE_BOARD));
    ws.send("One", cx);
    ws.fake.reply(answer("Second.", ON_THE_BOARD));
    ws.send("Two", cx);
    let ids = ws.message_ids(cx);
    let (first, second) = (ids[1], ids[3]);
    ws.take_events(cx);

    // From the message list: the marked answer.
    ws.panel.update_in(cx, |panel, window, cx| {
        panel.focus_area(PanelArea::Messages, window, cx)
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("up up");
    assert_eq!(
        cx.update(|window, cx| ws.panel.read(cx).draw_target(window, cx)),
        Some(first)
    );
    cx.simulate_keystrokes("cmd-shift-d");
    assert_eq!(
        draws(&ws.take_events(cx)),
        [(first, DrawingTarget::OpenWhiteboard)]
    );

    // `enter enter` on the marked answer: the menu, then its first item,
    // which is still Insert at Caret.
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        menu_labels(cx).last().map(String::as_str),
        Some("Draw on Whiteboard")
    );
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(matches!(
        ws.take_events(cx).as_slice(),
        [AssistantPanelEvent::Apply {
            kind: ApplyKind::InsertAtCaret,
            message,
            ..
        }] if *message == first
    ));

    // From elsewhere in the panel: the latest answer.
    ws.panel
        .update_in(cx, |panel, window, cx| panel.focus(window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-shift-d");
    assert_eq!(
        draws(&ws.take_events(cx)),
        [(second, DrawingTarget::OpenWhiteboard)]
    );

    // The command reaches a shown panel from the text.
    ws.editor
        .update_in(cx, |editor, window, _| editor.focus(window));
    cx.run_until_parked();
    assert_eq!(ws.focus(cx), Focus::Text);
    cx.dispatch_action(DrawAnswer);
    cx.run_until_parked();
    assert_eq!(
        draws(&ws.take_events(cx)),
        [(second, DrawingTarget::OpenWhiteboard)]
    );
    assert_eq!(ws.focus(cx), Focus::Text);
}

#[gpui::test]
fn a_drawing_is_shown_as_it_is_written_and_offered_once_it_is_whole(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    open_board(&ws, cx);
    ws.fake.hold();
    let body = ON_THE_BOARD;
    let (head, tail) = body.split_at(body.len() / 2);
    ws.fake.push(FakeReply::pieces([
        "Here it is.\n\n``".to_string(),
        "`whiteboard\n".to_string(),
        head.to_string(),
        format!("{tail}\n```\n"),
    ]));
    ws.send("Draw", cx);

    // The first half of a fence is not shown as code.
    ws.advance_to_text("Here it is.\n\n``", cx);
    let id = *ws.message_ids(cx).last().unwrap();
    let shown = answer_text(&ws, id, cx);
    assert!(!shown.contains('`'), "{shown:?}");

    // The open block: "Drawing…", no buttons, the key says why.
    ws.advance_to_text(&format!("Here it is.\n\n```whiteboard\n{head}"), cx);
    let shown = answer_text(&ws, id, cx);
    assert!(shown.contains("Drawing\u{2026}"), "{shown:?}");
    assert!(!shown.contains("\"add\""), "{shown:?}");
    assert!(try_bounds(format!("assistant-action-draw-{}", id.0), cx).is_none());
    ws.take_events(cx);
    cx.simulate_keystrokes("cmd-shift-d");
    assert!(draws(&ws.take_events(cx)).is_empty());
    assert_eq!(last_toast(&ws, cx).as_deref(), Some(STILL_WRITTEN));

    ws.fake.release();
    cx.run_until_parked();
    bounds(format!("assistant-action-draw-{}", id.0), cx);
    assert!(answer_text(&ws, id, cx).contains("Drawing: adds"));
}

#[gpui::test]
fn a_drawing_that_cannot_be_used_says_why(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    open_board(&ws, cx);
    ws.fake.reply(answer("Here.", "{\"add\": [}"));
    ws.send("Draw", cx);
    let id = *ws.message_ids(cx).last().unwrap();
    let shown = answer_text(&ws, id, cx);
    assert!(shown.contains("Drawing that cannot be used: "), "{shown}");

    // The button is there, dimmed with the reason; it does nothing.
    let ActionState::Disabled(reason) = state(&ws, ACTION_DRAW, id, cx) else {
        panic!("the button is not dimmed");
    };
    assert!(
        reason.starts_with("Drawing that cannot be used: it is not JSON"),
        "{reason}"
    );
    ws.take_events(cx);
    click(format!("assistant-action-draw-{}", id.0), cx);
    assert!(ws.take_events(cx).is_empty());

    // The key says it.
    cx.simulate_keystrokes("cmd-shift-d");
    assert!(ws.take_events(cx).is_empty());
    let toast = last_toast(&ws, cx).unwrap();
    assert!(
        toast.starts_with("Nothing was drawn: it is not JSON"),
        "{toast}"
    );

    // The menu item is dimmed, with the reason cut short.
    let row = bounds(format!("assistant-message-{}", id.0), cx);
    right_click_at(point(row.right() - px(8.), row.bottom() - px(8.)), cx);
    let last = menu_labels(cx).pop().unwrap();
    assert!(
        last.starts_with("Draw on Whiteboard (Drawing that cannot be used"),
        "{last}"
    );
    assert!(
        last.chars().count() <= "Draw on Whiteboard ()".len() + 60,
        "{last}"
    );
    let chosen = cx.update(|window, cx| innerword_ui::ContextMenu::choose(&last, window, cx));
    cx.run_until_parked();
    let _ = chosen;
    assert!(ws.take_events(cx).is_empty());
}

#[gpui::test]
fn an_answer_that_is_only_a_drawing_cannot_be_inserted_or_copied(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    open_board(&ws, cx);
    ws.fake.reply(answer("", ON_THE_BOARD));
    ws.send("Draw", cx);
    let id = *ws.message_ids(cx).last().unwrap();
    for action in [ACTION_INSERT, ACTION_COPY] {
        assert_eq!(
            state(&ws, action, id, cx),
            ActionState::Disabled(ONLY_A_DRAWING.into())
        );
    }
    assert_eq!(state(&ws, ACTION_DRAW, id, cx), ActionState::Enabled);
    ws.take_events(cx);
    cx.simulate_keystrokes("cmd-enter cmd-shift-c");
    click(format!("assistant-action-copy-{}", id.0), cx);
    assert!(ws.take_events(cx).is_empty());
    cx.simulate_keystrokes("cmd-shift-d");
    assert_eq!(draws(&ws.take_events(cx)).len(), 1);
}

#[gpui::test]
fn the_line_names_the_elements_the_host_names(cx: &mut TestAppContext) {
    struct Names;
    impl ElementNames for Names {
        fn name(&self, id: &str) -> Option<ElementName> {
            (id == "api").then(|| ElementName {
                kind: "rectangle".into(),
                text: Some("Public API".into()),
            })
        }
    }

    let (ws, cx) = open_panel(cx);
    open_board(&ws, cx);
    ws.fake.reply(answer("Coloured.", ON_THE_BOARD));
    ws.send("Colour it", cx);
    let id = *ws.message_ids(cx).last().unwrap();
    let before = answer_text(&ws, id, cx);
    assert!(!before.contains("Public API"), "{before}");
    ws.panel.update(cx, |panel, cx| {
        panel.set_element_names(Some(Arc::new(Names)), cx)
    });
    cx.run_until_parked();
    let after = answer_text(&ws, id, cx);
    assert!(after.contains("Public API"), "{after}");
    ws.panel
        .update(cx, |panel, cx| panel.set_element_names(None, cx));
    cx.run_until_parked();
    assert_eq!(answer_text(&ws, id, cx), before);
}

/// A drawn answer's line reads as it did when it was drawn: what it removed is named as
/// it was, never said to be missing from the whiteboard (section 14 of the design).
#[gpui::test]
fn a_drawn_answers_line_keeps_its_names(cx: &mut TestAppContext) {
    struct Names(bool);
    impl ElementNames for Names {
        fn name(&self, id: &str) -> Option<ElementName> {
            match id {
                "api" => Some(ElementName {
                    kind: "rectangle".into(),
                    text: Some("API".into()),
                }),
                "old" if self.0 => Some(ElementName {
                    kind: "text".into(),
                    text: Some("Old note".into()),
                }),
                _ => None,
            }
        }
    }
    const REMOVES: &str = r#"{"update": [{"id": "api", "strokeColor": "red"}], "remove": ["old"]}"#;

    let (ws, cx) = open_panel(cx);
    open_board(&ws, cx);
    ws.fake.reply(answer("Tidied.", REMOVES));
    ws.send("Tidy it", cx);
    let id = *ws.message_ids(cx).last().unwrap();
    let before_drawn = Names(true);
    ws.panel.update(cx, |panel, cx| {
        panel.set_element_names(Some(Arc::new(Names(true))), cx)
    });
    cx.run_until_parked();
    let line = answer_text(&ws, id, cx);
    assert!(line.contains("removes the text “Old note”"), "{line}");
    // The host drew it, keeping the names of the press; the whiteboard has no "old" now.
    let drawing = innerword_assistant::answer_drawing(&answer("Tidied.", REMOVES))
        .unwrap()
        .unwrap();
    let kept = innerword_assistant::names_the_drawing_uses(&drawing, &before_drawn);
    ws.panel
        .update(cx, |panel, cx| panel.mark_drawn(id, kept, cx));
    ws.panel.update(cx, |panel, cx| {
        panel.set_element_names(Some(Arc::new(Names(false))), cx)
    });
    cx.run_until_parked();
    let drawn = answer_text(&ws, id, cx);
    assert_eq!(drawn, line, "the line reads as it did when it was drawn");
    assert!(!drawn.contains("has no"), "{drawn}");
}

// ----- drawn before -------------------------------------------------------------------

#[gpui::test]
fn an_answer_drawn_before_says_so_and_its_key_wants_a_second_press(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    open_board(&ws, cx);
    ws.fake.reply(answer("Here.", ON_THE_BOARD));
    ws.send("Draw", cx);
    let id = *ws.message_ids(cx).last().unwrap();
    ws.take_events(cx);
    click(format!("assistant-action-draw-{}", id.0), cx);
    assert_eq!(draws(&ws.take_events(cx)).len(), 1);

    // The host drew it: the answer says so, and is stored so.
    ws.panel
        .update(cx, |panel, cx| panel.mark_drawn(id, Default::default(), cx));
    cx.run_until_parked();
    let stored = ws.stored(7, cx);
    assert!(stored.message(id).unwrap().drawn_at.is_some());
    let (label, tooltip) = words(&ws, id, cx);
    assert_eq!(label, "Draw Again");
    assert!(tooltip.starts_with("Already drawn once"), "{tooltip}");
    assert!(debug_state(&ws, cx).contains("drawn=yes"));
    let row = bounds(format!("assistant-message-{}", id.0), cx);
    right_click_at(point(row.right() - px(8.), row.bottom() - px(8.)), cx);
    assert_eq!(
        menu_labels(cx).last().map(String::as_str),
        Some("Draw Again on Whiteboard")
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    // The button draws it again at once.
    click(format!("assistant-action-draw-{}", id.0), cx);
    assert_eq!(draws(&ws.take_events(cx)).len(), 1);

    // The key: the first press says so, a second one within the time
    // draws.
    ws.panel
        .update_in(cx, |panel, window, cx| panel.focus(window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-shift-d");
    assert!(draws(&ws.take_events(cx)).is_empty());
    assert_eq!(last_toast(&ws, cx).as_deref(), Some(DRAWN_ALREADY));
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.simulate_keystrokes("cmd-shift-d");
    assert_eq!(draws(&ws.take_events(cx)).len(), 1);
    // And the next press starts again.
    cx.simulate_keystrokes("cmd-shift-d");
    assert!(draws(&ws.take_events(cx)).is_empty());
    // Too late for a second press: it says so again.
    cx.executor().advance_clock(Duration::from_secs(6));
    cx.simulate_keystrokes("cmd-shift-d");
    assert!(draws(&ws.take_events(cx)).is_empty());
    cx.simulate_keystrokes("cmd-shift-d");
    assert_eq!(draws(&ws.take_events(cx)).len(), 1);

    // A stored conversation brings the mark back.
    open_item(&ws, DocumentRef::new(3, "Characters"), cx);
    open_board(&ws, cx);
    assert_eq!(words(&ws, id, cx).0, "Draw Again");
}

// ----- over a document ------------------------------------------------------------------

#[gpui::test]
fn over_a_document_an_answer_makes_a_new_whiteboard(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    offer_new_boards(&ws, cx);
    assert!(debug_state(&ws, cx).contains("draw=new"));
    let text = answer("Who serves whom, drawn.", NEW_BOARD);
    ws.fake.reply(&text);
    ws.send("Draw who serves whom", cx);
    let id = *ws.message_ids(cx).last().unwrap();
    assert!(answer_text(&ws, id, cx).contains("Who serves whom"));
    assert_eq!(
        words(&ws, id, cx),
        ("New Whiteboard".into(), "New Whiteboard from Answer".into())
    );
    ws.take_events(cx);
    click(format!("assistant-action-draw-{}", id.0), cx);
    assert_eq!(
        ws.take_events(cx),
        [AssistantPanelEvent::Draw {
            message: id,
            target: DrawingTarget::NewWhiteboard,
            drawing: answer_drawing(&text).unwrap().unwrap(),
        }]
    );
    let row = bounds(format!("assistant-message-{}", id.0), cx);
    right_click_at(point(row.right() - px(8.), row.bottom() - px(8.)), cx);
    assert_eq!(
        menu_labels(cx).last().map(String::as_str),
        Some("New Whiteboard from Answer")
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    // Made once: its words, and the key's first press.
    ws.panel
        .update(cx, |panel, cx| panel.mark_drawn(id, Default::default(), cx));
    cx.run_until_parked();
    assert_eq!(words(&ws, id, cx).0, "New Whiteboard Again");
    ws.panel
        .update_in(cx, |panel, window, cx| panel.focus(window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-shift-d");
    assert_eq!(last_toast(&ws, cx).as_deref(), Some(MADE_ALREADY));
    cx.simulate_keystrokes("cmd-shift-d");
    assert_eq!(
        draws(&ws.take_events(cx)),
        [(id, DrawingTarget::NewWhiteboard)]
    );

    // A drawing that changes elements cannot make a new whiteboard.
    ws.fake.reply(answer("Changed.", ON_THE_BOARD));
    ws.send("Change it", cx);
    let changes = *ws.message_ids(cx).last().unwrap();
    let ActionState::Disabled(reason) = state(&ws, ACTION_DRAW, changes, cx) else {
        panic!("not dimmed");
    };
    assert!(
        reason.contains("changes elements of a whiteboard"),
        "{reason}"
    );
    cx.simulate_keystrokes("cmd-shift-d");
    assert!(draws(&ws.take_events(cx)).is_empty());
    let toast = last_toast(&ws, cx).unwrap();
    assert!(
        toast.starts_with("Nothing was drawn: it changes elements of a whiteboard"),
        "{toast}"
    );
}

// ----- where nothing can be drawn -------------------------------------------------------

#[gpui::test]
fn where_nothing_can_be_drawn_there_is_no_button_and_the_key_says_why(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    // Chapter 1 in an app that cannot show whiteboards: no target.
    ws.fake.reply(answer("Here.", NEW_BOARD));
    ws.send("Draw", cx);
    let id = *ws.message_ids(cx).last().unwrap();
    assert!(debug_state(&ws, cx).contains("draw=none"));
    assert!(try_bounds(format!("assistant-action-draw-{}", id.0), cx).is_none());
    assert_eq!(state(&ws, ACTION_DRAW, id, cx), ActionState::Hidden);
    // The line is shown all the same.
    assert!(answer_text(&ws, id, cx).contains("Drawing: adds"));
    ws.take_events(cx);
    cx.simulate_keystrokes("cmd-shift-d");
    assert!(ws.take_events(cx).is_empty());
    assert_eq!(last_toast(&ws, cx).as_deref(), Some(DRAWING_UNAVAILABLE));

    // The host's own reason.
    let why = "This whiteboard could not be shown, so nothing can be added to it.";
    ws.panel.update(cx, |panel, cx| {
        panel.set_drawing_target(DrawingTarget::None, Some(why.into()), cx)
    });
    cx.simulate_keystrokes("cmd-shift-d");
    assert_eq!(last_toast(&ws, cx).as_deref(), Some(why));

    // Drawable again: the button comes.
    offer_new_boards(&ws, cx);
    bounds(format!("assistant-action-draw-{}", id.0), cx);

    // An answer without a drawing.
    ws.fake.reply("Just words.");
    ws.send("Words", cx);
    let plain = *ws.message_ids(cx).last().unwrap();
    assert!(try_bounds(format!("assistant-action-draw-{}", plain.0), cx).is_none());
    cx.simulate_keystrokes("cmd-shift-d");
    assert!(draws(&ws.take_events(cx)).is_empty());
    assert_eq!(last_toast(&ws, cx).as_deref(), Some(NO_DRAWING));
}

#[gpui::test]
fn with_no_answer_yet_the_key_says_so(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    offer_new_boards(&ws, cx);
    ws.panel
        .update_in(cx, |panel, window, cx| panel.focus(window, cx));
    cx.run_until_parked();
    ws.take_events(cx);
    cx.simulate_keystrokes("cmd-shift-d");
    assert!(draws(&ws.take_events(cx)).is_empty());
    assert_eq!(last_toast(&ws, cx).as_deref(), Some(NO_ANSWER_YET));
}

/// The list of shortcuts searches titles only: the key's row is found by "draw" and
/// names both of its meanings.
#[test]
fn the_drawing_command_is_titled_by_both_meanings() {
    let command = innerword_assistant_ui::commands()
        .into_iter()
        .find(|command| command.action_name() == gpui::Action::name(&DrawAnswer))
        .unwrap();
    assert_eq!(
        command.title.as_ref(),
        innerword_assistant_ui::DRAW_ANSWER_TITLE
    );
    assert!(command.title.to_lowercase().starts_with("draw"));
    assert!(command.title.contains("new whiteboard"));
}

// ----- over a page ----------------------------------------------------------------------

/// Over a page an answer makes a new whiteboard as over a document: the same
/// button, menu item and key, its own target in the event and its own prompt.
#[gpui::test]
fn over_a_page_an_answer_makes_a_new_whiteboard(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    open_item(
        &ws,
        DocumentRef::new(12, "Orbitarium")
            .with_kind(ItemKind::Page)
            .with_drawing(DrawingTarget::NewWhiteboardBesidePage, None),
        cx,
    );
    assert!(debug_state(&ws, cx).contains("draw=new"));
    let text = answer("The steps the page describes, drawn.", NEW_BOARD);
    ws.fake.reply(&text);
    ws.send("Draw the steps", cx);
    assert_eq!(
        ws.fake.last_started().unwrap().system_prompt,
        side_system_prompt(false, DrawingTarget::NewWhiteboardBesidePage)
    );
    let id = *ws.message_ids(cx).last().unwrap();
    assert!(
        answer_text(&ws, id, cx).contains("(new whiteboard \u{201c}Who serves whom\u{201d})"),
        "{}",
        answer_text(&ws, id, cx)
    );
    assert_eq!(
        words(&ws, id, cx),
        ("New Whiteboard".into(), "New Whiteboard from Answer".into())
    );
    // The drawing is offered.
    assert_eq!(state(&ws, ACTION_DRAW, id, cx), ActionState::Enabled);
    ws.take_events(cx);
    click(format!("assistant-action-draw-{}", id.0), cx);
    assert_eq!(
        ws.take_events(cx),
        [AssistantPanelEvent::Draw {
            message: id,
            target: DrawingTarget::NewWhiteboardBesidePage,
            drawing: answer_drawing(&text).unwrap().unwrap(),
        }]
    );
    let row = bounds(format!("assistant-message-{}", id.0), cx);
    right_click_at(point(row.right() - px(8.), row.bottom() - px(8.)), cx);
    assert_eq!(
        menu_labels(cx).last().map(String::as_str),
        Some("New Whiteboard from Answer")
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    ws.panel
        .update_in(cx, |panel, window, cx| panel.focus(window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-shift-d");
    assert_eq!(
        draws(&ws.take_events(cx)),
        [(id, DrawingTarget::NewWhiteboardBesidePage)]
    );

    // A drawing that changes elements cannot make a new whiteboard here either.
    ws.fake.reply(answer("Changed.", ON_THE_BOARD));
    ws.send("Change it", cx);
    let changes = *ws.message_ids(cx).last().unwrap();
    let ActionState::Disabled(reason) = state(&ws, ACTION_DRAW, changes, cx) else {
        panic!("not dimmed");
    };
    assert!(
        reason.contains("changes elements of a whiteboard"),
        "{reason}"
    );
}

// ----- the conversation of each item ---------------------------------------------------

#[gpui::test]
fn each_item_starts_its_chat_once_with_its_own_prompt(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    offer_new_boards(&ws, cx);
    ws.fake.reply("One.");
    ws.send("About the chapter", cx);
    ws.fake.reply("Two.");
    ws.send("More about the chapter", cx);
    let started = ws.fake.started();
    assert_eq!(started.len(), 1);
    assert_eq!(
        started[0].system_prompt,
        side_system_prompt(false, DrawingTarget::NewWhiteboard)
    );

    open_board(&ws, cx);
    ws.fake.reply("Three.");
    ws.send("About the whiteboard", cx);
    ws.fake.reply("Four.");
    ws.send("More about the whiteboard", cx);
    let started = ws.fake.started();
    assert_eq!(started.len(), 2);
    assert_eq!(
        started[1].system_prompt,
        side_system_prompt(false, DrawingTarget::OpenWhiteboard)
    );

    // An item that offers nothing.
    open_item(&ws, DocumentRef::new(3, "Characters"), cx);
    ws.fake.reply("Five.");
    ws.send("About the notes", cx);
    let started = ws.fake.started();
    assert_eq!(started.len(), 3);
    assert_eq!(
        started[2].system_prompt,
        side_system_prompt(false, DrawingTarget::None)
    );

    // Back to the chapter: its chat starts again with its own prompt.
    open_item(
        &ws,
        DocumentRef::new(1, "Chapter 1").with_drawing(DrawingTarget::NewWhiteboard, None),
        cx,
    );
    ws.fake.reply("Six.");
    ws.send("Again", cx);
    let started = ws.fake.started();
    assert_eq!(started.len(), 4);
    assert_eq!(
        started[3].system_prompt,
        side_system_prompt(false, DrawingTarget::NewWhiteboard)
    );
}

#[gpui::test]
fn a_conversation_that_goes_on_elsewhere_keeps_its_target(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    open_board(&ws, cx);
    ws.fake.hold();
    ws.fake.push(FakeReply::pieces([
        "Here.\n\n```whiteboard\n".to_string(),
        format!("{ON_THE_BOARD}\n```\n"),
    ]));
    ws.send("Draw", cx);
    ws.advance_to_text("Here.\n\n```whiteboard\n", cx);
    // Another item while the answer is written: the whiteboard's chat is
    // parked, and comes back with its target.
    offer_new_boards_on(&ws, 1, cx);
    assert_eq!(
        ws.panel
            .read_with(cx, |panel, _| panel.answering_elsewhere()),
        [Some(7)]
    );
    ws.fake.release();
    cx.run_until_parked();
    open_board(&ws, cx);
    let id = *ws.message_ids(cx).last().unwrap();
    assert_eq!(state(&ws, ACTION_DRAW, id, cx), ActionState::Enabled);
    assert_eq!(words(&ws, id, cx).0, "Draw");
}

fn offer_new_boards_on(ws: &PanelFixture, number: i64, cx: &mut VisualTestContext) {
    open_item(
        ws,
        DocumentRef::new(number, format!("Chapter {number}"))
            .with_drawing(DrawingTarget::NewWhiteboard, None),
        cx,
    );
}

// ----- what the host does with the panel ----------------------------------------------

#[gpui::test]
fn over_a_whiteboard_the_empty_chat_says_what_it_is_for(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    let note = "Drawn from an answer about \u{201c}Chapter 2\u{201d}.";
    open_item(
        &ws,
        DocumentRef::new(9, "Who serves whom")
            .with_drawing(DrawingTarget::OpenWhiteboard, None)
            .with_origin_note(note),
        cx,
    );
    ws.panel.read_with(cx, |panel, cx| {
        let list = panel.messages().read(cx);
        let empty = list.empty_state().unwrap();
        assert_eq!(empty.title.as_ref(), "Ask about this whiteboard");
        assert_eq!(empty.note.as_ref(), note);
        assert_eq!(empty.examples.len(), 3);
        assert_eq!(
            panel
                .composer()
                .read(cx)
                .editor()
                .read(cx)
                .config()
                .placeholder
                .as_ref(),
            "Ask about this whiteboard"
        );
    });
    bounds("assistant-empty-note", cx);

    // A document keeps its own.
    offer_new_boards_on(&ws, 1, cx);
    ws.panel.read_with(cx, |panel, cx| {
        let list = panel.messages().read(cx);
        assert_eq!(
            list.empty_state().unwrap().title.as_ref(),
            "Ask about this document"
        );
        assert!(list.empty_state().unwrap().note.is_empty());
    });

    // A page says "page", in the empty state, the composer and the note about
    // where the text goes alike; and a whiteboard that cannot be drawn on now
    // is still a whiteboard.
    ws.panel
        .update(cx, |panel, cx| panel.set_privacy_notice(true, cx));
    let sent = |ws: &PanelFixture, cx: &mut VisualTestContext| {
        ws.panel.read_with(cx, |panel, _| {
            let notices = panel.notices();
            assert_eq!(notices[0].0, PanelNotice::Privacy);
            let advice = notices[0].1.advice.to_string();
            advice
                .split(" and what the assistant reads")
                .next()
                .unwrap_or_default()
                .to_string()
        })
    };
    assert_eq!(sent(&ws, cx), "Your messages, the open document");
    let placeholder = |ws: &PanelFixture, cx: &mut VisualTestContext| {
        ws.panel.read_with(cx, |panel, cx| {
            let list = panel.messages().read(cx);
            (
                list.empty_state().unwrap().title.to_string(),
                panel
                    .composer()
                    .read(cx)
                    .editor()
                    .read(cx)
                    .config()
                    .placeholder
                    .to_string(),
            )
        })
    };
    open_item(
        &ws,
        DocumentRef::new(12, "Orbitarium").with_kind(ItemKind::Page),
        cx,
    );
    let (title, words) = placeholder(&ws, cx);
    assert_eq!(title, "Ask about this page");
    assert_eq!(words, "Ask about this page");
    assert_eq!(sent(&ws, cx), "Your messages, the open page");
    open_item(
        &ws,
        DocumentRef::new(13, "Plan")
            .with_kind(ItemKind::Whiteboard)
            .with_drawing(DrawingTarget::None, Some("Still loading.".into())),
        cx,
    );
    let (title, words) = placeholder(&ws, cx);
    assert_eq!(title, "Ask about this whiteboard");
    assert_eq!(words, "Ask about this whiteboard");
    assert_eq!(sent(&ws, cx), "Your messages, the open whiteboard");
}

fn ws_last(ws: &PanelFixture, cx: &App) -> MessageId {
    *ws.panel
        .read(cx)
        .transcript()
        .messages
        .last()
        .map(|message| &message.id)
        .unwrap()
}

#[gpui::test]
fn the_host_offers_a_message_and_shows_an_answer(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    open_board(&ws, cx);
    ws.fake.reply(answer("First.", ON_THE_BOARD));
    ws.send("One", cx);
    ws.fake.reply("Second.");
    ws.send("Two", cx);
    let first = ws.message_ids(cx)[1];

    // The keyboard goes back to the text; the panel marks an answer and
    // leaves the keyboard where it is.
    ws.editor
        .update_in(cx, |editor, window, _| editor.focus(window));
    cx.run_until_parked();
    ws.panel
        .update(cx, |panel, cx| panel.show_answer(first, cx));
    cx.run_until_parked();
    assert_eq!(ws.focus(cx), Focus::Text);
    ws.panel.read_with(cx, |panel, cx| {
        assert_eq!(panel.messages().read(cx).selected(), Some(first));
    });
    // The mark is drawn although the keyboard is elsewhere, and the key of
    // a drawing acts on that answer, not on the latest.
    let mark = bounds("assistant-message-mark", cx);
    let row = bounds(format!("assistant-message-{}", first.0), cx);
    assert!(row.contains(&mark.center()), "{mark:?} in {row:?}");
    ws.take_events(cx);
    ws.panel
        .update_in(cx, |panel, window, cx| panel.focus(window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-shift-d");
    assert_eq!(
        draws(&ws.take_events(cx)),
        [(first, DrawingTarget::OpenWhiteboard)]
    );
    // Marking another message takes the mark shown away.
    let last = cx.update(|_, cx| ws_last(&ws, cx));
    ws.panel.update(cx, |panel, cx| {
        panel
            .messages()
            .update(cx, |list, cx| list.select(Some(last), cx))
    });
    ws.panel.read_with(cx, |panel, cx| {
        assert_eq!(panel.messages().read(cx).shown(), None);
    });
    ws.editor
        .update_in(cx, |editor, window, _| editor.focus(window));
    cx.run_until_parked();

    // A message is offered into an empty composer only.
    let offered = ws.panel.update(cx, |panel, cx| {
        panel.offer_message("Write the drawing again.", cx)
    });
    assert!(offered);
    let again = ws
        .panel
        .update(cx, |panel, cx| panel.offer_message("Something else.", cx));
    assert!(!again);
    ws.panel.read_with(cx, |panel, cx| {
        assert_eq!(
            panel.composer().read(cx).text(cx),
            "Write the drawing again."
        );
    });
    assert_eq!(ws.focus(cx), Focus::Text);
}

/// The chip of the open item has the icon the library's row has, and its
/// tooltip names the item by what it is.
#[test]
fn the_open_item_s_chip_has_the_icon_of_its_kind() {
    use innerword_ui::IconName;
    assert_eq!(ItemKind::Document.icon(), IconName::Page);
    assert_eq!(ItemKind::Page.icon(), IconName::Globe);
    assert_eq!(ItemKind::Whiteboard.icon(), IconName::Whiteboard);
    assert_eq!(ItemKind::Document.noun(), "document");
    assert_eq!(ItemKind::Page.noun(), "page");
    assert_eq!(ItemKind::Whiteboard.noun(), "whiteboard");
}
