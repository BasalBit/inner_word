//! Document layout: which block is where, for a document of any length.
//!
//! - [`style`]: attributes to fonts, colours and block metrics.
//! - [`block`]: one block's shaped lines, visual rows and hit testing.
//! - [`pages`]: placement of block boxes in flow view and on sheets.
//! - this module: [`DocumentLayout`], which ties them together.
//!
//! # Design
//!
//! A book has ten thousand blocks and a window shows thirty, so the layout
//! keeps two things apart:
//!
//! - A **slot** per block (a few dozen bytes): its [`Measure`] (row count,
//!   line height, spacing), whether that measure was really shaped or is an
//!   estimate from the text length, its word counts, and its placement.
//! - A **shaped layout** ([`BlockLayout`]: glyphs, rows) only for blocks
//!   that are on screen or were asked about recently, in a small cache
//!   keyed by `(BlockId, Block::revision)`.
//!
//! Nothing is laid out up front. A frame shapes the blocks in the viewport;
//! every other block counts with its estimate. A background task
//! ([`DocumentLayout::refine`], driven by the editor in slices of a few
//! milliseconds) replaces the estimates with real measures, so the scroll
//! range and the page count become exact shortly after a document opens
//! without ever blocking typing.
//!
//! Changes arrive as writer-core `BlockSplice`s
//! ([`DocumentLayout::apply_change`]): the slots of the changed range are
//! replaced, everything else is kept. Typing in one block reshapes one
//! block.
//!
//! **Placement** (block tops, rows on sheets) is computed lazily from the
//! slots up to a watermark and resumed from the first block whose measure
//! changed. Queries place as far as they need.
//!
//! **Scrolling** is anchored to a block (`ScrollPosition`), not stored in
//! pixels, so measuring or editing blocks above the viewport never moves
//! what is on screen.
//!
//! The environment (viewport, column, style, text system) is installed by
//! the element's prepaint and kept, so geometry queries between frames lay
//! out on demand: a consumer that edits and then asks for a caret position
//! in the same key handler (Vim does) gets an answer for the edited
//! document.

pub(crate) mod block;
pub(crate) mod pages;
pub(crate) mod style;

use std::{collections::HashMap, ops::Range, rc::Rc, sync::Arc, time::Instant};

use gpui::{Bounds, Pixels, Point, WindowTextSystem, point, px, size};
use writer_core::{Block, BlockId, BlockSplice, Document, Stats};

pub(crate) use block::{BlockLayout, Measure};
pub(crate) use pages::PageGeometry;
pub(crate) use style::LayoutStyle;

use pages::{
    KEEP_LOOKAHEAD, Placement, Previous, RowShift, kept_height, place_flow, place_paged, row_shift,
};

/// Shaped layouts kept after an eviction pass.
const LAYOUT_CACHE_KEEP: usize = 500;
/// The cache is trimmed when it grows past this.
const LAYOUT_CACHE_LIMIT: usize = 800;
/// Word counts are computed at once for changes up to this many bytes of
/// text; larger ones are counted by the background task.
const EAGER_STATS_BYTES: usize = 64 * 1024;
/// Average glyph advance per byte and font size before any block was
/// measured (Latin text in a text face).
const DEFAULT_ADVANCE: f32 = 0.47;

/// What a layout pass needs from the outside. Captured at prepaint and kept
/// for on-demand layout between frames.
pub(crate) struct LayoutEnv {
    pub(crate) text_system: Arc<WindowTextSystem>,
    pub(crate) style: LayoutStyle,
    /// The editor element's bounds, in window coordinates.
    pub(crate) viewport: Bounds<Pixels>,
    /// Window x of the text column's left edge.
    pub(crate) column_left: Pixels,
    pub(crate) column_width: Pixels,
    /// Space above the first block (flow view) or the first sheet, and
    /// below the last one.
    pub(crate) pad_top: Pixels,
    pub(crate) pad_bottom: Pixels,
    /// Page view: the sheets.
    pub(crate) page: Option<PageGeometry>,
}

impl LayoutEnv {
    /// Blocks wrap and measure the same in both environments.
    fn same_shape(&self, other: &Self) -> bool {
        self.column_width == other.column_width && self.style.same_shape(&other.style)
    }

    /// Measured blocks are placed the same in both environments.
    fn same_flow(&self, other: &Self) -> bool {
        self.pad_top == other.pad_top
            && match (&self.page, &other.page) {
                (None, None) => true,
                (Some(a), Some(b)) => a.same_flow(b),
                _ => false,
            }
    }
}

/// Where the viewport's top edge is: at the very top, or `offset` pixels
/// below the top of a block's box (negative: above it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ScrollPosition {
    Top,
    Block { block: usize, offset: Pixels },
}

struct Slot {
    id: BlockId,
    stamp: u64,
    measure: Measure,
    /// `measure` comes from shaping, not from an estimate.
    measured: bool,
    /// Word counts of the block; `None` until counted.
    stats: Option<Stats>,
    /// Valid for slots below the placement watermark.
    place: Placement,
}

struct CachedLayout {
    stamp: u64,
    layout: Rc<BlockLayout>,
    epoch: u64,
}

pub(crate) struct DocumentLayout {
    env: Option<LayoutEnv>,
    /// One per block of the document, in order.
    slots: Vec<Slot>,
    /// Slots `..placed` have a valid placement.
    placed: usize,
    layouts: HashMap<BlockId, CachedLayout>,
    /// Frame counter for the layout cache.
    epoch: u64,
    /// The `EditorState::revision` the slots describe.
    revision: u64,
    scroll: ScrollPosition,
    /// Number of slots whose measure is an estimate.
    unmeasured: usize,
    /// Number of slots without word counts.
    uncounted: usize,
    /// Sum of the counted slots.
    stats_sum: Stats,
    /// The last complete sum (what the editor shows while counting).
    stats_shown: Stats,
    /// Where the background task continues measuring and counting.
    refine_cursor: usize,
    count_cursor: usize,
    /// Bumped whenever a slot's measure changes.
    measure_changes: u64,
    /// Average glyph advance per byte and font size, for estimates.
    advance: f32,
    calibration: (f32, f32),
    calibrated: bool,
}

fn empty_placement() -> Placement {
    Placement {
        top: 0.,
        end: 0.,
        page: 0,
        end_page: 0,
        shifts: None,
    }
}

fn minus(total: Stats, part: Stats) -> Stats {
    // Adding the default recomputes the reading time.
    Stats {
        words: total.words.saturating_sub(part.words),
        characters: total.characters.saturating_sub(part.characters),
        characters_no_spaces: total
            .characters_no_spaces
            .saturating_sub(part.characters_no_spaces),
        paragraphs: total.paragraphs.saturating_sub(part.paragraphs),
        reading_minutes: 0,
    } + Stats::default()
}

impl DocumentLayout {
    pub(crate) fn new(document: &Document, revision: u64) -> Self {
        let mut layout = Self {
            env: None,
            slots: Vec::new(),
            placed: 0,
            layouts: HashMap::new(),
            epoch: 0,
            revision,
            scroll: ScrollPosition::Top,
            unmeasured: 0,
            uncounted: 0,
            stats_sum: Stats::default(),
            stats_shown: Stats::default(),
            refine_cursor: 0,
            count_cursor: 0,
            measure_changes: 0,
            advance: DEFAULT_ADVANCE,
            calibration: (0., 0.),
            calibrated: false,
        };
        layout.reset(document, revision);
        layout
    }

    /// Starts over for a replaced document: every slot new, nothing
    /// cached, scrolled to the top. The environment is kept.
    pub(crate) fn reset(&mut self, document: &Document, revision: u64) {
        self.slots.clear();
        self.layouts.clear();
        self.scroll = ScrollPosition::Top;
        self.stats_shown = Stats::default();
        self.calibrated = false;
        self.calibration = (0., 0.);
        self.advance = DEFAULT_ADVANCE;
        self.rebuild(document, revision);
    }

    /// Brings the slots in line with `document` without knowing what
    /// changed: slots of unchanged blocks are kept.
    pub(crate) fn rebuild(&mut self, document: &Document, revision: u64) {
        let mut old: HashMap<BlockId, Slot> =
            self.slots.drain(..).map(|slot| (slot.id, slot)).collect();
        let mut budget = EAGER_STATS_BYTES;
        self.unmeasured = 0;
        self.uncounted = 0;
        self.stats_sum = Stats::default();
        let mut slots = Vec::with_capacity(document.block_count());
        for block in document.blocks() {
            let slot = self.make_slot(block, old.remove(&block.id()), &mut budget);
            self.count_in(&slot);
            slots.push(slot);
        }
        self.slots = slots;
        self.placed = 0;
        self.revision = revision;
        self.refine_cursor = 0;
        self.count_cursor = 0;
        self.measure_changes += 1;
        self.settle_stats();
    }

    /// A slot for `block`, reusing `old` (the slot the block had before)
    /// when the block did not change.
    fn make_slot(&self, block: &Block, old: Option<Slot>, budget: &mut usize) -> Slot {
        let (id, stamp) = (block.id(), block.revision());
        match old {
            Some(old) if old.stamp == stamp => old,
            old => {
                // A changed block usually keeps its row count: its old
                // measure is the best estimate.
                let measure = match (&old, &self.env) {
                    (Some(old), _) => old.measure,
                    (None, Some(env)) => {
                        Measure::estimate(block, &env.style, env.column_width, self.advance)
                    }
                    (None, None) => Measure::default(),
                };
                let stats = if block.len() <= *budget {
                    *budget -= block.len();
                    Some(Stats::of_text(block.text()))
                } else {
                    *budget = 0;
                    None
                };
                Slot {
                    id,
                    stamp,
                    measure,
                    measured: false,
                    stats,
                    place: empty_placement(),
                }
            }
        }
    }

    fn count_in(&mut self, slot: &Slot) {
        if !slot.measured {
            self.unmeasured += 1;
        }
        match slot.stats {
            Some(stats) => self.stats_sum += stats,
            None => self.uncounted += 1,
        }
    }

    fn count_out(&mut self, slot: &Slot) {
        if !slot.measured {
            self.unmeasured -= 1;
        }
        match slot.stats {
            Some(stats) => self.stats_sum = minus(self.stats_sum, stats),
            None => self.uncounted -= 1,
        }
    }

    fn settle_stats(&mut self) {
        if self.uncounted == 0 {
            self.stats_shown = self.stats_sum;
        }
    }

    /// The state's revision the slots describe.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// Applies a change reported by the engine.
    pub(crate) fn apply_change(
        &mut self,
        document: &Document,
        revision: u64,
        splice: Option<BlockSplice>,
    ) {
        let Some(splice) = splice else {
            if self.slots.len() == document.block_count() {
                self.revision = revision;
            } else {
                self.rebuild(document, revision);
            }
            return;
        };
        let old_range = splice.old_range();
        let new_range = splice.new_range();
        if old_range.end > self.slots.len()
            || self.slots.len() - splice.old_len + splice.new_len != document.block_count()
            || new_range.end > document.block_count()
        {
            // The splice does not describe our slots: start from the ids.
            // (The row oracle of `Editor::edit_with_rows` can have brought
            // the slots in line with a state in the middle of a mutation.
            // Rebuilding does not move the scroll anchor, so the splice
            // still describes how the anchor's block moved.)
            self.rebuild(document, revision);
            self.shift_scroll_anchor(splice);
            if let ScrollPosition::Block { block, .. } = &mut self.scroll {
                *block = (*block).min(self.slots.len().saturating_sub(1));
            }
            return;
        }
        let removed: Vec<Slot> = self.slots.drain(old_range).collect();
        for slot in &removed {
            self.count_out(slot);
        }
        let mut old: HashMap<BlockId, Slot> =
            removed.into_iter().map(|slot| (slot.id, slot)).collect();
        let mut budget = EAGER_STATS_BYTES;
        let mut added = Vec::with_capacity(splice.new_len);
        for block in &document.blocks()[new_range] {
            let slot = self.make_slot(block, old.remove(&block.id()), &mut budget);
            self.count_in(&slot);
            added.push(slot);
        }
        self.slots.splice(splice.start..splice.start, added);
        // Headings just before the change are kept with it.
        self.placed = self.placed.min(splice.start.saturating_sub(KEEP_LOOKAHEAD));
        self.revision = revision;
        self.refine_cursor = self.refine_cursor.min(splice.start);
        self.count_cursor = self.count_cursor.min(splice.start);
        self.measure_changes += 1;
        self.shift_scroll_anchor(splice);
        self.settle_stats();
    }

    /// Keeps the scroll position on its block when blocks before it were
    /// added or removed.
    fn shift_scroll_anchor(&mut self, splice: BlockSplice) {
        if let ScrollPosition::Block { block, .. } = &mut self.scroll {
            if *block >= splice.start + splice.old_len {
                *block = *block + splice.new_len - splice.old_len;
            } else if *block >= splice.start {
                let within = (*block - splice.start).min(splice.new_len.saturating_sub(1));
                *block = splice.start + within;
            }
        }
    }

    /// Installs the environment of a new frame. Measures are dropped when
    /// the column width or anything else that shapes text changed;
    /// placement restarts when only the pages or the padding changed;
    /// shaped layouts are dropped when colours changed.
    pub(crate) fn set_env(&mut self, env: LayoutEnv, document: &Document) {
        let (reshape, replace, recolor) = match &self.env {
            None => (true, true, true),
            Some(old) => (
                !old.same_shape(&env),
                !old.same_flow(&env),
                old.style.colors != env.style.colors,
            ),
        };
        self.env = Some(env);
        if reshape {
            self.layouts.clear();
            self.calibrated = false;
            self.calibration = (0., 0.);
            self.estimate_all(document);
        } else if recolor {
            self.layouts.clear();
        }
        if reshape || replace {
            self.placed = 0;
            self.measure_changes += 1;
        }
    }

    pub(crate) fn env(&self) -> Option<&LayoutEnv> {
        self.env.as_ref()
    }

    /// Replaces every measure with an estimate for the current
    /// environment.
    fn estimate_all(&mut self, document: &Document) {
        let Some(env) = &self.env else {
            return;
        };
        if self.slots.len() != document.block_count() {
            return;
        }
        for (slot, block) in self.slots.iter_mut().zip(document.blocks()) {
            slot.measure = Measure::estimate(block, &env.style, env.column_width, self.advance);
            slot.measured = false;
        }
        self.unmeasured = self.slots.len();
        self.placed = 0;
        self.refine_cursor = 0;
        self.measure_changes += 1;
    }

    /// Estimates again for the blocks that were not measured yet, after
    /// the average advance became known.
    fn estimate_unmeasured(&mut self, document: &Document) {
        let Some(env) = &self.env else {
            return;
        };
        if self.slots.len() != document.block_count() {
            return;
        }
        let mut first = None;
        for (index, (slot, block)) in self.slots.iter_mut().zip(document.blocks()).enumerate() {
            if slot.measured {
                continue;
            }
            let measure = Measure::estimate(block, &env.style, env.column_width, self.advance);
            if measure != slot.measure {
                slot.measure = measure;
                first.get_or_insert(index);
            }
        }
        if let Some(first) = first {
            self.placed = self.placed.min(first.saturating_sub(KEEP_LOOKAHEAD));
            self.measure_changes += 1;
        }
    }

    /// Starts a frame: ages the layout cache.
    pub(crate) fn begin_frame(&mut self) {
        self.epoch += 1;
        if self.layouts.len() > LAYOUT_CACHE_LIMIT {
            let mut epochs: Vec<u64> = self.layouts.values().map(|cached| cached.epoch).collect();
            epochs.sort_unstable_by(|a, b| b.cmp(a));
            let cutoff = epochs[LAYOUT_CACHE_KEEP.min(epochs.len() - 1)];
            // Whatever the last frame used stays, however much that was.
            let recent = self.epoch.saturating_sub(1);
            self.layouts
                .retain(|_, cached| cached.epoch > cutoff || cached.epoch >= recent);
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.slots.len()
    }

    // --- Measuring ---------------------------------------------------------

    /// Records the real measure of block `index`.
    fn note_measure(&mut self, index: usize, measure: Measure, sample: Option<(f32, f32)>) {
        let Some(slot) = self.slots.get_mut(index) else {
            return;
        };
        if !slot.measured {
            slot.measured = true;
            self.unmeasured -= 1;
            if let Some((width, units)) = sample
                && !self.calibrated
            {
                self.calibration.0 += width;
                self.calibration.1 += units;
            }
        }
        if slot.measure != measure {
            slot.measure = measure;
            // Headings just before are kept with this block.
            self.placed = self.placed.min(index.saturating_sub(KEEP_LOOKAHEAD));
            self.measure_changes += 1;
        }
    }

    /// The shaped layout of block `index`, shaping it if needed. `None`
    /// without an environment or for an index outside the document.
    pub(crate) fn layout(&mut self, index: usize, document: &Document) -> Option<Rc<BlockLayout>> {
        let block = document.block(index)?;
        let env = self.env.as_ref()?;
        if index >= self.slots.len() {
            return None;
        }
        let (id, stamp) = (block.id(), block.revision());
        if let Some(cached) = self.layouts.get_mut(&id)
            && cached.stamp == stamp
        {
            cached.epoch = self.epoch;
            let layout = cached.layout.clone();
            if !self.slots[index].measured {
                self.note_measure(index, layout.measure, None);
            }
            return Some(layout);
        }
        let layout = Rc::new(BlockLayout::shape(
            block,
            &env.style,
            env.column_width,
            &env.text_system,
        ));
        self.layouts.insert(
            id,
            CachedLayout {
                stamp,
                layout: layout.clone(),
                epoch: self.epoch,
            },
        );
        let sample = (!block.kind().is_code() && !block.is_void()).then(|| {
            (
                f32::from(layout.natural_width),
                block.len() as f32 * f32::from(layout.font_size),
            )
        });
        self.note_measure(index, layout.measure, sample);
        Some(layout)
    }

    /// Measures block `index` without keeping its shaped layout.
    fn measure_only(&mut self, index: usize, document: &Document) {
        let (Some(block), Some(env)) = (document.block(index), self.env.as_ref()) else {
            return;
        };
        let (measure, _) =
            block::measure_block(block, &env.style, env.column_width, &env.text_system);
        self.note_measure(index, measure, None);
    }

    pub(crate) fn is_measured(&self, index: usize) -> bool {
        self.slots.get(index).is_some_and(|slot| slot.measured)
    }

    /// Whether estimates or missing word counts remain.
    pub(crate) fn pending(&self) -> bool {
        (self.unmeasured > 0 && self.env.is_some()) || self.uncounted > 0
    }

    /// Whether every block's measure is real.
    pub(crate) fn settled(&self) -> bool {
        self.unmeasured == 0
    }

    /// One slice of background work: counts the words of blocks that have
    /// no counts yet (first, because it is quick and the status bar waits
    /// for it), then measures blocks that only have an estimate, until
    /// `deadline` passes or `max_blocks` were handled. Returns whether work
    /// remains.
    pub(crate) fn refine(
        &mut self,
        document: &Document,
        deadline: Option<Instant>,
        max_blocks: usize,
    ) -> bool {
        let count = self.slots.len();
        if count != document.block_count() {
            return false;
        }
        let expired = |handled: usize| {
            handled >= max_blocks || deadline.is_some_and(|deadline| Instant::now() >= deadline)
        };
        let mut handled = 0;
        let mut scanned = 0;
        let mut index = self.count_cursor;
        while self.uncounted > 0 && scanned < count && !expired(handled) {
            if index >= count {
                index = 0;
            }
            if self.slots[index].stats.is_none() {
                let stats = Stats::of_text(document.blocks()[index].text());
                self.slots[index].stats = Some(stats);
                self.stats_sum += stats;
                self.uncounted -= 1;
                handled += 1;
            }
            index += 1;
            scanned += 1;
        }
        self.count_cursor = index;
        self.settle_stats();

        if self.env.is_some() {
            let mut scanned = 0;
            let mut index = self.refine_cursor;
            while self.unmeasured > 0 && scanned < count && !expired(handled) {
                if index >= count {
                    index = 0;
                }
                if !self.slots[index].measured {
                    self.measure_only(index, document);
                    handled += 1;
                }
                index += 1;
                scanned += 1;
            }
            self.refine_cursor = index;
        }
        self.pending()
    }

    /// Measures every block now (compact editors, tests).
    pub(crate) fn measure_all(&mut self, document: &Document) {
        while self.refine(document, None, usize::MAX) {}
    }

    /// Makes the estimates follow the text that was really measured.
    fn calibrate(&mut self, document: &Document) {
        let (width, units) = self.calibration;
        if self.calibrated || units < 4000. {
            return;
        }
        self.calibrated = true;
        let advance = (width / units).clamp(0.2, 1.2);
        if (advance - self.advance).abs() > 0.01 {
            self.advance = advance;
            self.estimate_unmeasured(document);
        }
    }

    // --- Placement ---------------------------------------------------------

    fn place_next(&mut self) {
        let index = self.placed;
        let previous = index.checked_sub(1).map(|previous| {
            let slot = &self.slots[previous];
            Previous {
                measure: &slot.measure,
                end: slot.place.end,
                end_page: slot.place.end_page,
            }
        });
        let measure = &self.slots[index].measure;
        let (pad_top, page) = self.env.as_ref().map_or((0., None), |env| {
            (f32::from(env.pad_top), env.page.as_ref())
        });
        let place = match page {
            Some(geometry) => place_paged(
                geometry,
                pad_top,
                previous,
                measure,
                kept_height(
                    measure,
                    self.slots[index + 1..].iter().map(|slot| &slot.measure),
                ),
            ),
            None => place_flow(previous, measure, pad_top),
        };
        self.slots[index].place = place;
        self.placed = index + 1;
    }

    fn place_through(&mut self, index: usize) {
        let end = (index + 1).min(self.slots.len());
        while self.placed < end {
            self.place_next();
        }
    }

    fn place_all(&mut self) {
        while self.placed < self.slots.len() {
            self.place_next();
        }
    }

    /// The placement of block `index` (clamped to the last block).
    fn placement(&mut self, index: usize) -> Option<&Placement> {
        let index = index.min(self.slots.len().checked_sub(1)?);
        self.place_through(index);
        Some(&self.slots[index].place)
    }

    /// Content-space y of the top of block `index`'s box (clamped).
    pub(crate) fn top(&mut self, index: usize) -> Pixels {
        px(self.placement(index).map_or(0., |place| place.top))
    }

    /// The block whose box top is the last one at or above content-space
    /// `y`, clamped to the first block.
    pub(crate) fn block_at_y(&mut self, y: Pixels) -> usize {
        let count = self.slots.len();
        if count == 0 {
            return 0;
        }
        let y = f32::from(y);
        while self.placed < count
            && (self.placed == 0 || self.slots[self.placed - 1].place.top <= y)
        {
            self.place_next();
        }
        self.slots[..self.placed]
            .partition_point(|slot| slot.place.top <= y)
            .saturating_sub(1)
    }

    /// Like [`Self::block_at_y`], but the answer is a block whose measure
    /// is real, so it stays the block at `y` when it is shaped.
    pub(crate) fn measured_block_at_y(&mut self, y: Pixels, document: &Document) -> usize {
        loop {
            let index = self.block_at_y(y);
            if self.is_measured(index) || self.layout(index, document).is_none() {
                return index;
            }
        }
    }

    /// Number of sheets in page view (1 in flow view).
    pub(crate) fn page_count(&mut self) -> usize {
        self.place_all();
        self.slots.last().map_or(1, |slot| {
            slot.place.end_page as usize + 1 + usize::from(slot.measure.page_break)
        })
    }

    /// Height of the scrollable content, padding included.
    pub(crate) fn content_height(&mut self) -> Pixels {
        let pages = self.page_count();
        let Some(env) = &self.env else {
            return px(0.);
        };
        match &env.page {
            Some(geometry) => {
                geometry.sheet_top(pages, env.pad_top) - geometry.gap + env.pad_bottom
            }
            None => px(self.slots.last().map_or(0., |slot| slot.place.end)) + env.pad_bottom,
        }
    }

    /// Shapes the blocks in the viewport (`height` pixels from the scroll
    /// position) until their placement no longer changes, and returns
    /// their range. After this, everything a frame shows is real and the
    /// scroll position is inside the content.
    pub(crate) fn stabilize(&mut self, document: &Document, height: Pixels) -> Range<usize> {
        let count = self.slots.len();
        if count == 0 {
            return 0..0;
        }
        let mut range = 0..0;
        for _ in 0..8 {
            let before = self.measure_changes;
            self.clamp_scroll();
            let top = self.scroll_y();
            let bottom = top + height;
            let first = self.block_at_y(top);
            let mut index = first;
            loop {
                if self.layout(index, document).is_none() {
                    break;
                }
                if index + 1 >= count || self.top(index + 1) >= bottom {
                    break;
                }
                index += 1;
            }
            range = first..index + 1;
            if self.measure_changes == before {
                break;
            }
        }
        self.calibrate(document);
        range
    }

    // --- Scrolling ---------------------------------------------------------

    pub(crate) fn scroll_position(&self) -> ScrollPosition {
        self.scroll
    }

    pub(crate) fn set_scroll_position(&mut self, position: ScrollPosition) {
        self.scroll = position;
    }

    /// How far the content is scrolled, in pixels from its top.
    pub(crate) fn scroll_y(&mut self) -> Pixels {
        match self.scroll {
            ScrollPosition::Top => px(0.),
            ScrollPosition::Block { block, offset } => (self.top(block) + offset).max(px(0.)),
        }
    }

    pub(crate) fn set_scroll_y(&mut self, y: Pixels) {
        if y <= px(0.) || self.slots.is_empty() {
            self.scroll = ScrollPosition::Top;
            return;
        }
        let block = self.block_at_y(y);
        self.scroll = ScrollPosition::Block {
            block,
            offset: y - self.top(block),
        };
    }

    pub(crate) fn max_scroll(&mut self) -> Pixels {
        let height = self.content_height();
        let viewport = self
            .env
            .as_ref()
            .map_or(px(0.), |env| env.viewport.size.height);
        (height - viewport).max(px(0.))
    }

    /// Keeps the scroll position inside the content.
    pub(crate) fn clamp_scroll(&mut self) {
        if self.scroll == ScrollPosition::Top {
            return;
        }
        let max = self.max_scroll();
        if self.scroll_y() > max {
            self.set_scroll_y(max);
        }
    }

    // --- Statistics --------------------------------------------------------

    /// Word counts of the document: exact when nothing is waiting to be
    /// counted, otherwise the last complete result.
    pub(crate) fn stats(&self) -> Stats {
        self.stats_shown
    }

    pub(crate) fn stats_complete(&self) -> bool {
        self.uncounted == 0
    }

    #[cfg(test)]
    pub(crate) fn cached_layouts(&self) -> usize {
        self.layouts.len()
    }

    #[cfg(test)]
    pub(crate) fn unmeasured(&self) -> usize {
        self.unmeasured
    }
}

/// A block's shaped layout together with where it is.
pub(crate) struct Placed {
    pub(crate) layout: Rc<BlockLayout>,
    /// Content-space y of the top of the block's box.
    pub(crate) top: Pixels,
    shifts: Option<Rc<[RowShift]>>,
}

/// Rows of a block that sit on one sheet (the whole block in flow view).
pub(crate) struct Segment {
    /// Content-space top and bottom of the segment's part of the box.
    pub(crate) top: Pixels,
    pub(crate) bottom: Pixels,
}

impl Placed {
    fn shift(&self, row: usize) -> Pixels {
        row_shift(self.shifts.as_deref(), row)
    }

    /// Content-space y of the top of row `row`.
    pub(crate) fn row_top(&self, row: usize) -> Pixels {
        let row = row.min(self.layout.rows.len() - 1);
        self.top + self.layout.rows[row].y + self.shift(row)
    }

    /// First row of each segment, with the row count as the last entry.
    fn segment_starts(&self) -> Vec<usize> {
        let rows = self.layout.rows.len();
        let mut starts = vec![0];
        if let Some(shifts) = &self.shifts {
            starts.extend(
                shifts
                    .iter()
                    .map(|shift| shift.first_row as usize)
                    .filter(|row| *row > 0 && *row < rows),
            );
        }
        starts.push(rows);
        starts
    }

    /// The rows on each sheet.
    pub(crate) fn segments(&self) -> Vec<Segment> {
        let starts = self.segment_starts();
        let pad_top = px(self.layout.measure.pad_top);
        let pad_bottom = px(self.layout.measure.pad_bottom);
        let last = starts.len() - 2;
        starts
            .windows(2)
            .enumerate()
            .map(|(ix, pair)| {
                let rows = pair[0]..pair[1];
                let top = self.row_top(rows.start) - if ix == 0 { pad_top } else { px(0.) };
                let bottom = self.row_top(rows.end - 1)
                    + self.layout.line_height
                    + if ix == last { pad_bottom } else { px(0.) };
                Segment { top, bottom }
            })
            .collect()
    }

    /// The row at content-space `y`, clamped: above the block its first
    /// row, between two sheets the last row of the upper one.
    pub(crate) fn row_at(&self, y: Pixels) -> usize {
        let starts = self.segment_starts();
        let mut found = 0;
        for pair in starts.windows(2) {
            let top = self.row_top(pair[0]);
            if y < top && pair[0] > 0 {
                break;
            }
            let within = ((y - top) / self.layout.line_height).floor().max(0.) as usize;
            found = pair[0] + within.min(pair[1] - pair[0] - 1);
        }
        found
    }

    /// The caret line box at `offset`: zero width, one row high. x from
    /// the column's left edge, y in content space.
    pub(crate) fn line_box(&self, offset: usize) -> Bounds<Pixels> {
        let (row, bounds) = self.layout.caret_bounds(offset);
        self.lift(row, bounds)
    }

    /// One rectangle per row for the bytes in `range`, one row high. Only
    /// the rows in `rows` are looked at.
    pub(crate) fn rects(&self, range: Range<usize>, rows: Range<usize>) -> Vec<Bounds<Pixels>> {
        self.layout
            .rects_for_range(range, rows)
            .into_iter()
            .map(|(row, bounds)| self.lift(row, bounds))
            .collect()
    }

    /// Like [`Self::rects`], but only as high as the text's ink: what
    /// highlights and tints behind text cover.
    pub(crate) fn ink_rects(&self, range: Range<usize>, rows: Range<usize>) -> Vec<Bounds<Pixels>> {
        let (ink_top, ink_height) = self.layout.ink_box();
        self.layout
            .rects_for_range(range, rows)
            .into_iter()
            .map(|(row, bounds)| {
                let lifted = self.lift(row, bounds);
                Bounds::new(
                    point(lifted.origin.x, lifted.origin.y + ink_top),
                    size(lifted.size.width, ink_height),
                )
            })
            .collect()
    }

    /// Block-local bounds of something on `row` to content space.
    fn lift(&self, row: usize, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
        Bounds::new(
            point(
                bounds.origin.x,
                bounds.origin.y + self.top + self.shift(row),
            ),
            bounds.size,
        )
    }
}

/// The layout together with the document it describes: what geometry code
/// works with.
pub(crate) struct Lay<'a> {
    pub(crate) layout: &'a mut DocumentLayout,
    pub(crate) document: &'a Document,
}

impl Lay<'_> {
    /// The environment. `Editor::with_layout` only builds a `Lay` when
    /// there is one.
    pub(crate) fn env(&self) -> &LayoutEnv {
        self.layout
            .env
            .as_ref()
            .expect("a Lay is only built for a layout with an environment")
    }

    pub(crate) fn count(&self) -> usize {
        self.layout.count()
    }

    /// Block `index`, shaped and placed.
    pub(crate) fn placed(&mut self, index: usize) -> Option<Placed> {
        let layout = self.layout.layout(index, self.document)?;
        let place = self.layout.placement(index)?;
        Some(Placed {
            layout,
            top: px(place.top),
            shifts: place.shifts.clone(),
        })
    }

    /// The block at content-space `y` (see
    /// [`DocumentLayout::measured_block_at_y`]).
    pub(crate) fn block_at(&mut self, y: Pixels) -> usize {
        self.layout.measured_block_at_y(y, self.document)
    }

    pub(crate) fn scroll_y(&mut self) -> Pixels {
        self.layout.scroll_y()
    }

    /// Window position of the point (column left, content top): add it to
    /// column-local x and content-space y.
    pub(crate) fn origin(&mut self) -> Point<Pixels> {
        let scroll_y = self.layout.scroll_y();
        let env = self.env();
        point(env.column_left, env.viewport.top() - scroll_y)
    }

    /// The sheet (0-based) that content-space `y` is on; 0 in flow view.
    pub(crate) fn page_at(&self, y: Pixels) -> usize {
        let env = self.env();
        env.page
            .as_ref()
            .map_or(0, |geometry| geometry.page_at(y, env.pad_top))
    }
}
