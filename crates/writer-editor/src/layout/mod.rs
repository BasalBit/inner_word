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
//! **Numbered lists** are the one place where a block's layout depends on
//! its neighbours: the gutter of every item is as wide as the list's
//! largest number needs, so the text of the items lines up. Each slot
//! keeps the digit count of its list ([`Marker`]); a change that adds,
//! removes or re-levels list items recounts the lists it touches
//! (`DocumentLayout::number_lists`), and items whose gutter changed are
//! measured again.
//!
//! **Scrolling** is anchored to a block (`ScrollPosition`), not stored in
//! pixels, so measuring or editing blocks above the viewport never moves
//! what is on screen.
//!
//! **Units.** Everything in here is in layout units: the document's own
//! points, with x from the left edge of the text column and y from the top
//! of the scrollable content. The zoom of the view is not part of any
//! measure, wrap or placement: it is one number of the environment
//! ([`LayoutEnv::zoom`]) that [`View`] applies when layout coordinates
//! become window coordinates, and takes out again when a pointer position
//! comes in. So the same rows stand on the same sheets at every zoom, and
//! a change of zoom lays nothing out again (in flow view only when the
//! column has to become narrower than its full width).
//!
//! The environment (viewport, column, style, text system) is installed by
//! the element's prepaint and kept, so geometry queries between frames lay
//! out on demand: a consumer that edits and then asks for a caret position
//! in the same key handler (Vim does) gets an answer for the edited
//! document.

pub(crate) mod block;
pub(crate) mod pages;
pub(crate) mod style;
pub(crate) mod wrap;

use std::{collections::HashMap, ops::Range, rc::Rc, sync::Arc, time::Instant};

use gpui::{Bounds, Pixels, Point, SharedString, TextRun, WindowTextSystem, point, px, size};
use writer_core::{Block, BlockId, BlockKind, BlockSplice, Document, Stats};

use block::Reflow;
pub(crate) use block::{BlockLayout, Measure};
pub(crate) use pages::PageGeometry;
pub(crate) use style::LayoutStyle;
use style::Marker;

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
    /// Window pixels per layout unit: the zoom of the view (1 in a
    /// compact editor).
    pub(crate) zoom: f32,
    /// Window x of the text column's left edge while the view is not
    /// scrolled sideways.
    pub(crate) column_left: Pixels,
    /// Width of the text column, in layout units.
    pub(crate) column_width: Pixels,
    /// Space above the first block (flow view) or the first sheet, and
    /// below the last one, in layout units.
    pub(crate) pad_top: Pixels,
    pub(crate) pad_bottom: Pixels,
    /// Page view: the sheets.
    pub(crate) page: Option<PageGeometry>,
    /// How far the view can be scrolled sideways, in layout units: 0
    /// unless a sheet is wider than the room it has.
    pub(crate) max_scroll_x: Pixels,
    /// Where the view rests sideways until someone scrolls it: the text
    /// column in the middle of the room, or at its left edge when the
    /// column alone is wider than the room. In layout units.
    pub(crate) rest_scroll_x: Pixels,
}

/// How layout coordinates (column-local x, content-space y, in layout
/// units) become window coordinates, and back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct View {
    /// Window position of the point (column left, content top).
    pub(crate) origin: Point<Pixels>,
    /// Window pixels per layout unit.
    pub(crate) zoom: f32,
}

impl View {
    /// A layout length in window pixels.
    pub(crate) fn len(&self, length: Pixels) -> Pixels {
        length * self.zoom
    }

    /// Window x of a column-local x.
    pub(crate) fn x(&self, x: Pixels) -> Pixels {
        self.origin.x + x * self.zoom
    }

    /// Window y of a content-space y.
    pub(crate) fn y(&self, y: Pixels) -> Pixels {
        self.origin.y + y * self.zoom
    }

    pub(crate) fn point(&self, at: Point<Pixels>) -> Point<Pixels> {
        point(self.x(at.x), self.y(at.y))
    }

    /// A rectangle of the layout in the window.
    pub(crate) fn rect(&self, rect: Bounds<Pixels>) -> Bounds<Pixels> {
        Bounds::new(
            self.point(rect.origin),
            size(rect.size.width * self.zoom, rect.size.height * self.zoom),
        )
    }

    /// Column-local x of a window x.
    pub(crate) fn layout_x(&self, x: Pixels) -> Pixels {
        (x - self.origin.x) / self.zoom
    }

    /// Content-space y of a window y.
    pub(crate) fn layout_y(&self, y: Pixels) -> Pixels {
        (y - self.origin.y) / self.zoom
    }
}

impl LayoutEnv {
    /// Height of the viewport in layout units: how much of the content a
    /// frame shows.
    pub(crate) fn view_height(&self) -> Pixels {
        self.viewport.size.height / self.zoom
    }

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

/// Where the viewport's top edge is: at the very top, or `offset` layout
/// units below the top of a block's box (negative: above it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ScrollPosition {
    Top,
    Block { block: usize, offset: Pixels },
}

struct Slot {
    id: BlockId,
    stamp: u64,
    /// What the block is to list numbering, see [`list_key`].
    list: u8,
    /// [`Marker::digits`] of the numbered list the block is in.
    digits: u8,
    measure: Measure,
    /// `measure` comes from shaping, not from an estimate.
    measured: bool,
    /// Word counts of the block; `None` until counted.
    stats: Option<Stats>,
    /// The block has too much text to count on the main thread: the
    /// editor has it counted on a background thread.
    large: bool,
    /// Valid for slots below the placement watermark.
    place: Placement,
}

struct CachedLayout {
    stamp: u64,
    /// The `Slot::digits` the layout was shaped with.
    digits: u8,
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
    /// How far the view is scrolled sideways, in layout units; `None`
    /// while it rests where the environment puts it
    /// ([`LayoutEnv::rest_scroll_x`]).
    scroll_x: Option<Pixels>,
    /// Number of slots whose measure is an estimate.
    unmeasured: usize,
    /// Number of slots without word counts.
    uncounted: usize,
    /// How many of those are `Slot::large`.
    uncounted_large: usize,
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
    /// Widths of a digit and of a period in ems of the body font, for the
    /// gutters of numbered lists. Measured when first needed.
    marker_em: Option<(f32, f32)>,
    /// Very long blocks whose rows an edit left with a seam (see
    /// `block`): the background layout finishes their reflow.
    seamed: Vec<BlockId>,
}

/// What a block is to list numbering: bit 7 for a numbered item, the low
/// bits its indent. 0 is a block that ends every list (not numbered, not
/// indented). Two runs of blocks with the same keys number alike.
fn list_key(block: &Block) -> u8 {
    let numbered = matches!(block.kind(), BlockKind::NumberedItem);
    (u8::from(numbered) << 7) | block.indent().min(0x7f)
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
            scroll_x: None,
            unmeasured: 0,
            uncounted: 0,
            uncounted_large: 0,
            stats_sum: Stats::default(),
            stats_shown: Stats::default(),
            refine_cursor: 0,
            count_cursor: 0,
            measure_changes: 0,
            advance: DEFAULT_ADVANCE,
            calibration: (0., 0.),
            calibrated: false,
            marker_em: None,
            seamed: Vec::new(),
        };
        layout.reset(document, revision);
        layout
    }

    /// Starts over for a replaced document: every slot new, nothing
    /// cached, scrolled to the top. The environment is kept.
    pub(crate) fn reset(&mut self, document: &Document, revision: u64) {
        self.slots.clear();
        self.layouts.clear();
        self.seamed.clear();
        self.scroll = ScrollPosition::Top;
        self.scroll_x = None;
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
        self.uncounted_large = 0;
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
        self.number_lists(document, 0..self.slots.len());
        self.settle_stats();
    }

    /// A slot for `block`, reusing `old` (the slot the block had before)
    /// when the block did not change. A new slot has no list digits yet:
    /// the caller copies them or recounts the lists.
    fn make_slot(&self, block: &Block, old: Option<Slot>, budget: &mut usize) -> Slot {
        let (id, stamp) = (block.id(), block.revision());
        match old {
            Some(old) if old.stamp == stamp => old,
            old => {
                // A changed block usually keeps its row count: its old
                // measure is the best estimate.
                let measure = match (&old, &self.env) {
                    (Some(old), _) => old.measure,
                    (None, Some(env)) => Measure::estimate(
                        block,
                        &env.style,
                        env.column_width,
                        self.advance,
                        Marker::NONE,
                    ),
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
                    list: list_key(block),
                    digits: 0,
                    measure,
                    measured: false,
                    stats,
                    large: block.len() > EAGER_STATS_BYTES,
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
            None => {
                self.uncounted += 1;
                self.uncounted_large += usize::from(slot.large);
            }
        }
    }

    fn count_out(&mut self, slot: &Slot) {
        if !slot.measured {
            self.unmeasured -= 1;
        }
        match slot.stats {
            Some(stats) => self.stats_sum = minus(self.stats_sum, stats),
            None => {
                self.uncounted -= 1;
                self.uncounted_large -= usize::from(slot.large);
            }
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
        let old_lists: Vec<(u8, u8)> = removed
            .iter()
            .map(|slot| (slot.list, slot.digits))
            .collect();
        let mut old: HashMap<BlockId, Slot> =
            removed.into_iter().map(|slot| (slot.id, slot)).collect();
        let mut budget = EAGER_STATS_BYTES;
        let mut added = Vec::with_capacity(splice.new_len);
        for block in &document.blocks()[new_range] {
            let slot = self.make_slot(block, old.remove(&block.id()), &mut budget);
            added.push(slot);
        }
        // The usual change (typing) leaves list items list items at the
        // same levels: the lists number as before.
        let same_lists = old_lists.len() == added.len()
            && old_lists
                .iter()
                .zip(&added)
                .all(|((list, _), slot)| *list == slot.list);
        if same_lists {
            for ((_, digits), slot) in old_lists.iter().zip(&mut added) {
                slot.digits = *digits;
            }
        }
        for slot in &added {
            self.count_in(slot);
        }
        self.slots.splice(splice.start..splice.start, added);
        // Headings just before the change are kept with it.
        self.placed = self.placed.min(splice.start.saturating_sub(KEEP_LOOKAHEAD));
        self.revision = revision;
        self.refine_cursor = self.refine_cursor.min(splice.start);
        self.count_cursor = self.count_cursor.min(splice.start);
        self.measure_changes += 1;
        if !same_lists {
            self.number_lists(document, splice.start..splice.start + splice.new_len);
        }
        self.shift_scroll_anchor(splice);
        self.settle_stats();
    }

    /// Recounts the numbered lists that the blocks in `touched` belong to
    /// or border on, and gives every item of a list the digit count of
    /// the list's largest number. Items whose digits change will be
    /// measured again (their gutter changes, so their text may wrap
    /// differently). Linear in the size of those lists.
    fn number_lists(&mut self, document: &Document, touched: Range<usize>) {
        let blocks = document.blocks();
        if self.slots.len() != blocks.len() {
            return;
        }
        // Widen to where no list is open: a block with key 0 ends them all.
        let mut start = touched.start.min(blocks.len());
        while start > 0 && self.slots[start - 1].list != 0 {
            start -= 1;
        }
        let mut end = touched.end.clamp(start, blocks.len());
        while end < blocks.len() && self.slots[end].list != 0 {
            end += 1;
        }
        if start == end {
            return;
        }
        // The same counting as `Document::numbered_ordinals`: one open
        // list per indent level; deeper levels close when a shallower
        // block comes, a level closes at a block of it that is not
        // numbered.
        let mut digits = vec![0u8; end - start];
        let mut open: Vec<Vec<usize>> = Vec::new();
        let close = |members: &mut Vec<usize>, digits: &mut Vec<u8>| {
            let wide = Marker::digits_for(members.len());
            for member in members.drain(..) {
                digits[member] = wide;
            }
        };
        for (offset, slot) in self.slots[start..end].iter().enumerate() {
            let level = (slot.list & 0x7f) as usize;
            if open.len() <= level {
                open.resize_with(level + 1, Vec::new);
            }
            for members in &mut open[level + 1..] {
                close(members, &mut digits);
            }
            if slot.list & 0x80 != 0 {
                open[level].push(offset);
            } else {
                close(&mut open[level], &mut digits);
            }
        }
        for members in &mut open {
            close(members, &mut digits);
        }
        let mut first_changed = None;
        for (offset, digits) in digits.into_iter().enumerate() {
            let slot = &mut self.slots[start + offset];
            if slot.digits == digits {
                continue;
            }
            slot.digits = digits;
            if slot.measured {
                slot.measured = false;
                self.unmeasured += 1;
            }
            first_changed.get_or_insert(start + offset);
        }
        if let Some(first) = first_changed {
            self.placed = self.placed.min(first.saturating_sub(KEEP_LOOKAHEAD));
            self.refine_cursor = self.refine_cursor.min(first);
            self.measure_changes += 1;
        }
    }

    /// Widths of a digit (the widest one) and of a period in ems of the
    /// body font: what the numbers of a numbered list are set in.
    fn marker_em(&mut self) -> (f32, f32) {
        if let Some(em) = self.marker_em {
            return em;
        }
        let Some(env) = &self.env else {
            return (0.6, 0.3);
        };
        const SIZE: f32 = 100.;
        let text: SharedString = "0123456789.".into();
        let run = TextRun {
            len: text.len(),
            font: env.style.body_font.clone(),
            color: env.style.colors.ink,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = env.text_system.shape_line(text, px(SIZE), &[run], None);
        let mut edges: Vec<f32> = line
            .runs
            .iter()
            .flat_map(|run| run.glyphs.iter().map(|glyph| f32::from(glyph.position.x)))
            .collect();
        edges.push(f32::from(line.width));
        let em = if edges.len() == 12 {
            let digit = edges[..11]
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .fold(0., f32::max);
            (digit / SIZE, (edges[11] - edges[10]) / SIZE)
        } else {
            // A font that shapes digits into something else: an average.
            let each = f32::from(line.width) / 11. / SIZE;
            (each, each)
        };
        self.marker_em = Some(em);
        em
    }

    /// The marker for a block whose list has `digits` (see [`Slot`]).
    fn marker(&mut self, digits: u8) -> Marker {
        if digits == 0 {
            return Marker::NONE;
        }
        let (digit_em, period_em) = self.marker_em();
        Marker {
            digits,
            digit_em,
            period_em,
        }
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
            self.seamed.clear();
            self.marker_em = None;
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
        if self.slots.len() != document.block_count() {
            return;
        }
        let (digit_em, period_em) = self.marker_em();
        let Some(env) = &self.env else {
            return;
        };
        for (slot, block) in self.slots.iter_mut().zip(document.blocks()) {
            let marker = Marker {
                digits: slot.digits,
                digit_em,
                period_em,
            };
            slot.measure =
                Measure::estimate(block, &env.style, env.column_width, self.advance, marker);
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
        if self.slots.len() != document.block_count() {
            return;
        }
        let (digit_em, period_em) = self.marker_em();
        let Some(env) = &self.env else {
            return;
        };
        let mut first = None;
        for (index, (slot, block)) in self.slots.iter_mut().zip(document.blocks()).enumerate() {
            if slot.measured {
                continue;
            }
            let marker = Marker {
                digits: slot.digits,
                digit_em,
                period_em,
            };
            let measure =
                Measure::estimate(block, &env.style, env.column_width, self.advance, marker);
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
        self.env.as_ref()?;
        let digits = self.slots.get(index)?.digits;
        let (id, stamp) = (block.id(), block.revision());
        if let Some(cached) = self.layouts.get_mut(&id)
            && cached.stamp == stamp
            && cached.digits == digits
        {
            cached.epoch = self.epoch;
            let layout = cached.layout.clone();
            if !self.slots[index].measured {
                self.note_measure(index, layout.measure, None);
            }
            return Some(layout);
        }
        self.shape(index, document, Reflow::Edit)
    }

    /// Shapes block `index` and caches the result.
    fn shape(
        &mut self,
        index: usize,
        document: &Document,
        reflow: Reflow,
    ) -> Option<Rc<BlockLayout>> {
        let block = document.block(index)?;
        let digits = self.slots.get(index)?.digits;
        let (id, stamp) = (block.id(), block.revision());
        let marker = self.marker(digits);
        let env = self.env.as_ref()?;
        // The layout of the block as it was before: a very long block
        // takes over from it what did not change.
        let previous = self.layouts.get(&id).map(|cached| cached.layout.clone());
        let layout = Rc::new(BlockLayout::shape(
            block,
            &env.style,
            env.column_width,
            &env.text_system,
            marker,
            previous.as_deref(),
            reflow,
        ));
        self.layouts.insert(
            id,
            CachedLayout {
                stamp,
                digits,
                layout: layout.clone(),
                epoch: self.epoch,
            },
        );
        let seamed = self.seamed.iter().position(|seamed| *seamed == id);
        match (layout.has_seam(), seamed) {
            (true, None) => self.seamed.push(id),
            (false, Some(at)) => {
                self.seamed.swap_remove(at);
            }
            _ => {}
        }
        let sample = (!block.kind().is_code() && !block.is_void()).then(|| {
            (
                f32::from(layout.natural_width),
                block.len() as f32 * f32::from(layout.font_size),
            )
        });
        self.note_measure(index, layout.measure, sample);
        Some(layout)
    }

    /// Continues the reflow of one block that an edit left with a seam.
    /// Returns whether it did anything.
    fn mend(&mut self, document: &Document) -> bool {
        while let Some(id) = self.seamed.last().copied() {
            let cached = self
                .layouts
                .get(&id)
                .filter(|cached| cached.layout.has_seam())
                .map(|cached| cached.stamp);
            let index = self.slots.iter().position(|slot| slot.id == id);
            match (cached, index) {
                (Some(_), Some(index)) => {
                    return self.shape(index, document, Reflow::Mend).is_some();
                }
                // The block or its layout is gone.
                _ => {
                    self.seamed.pop();
                }
            }
        }
        false
    }

    /// Measures block `index` without keeping its shaped layout.
    fn measure_only(&mut self, index: usize, document: &Document) {
        let Some(digits) = self.slots.get(index).map(|slot| slot.digits) else {
            return;
        };
        let marker = self.marker(digits);
        let (Some(block), Some(env)) = (document.block(index), self.env.as_ref()) else {
            return;
        };
        let (measure, _) = block::measure_block(
            block,
            &env.style,
            env.column_width,
            &env.text_system,
            marker,
        );
        self.note_measure(index, measure, None);
    }

    pub(crate) fn is_measured(&self, index: usize) -> bool {
        self.slots.get(index).is_some_and(|slot| slot.measured)
    }

    /// Whether there is work for [`Self::refine`]: estimates, missing
    /// word counts (of blocks it counts itself) or unfinished reflows.
    pub(crate) fn pending(&self) -> bool {
        (self.unmeasured > 0 && self.env.is_some())
            || self.uncounted > self.uncounted_large
            || (!self.seamed.is_empty() && self.env.is_some())
    }

    /// The blocks whose words a background thread has to count: `(index,
    /// id, revision stamp)` of every very long block without counts.
    pub(crate) fn large_uncounted(&self) -> Vec<(usize, BlockId, u64)> {
        if self.uncounted_large == 0 {
            return Vec::new();
        }
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.large && slot.stats.is_none())
            .map(|(index, slot)| (index, slot.id, slot.stamp))
            .collect()
    }

    /// Records the counts of a very long block that were made elsewhere,
    /// if the block is still the one that was counted. Returns whether it
    /// was.
    pub(crate) fn set_block_stats(&mut self, id: BlockId, stamp: u64, stats: Stats) -> bool {
        let Some(slot) = self
            .slots
            .iter_mut()
            .find(|slot| slot.id == id && slot.stamp == stamp && slot.stats.is_none())
        else {
            return false;
        };
        slot.stats = Some(stats);
        self.uncounted -= 1;
        self.uncounted_large -= usize::from(slot.large);
        self.stats_sum += stats;
        self.settle_stats();
        true
    }

    /// Whether every block's measure is real and final (no reflow of a
    /// very long block is still under way).
    pub(crate) fn settled(&self) -> bool {
        self.unmeasured == 0 && self.seamed.is_empty()
    }

    /// One slice of background work: counts the words of blocks that have
    /// no counts yet (first, because it is quick and the status bar waits
    /// for it), then measures blocks that only have an estimate, until
    /// `deadline` passes or `max_blocks` were handled. Returns whether work
    /// remains. Very long blocks are only counted here with `count_large`;
    /// otherwise they are left to a background thread (see
    /// [`Self::large_uncounted`]).
    pub(crate) fn refine(
        &mut self,
        document: &Document,
        deadline: Option<Instant>,
        max_blocks: usize,
        count_large: bool,
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
        let leave = if count_large { 0 } else { self.uncounted_large };
        while self.uncounted > leave && scanned < count && !expired(handled) {
            if index >= count {
                index = 0;
            }
            let slot = &self.slots[index];
            if slot.stats.is_none() && (count_large || !slot.large) {
                let large = slot.large;
                let stats = Stats::of_text(document.blocks()[index].text());
                self.slots[index].stats = Some(stats);
                self.stats_sum += stats;
                self.uncounted -= 1;
                self.uncounted_large -= usize::from(large);
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
            // Reflows that an edit to a very long block cut short.
            while !self.seamed.is_empty() && !expired(handled) {
                if !self.mend(document) {
                    break;
                }
                handled += 1;
            }
        }
        self.pending()
    }

    /// Measures and counts every block now (compact editors, tests).
    pub(crate) fn measure_all(&mut self, document: &Document) {
        while self.refine(document, None, usize::MAX, true) {}
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

    /// Content-space y of the top of the first row of block `index`
    /// (clamped): its box top plus its inner padding. Needs no shaping.
    pub(crate) fn first_row_top(&mut self, index: usize) -> Pixels {
        let Some(last) = self.slots.len().checked_sub(1) else {
            return px(0.);
        };
        let index = index.min(last);
        self.place_through(index);
        let slot = &self.slots[index];
        px(slot.place.top + slot.measure.pad_top)
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

    /// Shapes the blocks in the viewport (`height` layout units from the
    /// scroll position) until their placement no longer changes, and returns
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

    /// How far the content is scrolled, in layout units from its top.
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
        let viewport = self.env.as_ref().map_or(px(0.), LayoutEnv::view_height);
        (height - viewport).max(px(0.))
    }

    /// How far the view is scrolled sideways, in layout units: where it
    /// was put, or where it rests, inside what the environment allows.
    pub(crate) fn scroll_x(&self) -> Pixels {
        let Some(env) = &self.env else {
            return px(0.);
        };
        self.scroll_x
            .unwrap_or(env.rest_scroll_x)
            .clamp(px(0.), env.max_scroll_x)
    }

    /// Scrolls sideways to `x` layout units; `None` lets the view rest
    /// where the environment puts it.
    pub(crate) fn set_scroll_x(&mut self, x: Option<Pixels>) {
        self.scroll_x = x;
    }

    pub(crate) fn max_scroll_x(&self) -> Pixels {
        self.env.as_ref().map_or(px(0.), |env| env.max_scroll_x)
    }

    /// The transform from layout coordinates to the window, for the
    /// scroll position as it is now.
    pub(crate) fn view(&mut self) -> View {
        let scroll_y = self.scroll_y();
        let scroll_x = self.scroll_x();
        match &self.env {
            Some(env) => View {
                origin: point(
                    env.column_left - scroll_x * env.zoom,
                    env.viewport.top() - scroll_y * env.zoom,
                ),
                zoom: env.zoom,
            },
            None => View {
                origin: Point::default(),
                zoom: 1.,
            },
        }
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

    /// The layout of block `index` shaped from scratch: nothing taken from
    /// the cache, nothing put into it.
    #[cfg(test)]
    pub(crate) fn fresh_layout(&mut self, index: usize, document: &Document) -> BlockLayout {
        let block = document.block(index).expect("a block");
        let marker = self.marker(self.slots[index].digits);
        let env = self.env.as_ref().expect("laid out");
        BlockLayout::shape(
            block,
            &env.style,
            env.column_width,
            &env.text_system,
            marker,
            None,
            Reflow::Edit,
        )
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

    /// The line box of the position at `offset`: zero width, one row high.
    /// x from the column's left edge, y in content space.
    pub(crate) fn line_box(&self, offset: usize) -> Bounds<Pixels> {
        self.caret_box(offset, false)
    }

    /// The line box of a caret at `offset`: with `upstream`, a caret at a
    /// soft wrap is at the end of the row that ends there.
    pub(crate) fn caret_box(&self, offset: usize, upstream: bool) -> Bounds<Pixels> {
        let (row, bounds) = self.layout.caret_bounds_on(offset, upstream);
        self.lift(row, bounds)
    }

    /// One rectangle per row for the bytes in `range`, one row high. Only
    /// the rows in `rows` are looked at.
    pub(crate) fn rects(&self, range: Range<usize>, rows: Range<usize>) -> Vec<Bounds<Pixels>> {
        self.row_rects(range, rows)
            .into_iter()
            .map(|(_, bounds)| bounds)
            .collect()
    }

    /// [`Self::rects`] with the row each rectangle is on.
    pub(crate) fn row_rects(
        &self,
        range: Range<usize>,
        rows: Range<usize>,
    ) -> Vec<(usize, Bounds<Pixels>)> {
        self.layout
            .rects_for_range(range, rows)
            .into_iter()
            .map(|(row, bounds)| (row, self.lift(row, bounds)))
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

    /// The transform from layout coordinates (column-local x,
    /// content-space y) to the window.
    pub(crate) fn view(&mut self) -> View {
        self.layout.view()
    }

    /// Content-space y of the bottom edge of the viewport.
    pub(crate) fn view_bottom(&mut self) -> Pixels {
        self.layout.scroll_y() + self.env().view_height()
    }

    /// The sheet (0-based) that content-space `y` is on; 0 in flow view.
    pub(crate) fn page_at(&self, y: Pixels) -> usize {
        let env = self.env();
        env.page
            .as_ref()
            .map_or(0, |geometry| geometry.page_at(y, env.pad_top))
    }
}
