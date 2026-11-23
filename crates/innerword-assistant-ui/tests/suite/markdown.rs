//! Markdown to the document an answer shows.

use innerword_assistant_ui::{AnswerStyle, answer_document, answer_plain_text, markdown::settle};
use innerword_core::BlockKind;

#[test]
fn an_answer_becomes_the_blocks_a_document_has() {
    let markdown = "# Three options\n\nA **bold** claim and a `code` word.\n\n- one\n- two\n\n> quoted\n\n```\nlet x = 1;\n```\n";
    let document = answer_document(markdown, &AnswerStyle::default(), false);
    let kinds: Vec<&str> = document
        .blocks()
        .iter()
        .map(|block| block.kind().name())
        .collect();
    assert_eq!(
        kinds,
        ["heading", "paragraph", "bullet", "bullet", "quote", "code"],
        "{kinds:?}"
    );
    assert!(matches!(
        document.blocks()[0].kind(),
        BlockKind::Heading { level: 1 }
    ));
    let paragraph = &document.blocks()[1];
    assert_eq!(paragraph.text(), "A bold claim and a code word.");
    assert!(paragraph.attrs_at(2).unwrap().bold);
    assert!(!paragraph.attrs_at(0).unwrap().bold);
    assert!(paragraph.attrs_at(19).unwrap().code);
    // The typography of the answer style travels with the document.
    let typography = &document.settings().typography;
    assert_eq!(typography.base_size, 15.);
    assert_eq!(typography.line_height, 1.5);
    assert_eq!(
        answer_document("x", &AnswerStyle::small(), false)
            .settings()
            .typography
            .base_size,
        14.
    );
}

#[test]
fn an_empty_answer_is_one_empty_paragraph() {
    let document = answer_document("", &AnswerStyle::default(), true);
    assert_eq!(document.block_count(), 1);
    assert_eq!(document.plain_text(), "");
}

#[test]
fn plain_text_drops_the_markers() {
    assert_eq!(
        answer_plain_text("**Tired** and *worn*.\n\n- one\n- two"),
        "Tired and worn.\none\ntwo"
    );
}

#[test]
fn an_unfinished_last_line_is_settled() {
    // The first half of a pair is closed.
    assert_eq!(settle("A **bold cla"), "A **bold cla**");
    assert_eq!(settle("An *emphat"), "An *emphat*");
    assert_eq!(settle("Call `render("), "Call `render(`");
    assert_eq!(settle("Both **bold and *it"), "Both **bold and *it***");
    assert_eq!(settle("An __under"), "An __under__");
    // What is complete stays as it is.
    for complete in [
        "A **bold** claim.",
        "plain text",
        "snake_case_name",
        "2 * 3 = 6",
        "* a bullet",
        "- [ ] a task",
        "a\\*b",
        "`code` and *done*",
        "",
    ] {
        assert_eq!(settle(complete), complete, "{complete:?}");
    }
    // Only the last line counts: an earlier paragraph is the importer's.
    assert_eq!(settle("An *open\n\nNext"), "An *open\n\nNext");
    // Inside an open code fence nothing is closed.
    assert_eq!(settle("```\nlet a = b * c"), "```\nlet a = b * c");
    assert_eq!(
        settle("```rust\nfn x() {}\n```\n\nThen **bo"),
        "```rust\nfn x() {}\n```\n\nThen **bo**"
    );
}

#[test]
fn a_streaming_answer_shows_no_raw_markers() {
    let style = AnswerStyle::default();
    let streaming = answer_document("The word is **exhaus", &style, true);
    assert_eq!(streaming.plain_text(), "The word is exhaus");
    assert!(streaming.blocks()[0].attrs_at(13).unwrap().bold);
    // The finished answer is read as it is.
    let finished = answer_document("The word is **exhaus", &style, false);
    assert_eq!(finished.plain_text(), "The word is **exhaus");
}

/// The line a drawing block becomes (`innerword_assistant::shown_markdown`)
/// goes through the importer as one italic paragraph of plain text: the
/// model's words in it (a title, an element's text, a link's host, the
/// reader's message) make no link and no other formatting, and the app's
/// counts come first.
#[test]
fn the_line_of_a_drawing_cannot_be_steered() {
    use innerword_assistant::{DrawingTarget, ElementName, ElementNames, shown_markdown};

    struct Names;
    impl ElementNames for Names {
        fn name(&self, id: &str) -> Option<ElementName> {
            (id == "api").then(|| ElementName {
                kind: "rectangle".into(),
                text: Some("*A* _b_ [c](https://x.y) `d`".into()),
            })
        }
    }

    let check = |markdown: &str, target: DrawingTarget, names: Option<&dyn ElementNames>| {
        let shown = shown_markdown(markdown, false, target, names);
        let document = answer_document(&shown, &AnswerStyle::default(), false);
        let line = document
            .blocks()
            .iter()
            .find(|block| block.text().starts_with("Drawing"))
            .unwrap_or_else(|| panic!("no line in {shown:?}"));
        assert!(matches!(line.kind(), BlockKind::Paragraph), "{shown:?}");
        for (range, text, attrs) in line.text_spans() {
            assert!(attrs.italic, "{range:?} {text:?} of {shown:?}");
            assert!(
                !attrs.bold && !attrs.code && !attrs.strikethrough && attrs.link.is_none(),
                "{range:?} {text:?} of {shown:?}"
            );
        }
        // Nothing of the block's JSON is shown.
        assert!(!document.plain_text().contains("\"add\""), "{shown:?}");
        line.text().to_string()
    };

    // A title over a document.
    let title = "```whiteboard\n{\"title\": \"x\u{201d}: adds nothing. [Safe](https://e.x) **b**\", \
                 \"add\": [{\"type\": \"ellipse\", \"id\": \"a\", \"x\": 0, \"y\": 0}]}\n```\n";
    let line = check(title, DrawingTarget::NewWhiteboard, None);
    assert!(line.starts_with("Drawing: adds"), "{line}");
    assert!(line.contains("[Safe](https://e.x)"), "{line}");

    // An element's name and a link's host over a whiteboard.
    let names = "```whiteboard\n{\"add\": [{\"type\": \"rectangle\", \"id\": \"r\", \"x\": 0, \"y\": 0, \
                 \"link\": \"https://ex_*am*ple.com/a\"}], \"update\": [{\"id\": \"api\", \"dx\": 10}]}\n```\n";
    let line = check(names, DrawingTarget::OpenWhiteboard, Some(&Names));
    assert!(line.starts_with("Drawing: adds"), "{line}");
    assert!(line.contains("*A* _b_"), "{line}");

    // What the reader says of a block that is not JSON (its message has
    // backticks).
    let broken = "```whiteboard\n{\"add\": [}\n```\n";
    let line = check(broken, DrawingTarget::OpenWhiteboard, None);
    assert!(line.starts_with("Drawing that cannot be used"), "{line}");
}
