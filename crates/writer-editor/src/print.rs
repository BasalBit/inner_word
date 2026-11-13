//! The print pass: the document as page view lays it out, as data for paper.
//!
//! [`Editor::print_pass`] takes a copy of the document as it is at that
//! moment and lays it out with a layout of its own: the document's paper,
//! margins and typography, Paper's colours, zoom 1. What the editor shows
//! (flow view, any zoom, Night, a selection, spelling marks) has no part in
//! it, and later edits are not in it. [`PrintPass::advance`] then walks the
//! blocks in slices of a few milliseconds and hands out finished
//! [`writer_print::Page`]s: plain data that a PDF writer draws.
//!
//! # The same pages as page view
//!
//! The pass does not have a pagination of its own. It builds the
//! `LayoutEnv` of page view's branch of `Editor::install_layout_env`
//! (without the zoom and the room of a window) and asks the same
//! `DocumentLayout` where every block and every row is. Colours do not
//! shape text (`LayoutStyle::same_shape`), so the rows are page view's rows
//! whatever theme the window has. The contract the tests hold: the pass
//! hands out exactly [`Editor::page_count`] pages (once the editor's own
//! layout is settled; until then that number rests on estimates), and the
//! rows on each page are the rows page view has on it.
//!
//! The pass estimates nothing. Where a block goes depends on the measures
//! of the blocks before it and of at most `KEEP_LOOKAHEAD` after it (a
//! heading stays with what follows), so it shapes that far ahead and then
//! places the block for good. A page is handed out once a block starts on
//! a later one: that page and every page between it and the block's own,
//! so a sheet with nothing to print on it (two page breaks in a row) is
//! handed out with its number like any other. After the last block comes
//! what is left up to the layout's page count, which counts the empty
//! sheet after a page break that ends the document.
//!
//! # What a page holds
//!
//! In painting order, block by block: the block's decorations (bullet,
//! to-do box, quote bar, code panel, rule), the picture of an image block
//! (see "Pictures" below), then for every row its
//! backgrounds (highlights, the chips of inline code), the row itself
//! ([`writer_print::TextRow`]: its text, the font and colour of every
//! run, and where each shaped glyph starts), its underlines and
//! strikethroughs, then the block's labels (a list number, the language of
//! a code block) and strokes (the tick of a to-do). Last on a page: its
//! number.
//!
//! All of the geometry is the screen's: the decorations come from
//! `decorations::plan_block` through a sink of this module, the rectangles
//! of a row from `paint::PlannedRow`, the glyph starts from
//! `BlockLayout::for_each_glyph` (alignment, justification and the smaller
//! size of inline code are in them), the page number from
//! `decorations::PageNumber`. The view those are asked with is a sheet of
//! paper: its origin at the sheet's top left corner, one pixel to the
//! point. A block is handed to them as it lies on that sheet
//! (`Placed::on_sheet`), so nothing on a page is worked out from a y far
//! down a long document: page 1,500 of a book is as exact as page 1.
//!
//! Left out by not being added: everything a frame adds for whoever edits
//! (comment tints, find marks, spelling marks, the selection, the caret,
//! the placeholder of an empty document, the scroll bar, the desk, sheet
//! shadows and borders), and the two things the decorations mark as
//! "screen only" or as a surface fill: the page break marker, and the
//! paper-coloured inside of an open to-do box.
//!
//! Colours are Paper's. A colour with alpha (a highlight) is blended onto
//! white when its item is made: a printed page has no transparency.
//!
//! # Pictures
//!
//! An image block puts its picture on the page at its box: the box the
//! layout gave it, which is page view's (the same share of the column,
//! never taller than the text area, moved to the next sheet as a whole
//! when it does not fit). The item carries the stored bytes as they are
//! (PNG, JPEG or GIF; the PDF writer decodes them) and the image's id as
//! its key, so a picture that is in a document twice is in the PDF once.
//! The bytes come from whoever made the pass ([`PrintPass::set_images`]):
//! the pass reads no library. Without them, and for an image nobody has
//! bytes for, the box is drawn as a hairline with the image's description
//! in its middle, so the page still says that a picture belongs there.
//!
//! # Fonts
//!
//! A row names its fonts as the layout asked for them (`gpui::Font` as a
//! [`writer_print::FontSpec`]: family, weight, style, fallback families),
//! never as the font a glyph was shaped with: a `FontId` cannot be turned
//! back into a font for a glyph CoreText took from a fallback font
//! (`docs/gpui-notes.md`). The PDF writer finds the faces again the way
//! GPUI did and lets CoreText find the fallbacks again.

use std::{
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{App, Bounds, Font, FontStyle, Hsla, Pixels, Rgba, Window, point, px, size};
use writer_core::{BlockKind, Document, ImageId};
use writer_print::{
    Anchor, Color, FontSpec, GlyphStart, ImageItem, Item, Label, Page, RectItem, RowRun,
    StrokeItem, TextRow,
};
use writer_ui::theme::{self, ActiveTheme, Theme};

use crate::{
    decorations::{
        self, BlockContext, CheckboxHit, DecorationSink, LabelAnchor, LabelShape, PageNumber,
        RectShape, ShapeFill, StrokeShape,
    },
    editor::Editor,
    layout::{
        DocumentLayout, Lay, LayoutEnv, LayoutStyle, PageGeometry, View, block::font_runs,
        pages::DESK_PADDING,
    },
    paint::PlannedRow,
};

/// How many numbers of numbered lists are worked out at a time.
const ORDINALS_AT_A_TIME: usize = 256;

/// How far a [`PrintPass`] has come.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrintProgress {
    /// The blocks whose rows are on their pages.
    pub blocks_done: usize,
    /// The blocks of the document.
    pub blocks: usize,
    /// The pages that were handed out.
    pub pages_done: usize,
}

/// A pass over a document that lays it out for paper and hands out its pages.
/// Made by [`Editor::print_pass`].
///
/// ```ignore
/// let mut pass = editor.read(cx).print_pass(window, cx);
/// let mut pages = Vec::new();
/// while !pass.advance(Duration::from_millis(8), &mut pages) {
///     // let the window draw, show `pass.progress()`, look for a cancel
/// }
/// ```
pub struct PrintPass {
    /// The document as it was when the pass was made.
    document: Document,
    /// A layout of that document as page view lays it out, at zoom 1.
    layout: DocumentLayout,
    geometry: PageGeometry,
    /// The next block to put on its pages.
    next: usize,
    /// The pages that were handed out.
    handed: usize,
    /// The items of the pages that are begun and not handed out: the first
    /// is page `handed`.
    open: VecDeque<Vec<Item>>,
    done: bool,
    /// The page number of every sheet, without its digits; `None` when the
    /// bottom margin has no room for one.
    number: Option<NumberPlace>,
    /// The fonts the layout asked for, as the page names them.
    specs: HashMap<Font, Arc<FontSpec>>,
    /// The numbers of the numbered items among the blocks from `.0` on.
    ordinals: (usize, Vec<Option<usize>>),
    /// Where the bytes of a picture come from ([`PrintPass::set_images`]).
    images: Option<ImageBytes>,
}

/// Answers with the stored bytes of an image, or with `None` when there
/// are none.
type ImageBytes = Rc<dyn Fn(ImageId) -> Option<Arc<[u8]>>>;

/// Where and how the number of a page is set.
struct NumberPlace {
    font: Arc<FontSpec>,
    size: f32,
    color: Color,
    baseline: f32,
}

impl Editor {
    /// A pass over the document as it is now, laid out as page view lays it
    /// out (the document's paper, margins and typography; Paper's colours;
    /// zoom 1), whatever this editor shows: flow view, another zoom, Night.
    /// It holds a copy of the document: later edits are not in it.
    ///
    /// The only thing taken from the window is its text system (GPUI has
    /// none without a window).
    pub fn print_pass(&self, window: &Window, cx: &App) -> PrintPass {
        let document = self.state.document().clone();
        let settings = document.settings();
        let geometry = PageGeometry::new(&settings.page);
        // Paper's colours and this Mac's fonts: what page view shows in the
        // light theme.
        let paper = Theme::paper(cx.theme().fonts.clone());
        let style = LayoutStyle::new(&paper, &settings.typography, false, true)
            .with_page_content(settings.page.content_size());
        let pad_top = px(DESK_PADDING);
        let env = LayoutEnv {
            text_system: window.text_system().clone(),
            style,
            viewport: Bounds::new(
                point(px(0.), px(0.)),
                size(geometry.sheet_width, geometry.sheet_height),
            ),
            zoom: 1.,
            column_left: geometry.margin_left,
            column_width: geometry.content_width,
            pad_top,
            pad_bottom: pad_top,
            page: Some(geometry),
            max_scroll_x: px(0.),
            rest_scroll_x: px(0.),
        };
        // The page number as page view draws it: the interface font, small,
        // faint, in the lower part of the bottom margin.
        let number =
            PageNumber::in_margin(geometry.margin_bottom, theme::UI_TEXT_XS).map(|number| {
                let sheet = Bounds::new(
                    point(px(0.), px(0.)),
                    size(geometry.sheet_width, geometry.sheet_height),
                );
                // Shaped once for the metrics of the font: where the baseline
                // lies in the number's box.
                let line = decorations::shape_label(
                    &env.text_system,
                    "0".into(),
                    env.style.ui_font.clone(),
                    number.font_size,
                    env.style.colors.faint,
                );
                let top = number.top(sheet, geometry.margin_bottom);
                let baseline =
                    top + (number.line_height - line.ascent - line.descent) / 2. + line.ascent;
                NumberPlace {
                    font: Arc::new(font_spec(&env.style.ui_font)),
                    size: number.font_size.into(),
                    color: on_paper(env.style.colors.faint),
                    baseline: baseline.into(),
                }
            });
        let mut layout = DocumentLayout::new(&document, self.state.revision());
        layout.set_env(env, &document);
        PrintPass {
            document,
            layout,
            geometry,
            next: 0,
            handed: 0,
            open: VecDeque::new(),
            done: false,
            number,
            specs: HashMap::new(),
            ordinals: (0, Vec::new()),
            images: None,
        }
    }
}

impl PrintPass {
    /// The size of the document's paper in points, orientation applied.
    pub fn paper(&self) -> (f32, f32) {
        self.document.settings().page.page_size()
    }

    /// Where the pass gets the bytes of a picture (PNG, JPEG or GIF, as
    /// they are stored). It is asked once for every image block, when the
    /// block is put on its page, on the thread the pass runs on: whoever
    /// prints reads the bytes beforehand and answers from memory. Without
    /// it, and for an id it answers `None` for, the picture's box is
    /// drawn with its description. Set it before the first
    /// [`PrintPass::advance`].
    pub fn set_images(&mut self, images: Rc<dyn Fn(ImageId) -> Option<Arc<[u8]>>>) {
        self.images = Some(images);
    }

    /// The images of the document the pass holds, each once, in the order
    /// they come: what [`PrintPass::set_images`] will be asked for.
    pub fn images(&self) -> Vec<ImageId> {
        let mut seen = std::collections::HashSet::new();
        self.document
            .images()
            .map(|(_, image)| image.id)
            .filter(|id| seen.insert(*id))
            .collect()
    }

    /// How far the pass has come.
    pub fn progress(&self) -> PrintProgress {
        PrintProgress {
            blocks_done: self.next.min(self.document.block_count()),
            blocks: self.document.block_count(),
            pages_done: self.handed,
        }
    }

    /// Works for about `budget` (always at least one block) and appends the
    /// pages it finished to `out`, in order and without a gap: every sheet
    /// page view has, also one with nothing to print on it (it holds only a
    /// page break, or it is the sheet after a page break that ends the
    /// document), with its page number. True when the last page has been
    /// handed out. A document always has one page.
    pub fn advance(&mut self, budget: Duration, out: &mut Vec<Page>) -> bool {
        if self.done {
            return true;
        }
        let started = Instant::now();
        loop {
            if self.next >= self.document.block_count() {
                // What is left: the pages that are begun, and the empty
                // sheet after a page break that ends the document.
                let pages = self
                    .layout
                    .page_count()
                    .max(self.handed + self.open.len())
                    .max(1);
                self.hand_out(pages, out);
                self.done = true;
                return true;
            }
            // Keeps the cache of shaped blocks at its limit.
            self.layout.begin_frame();
            self.place_block(self.next, out);
            self.next += 1;
            if started.elapsed() >= budget {
                return false;
            }
        }
    }

    /// Every page of the document, in one go. For a chapter; a book is
    /// better taken in slices ([`Self::advance`]), with the window drawing
    /// in between.
    pub fn run_to_end(&mut self) -> Vec<Page> {
        let mut pages = Vec::new();
        while !self.advance(Duration::MAX, &mut pages) {}
        pages
    }

    /// Hands out every page before `upto`.
    fn hand_out(&mut self, upto: usize, out: &mut Vec<Page>) {
        while self.handed < upto {
            let mut items = self.open.pop_front().unwrap_or_default();
            if let Some(number) = &self.number {
                items.push(Item::Label(Label {
                    text: (self.handed + 1).to_string(),
                    font: number.font.clone(),
                    size: number.size,
                    color: number.color,
                    x: f32::from(self.geometry.sheet_width) / 2.,
                    baseline: number.baseline,
                    anchor: Anchor::Middle,
                }));
            }
            out.push(Page {
                width: self.geometry.sheet_width.into(),
                height: self.geometry.sheet_height.into(),
                items,
            });
            self.handed += 1;
        }
    }

    /// The view of a sheet of paper: a block as it lies on the sheet
    /// (`Placed::on_sheet`) to points from the sheet's top left corner.
    fn sheet_view(&self) -> View {
        View {
            origin: point(self.geometry.margin_left, px(0.)),
            zoom: 1.,
        }
    }

    /// The number of block `index` in its numbered list.
    fn ordinal(&mut self, index: usize) -> Option<usize> {
        let (start, ordinals) = &self.ordinals;
        if index < *start || index >= start + ordinals.len() {
            let range = index..index + ORDINALS_AT_A_TIME;
            self.ordinals = (index, self.document.numbered_ordinals(range));
        }
        let (start, ordinals) = &self.ordinals;
        ordinals.get(index - start).copied().flatten()
    }

    /// Puts block `index` on its pages, and hands out the pages before the
    /// one it starts on.
    fn place_block(&mut self, index: usize, out: &mut Vec<Page>) {
        let ordinal = self.ordinal(index);
        // The blocks that can still move this one are measured first: a
        // heading goes to the next sheet with what follows it.
        let ahead = (index + crate::layout::pages::KEEP_LOOKAHEAD)
            .min(self.document.block_count().saturating_sub(1));
        for later in index..=ahead {
            self.layout.layout(later, &self.document);
        }
        let placed = Lay {
            layout: &mut self.layout,
            document: &self.document,
        }
        .placed(index);
        let Some(placed) = placed else {
            return;
        };
        // Whatever is on an earlier page is complete now.
        self.hand_out(placed.row_page(0), out);
        // (Blocks come in order: none starts on a page that was handed out.)
        let first_page = placed.row_page(0).max(self.handed);
        // The rows of each sheet the block is on.
        let starts = placed.segment_starts();

        let (Some(block), Some(env)) = (self.document.block(index), self.layout.env()) else {
            return;
        };
        let view = self.sheet_view();
        let sheet_height = self.geometry.sheet_height;
        let text_top = self.geometry.margin_top;
        let layout = &placed.layout;
        let fonts = if layout.void {
            Vec::new()
        } else {
            font_runs(block, &env.style)
        };
        let colors = &layout.decorations.colors;
        let quote_continues = matches!(block.kind(), BlockKind::Quote)
            && self
                .document
                .block(index + 1)
                .is_some_and(|next| matches!(next.kind(), BlockKind::Quote));

        let mut sheets: Vec<Sheet> = Vec::with_capacity(starts.len() - 1);
        for (segment, rows) in starts.windows(2).enumerate() {
            let mut sheet = Sheet::default();
            // The block as it lies on this sheet.
            let on_sheet = placed.on_sheet(segment, text_top);

            // Decorations: planned for this sheet, and what lies on it is
            // kept (the rest is another sheet's).
            decorations::plan_block(
                &mut SheetSink {
                    sheet: &mut sheet,
                    height: sheet_height,
                    specs: &mut self.specs,
                },
                &BlockContext {
                    index,
                    block,
                    placed: &on_sheet,
                    view,
                    ordinal,
                    quote_continues,
                    env,
                },
            );

            // The picture of an image block (a void block: one segment).
            if let (Some(picture), Some(image)) = (on_sheet.image_rect(), block.as_image()) {
                let bounds = view.rect(picture);
                let bytes = self.images.as_ref().and_then(|images| images(image.id));
                match bytes {
                    Some(bytes) => sheet.rows.push(Item::Image(ImageItem {
                        key: *image.id.as_bytes(),
                        bytes,
                        x: bounds.left().into(),
                        y: bounds.top().into(),
                        width: bounds.size.width.into(),
                        height: bounds.size.height.into(),
                    })),
                    None => {
                        // No bytes: the box, and what the picture shows in
                        // words.
                        let faint = on_paper(env.style.colors.faint);
                        sheet.rows.push(Item::Rect(RectItem {
                            x: bounds.left().into(),
                            y: bounds.top().into(),
                            width: bounds.size.width.into(),
                            height: bounds.size.height.into(),
                            radius: 0.,
                            fill: None,
                            border: Some((0.5, on_paper(env.style.colors.border_strong))),
                        }));
                        let font_size = theme::UI_TEXT_SM;
                        let line_height = (font_size * 1.4).round();
                        let room = bounds.size.width - px(16.);
                        if !image.description.is_empty()
                            && bounds.size.height >= line_height
                            && let Some(line) = decorations::shape_label_within(
                                &env.text_system,
                                &image.description,
                                env.style.ui_font.clone(),
                                font_size,
                                env.style.colors.faint,
                                room,
                            )
                        {
                            let middle = bounds.center();
                            let top = middle.y - line_height / 2.;
                            let baseline =
                                top + (line_height - line.ascent - line.descent) / 2. + line.ascent;
                            sheet.labels.push(Item::Label(Label {
                                text: line.text.to_string(),
                                font: spec_of(&mut self.specs, &env.style.ui_font),
                                size: font_size.into(),
                                color: faint,
                                x: middle.x.into(),
                                baseline: baseline.into(),
                                anchor: Anchor::Middle,
                            }));
                        }
                    }
                }
            }

            // Rows.
            for row_ix in rows[0]..rows[1] {
                if layout.void {
                    break;
                }
                let planned = PlannedRow {
                    layout: layout.clone(),
                    row: row_ix,
                    origin: view.point(point(layout.text_left, on_sheet.row_top(row_ix))),
                    zoom: 1.,
                };
                planned.for_each_background(|bounds, color, radius| {
                    sheet.rows.push(Item::Rect(RectItem {
                        radius: radius.into(),
                        ..filled(bounds, color)
                    }));
                });
                let row = &layout.rows[row_ix];
                let text = block.text().get(row.range.clone()).unwrap_or("");
                if let (false, Some(line)) = (text.is_empty(), layout.lines.get(row.line)) {
                    let mut glyphs = Vec::new();
                    layout.for_each_glyph(row_ix, |_, glyph, x, scale| {
                        let byte = (line.start + glyph.index).saturating_sub(row.range.start);
                        glyphs.push(GlyphStart {
                            byte: byte.min(text.len()) as u32,
                            x: x.into(),
                            scale,
                        });
                    });
                    // The fonts and the colours of the row: a run ends where
                    // either changes.
                    let mut runs: Vec<RowRun> = Vec::new();
                    let mut at = row.range.start;
                    while at < row.range.end {
                        let font_ix = fonts.partition_point(|(end, _)| *end <= at);
                        let color_ix = colors.partition_point(|(end, _)| *end <= at);
                        let font_end = fonts.get(font_ix).map_or(row.range.end, |(end, _)| *end);
                        let (color_end, color) = colors
                            .get(color_ix)
                            .map_or((row.range.end, layout.ink), |(end, color)| (*end, *color));
                        let end = font_end.min(color_end).min(row.range.end).max(at + 1);
                        let font = fonts
                            .get(font_ix)
                            .or(fonts.last())
                            .map_or(&env.style.body_font, |(_, font)| font);
                        let font = spec_of(&mut self.specs, font);
                        runs.push(RowRun {
                            end: (end - row.range.start).min(text.len()),
                            font,
                            color: on_paper(color),
                        });
                        at = end;
                    }
                    sheet.rows.push(Item::Row(TextRow {
                        text: text.to_string(),
                        x: planned.origin.x.into(),
                        baseline: (planned.origin.y + layout.baseline).into(),
                        size: layout.font_size.into(),
                        runs,
                        glyphs,
                        width: row.width.into(),
                    }));
                }
                planned.for_each_line(|bounds, color| {
                    sheet.rows.push(Item::Rect(filled(bounds, color)));
                });
            }
            sheets.push(sheet);
        }

        for (offset, sheet) in sheets.into_iter().enumerate() {
            let ix = first_page + offset - self.handed;
            while self.open.len() <= ix {
                self.open.push_back(Vec::new());
            }
            let items = &mut self.open[ix];
            items.extend(sheet.back);
            items.extend(sheet.rows);
            items.extend(sheet.labels);
            items.extend(sheet.strokes);
        }
    }
}

/// What one block puts on one sheet, in painting order.
#[derive(Default)]
struct Sheet {
    /// The block's decorations behind its text.
    back: Vec<Item>,
    /// Its rows: backgrounds, text, underlines.
    rows: Vec<Item>,
    /// Its labels and its strokes, over the rest.
    labels: Vec<Item>,
    strokes: Vec<Item>,
}

/// The sink of a sheet of paper: the shapes of a block's decorations become
/// the items of a printed page. The shapes are planned in the sheet's own
/// view, so a shape that lies on another sheet (a later part of a block
/// that goes on over several) is outside it and left to that sheet's turn.
struct SheetSink<'a> {
    sheet: &'a mut Sheet,
    height: Pixels,
    specs: &'a mut HashMap<Font, Arc<FontSpec>>,
}

impl SheetSink<'_> {
    /// Whether a y of a shape is on this sheet.
    fn holds(&self, y: Pixels) -> bool {
        y >= px(0.) && y <= self.height
    }
}

impl DecorationSink for SheetSink<'_> {
    fn rect(&mut self, rect: RectShape) {
        if rect.screen_only || !self.holds(rect.bounds.center().y) {
            return;
        }
        let fill = match rect.fill {
            ShapeFill::Color(color) => Some(on_paper(color)),
            // Paper is the surface.
            ShapeFill::None | ShapeFill::Surface(_) => None,
        };
        let border = rect
            .border
            .map(|(width, color)| (f32::from(width), on_paper(color)));
        if fill.is_none() && border.is_none() {
            return;
        }
        let bounds = rect.bounds;
        self.sheet.back.push(Item::Rect(RectItem {
            x: bounds.left().into(),
            y: bounds.top().into(),
            width: bounds.size.width.into(),
            height: bounds.size.height.into(),
            radius: rect.radius.into(),
            fill,
            border,
        }));
    }

    fn stroke(&mut self, stroke: StrokeShape) {
        let Some(first) = stroke.points.first() else {
            return;
        };
        if !self.holds(first.y) {
            return;
        }
        self.sheet.strokes.push(Item::Stroke(StrokeItem {
            points: stroke
                .points
                .iter()
                .map(|point| (point.x.into(), point.y.into()))
                .collect(),
            width: stroke.width.into(),
            color: on_paper(stroke.color),
        }));
    }

    fn label(&mut self, label: LabelShape) {
        if label.screen_only || !self.holds(label.origin.y + label.line_height / 2.) {
            return;
        }
        self.sheet.labels.push(Item::Label(Label {
            baseline: label.baseline().into(),
            x: label.anchor_x.into(),
            anchor: match label.anchor {
                LabelAnchor::Middle => Anchor::Middle,
                LabelAnchor::End => Anchor::End,
            },
            text: label.text.to_string(),
            font: spec_of(self.specs, &label.font),
            size: label.font_size.into(),
            color: on_paper(label.color),
        }));
    }

    fn checkbox(&mut self, _: CheckboxHit) {}
}

/// A filled rectangle with square corners.
fn filled(bounds: Bounds<Pixels>, color: Hsla) -> RectItem {
    RectItem {
        x: bounds.left().into(),
        y: bounds.top().into(),
        width: bounds.size.width.into(),
        height: bounds.size.height.into(),
        radius: 0.,
        fill: Some(on_paper(color)),
        border: None,
    }
}

/// A colour as it is on white paper: what shows through a translucent one is
/// the paper.
pub(crate) fn on_paper(color: Hsla) -> Color {
    let rgba: Rgba = color.into();
    let alpha = rgba.a.clamp(0., 1.);
    let part = |value: f32| (value * alpha + (1. - alpha)).clamp(0., 1.);
    Color {
        r: part(rgba.r),
        g: part(rgba.g),
        b: part(rgba.b),
    }
}

/// A font as a page names it: the layout's own words for it.
fn font_spec(font: &Font) -> FontSpec {
    FontSpec {
        family: font.family.to_string(),
        weight: font.weight.0,
        italic: !matches!(font.style, FontStyle::Normal),
        fallbacks: font
            .fallbacks
            .as_ref()
            .map(|fallbacks| fallbacks.fallback_list().to_vec())
            .unwrap_or_default(),
    }
}

/// The shared spec of a font: the rows of a document name a handful.
fn spec_of(specs: &mut HashMap<Font, Arc<FontSpec>>, font: &Font) -> Arc<FontSpec> {
    if let Some(spec) = specs.get(font) {
        return spec.clone();
    }
    let spec = Arc::new(font_spec(font));
    specs.insert(font.clone(), spec.clone());
    spec
}
