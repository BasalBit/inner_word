//! The drawings of answers: the blocks found, read and joined, what the side chat shows
//! in their place, what Insert takes, the summary's words, the target of a chat and the
//! record of an answer drawn (`docs/assistant-draws-design.md`, sections 2, 3 and 9.2).

use std::collections::HashMap;
use std::sync::Arc;

use innerword_assistant::{
    ChatSession, Drawing, DrawingError, DrawingProblem, DrawingTarget, ElementName, ElementNames,
    FakeBackend, OpenDocument, OpenKind, Role, Transcript, answer_drawing, drawing_blocks,
    drawing_fits, drawing_history_note, drawing_sentence, escape_markdown, shown_markdown,
    side_system_prompt, without_drawings,
};

const BLOCK: &str = r##"{"add": [{"type": "ellipse", "id": "db", "x": 420, "y": 80}, {"type": "arrow", "id": "reads", "x": 300, "y": 125, "start": {"id": "api"}, "end": {"id": "db"}}], "update": [{"id": "api", "backgroundColor": "#a5d8ff"}]}"##;

fn answer(before: &str, body: &str, after: &str) -> String {
    format!("{before}\n\n```whiteboard\n{body}\n```\n\n{after}")
}

fn drawing(json: &str) -> Drawing {
    Drawing::from_json(json).unwrap()
}

struct Names(HashMap<&'static str, ElementName>);

impl ElementNames for Names {
    fn name(&self, id: &str) -> Option<ElementName> {
        self.0.get(id).cloned()
    }
}

fn names() -> Names {
    let mut map = HashMap::new();
    map.insert(
        "api",
        ElementName {
            kind: "rectangle".into(),
            text: Some("API".into()),
        },
    );
    map.insert(
        "pic",
        ElementName {
            kind: "image".into(),
            text: None,
        },
    );
    map.insert(
        "a1",
        ElementName {
            kind: "arrow".into(),
            text: None,
        },
    );
    map.insert(
        "f",
        ElementName {
            kind: "frame".into(),
            text: Some("Back*end*\nthe [server](https://x.y) side, which is a long name".into()),
        },
    );
    Names(map)
}

// ----- the blocks ---------------------------------------------------------------------

#[test]
fn blocks_are_found_by_their_fence() {
    // Backticks and tildes, the info string without case and with more words, a longer
    // fence, three spaces of indent.
    for (open, close) in [
        ("```whiteboard", "```"),
        ("~~~whiteboard", "~~~"),
        ("```Whiteboard", "```"),
        ("``` whiteboard json", "```"),
        ("````whiteboard", "`````"),
        ("   ```whiteboard", "  ```"),
    ] {
        let text = format!("Here.\n{open}\n[]\n{close}\nAfter.");
        let blocks = drawing_blocks(&text);
        assert_eq!(blocks.len(), 1, "{open}");
        assert!(blocks[0].closed);
        assert_eq!(blocks[0].body, "[]\n");
        assert_eq!(blocks[0].range.start, "Here.\n".len());
        assert_eq!(
            &text[blocks[0].range.clone()],
            format!("{open}\n[]\n{close}\n")
        );
    }
    // Not a drawing: another info string, four spaces, a shorter closing fence, a fence
    // of the other character.
    for text in [
        "```json\n[]\n```\n",
        "```whiteboards\n[]\n```\n",
        "    ```whiteboard\n[]\n```\n",
    ] {
        assert!(drawing_blocks(text).is_empty(), "{text}");
    }
    let shorter = drawing_blocks("````whiteboard\n[]\n```\nmore\n````\n");
    assert_eq!(shorter[0].body, "[]\n```\nmore\n");
    let other = drawing_blocks("```whiteboard\n[]\n~~~\n```");
    assert_eq!(other[0].body, "[]\n~~~\n");
    // A whiteboard fence inside a four-backtick block is text of that block.
    let inside = "````markdown\n```whiteboard\n[]\n```\n````\n";
    assert!(drawing_blocks(inside).is_empty());
    // An unclosed block runs to the end.
    let open = "Drawing:\n```whiteboard\n{\"add\": [";
    let blocks = drawing_blocks(open);
    assert_eq!(blocks.len(), 1);
    assert!(!blocks[0].closed);
    assert_eq!(blocks[0].range, "Drawing:\n".len()..open.len());
    assert_eq!(blocks[0].body, "{\"add\": [");
    // Two blocks.
    let two = format!("{}\n{}", answer("A", "[]", "B"), answer("C", "[]", "D"));
    assert_eq!(drawing_blocks(&two).len(), 2);
}

#[test]
fn the_drawing_of_an_answer() {
    assert_eq!(answer_drawing("No drawing here.\n\n```json\n[]\n```"), None);
    let one = answer_drawing(&answer("Here.", BLOCK, "There."))
        .unwrap()
        .unwrap();
    assert_eq!(one.add.len(), 2);
    assert_eq!(one.changed_ids(), vec!["api"]);

    let joined = answer_drawing(&format!(
        "{}\n{}",
        answer("A", r#"{"title": "One", "remove": ["x"]}"#, ""),
        answer(
            "B",
            r#"{"title": "Two", "add": [{"type": "text", "x": 0, "y": 0, "text": "Hi"}]}"#,
            ""
        )
    ))
    .unwrap()
    .unwrap();
    assert_eq!(joined.title.as_deref(), Some("One"));
    assert_eq!(joined.add.len(), 1);
    assert_eq!(joined.remove, vec!["x"]);

    // A problem, with the ordinal of its block when there are several.
    let problem = answer_drawing(&format!(
        "{}\n{}",
        answer("A", "[]", ""),
        answer("B", "{\"add\": [", "")
    ))
    .unwrap()
    .unwrap_err();
    assert_eq!(
        problem,
        DrawingProblem::Unreadable {
            block: Some(0),
            of: 2,
            error: DrawingError::Empty
        }
    );
    assert_eq!(
        problem.reason(),
        "its first block adds, changes and removes nothing"
    );
    let not_json = answer_drawing(&format!(
        "{}\n{}",
        answer(
            "A",
            "[{\"type\": \"text\", \"x\": 0, \"y\": 0, \"text\": \"a\"}]",
            ""
        ),
        answer("B", "{\"add\": [", "")
    ))
    .unwrap()
    .unwrap_err();
    assert!(
        not_json
            .reason()
            .starts_with("its second block is not JSON (line "),
        "{}",
        not_json.reason()
    );
    let picture = answer_drawing(&format!(
        "{}\n{}",
        answer(
            "A",
            "[{\"type\": \"text\", \"x\": 0, \"y\": 0, \"text\": \"a\"}]",
            ""
        ),
        answer("B", r#"[{"type": "image", "x": 0, "y": 0}]"#, "")
    ))
    .unwrap()
    .unwrap_err();
    assert_eq!(
        picture.reason(),
        "in its second block, the element at index 0 of add is a picture, and a drawing from an answer cannot bring one"
    );
    let unfinished = answer_drawing("Here:\n```whiteboard\n[")
        .unwrap()
        .unwrap_err();
    assert_eq!(unfinished, DrawingProblem::NotFinished { block: 0, of: 1 });
    assert_eq!(unfinished.reason(), "it was not finished");
    assert_eq!(
        DrawingProblem::ChangesAWhiteboard.reason(),
        "it changes elements of a whiteboard: open that whiteboard and ask there"
    );
}

#[test]
fn which_drawings_fit_which_target() {
    let adds = drawing(r#"[{"type": "text", "x": 0, "y": 0, "text": "a"}]"#);
    let changes = drawing(BLOCK);
    assert_eq!(
        drawing_fits(&adds, DrawingTarget::None),
        Err(DrawingProblem::NotHere)
    );
    assert_eq!(drawing_fits(&adds, DrawingTarget::NewWhiteboard), Ok(()));
    assert_eq!(drawing_fits(&adds, DrawingTarget::OpenWhiteboard), Ok(()));
    assert_eq!(
        drawing_fits(&changes, DrawingTarget::NewWhiteboard),
        Err(DrawingProblem::ChangesAWhiteboard)
    );
    assert_eq!(
        drawing_fits(&changes, DrawingTarget::OpenWhiteboard),
        Ok(())
    );

    let board = OpenKind::Board;
    let text = OpenKind::Text;
    let page = OpenKind::Page { source_url: None };
    assert_eq!(
        DrawingTarget::of(Some(&board), true),
        DrawingTarget::OpenWhiteboard
    );
    assert_eq!(
        DrawingTarget::of(Some(&text), true),
        DrawingTarget::NewWhiteboard
    );
    assert_eq!(DrawingTarget::of(Some(&page), true), DrawingTarget::None);
    assert_eq!(DrawingTarget::of(None, true), DrawingTarget::None);
    assert_eq!(DrawingTarget::of(Some(&board), false), DrawingTarget::None);
    assert_eq!(DrawingTarget::of(Some(&text), false), DrawingTarget::None);
    assert_eq!(DrawingTarget::default(), DrawingTarget::None);
}

// ----- what is shown and what is taken --------------------------------------------------

#[test]
fn insert_takes_the_answer_without_its_drawing() {
    let text = answer(
        "I put the database beside the API.",
        BLOCK,
        "It reads from it.",
    );
    assert_eq!(
        without_drawings(&text),
        "I put the database beside the API.\n\nIt reads from it."
    );
    assert_eq!(
        without_drawings(&format!("```whiteboard\n{BLOCK}\n```\n")),
        ""
    );
    let plain = "No drawing.\n\n- a\n- b\n";
    assert_eq!(without_drawings(plain), plain);
    // Two blocks in the middle of three paragraphs, blank lines around them made one.
    let two = "One.\n\n\n```whiteboard\n[]\n```\n\n\nTwo.\n```whiteboard\n[]\n```\nThree.\n";
    assert_eq!(without_drawings(two), "One.\n\nTwo.\n\nThree.");
    // An indented line after the block keeps its indent.
    assert_eq!(
        without_drawings("A\n```whiteboard\n[]\n```\n    code\n"),
        "A\n\n    code"
    );
}

#[test]
fn the_chat_shows_a_line_in_place_of_a_block() {
    let text = answer("Here it is.", BLOCK, "Press Draw.");
    let shown = shown_markdown(&text, false, DrawingTarget::OpenWhiteboard, None);
    assert_eq!(
        shown,
        "Here it is.\n\n*Drawing: adds an ellipse and an arrow; changes api.*\n\nPress Draw."
    );
    let shown = shown_markdown(&text, false, DrawingTarget::OpenWhiteboard, Some(&names()));
    assert!(
        shown.contains("*Drawing: adds an ellipse and an arrow; changes the rectangle “API”.*")
    );
    assert!(!shown.contains("backgroundColor"), "never the JSON");

    // Unusable.
    let bad = answer("Here.", "{\"add\": [", "");
    let shown = shown_markdown(&bad, false, DrawingTarget::OpenWhiteboard, None);
    assert!(
        shown.starts_with("Here.\n\n*Drawing that cannot be used: it is not JSON \\(line "),
        "{shown}"
    );
    assert!(shown.ends_with(".*"));

    // Open while the answer is written, and after it ended without the fence.
    let open = "Here:\n\n```whiteboard\n{\"add\": [{\"type\": \"ell";
    assert_eq!(
        shown_markdown(open, true, DrawingTarget::OpenWhiteboard, None),
        "Here:\n\n*Drawing…*"
    );
    assert_eq!(
        shown_markdown(open, false, DrawingTarget::OpenWhiteboard, None),
        "Here:\n\n*Drawing that cannot be used: it was not finished.*"
    );

    // Nothing to replace: the text itself.
    let plain = "Just words.";
    assert!(matches!(
        shown_markdown(plain, true, DrawingTarget::OpenWhiteboard, None),
        std::borrow::Cow::Borrowed("Just words.")
    ));
}

#[test]
fn a_fence_being_written_is_held_back_until_its_line_ends() {
    for partial in [
        "Here:\n`",
        "Here:\n``",
        "Here:\n```",
        "Here:\n```whi",
        "Here:\n   ~~~White",
    ] {
        assert_eq!(
            shown_markdown(partial, true, DrawingTarget::OpenWhiteboard, None),
            "Here:\n",
            "{partial:?}"
        );
        // An answer that is finished shows what it has.
        assert_eq!(
            shown_markdown(partial, false, DrawingTarget::OpenWhiteboard, None),
            partial
        );
    }
    // A line that cannot become a whiteboard fence, or that has ended, is shown.
    for text in ["Here:\n```python", "Here:\n`code` here", "Here:\n```\n"] {
        assert_eq!(
            shown_markdown(text, true, DrawingTarget::OpenWhiteboard, None),
            text
        );
    }
    // Once the line ends as a whiteboard fence, the block is "Drawing…".
    assert_eq!(
        shown_markdown(
            "Here:\n```whiteboard\n",
            true,
            DrawingTarget::OpenWhiteboard,
            None
        ),
        "Here:\n\n*Drawing…*"
    );
}

#[test]
fn the_summary_says_what_the_drawing_does() {
    let d = drawing(
        r#"{"title": "Who *serves* whom", "add": [
            {"type": "rectangle", "x": 0, "y": 0}, {"type": "rectangle", "x": 0, "y": 0},
            {"type": "rectangle", "x": 0, "y": 0}, {"type": "rectangle", "x": 0, "y": 0},
            {"type": "arrow", "x": 0, "y": 0}, {"type": "arrow", "x": 0, "y": 0},
            {"type": "arrow", "x": 0, "y": 0}, {"type": "frame", "x": 0, "y": 0},
            {"type": "freedraw", "x": 0, "y": 0}]}"#,
    );
    assert_eq!(
        drawing_sentence(&d, DrawingTarget::OpenWhiteboard, None),
        "Drawing: adds 4 rectangles, 3 arrows, a drawn line and a frame."
    );
    assert_eq!(
        drawing_sentence(&d, DrawingTarget::NewWhiteboard, None),
        "Drawing: adds 4 rectangles, 3 arrows, a drawn line and a frame (new whiteboard “Who \\*serves\\* whom”)."
    );

    let many = drawing(
        r#"{"update": [{"id": "api", "dx": 1}, {"id": "db", "dx": 1}, {"id": "cache", "dx": 1},
                       {"id": "a", "dx": 1}, {"id": "b", "dx": 1}, {"id": "c", "dx": 1}, {"id": "d", "dx": 1}],
            "remove": ["db"]}"#,
    );
    assert_eq!(
        drawing_sentence(&many, DrawingTarget::OpenWhiteboard, None),
        "Drawing: changes 7 elements (api, db, cache and 4 more); removes db."
    );
    let three = drawing(
        r#"{"update": [{"id": "api", "dx": 1}, {"id": "db", "dx": 1}, {"id": "my_cache", "dx": 1}]}"#,
    );
    assert_eq!(
        drawing_sentence(&three, DrawingTarget::OpenWhiteboard, None),
        "Drawing: changes api, db and my\\_cache."
    );

    // With the names of the whiteboard's elements: a picture is always "a picture".
    let named = drawing(
        r#"{"update": [{"id": "api", "text": "Gateway"}, {"id": "f", "dx": 1}, {"id": "a1", "dx": 2}, {"id": "zz", "dx": 3}],
            "remove": ["pic"]}"#,
    );
    let sentence = drawing_sentence(&named, DrawingTarget::OpenWhiteboard, Some(&names()));
    assert_eq!(
        sentence,
        "Drawing: changes 4 elements (the rectangle “API”, the frame “Back\\*end\\* the \\[server\\]\\(https://x\\.y\\) side…”, an arrow and 1 more); removes a picture."
    );

    // Links by host, at most three, and a link removed.
    let links = drawing(
        r#"{"add": [{"type": "text", "x": 0, "y": 0, "text": "a", "link": "https://Example.com/x"},
                    {"type": "text", "x": 0, "y": 0, "text": "b", "link": "https://example.com/y"},
                    {"type": "text", "x": 0, "y": 0, "text": "c", "link": "mailto:ada@example.org"},
                    {"type": "text", "x": 0, "y": 0, "text": "d", "link": "http://b_c.example/"},
                    {"type": "text", "x": 0, "y": 0, "text": "e", "link": "http://e.example/"}],
            "update": [{"id": "api", "link": null}]}"#,
    );
    assert_eq!(
        drawing_sentence(&links, DrawingTarget::OpenWhiteboard, Some(&names())),
        "Drawing: adds 5 texts; changes the rectangle “API”; links to example\\.com, ada@example\\.org, b\\_c\\.example and 1 more; removes the link of the rectangle “API”."
    );
}

#[test]
fn the_history_note_says_what_the_drawing_did() {
    let d = drawing(BLOCK);
    assert_eq!(
        drawing_history_note(&d),
        "[Inner Word replaced a whiteboard block here: it added an ellipse and an arrow and changed api.]"
    );
    let all = drawing(
        r#"{"title": "Plan", "add": [{"type": "frame", "x": 0, "y": 0}], "update": [{"id": "a", "dx": 1}], "remove": ["b"]}"#,
    );
    assert_eq!(
        drawing_history_note(&all),
        "[Inner Word replaced a whiteboard block here: it added a frame, changed a and removed b, titled “Plan”.]"
    );
}

#[test]
fn escaping_makes_markdown_plain_text() {
    assert_eq!(
        escape_markdown("\\`*_{}[]()#+-.!|<>~\""),
        "\\\\\\`\\*\\_\\{\\}\\[\\]\\(\\)\\#\\+\\-\\.\\!\\|\\<\\>\\~\\\""
    );
    assert_eq!(escape_markdown("a\nb\r\nc\rd"), "a b c d");
    assert_eq!(
        escape_markdown("plain words, “quoted”"),
        "plain words, “quoted”"
    );
}

// ----- the chat: its target and its record ----------------------------------------------

fn pump(chat: &mut ChatSession) {
    let events = chat.events();
    for event in events.drain() {
        chat.apply(event);
    }
}

fn whiteboard() -> OpenDocument {
    OpenDocument {
        id: Some(14),
        title: "Plan".into(),
        path: "Plan".into(),
        markdown: "board:14 “Plan”".into(),
        kind: OpenKind::Board,
    }
}

#[test]
fn a_chat_starts_with_the_prompt_of_its_target() {
    // Without a target: today's prompt.
    let fake = FakeBackend::new();
    let mut chat = ChatSession::side(Arc::new(fake.clone()), None);
    assert_eq!(chat.drawing_target(), DrawingTarget::None);
    chat.ask_side("Hello?", Some(&whiteboard()));
    pump(&mut chat);
    assert_eq!(
        fake.last_started().unwrap().system_prompt,
        side_system_prompt(false, DrawingTarget::None)
    );

    // Made with a target: started once with that target's prompt.
    let fake = FakeBackend::new();
    let mut chat = ChatSession::side(Arc::new(fake.clone()), None)
        .with_drawing_target(DrawingTarget::OpenWhiteboard);
    assert_eq!(chat.drawing_target(), DrawingTarget::OpenWhiteboard);
    chat.prepare();
    chat.ask_side("Add a database.", Some(&whiteboard()));
    pump(&mut chat);
    assert_eq!(fake.started().len(), 1);
    assert_eq!(
        fake.last_started().unwrap().system_prompt,
        side_system_prompt(false, DrawingTarget::OpenWhiteboard)
    );

    // The same target again restarts nothing; another one restarts with what was said.
    chat.set_drawing_target(DrawingTarget::OpenWhiteboard);
    chat.ask_side("And a cache?", Some(&whiteboard()));
    pump(&mut chat);
    assert_eq!(fake.started().len(), 1);
    chat.set_drawing_target(DrawingTarget::NewWhiteboard);
    chat.ask_side("Now as a new one?", Some(&whiteboard()));
    pump(&mut chat);
    let started = fake.started();
    assert_eq!(started.len(), 2);
    assert_eq!(
        started[1].system_prompt,
        side_system_prompt(false, DrawingTarget::NewWhiteboard)
    );
    assert!(
        started[1]
            .history
            .iter()
            .any(|message| message.text.contains("And a cache?")),
        "the new conversation is told what was said"
    );

    // `prepare` after a change starts the new target's conversation.
    let fake = FakeBackend::new();
    let mut chat = ChatSession::side(Arc::new(fake.clone()), None);
    chat.set_drawing_target(DrawingTarget::NewWhiteboard);
    chat.prepare();
    assert_eq!(fake.started().len(), 1);
    assert_eq!(
        fake.last_started().unwrap().system_prompt,
        side_system_prompt(false, DrawingTarget::NewWhiteboard)
    );

    // A quick chat ignores it, and a prompt of the caller's wins.
    let fake = FakeBackend::new();
    let mut quick = ChatSession::quick(Arc::new(fake.clone()))
        .with_drawing_target(DrawingTarget::OpenWhiteboard);
    quick.ask_quick("A word?", None);
    pump(&mut quick);
    assert_eq!(
        fake.last_started().unwrap().system_prompt,
        innerword_assistant::QUICK_SYSTEM_PROMPT
    );
    let fake = FakeBackend::new();
    let mut own = ChatSession::side(Arc::new(fake.clone()), None)
        .with_system_prompt("Mine.")
        .with_drawing_target(DrawingTarget::OpenWhiteboard);
    own.ask_side("Hello?", None);
    pump(&mut own);
    own.set_drawing_target(DrawingTarget::NewWhiteboard);
    own.ask_side("Again?", None);
    pump(&mut own);
    assert_eq!(fake.started().len(), 1);
    assert_eq!(fake.last_started().unwrap().system_prompt, "Mine.");
}

#[test]
fn an_answer_drawn_is_recorded_and_kept() {
    let fake = FakeBackend::new();
    fake.reply(answer("Here.", BLOCK, ""));
    let mut chat = ChatSession::side(Arc::new(fake.clone()), None)
        .with_drawing_target(DrawingTarget::OpenWhiteboard);
    let question = chat.ask_side("Draw it.", Some(&whiteboard()));
    pump(&mut chat);
    let answer = chat.transcript().last_answer().unwrap().id;
    assert!(!chat.mark_drawn(question, 5), "a question is no answer");
    assert!(chat.mark_drawn(answer, 1_795_000_000_000));
    assert_eq!(
        chat.transcript().message(answer).unwrap().drawn_at,
        Some(1_795_000_000_000)
    );
    let json = chat.transcript().to_json();
    assert!(json.contains("\"drawn_at\":1795000000000"));
    let back = Transcript::from_json(&json).unwrap();
    assert_eq!(
        back.message(answer).unwrap().drawn_at,
        Some(1_795_000_000_000)
    );
    assert_eq!(back.message(answer).unwrap().role, Role::Assistant);
    // A message never drawn writes no field, and an old transcript reads.
    assert_eq!(back.message(question).unwrap().drawn_at, None);
    let old = r#"{"messages":[{"id":1,"role":"user","text":"Hi","state":"done"}]}"#;
    assert_eq!(
        Transcript::from_json(old).unwrap().messages[0].drawn_at,
        None
    );
}
