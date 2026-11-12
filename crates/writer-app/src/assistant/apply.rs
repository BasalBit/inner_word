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
use writer_core::{
    Block, BlockKind, Document, EditorState, Fragment, InlineAttrs, Position, TextRange,
};
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
///
/// An answer of one paragraph is running text: it goes into the block the
/// caret is in, which keeps its kind, and takes the character formatting
/// typed text would take there, with the answer's own emphasis on top. An
/// answer of anything else (several blocks, a heading, a list, a quote,
/// code) goes in as whole blocks at the caret: an empty block there is
/// replaced by them, a block with the caret inside it is split around
/// them, and nothing of the answer is merged into the neighbouring text.
pub fn insert_at_caret(editor: &Entity<Editor>, markdown: &str, cx: &mut App) -> bool {
    let fragment = markdown_fragment(markdown);
    if fragment.is_empty() || !can_edit(editor, cx) {
        return false;
    }
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.break_undo_group();
                state.grouped(|state| insert_fragment(state, &fragment))
            },
            cx,
        )
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

/// Puts `fragment` where the selection of `state` is. See
/// [`insert_at_caret`].
fn insert_fragment(state: &mut EditorState, fragment: &Fragment) -> bool {
    let start = state.selection().start();
    let in_code = state
        .document()
        .block(start.block)
        .is_some_and(|block| block.kind().is_code());
    let running_text =
        fragment.is_inline() && matches!(fragment.blocks()[0].kind(), BlockKind::Paragraph);
    if in_code {
        // Code takes the answer as plain text, as it takes a paste.
        return state.paste(fragment);
    }
    if running_text {
        let base = typed_formatting(state);
        return state.paste(&with_formatting_of(fragment, &base));
    }

    // Whole blocks. What is selected goes first, as with typing.
    if !state.selection().is_collapsed() {
        state.delete_selection();
    }
    let caret = state.selection().head;
    let (is_void, is_empty, length) = match state.document().block(caret.block) {
        Some(block) => (block.is_void(), block.is_empty(), block.len()),
        None => return false,
    };
    let replaces_empty = !is_void && is_empty;
    let index = if is_void || (caret.offset >= length && !replaces_empty) {
        caret.block + 1
    } else if caret.offset == 0 {
        caret.block
    } else {
        // In the middle of a block: the answer goes between its halves.
        state.insert_paragraph_break();
        caret.block + 1
    };
    if !state.insert_blocks_at(index, fragment) {
        return false;
    }
    let count = fragment.blocks().len();
    let last = index + count - 1;
    let end = Position::new(
        last,
        state.document().block(last).map_or(0, |block| block.len()),
    );
    if replaces_empty {
        // The empty block the caret was in is now under the answer: it
        // goes, the answer has taken its place.
        state.delete_range(TextRange::new(end, Position::new(last + 1, 0)));
    }
    state.select_range(TextRange::empty(end));
    true
}

/// The character formatting text typed at the selection of `state` would
/// take: that of the first character a selection replaces, or of the
/// character before a caret. A link is not carried on (an answer put after
/// a link is not part of it), and comment anchors never travel.
fn typed_formatting(state: &EditorState) -> InlineAttrs {
    let selection = state.selection();
    let start = selection.start();
    let Some(block) = state.document().block(start.block) else {
        return InlineAttrs::default();
    };
    let attrs = if selection.is_collapsed() {
        block.attrs_before(start.offset)
    } else {
        block.attrs_at(start.offset)
    };
    let mut base = attrs.cloned().unwrap_or_default();
    base.comments.clear();
    base.link = None;
    base
}

/// The single text block of `fragment` as a paragraph whose text carries
/// `base` under its own formatting: a style either has is on; a colour, a
/// font, a highlight or a link of the answer's own wins.
fn with_formatting_of(fragment: &Fragment, base: &InlineAttrs) -> Fragment {
    let Some(source) = fragment.blocks().first() else {
        return fragment.clone();
    };
    let mut block = Block::paragraph("");
    for (_, text, own) in source.text_spans() {
        block = block.with_span(
            text,
            InlineAttrs {
                bold: own.bold || base.bold,
                italic: own.italic || base.italic,
                underline: own.underline || base.underline,
                strikethrough: own.strikethrough || base.strikethrough,
                code: own.code || base.code,
                highlight: own.highlight.or(base.highlight),
                color: own.color.or(base.color),
                font: own.font.clone().or_else(|| base.font.clone()),
                link: own.link.clone(),
                comments: Default::default(),
            },
        );
    }
    Fragment::new(vec![block])
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
            |state| {
                state.break_undo_group();
                state.grouped(|state| state.insert_blocks_at(index, &fragment))
            },
            cx,
        )
    })
}
