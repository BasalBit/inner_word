//! The headless editing engine.

mod blocks;
mod clipboard;
mod comments;
mod format;
mod rules;
mod search_cmds;
mod text_cmds;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::ops::Range;

pub use format::{Choice, FormatState, LinkInfo, Toggle};

use crate::block::{Block, BlockId};
use crate::comments::CommentThread;
use crate::attrs::CommentId;
use crate::document::Document;
use crate::history::{EditKind, Entry, History, Op, push_op};
use crate::movement::Movement;
use crate::position::{Position, Selection, TextRange};
use crate::settings::DocumentSettings;
use crate::stats::Stats;
use crate::attrs::InlineAttrs;
use crate::text::{self, CharClass};

/// Switches for automatic text transformations while typing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EditorOptions {
    /// Markdown-style input rules (`# ` heading, `- ` bullet, `**bold**`, …).
    pub input_rules: bool,
    /// Smart punctuation (curly quotes, `--` em dash, `...` ellipsis).
    pub smart_punctuation: bool,
}

impl Default for EditorOptions {
    fn default() -> EditorOptions {
        EditorOptions {
            input_rules: true,
            smart_punctuation: true,
        }
    }
}

/// A changed range of blocks: `old_len` blocks starting at `start` (in the
/// document before the change) were replaced by `new_len` blocks (in the
/// document after it). Blocks outside the range are untouched apart from
/// shifting index by `new_len - old_len`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockSplice {
    /// First affected index (same in old and new document).
    pub start: usize,
    /// Number of blocks replaced in the old document.
    pub old_len: usize,
    /// Number of blocks in their place in the new document.
    pub new_len: usize,
}

impl BlockSplice {
    /// The affected indices in the old document.
    pub fn old_range(&self) -> Range<usize> {
        self.start..self.start + self.old_len
    }

    /// The affected indices in the new document.
    pub fn new_range(&self) -> Range<usize> {
        self.start..self.start + self.new_len
    }

    /// Composes `first` followed by `second` into one splice.
    pub fn then(first: BlockSplice, second: BlockSplice) -> BlockSplice {
        let a = first;
        let b = second;
        let s = a.start.min(b.start);
        // End of the union, in coordinates after `a`.
        let e1 = (a.start + a.new_len).max(b.start + b.old_len);
        let orig_end = e1 - a.new_len + a.old_len;
        let new_end = e1 - b.old_len + b.new_len;
        BlockSplice {
            start: s,
            old_len: orig_end - s,
            new_len: new_end - s,
        }
    }

    fn combine(a: Option<BlockSplice>, b: BlockSplice) -> BlockSplice {
        match a {
            None => b,
            Some(a) => BlockSplice::then(a, b),
        }
    }
}

/// What a transaction (or a batch of them) changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Change {
    /// The document revision after the change.
    pub revision: u64,
    /// The changed blocks, if any block changed.
    pub blocks: Option<BlockSplice>,
    /// Whether comment threads changed or text carrying comment anchors was
    /// edited (the comments panel may need a refresh).
    pub comments: bool,
    /// Whether the document settings changed.
    pub settings: bool,
}

impl Change {
    /// Merges a later change into this one.
    pub fn merge(self, later: Change) -> Change {
        Change {
            revision: later.revision.max(self.revision),
            blocks: match (self.blocks, later.blocks) {
                (Some(a), Some(b)) => Some(BlockSplice::then(a, b)),
                (a, b) => a.or(b),
            },
            comments: self.comments || later.comments,
            settings: self.settings || later.settings,
        }
    }
}

/// A transaction being built.
#[derive(Clone, Debug)]
struct Tx {
    kind: EditKind,
    before: Selection,
    ops: Vec<Op>,
    op_count: usize,
    splice: Option<BlockSplice>,
    comments: bool,
    first_class: Option<CharClass>,
    last_class: Option<CharClass>,
    /// Whether the command set the pending attributes on purpose.
    pending_set: bool,
}

/// The headless editor: a document, a selection, pending inline attributes
/// and the undo history. Every change to the document goes through it, so
/// the invariants and undo/redo always hold.
///
/// Mutating commands return `true` when the document changed. Out-of-range
/// positions are clamped; no command panics.
///
/// See the [crate documentation](crate) for an overview.
#[derive(Clone, Debug)]
pub struct EditorState {
    doc: Document,
    selection: Selection,
    pending: Option<InlineAttrs>,
    options: EditorOptions,
    history: History,
    tx: Option<Box<Tx>>,
    revision: u64,
    last_change: Option<Change>,
    unread: Option<Change>,
    stats_cache: HashMap<BlockId, (u64, Stats)>,
}

impl Default for EditorState {
    fn default() -> EditorState {
        EditorState::new(Document::new())
    }
}

impl EditorState {
    /// An editor over `doc` with the caret at the start.
    pub fn new(doc: Document) -> EditorState {
        EditorState {
            doc,
            selection: Selection::default(),
            pending: None,
            options: EditorOptions::default(),
            history: History::default(),
            tx: None,
            revision: 0,
            last_change: None,
            unread: None,
            stats_cache: HashMap::new(),
        }
    }

    /// The document.
    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// Consumes the editor, returning the document.
    pub fn into_document(self) -> Document {
        self.doc
    }

    /// The selection (always valid for the current document).
    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// The input-rule switches.
    pub fn options(&self) -> EditorOptions {
        self.options
    }

    /// Changes the input-rule switches.
    pub fn set_options(&mut self, options: EditorOptions) {
        self.options = options;
    }

    /// Attributes the next inserted text will take, if set explicitly (by a
    /// formatting toggle with a collapsed caret, or carried over by Enter or
    /// an input rule). Dropped when the caret moves.
    pub fn pending_attrs(&self) -> Option<&InlineAttrs> {
        self.pending.as_ref()
    }

    /// The attributes the next typed character would get at the caret: the
    /// pending attributes, else those inherited from the neighbouring text.
    pub fn current_attrs(&self) -> InlineAttrs {
        if let Some(p) = &self.pending {
            let mut p = p.clone();
            p.comments.retain(|id| self.doc.comments.contains_key(id));
            return p;
        }
        let h = self.selection.head;
        inherited_attrs(&self.doc.blocks[h.block], h.offset)
    }

    /// Takes the pending attributes for an insertion, dropping anchors of
    /// threads that no longer exist (a thread can be deleted while the
    /// caret, and so the pending attributes, stay put).
    pub(crate) fn take_pending(&mut self) -> Option<InlineAttrs> {
        let mut p = self.pending.take()?;
        p.comments.retain(|id| self.doc.comments.contains_key(id));
        Some(p)
    }

    /// The attributes text inserted at `pos` gets: the pending attributes
    /// (consumed), else those inherited from the neighbours.
    pub(crate) fn insertion_attrs(&mut self, pos: Position) -> InlineAttrs {
        self.take_pending()
            .unwrap_or_else(|| inherited_attrs(&self.doc.blocks[pos.block], pos.offset))
    }

    /// The document revision: increases by one with every transaction that
    /// changed the document (undo and redo included) and with every settings
    /// change. Starts at 0.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// What the most recent transaction changed.
    pub fn last_change(&self) -> Option<Change> {
        self.last_change
    }

    /// Everything that changed since the previous call, merged into one
    /// [`Change`]; `None` when nothing changed.
    pub fn take_changes(&mut self) -> Option<Change> {
        self.unread.take()
    }

    // ----- selection ----------------------------------------------------------------

    /// Sets the selection (clamped to valid caret positions). When it
    /// actually moves, pending attributes are dropped and the current undo
    /// group is closed.
    pub fn set_selection(&mut self, selection: Selection) {
        let s = self.normalized(selection);
        if s != self.selection {
            self.selection = s;
            self.pending = None;
            self.history.close();
        }
    }

    /// Places a caret at `pos` (clamped).
    pub fn set_caret(&mut self, pos: Position) {
        self.set_selection(Selection::caret(pos));
    }

    /// Moves the caret, or extends the selection when `extend` is set. A
    /// non-extending move with a selection starts from the selection end the
    /// movement points to; plain left/right just collapse the selection.
    pub fn move_caret(&mut self, movement: Movement, extend: bool) {
        let sel = self.selection;
        let new = if !extend && !sel.is_collapsed() {
            match movement {
                Movement::Left => Selection::caret(sel.start()),
                Movement::Right => Selection::caret(sel.end()),
                m => {
                    let from = if m.is_backward() { sel.start() } else { sel.end() };
                    Selection::caret(self.doc.moved(from, m))
                }
            }
        } else {
            let h = self.doc.moved(sel.head, movement);
            if extend {
                Selection::new(sel.anchor, h)
            } else {
                Selection::caret(h)
            }
        };
        self.set_selection(new);
    }

    /// Selects the whole document.
    pub fn select_all(&mut self) {
        self.set_selection(Selection::new(self.doc.start(), self.doc.end()));
    }

    /// Selects the word (or run of spaces, or punctuation mark) at `pos`, as
    /// a double click does.
    pub fn select_word_at(&mut self, pos: Position) {
        let p = self.doc.clamp_position(pos);
        let r = text::segment_at(&self.doc.blocks[p.block].text, p.offset);
        self.set_selection(Selection::new(
            Position::new(p.block, r.start),
            Position::new(p.block, r.end),
        ));
    }

    /// Selects the whole text of block `index` (clamped).
    pub fn select_block(&mut self, index: usize) {
        let i = index.min(self.doc.blocks.len() - 1);
        let len = self.doc.blocks[i].len();
        self.set_selection(Selection::new(Position::new(i, 0), Position::new(i, len)));
    }

    /// Selects `range` (clamped), head at the end.
    pub fn select_range(&mut self, range: TextRange) {
        self.set_selection(Selection::from_range(range));
    }

    /// Selects the whole blocks `range` (clamped; an empty range selects
    /// the block at `range.start`): from the start of the first to the end
    /// of the last, so [`EditorState::touched_blocks`] is exactly `range`.
    /// Linewise Vim operators use this before a block command.
    pub fn select_blocks(&mut self, range: Range<usize>) {
        let n = self.doc.blocks.len();
        let start = range.start.min(n - 1);
        let last = range.end.saturating_sub(1).clamp(start, n - 1);
        let len = self.doc.blocks[last].len();
        self.set_selection(Selection::new(
            Position::new(start, 0),
            Position::new(last, len),
        ));
    }

    // ----- undo -------------------------------------------------------------------

    /// Undoes the last undo step. Returns whether anything was undone.
    /// Closes an open undo group first.
    pub fn undo(&mut self) -> bool {
        if self.tx.is_some() {
            return false;
        }
        self.history.force_end_group();
        let Some(entry) = self.history.pop_undo() else {
            return false;
        };
        let (ops, splice, comments) = self.apply_ops(entry.ops);
        self.selection = self.normalized(entry.before);
        self.pending = None;
        self.history.redo.push(Entry {
            ops,
            open: false,
            ..entry
        });
        self.record_change(splice, comments, false);
        true
    }

    /// Redoes the last undone step. Returns whether anything was redone.
    pub fn redo(&mut self) -> bool {
        if self.tx.is_some() {
            return false;
        }
        self.history.force_end_group();
        let Some(entry) = self.history.redo.pop() else {
            return false;
        };
        let (ops, splice, comments) = self.apply_ops(entry.ops);
        self.selection = self.normalized(entry.after);
        self.pending = None;
        self.history.push_redone(Entry { ops, ..entry });
        self.record_change(splice, comments, false);
        true
    }

    /// Whether [`EditorState::undo`] would do something.
    pub fn can_undo(&self) -> bool {
        !self.history.undo.is_empty()
    }

    /// Whether [`EditorState::redo`] would do something.
    pub fn can_redo(&self) -> bool {
        !self.history.redo.is_empty()
    }

    /// Number of undo steps available.
    pub fn undo_depth(&self) -> usize {
        self.history.undo.len()
    }

    /// Ends the current undo group so the next edit starts a new step. The
    /// UI calls this on a typing pause timer.
    pub fn break_undo_group(&mut self) {
        self.history.close();
    }

    /// Starts grouping: every command until the matching
    /// [`EditorState::end_undo_group`] becomes one undo step. Calls nest.
    /// Used by the Vim engine for operators, insert sessions and repeat.
    pub fn begin_undo_group(&mut self) {
        self.history.begin_group();
    }

    /// Ends a group started by [`EditorState::begin_undo_group`].
    pub fn end_undo_group(&mut self) {
        self.history.end_group();
    }

    /// Runs `f` as one undo step.
    pub fn grouped<R>(&mut self, f: impl FnOnce(&mut EditorState) -> R) -> R {
        self.begin_undo_group();
        let r = f(self);
        self.end_undo_group();
        r
    }

    /// Sets the maximum number of undo steps kept (at least 1). Independently
    /// of it, the oldest steps are dropped once the history holds more than
    /// about [`UNDO_BYTE_BUDGET`](crate::UNDO_BYTE_BUDGET) bytes.
    pub fn set_undo_limit(&mut self, limit: usize) {
        self.history.limit = limit.max(1);
        self.history.trim();
    }

    // ----- settings, stats, invariants -----------------------------------------------

    /// Replaces the document settings (sanitised). Settings changes are not
    /// part of the undo history (they are view and page preferences), but
    /// they bump the revision and are reported with `settings: true`.
    pub fn set_settings(&mut self, settings: DocumentSettings) -> bool {
        let s = settings.sanitized();
        if s == self.doc.settings {
            return false;
        }
        self.doc.set_settings_raw(s);
        self.record_change(None, false, true);
        true
    }

    /// Shows or hides the document paginated (see [`EditorState::set_settings`]).
    pub fn set_page_view(&mut self, on: bool) -> bool {
        let mut s = self.doc.settings.clone();
        s.page_view = on;
        self.set_settings(s)
    }

    /// Statistics for the whole document. Per-block results are cached on
    /// `(id, revision)`, so repeated calls cost a lookup per block plus the
    /// blocks that changed.
    pub fn stats(&mut self) -> Stats {
        let mut total = Stats::default();
        for b in &self.doc.blocks {
            let s = match self.stats_cache.get(&b.id) {
                Some((rev, s)) if *rev == b.revision => *s,
                _ => {
                    let s = Stats::of_text(&b.text);
                    self.stats_cache.insert(b.id, (b.revision, s));
                    s
                }
            };
            total += s;
        }
        if self.stats_cache.len() > 2 * self.doc.blocks.len() + 64 {
            let live: std::collections::HashSet<BlockId> =
                self.doc.blocks.iter().map(|b| b.id).collect();
            self.stats_cache.retain(|id, _| live.contains(id));
        }
        total.finish()
    }

    /// Statistics for the selected text.
    pub fn selection_stats(&self) -> Stats {
        self.doc.stats_in(self.selection.range())
    }

    /// Checks the document invariants and that the selection is valid.
    pub fn check_invariants(&self) -> Result<(), String> {
        self.doc.check_invariants()?;
        for p in [self.selection.anchor, self.selection.head] {
            if !self.doc.is_valid_position(p) {
                return Err(format!("invalid selection position {p:?}"));
            }
        }
        Ok(())
    }

    // ----- transaction machinery (crate-private) ---------------------------------

    fn normalized(&self, s: Selection) -> Selection {
        Selection::new(self.doc.clamp_position(s.anchor), self.doc.clamp_position(s.head))
    }

    /// Runs `f` as one transaction of `kind` (or as part of the enclosing
    /// one). Returns whether `f` changed the document.
    pub(crate) fn edit(&mut self, kind: EditKind, f: impl FnOnce(&mut EditorState)) -> bool {
        let outer = self.tx.is_none();
        if outer {
            self.tx = Some(Box::new(Tx {
                kind,
                before: self.selection,
                ops: Vec::new(),
                op_count: 0,
                splice: None,
                comments: false,
                first_class: None,
                last_class: None,
                pending_set: false,
            }));
        }
        let before = self.tx_mut().op_count;
        f(self);
        let changed = self.tx_mut().op_count > before;
        if outer {
            let tx = self.tx.take().expect("transaction present");
            self.finish(*tx);
        }
        changed
    }

    fn tx_mut(&mut self) -> &mut Tx {
        self.tx.as_mut().expect("inside a transaction")
    }

    pub(crate) fn set_tx_classes(&mut self, first: Option<CharClass>, last: Option<CharClass>) {
        let tx = self.tx_mut();
        tx.first_class = first;
        tx.last_class = last;
    }

    fn finish(&mut self, tx: Tx) {
        self.selection = self.normalized(self.selection);
        // Pending attributes belong to a caret position: a command that moved
        // the caret drops them unless it set them on purpose.
        if !tx.pending_set && self.selection != tx.before {
            self.pending = None;
        }
        if tx.ops.is_empty() {
            return;
        }
        self.record_change(tx.splice, tx.comments, false);
        self.history.push(Entry {
            ops: tx.ops,
            before: tx.before,
            after: self.selection,
            kind: tx.kind,
            first_class: tx.first_class,
            last_class: tx.last_class,
            open: false,
            bytes: 0,
        });
        #[cfg(debug_assertions)]
        self.debug_check_local(tx.splice);
    }

    #[cfg(debug_assertions)]
    fn debug_check_local(&self, splice: Option<BlockSplice>) {
        if let Some(s) = splice {
            let from = s.start.saturating_sub(1);
            let to = (s.start + s.new_len + 1).min(self.doc.blocks.len());
            for i in from..to {
                if let Err(e) = self.doc.blocks[i].check() {
                    panic!("writer-core invariant violated after edit: {e}");
                }
            }
        }
        debug_assert!(!self.doc.blocks.is_empty());
        debug_assert!(self.doc.is_valid_position(self.selection.anchor));
        debug_assert!(self.doc.is_valid_position(self.selection.head));
    }

    fn record_change(&mut self, splice: Option<BlockSplice>, comments: bool, settings: bool) {
        self.revision += 1;
        let c = Change {
            revision: self.revision,
            blocks: splice,
            comments,
            settings,
        };
        self.unread = Some(match self.unread.take() {
            Some(u) => u.merge(c),
            None => c,
        });
        self.last_change = Some(c);
    }

    fn apply_ops(&mut self, ops: Vec<Op>) -> (Vec<Op>, Option<BlockSplice>, bool) {
        let mut inverse = Vec::with_capacity(ops.len());
        let mut splice = None;
        let mut comments = false;
        for op in ops.into_iter().rev() {
            match op {
                Op::Splice {
                    index,
                    remove,
                    insert,
                } => {
                    let new_len = insert.len();
                    comments |= insert.iter().any(has_anchors);
                    let removed = self.doc.splice_raw(index, remove, insert);
                    comments |= removed.iter().any(has_anchors);
                    splice = Some(BlockSplice::combine(
                        splice,
                        BlockSplice {
                            start: index,
                            old_len: remove,
                            new_len,
                        },
                    ));
                    inverse.push(Op::Splice {
                        index,
                        remove: new_len,
                        insert: removed,
                    });
                }
                Op::Thread { id, thread } => {
                    let prev = self.doc.set_thread_raw(id, thread);
                    comments = true;
                    inverse.push(Op::Thread { id, thread: prev });
                }
            }
        }
        (inverse, splice, comments)
    }

    /// Replaces `remove` blocks at `index` with `insert`, recording the inverse.
    pub(crate) fn splice(&mut self, index: usize, remove: usize, insert: Vec<Block>) {
        let new_len = insert.len();
        let mut comments = insert.iter().any(has_anchors);
        let removed = self.doc.splice_raw(index, remove, insert);
        comments |= removed.iter().any(has_anchors);
        let tx = self.tx_mut();
        push_op(
            &mut tx.ops,
            Op::Splice {
                index,
                remove: new_len,
                insert: removed,
            },
        );
        tx.op_count += 1;
        tx.splice = Some(BlockSplice::combine(
            tx.splice,
            BlockSplice {
                start: index,
                old_len: remove,
                new_len,
            },
        ));
        tx.comments |= comments;
    }

    /// Edits a copy of block `index` with `f` and stores it if it changed.
    pub(crate) fn update_block(&mut self, index: usize, f: impl FnOnce(&mut Block)) {
        let mut b = self.doc.blocks[index].clone();
        f(&mut b);
        if b == self.doc.blocks[index] {
            return;
        }
        b.mark_dirty();
        self.splice(index, 1, vec![b]);
    }

    /// Sets or deletes a comment thread, recording the inverse.
    pub(crate) fn set_thread(&mut self, id: CommentId, thread: Option<CommentThread>) {
        let prev = self.doc.set_thread_raw(id, thread);
        let tx = self.tx_mut();
        tx.ops.push(Op::Thread { id, thread: prev });
        tx.op_count += 1;
        tx.comments = true;
    }

    /// Sets the pending attributes from inside a command, keeping them even
    /// though the command moves the caret.
    pub(crate) fn set_pending(&mut self, attrs: InlineAttrs) {
        self.pending = Some(attrs);
        if let Some(tx) = self.tx.as_mut() {
            tx.pending_set = true;
        }
    }

    /// Sets the selection from inside a command (no group closing).
    pub(crate) fn set_sel(&mut self, s: Selection) {
        self.selection = s;
    }

    pub(crate) fn caret_to(&mut self, p: Position) {
        self.selection = Selection::caret(p);
    }

    /// The block range commands that act "on every block the selection
    /// touches" use. A selection that ends at offset 0 of a later text
    /// block does not touch that block; a selection that ends on a void
    /// block does (its only position is offset 0, and a range ending there
    /// includes it, see [`EditorState::delete_selection`]).
    pub fn touched_blocks(&self) -> Range<usize> {
        let r = self.selection.range();
        let mut end = r.end.block;
        if r.end.block > r.start.block && r.end.offset == 0 && !self.doc.blocks[end].is_void() {
            end -= 1;
        }
        r.start.block..end + 1
    }

    /// Byte ranges of the selection in every touched non-void block, in
    /// order; code blocks are included only with `include_code`.
    pub(crate) fn selected_ranges(&self, include_code: bool) -> Vec<(usize, Range<usize>)> {
        let r = self.selection.range();
        let mut out = Vec::new();
        for i in r.start.block..=r.end.block {
            let b = &self.doc.blocks[i];
            if b.is_void() || (!include_code && b.kind.is_code()) {
                continue;
            }
            let from = if i == r.start.block { r.start.offset } else { 0 };
            let to = if i == r.end.block { r.end.offset } else { b.len() };
            if from < to {
                out.push((i, from..to));
            }
        }
        out
    }
}

fn has_anchors(b: &Block) -> bool {
    b.runs.iter().any(|r| !r.attrs.comments.is_empty())
}

/// The attributes new text inherits at `offset` of `block`: those of the
/// preceding character (else the following one); the link and comment
/// anchors only when both neighbours share them (typing strictly inside).
pub(crate) fn inherited_attrs(block: &Block, offset: usize) -> InlineAttrs {
    let before = block.attrs_before(offset);
    let after = block.attrs_at(offset);
    let mut a = before.or(after).cloned().unwrap_or_default();
    match (before, after) {
        (Some(b), Some(f)) => {
            if b.link != f.link {
                a.link = None;
            }
            a.comments = b.comments.intersection(&f.comments).copied().collect();
        }
        _ => {
            a.link = None;
            a.comments.clear();
        }
    }
    if block.kind.is_code() {
        a = a.code_block_subset();
    }
    a
}
