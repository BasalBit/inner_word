//! Markdown to the document an answer shows.

use writer_assistant_ui::{AnswerStyle, answer_document, answer_plain_text, markdown::settle};
use writer_core::BlockKind;

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
