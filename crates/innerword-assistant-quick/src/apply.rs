//! Putting an answer into the text.
//!
//! An answer is Markdown. It goes into the document as formatted text,
//! through the Markdown importer, through the editor's mutation seam
//! ([`Editor::edit`]) and as one undo step. Afterwards the text that
//! changed is selected, so the writer sees what the button did and one
//! `cmd-z` takes it back.
//!
//! **Replace** puts the answer where the captured selection is:
//!
//! - An answer of one paragraph is running text. It goes into the block
//!   the selection starts in, which keeps its kind (a phrase of a heading
//!   that is replaced is still in that heading), and takes the character
//!   formatting of the first character it replaces, with the answer's own
//!   emphasis on top. So does an answer of one block of another kind (a
//!   lone list item, a lone heading) for a selection inside one block:
//!   replacing a few words never breaks their paragraph apart.
//! - Any other answer (several blocks, or one heading, list item, quote
//!   or code block for a selection of whole blocks) goes in as whole
//!   blocks, with the kinds the answer gave them: selected blocks are
//!   replaced by them, a block the selection was only a part of is split
//!   around them, and nothing of the answer is merged into the
//!   neighbouring text. This is what the side chat's Replace Selection
//!   does too.
//! - Inside a code block the answer is plain text, as a paste is.
//!
//! **Insert below** puts the answer as whole blocks after the block the
//! selection (or the caret) ended in, without touching that block.
//!
//! **Images.** A line `![description](image:<id>)` in an answer is an
//! image the document holds already: it comes back as that image, with
//! its width and its alignment ([`innerword_export::KnownImages`]). Any
//! other image in an answer (a path, an address) stays a link: **no file
//! is ever read because an answer names it**. The engine has no file
//! tools, so a path in an answer is invented or was planted in text the
//! model read.

use gpui::Context;
use innerword_core::{
    Block, BlockKind, Document, EditorState, Fragment, InlineAttrs, Position, TextRange,
};
use innerword_editor::Editor;
use innerword_export::{
    KnownImages, MarkdownFrontMatter, MarkdownImportOptions, from_markdown_with_images,
};

use crate::capture::Capture;

/// The blocks an answer becomes, for `document`: an image of the
/// document that the answer names (`image:<id>`) is that image again, as
/// the document has it; every other image is a link (see the
/// [module docs](self)).
pub fn answer_fragment(markdown: &str, document: &Document) -> Fragment {
    let options = MarkdownImportOptions {
        // An answer that opens with a rule is a rule, never data about
        // the text that would be left out.
        front_matter: MarkdownFrontMatter::Markdown,
        ..MarkdownImportOptions::default()
    };
    let mut images = KnownImages::of(document);
    from_markdown_with_images(markdown, &options, &mut images).fragment()
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

/// How an answer takes the place of a selection.
enum Replacement {
    /// Pasted into the block: running text, or plain text in code.
    Inline(Fragment),
    /// As whole blocks.
    Blocks(Fragment),
}

/// Replaces the captured selection with the answer and selects what was
/// put there (see the [module docs](self) for where an answer of several
/// blocks goes). Returns the new selection, or `None` when nothing was
/// done: the captured text has changed or is gone, the answer is empty,
/// or the editor is read-only.
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
    let fragment = answer_fragment(markdown, document);
    if fragment.is_empty() {
        return None;
    }
    let target = document.block(range.start.block)?;
    let within_one_block = range.start.block == range.end.block;
    let whole_blocks = range.start.offset == 0
        && document
            .block(range.end.block)
            .is_some_and(|block| range.end.offset == block.len());
    let running_text = fragment.is_inline()
        && (matches!(fragment.blocks()[0].kind(), BlockKind::Paragraph)
            || (within_one_block && !whole_blocks));
    let replacement = if target.kind().is_code() {
        Replacement::Inline(fragment)
    } else if running_text {
        Replacement::Inline(with_formatting_of(
            &fragment,
            &replaced_formatting(document, range),
        ))
    } else {
        Replacement::Blocks(fragment)
    };
    editor.edit(
        |state| {
            state.break_undo_group();
            state.grouped(|state| {
                state.select_range(range);
                let changed = match &replacement {
                    Replacement::Inline(fragment) => {
                        if !state.paste(fragment) {
                            return None;
                        }
                        TextRange::new(range.start, state.selection().end())
                    }
                    Replacement::Blocks(fragment) => replace_with_blocks(state, fragment)?,
                };
                state.select_range(changed);
                Some(changed)
            })
        },
        cx,
    )
}

/// Puts the blocks of `fragment` in the place of the selection of `state`,
/// as whole blocks: what is selected goes; a block that is empty then is
/// replaced by them, a block with the caret inside it is split around
/// them. Returns the range of what was put in.
fn replace_with_blocks(state: &mut EditorState, fragment: &Fragment) -> Option<TextRange> {
    if !state.selection().is_collapsed() {
        state.delete_selection();
    }
    let caret = state.selection().head;
    let (is_void, is_empty, length) = {
        let block = state.document().block(caret.block)?;
        (block.is_void(), block.is_empty(), block.len())
    };
    let replaces_empty = !is_void && is_empty;
    let first = if is_void || (caret.offset >= length && !replaces_empty) {
        caret.block + 1
    } else if caret.offset == 0 {
        caret.block
    } else {
        // In the middle of a block: the answer goes between its halves.
        state.insert_paragraph_break();
        caret.block + 1
    };
    if !state.insert_blocks_at(first, fragment) {
        return None;
    }
    let last = first + fragment.blocks().len() - 1;
    let end = Position::new(last, state.document().block(last).map_or(0, Block::len));
    if replaces_empty {
        // The empty block the selection left is under the answer now: it
        // goes, the answer has taken its place.
        state.delete_range(TextRange::new(end, Position::new(last + 1, 0)));
    }
    Some(TextRange::new(Position::new(first, 0), end))
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
    let fragment = answer_fragment(markdown, editor.document());
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
