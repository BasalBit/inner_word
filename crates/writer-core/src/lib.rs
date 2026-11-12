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
//! step cost in proportion to the blocks touched, never to the document
//! (inserting or removing a block also shifts the `Vec` of blocks, a
//! memmove of about 16 µs at 10,000 blocks).
//!
//! # Rules worth knowing
//!
//! - Positions are block indices plus byte offsets; re-read
//!   [`EditorState::selection`] after every command instead of keeping old
//!   positions. [`BlockId`]s are the stable handle.
//! - A void block (divider, page break) has one position, offset 0. A range
//!   with a void block at either end includes that block: deleting or
//!   copying it takes the divider along, and block commands touch it.
//! - A range that ends at offset 0 of a later text block does not include
//!   that block: block commands do not touch it
//!   ([`EditorState::touched_blocks`]), and when the range also starts at
//!   offset 0 (whole lines, as shift-down selects them) deleting it removes
//!   exactly those blocks and leaves the following block as it is. Any
//!   other range across blocks merges into its first block, which keeps
//!   its id and kind.
//! - Merging with an empty block removes the empty block: Backspace at the
//!   start of a paragraph below an empty block, or forward delete in an
//!   empty block above text, never changes the kind of the text.
//! - Typed text takes the pending attributes, else the formatting of the
//!   character before the caret, or of the first replaced character when
//!   it replaces a selection. Links and comment anchors continue only when
//!   typing strictly inside them.
//! - Pending attributes (from a toggle on a collapsed caret, Enter, or an
//!   input rule) apply to the next insertion and are dropped when the caret
//!   moves. Toggling one also ends the typing undo group.
//! - Comment anchors never travel through [`Fragment`]s; links do.
//! - Settings changes ([`EditorState::set_settings`]) are not undoable.
//! - The language a document is spell-checked in is a document setting
//!   ([`DocumentSettings::spelling_language`], a [`SpellingLanguage`];
//!   `None` follows the application). This crate only stores it: checking
//!   is done elsewhere.
//!
//! # Where to start
//!
//! - **Rendering** (editor, exporters): iterate [`Document::blocks`]; per
//!   block use [`Block::kind`], [`Block::alignment`], [`Block::indent`],
//!   [`Typography::size_of`] (the override or the kind's default size, from
//!   [`Document::settings`]) and [`Block::text_spans`] for the styled runs.
//!   Numbered items get their number from [`Document::numbered_ordinals`]
//!   (one pass over the visible range). Comment highlights come from the
//!   runs' [`InlineAttrs::comments`]; [`Document::range_in_block`] clips
//!   the selection or a search match to one block for drawing.
//! - **Input** (editor): [`EditorState::insert_text`] for typed text,
//!   [`EditorState::insert_paragraph_break`] (Enter),
//!   [`EditorState::insert_soft_break`] (shift-enter),
//!   [`EditorState::backspace`] / [`EditorState::delete_forward`] and the
//!   word and line variants, [`EditorState::move_caret`] with a
//!   [`Movement`], [`EditorState::set_selection`] for mouse and vertical
//!   moves, [`EditorState::select_word_at`] / [`EditorState::select_block`]
//!   for double and triple clicks, [`EditorState::set_todo_checked`] for a
//!   click on a to-do checkbox. IME: keep the marked range in the UI,
//!   update it with [`EditorState::replace_range`] inside
//!   [`EditorState::begin_undo_group`] / [`EditorState::end_undo_group`],
//!   and convert offsets with [`Block::utf16_to_offset`] /
//!   [`Block::offset_to_utf16`] (block-local). Call
//!   [`EditorState::break_undo_group`] on a typing pause.
//! - **Vim**: positions, [`Document::moved`], [`Document::block_range`] and
//!   the [`text`] helpers for motions and text objects;
//!   [`EditorState::delete_range`], [`EditorState::replace_range`]
//!   (keeps per-character formatting when the length in characters is the
//!   same), [`Document::fragment`] (charwise yank),
//!   [`Document::block_fragment`] (linewise yank),
//!   [`EditorState::paste`] (charwise put) and
//!   [`EditorState::insert_blocks_at`] (linewise put);
//!   [`EditorState::select_blocks`] before a block command for linewise
//!   operators; [`EditorState::grouped`] or
//!   [`EditorState::begin_undo_group`] / [`EditorState::end_undo_group`] to
//!   make an operator, an insert session or a repeat one undo step;
//!   [`Document::find_all`] and [`Document::flat_offset`] /
//!   [`Document::position_from_flat`] for searches.
//! - **Formatting UI**: [`EditorState::format_state`], the `toggle_*`,
//!   `set_*` and block commands; [`EditorState::touched_blocks`] tells which
//!   blocks a block command acts on.
//! - **Comments panel**: [`Document::all_comment_ranges`] once per refresh
//!   (it walks every block's runs), [`Document::comment`] for the thread,
//!   [`Document::is_orphaned`], [`Document::next_comment`] /
//!   [`Document::prev_comment`] for navigation.
//! - **Exporters**: blocks and runs as for rendering,
//!   [`Document::numbered_ordinals`] over the whole document,
//!   [`Document::all_comment_ranges`] and [`Document::comments`],
//!   [`Document::settings`], [`Highlight::reference_rgb`].
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
pub use document::{Document, TITLE_MAX_CHARS, TITLE_SENTENCE_CHARS, TITLE_TEXT_CHARS};
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
    DocumentSettings, HEADING_MAX_LINE_HEIGHT, Margins, Orientation, POINTS_PER_INCH, PageSetup,
    PaperSize, SpellingLanguage, Typography,
};
pub use stats::{Stats, WORDS_PER_MINUTE};
