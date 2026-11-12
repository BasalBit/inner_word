//! Putting an answer into the text.
//!
//! An answer is Markdown. It goes into the document as formatted text,
//! through the Markdown importer, through the editor's mutation seam
//! ([`Editor::edit`]) and as one undo step. Afterwards the text that
//! changed is selected, so the writer sees what the button did and one
//! `cmd-z` takes it back.
//!
//! - **Replace** puts the answer where the captured selection is. An
//!   answer of one paragraph is inline text: it goes into the paragraph
//!   the selection is in, which keeps its kind, and takes the character
//!   formatting of the first character it replaces (an answer for a
//!   phrase in a heading set in another font stays in that font), with
//!   the answer's own emphasis on top. An answer of several blocks is
//!   pasted as blocks, as the clipboard would paste them.
//! - **Insert below** puts the answer as whole blocks after the block the
//!   selection (or the caret) ended in, without touching that block.

use gpui::Context;
use writer_core::{Block, BlockKind, Document, Fragment, InlineAttrs, Position, TextRange};
use writer_editor::Editor;
use writer_export::{MarkdownImportOptions, from_markdown};

use crate::capture::Capture;

/// The blocks an answer becomes.
pub fn answer_fragment(markdown: &str) -> Fragment {
    from_markdown(markdown, &MarkdownImportOptions::default()).fragment()
}

/// Whether "Replace Selection" can run: text was selected when the chat
/// opened, it is still there unchanged, and the document may be edited.
pub fn can_replace(editor: &Editor, capture: &Capture) -> bool {
    !editor.is_read_only() && capture.is_intact(editor.document())
}

/// Whether "Insert Below" can run: the document may be edited.
pub fn can_insert(editor: &Editor) -> bool {
    !editor.is_read_only()
}

/// Replaces the captured selection with the answer and selects what was
/// put there. Returns the new selection, or `None` when nothing was done:
/// the captured text has changed or is gone, the answer is empty, or the
/// editor is read-only.
pub fn replace_selection(
    editor: &mut Editor,
    capture: &Capture,
    markdown: &str,
    cx: &mut Context<Editor>,
) -> Option<TextRange> {
    if editor.is_read_only() {
        return None;
    }
    let document = editor.document();
    let range = capture.intact_range(document)?;
    let mut fragment = answer_fragment(markdown);
    if fragment.is_empty() {
        return None;
    }
    if fragment.is_inline()
        && let Some(target) = document.block(range.start.block)
        && !target.kind().is_code()
    {
        fragment = with_formatting_of(&fragment, &replaced_formatting(document, range));
    }
    editor.edit(
        |state| {
            state.break_undo_group();
            state.grouped(|state| {
                state.select_range(range);
                if !state.paste(&fragment) {
                    return None;
                }
                let changed = TextRange::new(range.start, state.selection().end());
                state.select_range(changed);
                Some(changed)
            })
        },
        cx,
    )
}

/// Inserts the answer as new blocks after the block the captured
/// selection (or caret) ended in, and selects them. When that block is
/// gone, after the block the caret is in now. Returns the new selection,
/// or `None` when nothing was done (an empty answer, a read-only editor).
pub fn insert_below(
    editor: &mut Editor,
    capture: &Capture,
    markdown: &str,
    cx: &mut Context<Editor>,
) -> Option<TextRange> {
    if editor.is_read_only() {
        return None;
    }
    let fragment = answer_fragment(markdown);
    if fragment.is_empty() {
        return None;
    }
    let after = capture
        .end_block_index(editor.document())
        .unwrap_or_else(|| editor.selection().end().block);
    let first = after + 1;
    let count = fragment.blocks().len();
    editor.edit(
        |state| {
            state.break_undo_group();
            state.grouped(|state| {
                if !state.insert_blocks_at(first, &fragment) {
                    return None;
                }
                let last = first + count - 1;
                let end = state.document().block(last).map_or(0, Block::len);
                let inserted = TextRange::new(Position::new(first, 0), Position::new(last, end));
                state.select_range(inserted);
                Some(inserted)
            })
        },
        cx,
    )
}

/// The character formatting that text replacing `range` takes: that of
/// the first character that goes, as typed text takes it. The link is
/// taken only when the whole range carries it (a word inside a link stays
/// part of the link; a selection that merely starts in one does not turn
/// into a link). Comment anchors never travel.
fn replaced_formatting(document: &Document, range: TextRange) -> InlineAttrs {
    let Some(block) = document.block(range.start.block) else {
        return InlineAttrs::default();
    };
    let mut base = block
        .attrs_at(range.start.offset)
        .cloned()
        .unwrap_or_default();
    base.comments.clear();
    if base.link.is_some() {
        let end = if range.end.block == range.start.block {
            range.end.offset
        } else {
            block.len()
        };
        let whole = range.end.block == range.start.block
            && block
                .spans()
                .filter(|(span, _)| span.start < end && span.end > range.start.offset)
                .all(|(_, attrs)| attrs.link == base.link);
        if !whole {
            base.link = None;
        }
    }
    base
}

/// The single text block of `fragment` as a paragraph whose text carries
/// `base` under its own formatting.
fn with_formatting_of(fragment: &Fragment, base: &InlineAttrs) -> Fragment {
    let Some(source) = fragment.blocks().first() else {
        return fragment.clone();
    };
    let mut block = Block::new(BlockKind::Paragraph);
    for (_, text, attrs) in source.text_spans() {
        block = block.with_span(text, merged(attrs, base));
    }
    Fragment::new(vec![block])
}

/// `own` on top of `base`: a style either has is on; a colour, font,
/// highlight or link of its own wins.
fn merged(own: &InlineAttrs, base: &InlineAttrs) -> InlineAttrs {
    InlineAttrs {
        bold: own.bold || base.bold,
        italic: own.italic || base.italic,
        underline: own.underline || base.underline,
        strikethrough: own.strikethrough || base.strikethrough,
        code: own.code || base.code,
        highlight: own.highlight.or(base.highlight),
        color: own.color.or(base.color),
        font: own.font.clone().or_else(|| base.font.clone()),
        link: own.link.clone().or_else(|| base.link.clone()),
        comments: Default::default(),
    }
}
