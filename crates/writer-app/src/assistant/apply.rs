//! What the assistant reads from the text and what the user applies to it.
//!
//! The assistant advises and the user applies: an answer changes the
//! document only when one of its buttons is pressed. Everything here is one
//! undo step, and an answer's Markdown becomes formatted text on the way in
//! (a heading is a heading, `**bold**` is bold), exactly as "paste" of
//! Markdown read by the importer would.
//!
//! | Function | Used for |
//! |---|---|
//! | [`document_markdown`] | the side chat's context: the open document as it is right now, not as it was last saved |
//! | [`selection_markdown`] | the quick chat's context: the selected text and nothing else |
//! | [`insert_at_caret`] | "Insert at Caret" |
//! | [`replace_selection`] | "Replace Selection" |
//! | [`insert_below`] | "Insert Below": whole blocks under the selection, which stays as it is |

use gpui::{App, Entity};
use writer_core::{Block, Document, Fragment, Selection, TextRange};
use writer_editor::Editor;
use writer_export::{
    ExportSource, MarkdownFrontMatter, MarkdownImportOptions, MarkdownOptions, from_markdown,
    to_markdown,
};

/// The Markdown the assistant is given for a document: comments are left
/// out (they are the writer's notes to themselves, not text), everything
/// else as the Markdown export writes it.
fn markdown_options() -> MarkdownOptions {
    MarkdownOptions::default()
}

/// The document in `editor` as Markdown, as it is at this moment.
pub fn document_markdown(editor: &Editor) -> String {
    let document = editor.document();
    to_markdown(&ExportSource::single("", document), &markdown_options())
}

/// The selected text of `editor` as Markdown; `None` when nothing is
/// selected (or only white space is).
///
/// A selection inside one block is given as running text, whatever the
/// block is: three words out of a heading are three words, not a heading.
/// A selection over several blocks keeps their kinds.
pub fn selection_markdown(editor: &Editor) -> Option<String> {
    let selection = editor.selection();
    if selection.is_collapsed() {
        return None;
    }
    let fragment = editor.document().fragment(selection.range());
    if fragment.is_empty() || fragment.to_plain_text().trim().is_empty() {
        return None;
    }
    let document = if fragment.is_inline() {
        Document::from_blocks(vec![as_paragraph(&fragment.blocks()[0])])
    } else {
        fragment.to_document()
    };
    let markdown = to_markdown(&ExportSource::single("", &document), &markdown_options());
    let markdown = markdown.trim_end_matches('\n').to_string();
    (!markdown.trim().is_empty()).then_some(markdown)
}

/// The text of `block` with its inline formatting, as a paragraph.
fn as_paragraph(block: &Block) -> Block {
    block
        .text_spans()
        .fold(Block::paragraph(""), |paragraph, (_, text, attrs)| {
            paragraph.with_span(text, attrs.clone())
        })
}

/// An answer's Markdown as the blocks it stands for. Empty for an answer
/// with nothing in it.
pub fn markdown_fragment(markdown: &str) -> Fragment {
    let options = MarkdownImportOptions {
        // An answer that opens with a rule is a rule, never data about
        // the text.
        front_matter: MarkdownFrontMatter::Markdown,
        ..MarkdownImportOptions::default()
    };
    from_markdown(markdown, &options).fragment()
}

/// Whether the text in `editor` can be changed right now.
pub fn can_edit(editor: &Entity<Editor>, cx: &App) -> bool {
    !editor.read(cx).is_read_only()
}

/// Whether `editor` has a selection that "Replace Selection" would replace.
pub fn has_selection(editor: &Entity<Editor>, cx: &App) -> bool {
    !editor.read(cx).selection().is_collapsed()
}

/// Inserts `markdown` as formatted text at the caret, replacing the
/// selection when there is one (as typing would). One undo step. Returns
/// whether anything was inserted.
pub fn insert_at_caret(editor: &Entity<Editor>, markdown: &str, cx: &mut App) -> bool {
    let fragment = markdown_fragment(markdown);
    if fragment.is_empty() || !can_edit(editor, cx) {
        return false;
    }
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.grouped(|state| state.paste(&fragment)), cx)
    })
}

/// Replaces the selection with `markdown` as formatted text. One undo
/// step. Does nothing without a selection (the caller offers the button
/// only while there is one) and returns whether the text changed.
pub fn replace_selection(editor: &Entity<Editor>, markdown: &str, cx: &mut App) -> bool {
    if !has_selection(editor, cx) {
        return false;
    }
    insert_at_caret(editor, markdown, cx)
}

/// Replaces `range` of the document (the selection the question was asked
/// about, kept by whoever asked) with `markdown` as formatted text, when
/// the text in that range is still `expected`. One undo step.
///
/// The quick chat keeps the range of the selection it was opened on: the
/// selection itself may be gone by the time the answer is applied (a click
/// in the text), and the answer still belongs to those words. When the
/// words are no longer there (the text was edited meanwhile) nothing is
/// replaced and `false` is returned.
pub fn replace_range(
    editor: &Entity<Editor>,
    range: TextRange,
    expected: &str,
    markdown: &str,
    cx: &mut App,
) -> bool {
    let fragment = markdown_fragment(markdown);
    if fragment.is_empty() || !can_edit(editor, cx) {
        return false;
    }
    let still_there = {
        let document = editor.read(cx).document();
        document.fragment(range).to_plain_text() == expected
    };
    if !still_there {
        return false;
    }
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.grouped(|state| {
                    state.set_selection(Selection::from_range(range));
                    state.paste(&fragment)
                })
            },
            cx,
        )
    })
}

/// Inserts `markdown` as whole blocks under the block the selection (or
/// the caret) ends in. The text that was asked about stays as it is; the
/// caret goes to the start of what was inserted. One undo step.
pub fn insert_below(editor: &Entity<Editor>, markdown: &str, cx: &mut App) -> bool {
    let fragment = markdown_fragment(markdown);
    if fragment.is_empty() || !can_edit(editor, cx) {
        return false;
    }
    editor.update(cx, |editor, cx| {
        let index = editor.selection().end().block + 1;
        editor.edit(
            |state| state.grouped(|state| state.insert_blocks_at(index, &fragment)),
            cx,
        )
    })
}
