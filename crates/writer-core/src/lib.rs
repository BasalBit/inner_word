//! Document model and headless editing engine for The Writer.
//!
//! Pure Rust: no UI, no IO, no clock. Everything is deterministic.
//!
//! # The model
//!
//! A [`Document`] is an ordered, never-empty list of [`Block`]s plus
//! [`CommentThread`]s and [`DocumentSettings`]. A block has a stable
//! [`BlockId`], a [`BlockKind`] (paragraph, heading 1–6, bullet / numbered /
//! to-do item, quote, code block, divider, page break), an [`Alignment`], an
//! indent level `0..=8`, an optional text size override, and text with
//! run-length encoded [`InlineAttrs`] ([`Run`]s). A `\n` inside block text
//! is a soft line break; a code block is one block with multi-line text.
//! Dividers and page breaks are *void* blocks: they have no text and a
//! single caret position, offset 0.
//!
//! Positions are `(block index, byte offset)` ([`Position`]). Offsets are
//! UTF-8 byte offsets on char boundaries; caret positions are also on
//! grapheme cluster boundaries. [`text`] has the grapheme, word and UTF-16
//! helpers the UI and IME code need.
//!
//! Comment threads are anchored through the `comments` attribute of the
//! commented text, so anchors move with edits, survive splits and merges,
//! and come back with undo. A thread whose text was deleted is kept and
//! reported by [`Document::is_orphaned`].
//!
//! # The editing engine
//!
//! [`EditorState`] owns the document, the [`Selection`], pending inline
//! attributes and the undo history. Every change goes through it, as a
//! transaction that undo restores exactly (document and selection). It
//! offers layout-free movement ([`Movement`]), text commands with
//! Enter/Backspace semantics for every block kind, inline and block
//! formatting, Markdown-style input rules and smart punctuation, clipboard
//! [`Fragment`]s, comments, search and replace, and statistics.
//!
//! Change tracking for the UI: [`EditorState::revision`] increases with every
//! change, [`EditorState::take_changes`] reports the changed block range
//! ([`BlockSplice`]), and every block carries a [`Block::revision`] stamp
//! that changes whenever its content does. Cache layout per block on
//! `(id, revision)`; no diffing is needed. Typing, moving and undoing one
//! step cost in proportion to the blocks touched, never to the document.
//!
//! # Where to start
//!
//! - **Rendering** (editor, exporters): iterate [`Document::blocks`]; per
//!   block use [`Block::kind`], [`Block::alignment`], [`Block::indent`],
//!   [`Block::text_size`] (else [`Typography::size_for`] from
//!   [`Document::settings`]), and [`Block::text_spans`] for the styled runs.
//!   Numbered items get their number from [`Document::numbered_ordinals`].
//!   Comment highlights come from [`Document::all_comment_ranges`].
//! - **Input** (editor): [`EditorState::insert_text`] for typed and IME text,
//!   [`EditorState::insert_paragraph_break`] (Enter),
//!   [`EditorState::insert_soft_break`] (shift-enter),
//!   [`EditorState::backspace`] / [`EditorState::delete_forward`] and the
//!   word and line variants, [`EditorState::move_caret`] with a
//!   [`Movement`], [`EditorState::set_selection`] for mouse and vertical
//!   moves, [`Block::utf16_to_offset`] / [`Block::offset_to_utf16`] for IME
//!   ranges. Call [`EditorState::break_undo_group`] on a typing pause.
//! - **Vim**: positions, [`Document::moved`] and the [`text`] helpers for
//!   motions; [`EditorState::delete_range`], [`EditorState::replace_range`],
//!   [`EditorState::insert_blocks_at`], [`Document::fragment`] and
//!   [`EditorState::paste`] for operators and registers;
//!   [`EditorState::begin_undo_group`] / [`EditorState::end_undo_group`] to
//!   make an operator, an insert session or a repeat one undo step;
//!   [`Document::flat_offset`] / [`Document::position_from_flat`] for
//!   searches across blocks.
//! - **Formatting UI**: [`EditorState::format_state`], the `toggle_*`,
//!   `set_*` and block commands; [`EditorState::touched_blocks`] tells which
//!   blocks a block command acts on.
//! - **Persistence**: [`Document::to_json`] / [`Document::from_json`];
//!   [`Document::plain_text`] and [`Document::title`] for the library index.
//!
//! # Example
//!
//! ```
//! use writer_core::{EditorState, Document, Movement, InlineStyle};
//!
//! let mut ed = EditorState::new(Document::new());
//! ed.insert_text("Hello");
//! ed.insert_text(" ");
//! ed.insert_text("world");
//! assert_eq!(ed.document().plain_text(), "Hello world");
//!
//! // Select "world" and make it bold.
//! ed.move_caret(Movement::WordLeft, true);
//! ed.toggle_style(InlineStyle::Bold);
//! assert!(ed.document().blocks()[0].attrs_at(6).unwrap().bold);
//!
//! // Markdown-style input rules: "# " at the start makes a heading.
//! ed.move_caret(Movement::DocStart, false);
//! ed.insert_text("#");
//! ed.insert_text(" ");
//! assert_eq!(ed.document().blocks()[0].kind().heading_level(), Some(1));
//!
//! // One undo gives back the literally typed "# ", the next removes it,
//! // the next removes the bold.
//! ed.undo();
//! assert_eq!(ed.document().plain_text(), "# Hello world");
//! ed.undo();
//! ed.undo();
//! assert!(!ed.document().blocks()[0].attrs_at(6).unwrap().bold);
//!
//! // Documents round-trip through JSON.
//! let json = ed.document().to_json();
//! assert_eq!(&Document::from_json(&json).unwrap(), ed.document());
//! ```

#![warn(missing_docs)]

mod attrs;
mod block;
mod comments;
mod document;
mod editor;
mod fragment;
mod history;
mod movement;
mod position;
mod runs;
pub mod serial;
mod settings;
mod stats;
mod search;
pub mod text;

pub use attrs::{CommentId, Highlight, InlineAttrs, InlineStyle, Rgb};
pub use block::{
    Alignment, Block, BlockId, BlockKind, MAX_INDENT, MAX_TEXT_SIZE, MIN_TEXT_SIZE,
    TEXT_SIZE_SCALE, sanitize_text_size,
};
pub use comments::{CommentThread, Reply, ReplyId, Timestamp};
pub use document::{Document, TITLE_MAX_CHARS};
pub use editor::{
    BlockSplice, Change, Choice, EditorOptions, EditorState, FormatState, LinkInfo, Toggle,
};
pub use fragment::Fragment;
pub use history::{DEFAULT_UNDO_LIMIT, UNDO_BYTE_BUDGET};
pub use movement::Movement;
pub use position::{Position, Selection, TextRange};
pub use runs::Run;
pub use search::SearchOptions;
pub use serial::LoadError;
pub use settings::{
    DocumentSettings, Margins, Orientation, POINTS_PER_INCH, PageSetup, PaperSize, Typography,
};
pub use stats::{Stats, WORDS_PER_MINUTE};
