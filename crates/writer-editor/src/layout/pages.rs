//! Placement: where each block's box goes in the scrollable content, in
//! flow view (one column) and in page view (rows distributed over sheets).
//!
//! Placement is a pure function of the previous block's end, the block's
//! [`Measure`] and (for keeping a heading with what follows) the next
//! block's measure. `DocumentLayout` runs it block by block and stores the
//! results; nothing here shapes text.
//!
//! # Content space
//!
//! All y values are in content space: pixels from the top of the scrollable
//! content. In flow view a block's box simply follows the previous one
//! after a gap. In page view sheets are stacked with [`PAGE_GAP`] between
//! them; rows flow from sheet to sheet:
//!
//! - a row is never split and never crosses the bottom of the text area;
//! - a block may continue on the next sheet at a row boundary, unless its
//!   measure says [`Measure::keep_together`] and it fits on one sheet;
//! - no single row is left alone: a block of up to three rows is not
//!   split, a longer one leaves at least two rows at the bottom of a sheet
//!   and takes at least two to the top of the next;
//! - a heading moves to the next sheet together with the first row of the
//!   block after it, and a run of headings moves together
//!   ([`Measure::keep_with_next`], [`kept_height`]);
//! - the block after a page break starts a new sheet.
//!
//! A block that continues on later sheets keeps one box top; its later
//! rows are moved down by [`RowShift`]s (the distance to the next sheet's
//! text area). Everything that maps rows to y goes through
//! [`row_shift`] / [`Placement`].

use std::rc::Rc;

use gpui::{Bounds, Pixels, px};
use writer_core::PageSetup;

use super::{block::Measure, style::Group};

/// Space between two sheets, and around them, in pixels.
pub(crate) const PAGE_GAP: f32 = 20.;
/// Space between the sheets and the editor's top and bottom edges.
pub(crate) const DESK_PADDING: f32 = 24.;
/// Smallest space kept left and right of a sheet.
const DESK_SIDE_PADDING: f32 = 16.;
/// The page is never scaled below this.
const MIN_SCALE: f32 = 0.2;
/// Tolerance for "fits on the page" comparisons (scaled layouts are not
/// whole pixels).
const EPSILON: f32 = 0.01;
/// How many blocks after a heading can decide where the heading goes (a
/// run of headings is kept together with the text after it).
pub(crate) const KEEP_LOOKAHEAD: usize = 4;

/// The sheets of page view for one viewport, in layout pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PageGeometry {
    /// Window x of the sheets' left edge.
    pub(crate) sheet_left: Pixels,
    pub(crate) sheet_width: Pixels,
    pub(crate) sheet_height: Pixels,
    pub(crate) margin_top: Pixels,
    pub(crate) margin_bottom: Pixels,
    pub(crate) margin_left: Pixels,
    pub(crate) content_width: Pixels,
    pub(crate) content_height: Pixels,
    pub(crate) gap: Pixels,
    /// Layout pixels per point: 1, or less when the viewport is narrower
    /// than the page.
    pub(crate) scale: f32,
}

impl PageGeometry {
    pub(crate) fn new(setup: &PageSetup, viewport: Bounds<Pixels>) -> Self {
        let (page_width, page_height) = setup.page_size();
        let (content_width, content_height) = setup.content_size();
        let available = f32::from(viewport.size.width) - DESK_SIDE_PADDING * 2.;
        let scale = if page_width > 0. {
            (available / page_width).clamp(MIN_SCALE, 1.)
        } else {
            1.
        };
        let sheet_width = px(page_width * scale);
        let sheet_left = viewport.left()
            + ((viewport.size.width - sheet_width) / 2.)
                .max(px(0.))
                .round();
        Self {
            sheet_left,
            sheet_width,
            sheet_height: px(page_height * scale),
            margin_top: px(setup.margins.top * scale),
            margin_bottom: px(setup.margins.bottom * scale),
            margin_left: px(setup.margins.left * scale),
            content_width: px((content_width * scale).max(1.)),
            content_height: px((content_height * scale).max(1.)),
            gap: px(PAGE_GAP),
            scale,
        }
    }

    /// Whether `other` places blocks exactly as this geometry does (only
    /// the horizontal position may differ).
    pub(crate) fn same_flow(&self, other: &Self) -> bool {
        self.sheet_height == other.sheet_height
            && self.margin_top == other.margin_top
            && self.content_height == other.content_height
            && self.gap == other.gap
    }

    /// Distance from one sheet's top to the next one's.
    pub(crate) fn pitch(&self) -> Pixels {
        self.sheet_height + self.gap
    }

    /// Content-space y of the top of sheet `page` (0-based).
    pub(crate) fn sheet_top(&self, page: usize, pad_top: Pixels) -> Pixels {
        pad_top + self.pitch() * page as f32
    }

    /// The sheet whose pitch contains content-space `y` (0-based).
    pub(crate) fn page_at(&self, y: Pixels, pad_top: Pixels) -> usize {
        ((y - pad_top) / self.pitch()).floor().max(0.) as usize
    }
}

/// Rows from `first_row` on are `shift` further down than the block's box
/// alone would put them (cumulative: the last entry at or before a row
/// applies).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RowShift {
    pub(crate) first_row: u32,
    pub(crate) shift: f32,
}

/// The shift of row `row` for a block's shifts.
pub(crate) fn row_shift(shifts: Option<&[RowShift]>, row: usize) -> Pixels {
    let Some(shifts) = shifts else {
        return px(0.);
    };
    let ix = shifts.partition_point(|entry| entry.first_row as usize <= row);
    if ix == 0 {
        px(0.)
    } else {
        px(shifts[ix - 1].shift)
    }
}

/// Where one block goes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Placement {
    /// Content-space y of the top of the block's box.
    pub(crate) top: f32,
    /// Flow view: content-space y of the bottom of the box. Page view: y
    /// of the bottom of the box inside its last sheet's text area.
    pub(crate) end: f32,
    /// Sheet of the first row (0 in flow view).
    pub(crate) page: u32,
    /// Sheet of the last row.
    pub(crate) end_page: u32,
    pub(crate) shifts: Option<Rc<[RowShift]>>,
}

/// The end of the previous block.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Previous<'a> {
    pub(crate) measure: &'a Measure,
    pub(crate) end: f32,
    pub(crate) end_page: u32,
}

/// Vertical space between two neighbouring blocks.
pub(crate) fn gap(previous: &Measure, next: &Measure) -> f32 {
    if previous.group != Group::None && previous.group == next.group {
        previous.group_gap
    } else {
        previous.space_after + next.space_before
    }
}

/// The room a block that is kept with what follows needs on a sheet: its
/// own box, every heading directly after it, and the start
/// ([`Measure::lead`]) of the first block that is not a heading. `None`
/// for a block that is not kept with anything.
pub(crate) fn kept_height<'a>(
    measure: &Measure,
    following: impl Iterator<Item = &'a Measure>,
) -> Option<f32> {
    if !measure.keep_with_next {
        return None;
    }
    let mut total = measure.height();
    let mut previous = measure;
    for next in following.take(KEEP_LOOKAHEAD) {
        total += gap(previous, next);
        if !next.keep_with_next {
            return Some(total + next.lead());
        }
        total += next.height();
        previous = next;
    }
    Some(total)
}

/// Flow view: the box follows the previous one.
pub(crate) fn place_flow(previous: Option<Previous>, measure: &Measure, pad_top: f32) -> Placement {
    let top = match previous {
        Some(previous) => previous.end + gap(previous.measure, measure),
        None => pad_top,
    };
    Placement {
        top,
        end: top + measure.height(),
        page: 0,
        end_page: 0,
        shifts: None,
    }
}

/// Page view: the rows are distributed over sheets. `kept` is the block's
/// [`kept_height`].
pub(crate) fn place_paged(
    geometry: &PageGeometry,
    pad_top: f32,
    previous: Option<Previous>,
    measure: &Measure,
    kept: Option<f32>,
) -> Placement {
    let page_height = f32::from(geometry.content_height);
    let pitch = f32::from(geometry.pitch());
    let content_top = |page: u32| pad_top + pitch * page as f32 + f32::from(geometry.margin_top);
    let line_height = measure.line_height.max(1.);
    let rows = measure.rows.max(1);
    let whole = measure.height();

    let (mut page, mut y) = match previous {
        None => (0, 0.),
        Some(previous) if previous.measure.page_break => (previous.end_page + 1, 0.),
        Some(previous) => (
            previous.end_page,
            previous.end + gap(previous.measure, measure),
        ),
    };
    if y > 0. {
        // What has to fit under `y` for the block to start on this sheet.
        let mut needed = measure.lead();
        if needed > page_height + EPSILON {
            // Too tall to keep together even on an empty sheet.
            needed = measure.pad_top + line_height;
        }
        if let Some(kept) = kept
            && kept <= page_height + EPSILON
        {
            needed = needed.max(kept);
        }
        if y + needed > page_height + EPSILON {
            page += 1;
            y = 0.;
        }
    }

    let top = content_top(page) + y;
    let first_fit = (((page_height - y - measure.pad_top + EPSILON) / line_height).floor() as i64)
        .max(1) as u32;
    if rows <= first_fit {
        return Placement {
            top,
            end: y + whole,
            page,
            end_page: page,
            shifts: None,
        };
    }
    // The block continues on the following sheets: rows per sheet.
    let per_page = (((page_height + EPSILON) / line_height).floor() as i64).max(1) as u32;
    let mut chunks = vec![first_fit];
    let mut left = rows - first_fit;
    while left > 0 {
        let take = left.min(per_page);
        chunks.push(take);
        left -= take;
    }
    // The last row does not go to a sheet alone: it takes the one before
    // it along.
    if let [.., before, last] = chunks.as_mut_slice()
        && *last == 1
        && *before >= 3
    {
        *before -= 1;
        *last += 1;
    }
    let mut shifts = Vec::with_capacity(chunks.len() - 1);
    let mut row = chunks[0];
    let mut sheet = page;
    for rows_here in &chunks[1..] {
        sheet += 1;
        // Without the shift the row would sit at `natural`.
        let natural = top + measure.pad_top + row as f32 * line_height;
        shifts.push(RowShift {
            first_row: row,
            shift: content_top(sheet) - natural,
        });
        row += rows_here;
    }
    let rows_on_last = chunks[chunks.len() - 1];
    Placement {
        top,
        end: rows_on_last as f32 * line_height + measure.pad_bottom,
        page,
        end_page: sheet,
        shifts: Some(shifts.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(content_height: f32) -> PageGeometry {
        PageGeometry {
            sheet_left: px(0.),
            sheet_width: px(600.),
            sheet_height: px(content_height + 144.),
            margin_top: px(72.),
            margin_bottom: px(72.),
            margin_left: px(72.),
            content_width: px(456.),
            content_height: px(content_height),
            gap: px(PAGE_GAP),
            scale: 1.,
        }
    }

    fn paragraph(rows: u32) -> Measure {
        Measure {
            rows,
            line_height: 20.,
            space_after: 10.,
            ..Measure::default()
        }
    }

    /// Places `measures` in order and returns every row's (page, y in the
    /// text area, bottom).
    fn rows_of(geometry: &PageGeometry, measures: &[Measure]) -> Vec<Vec<(usize, f32, f32)>> {
        let pad_top = DESK_PADDING;
        let pitch = f32::from(geometry.pitch());
        let mut placed: Vec<Placement> = Vec::new();
        let mut out = Vec::new();
        for (ix, measure) in measures.iter().enumerate() {
            let previous = ix.checked_sub(1).map(|prev| Previous {
                measure: &measures[prev],
                end: placed[prev].end,
                end_page: placed[prev].end_page,
            });
            let placement = place_paged(
                geometry,
                pad_top,
                previous,
                measure,
                kept_height(measure, measures[ix + 1..].iter()),
            );
            let mut rows = Vec::new();
            for row in 0..measure.rows as usize {
                let y = placement.top
                    + measure.pad_top
                    + row as f32 * measure.line_height
                    + f32::from(row_shift(placement.shifts.as_deref(), row));
                let page = ((y - pad_top) / pitch).floor() as usize;
                let in_page = y - pad_top - pitch * page as f32 - f32::from(geometry.margin_top);
                rows.push((page, in_page, in_page + measure.line_height));
            }
            placed.push(placement);
            out.push(rows);
        }
        out
    }

    #[test]
    fn rows_never_cross_the_text_area() {
        let geometry = geometry(205.);
        let measures: Vec<Measure> = [3, 1, 7, 30, 2, 1, 1, 12, 5]
            .into_iter()
            .map(paragraph)
            .collect();
        let rows = rows_of(&geometry, &measures);
        let mut last = (0, -1.0_f32);
        for block in &rows {
            for (page, top, bottom) in block {
                assert!(*top >= -EPSILON, "row above the text area: {top}");
                assert!(
                    *bottom <= 205. + EPSILON,
                    "row below the text area: {bottom}"
                );
                assert!((*page, *top) > last, "rows go down the pages in order");
                last = (*page, *top);
            }
        }
        // 62 rows of 20px with 10px between blocks on 205px sheets: the
        // 30-row block alone spans four sheets.
        assert!(rows[3].first().unwrap().0 < rows[3].last().unwrap().0);
        assert_eq!(rows.last().unwrap().last().unwrap().0, 6);
    }

    #[test]
    fn page_break_heading_and_void_rules() {
        let geometry = geometry(200.);
        let page_break = Measure {
            page_break: true,
            keep_together: true,
            ..paragraph(1)
        };
        let heading = Measure {
            keep_with_next: true,
            keep_together: true,
            ..paragraph(2)
        };
        // A page break sends the next block to a new sheet.
        let rows = rows_of(&geometry, &[paragraph(2), page_break, paragraph(1)]);
        assert_eq!(rows[0][0].0, 0);
        assert_eq!(rows[1][0].0, 0);
        assert_eq!((rows[2][0].0, rows[2][0].1), (1, 0.));

        // 7 rows (140px) + gap: the two-row heading would fit (190px) but
        // the first row of the next block would not, so both move.
        let rows = rows_of(&geometry, &[paragraph(7), heading, paragraph(3)]);
        assert_eq!(rows[1][0].0, 1);
        assert_eq!(rows[1][0].1, 0.);
        assert_eq!(rows[2][0].0, 1);

        // Without the keep rule the heading would stay behind.
        let loose = Measure {
            keep_with_next: false,
            ..heading
        };
        let rows = rows_of(&geometry, &[paragraph(7), loose, paragraph(3)]);
        assert_eq!(rows[1][0].0, 0);
        assert_eq!(rows[2][0].0, 1);

        // A run of headings moves as one: the first would fit together
        // with the second (130 + 40 + 10 + 20 = 200), but the text after
        // the second would not.
        let rows = rows_of(&geometry, &[paragraph(6), heading, heading, paragraph(3)]);
        assert_eq!((rows[1][0].0, rows[1][0].1), (1, 0.));
        assert_eq!(rows[2][0].0, 1);
        assert_eq!(rows[3][0].0, 1);
        // With room for all of it nothing moves.
        let rows = rows_of(&geometry, &[paragraph(1), heading, heading, paragraph(3)]);
        assert!(rows.iter().all(|block| block[0].0 == 0));

        // A block that fills a sheet exactly leaves the next one on the
        // following sheet at its top, without the gap.
        let rows = rows_of(&geometry, &[paragraph(10), paragraph(1)]);
        assert_eq!(rows[0].last().unwrap().2, 200.);
        assert_eq!((rows[1][0].0, rows[1][0].1), (1, 0.));
    }

    #[test]
    fn no_row_is_left_alone() {
        let geometry = geometry(200.);
        let split = |before: u32, rows: u32| {
            let all = rows_of(&geometry, &[paragraph(before), paragraph(rows)]);
            let first_page = all[1][0].0;
            let on_first = all[1].iter().filter(|row| row.0 == first_page).count();
            (first_page, on_first, all[1].len() - on_first)
        };
        // Three rows fit under 130px, the block has five: 3 + 2.
        assert_eq!(split(6, 5), (0, 3, 2));
        // Four rows fit under 110px: the fifth would be alone on the next
        // sheet, so it takes the fourth along.
        assert_eq!(split(5, 5), (0, 3, 2));
        // All five fit under 90px.
        assert_eq!(split(4, 5), (0, 5, 0));
        // One row fits: a long block starts on the next sheet instead.
        assert_eq!(split(8, 6), (1, 6, 0));
        // Two rows fit: enough to start a long block.
        assert_eq!(split(7, 6), (0, 2, 4));
        // A block of three rows is never split.
        assert_eq!(split(7, 3), (1, 3, 0));
        assert_eq!(split(6, 3), (0, 3, 0));
        // A block longer than a sheet: full sheets, and the last row is
        // not alone (10 + 10 + 1 becomes 10 + 9 + 2).
        let all = rows_of(&geometry, &[paragraph(21)]);
        let per_sheet = |sheet: usize| all[0].iter().filter(|row| row.0 == sheet).count();
        assert_eq!((per_sheet(0), per_sheet(1), per_sheet(2)), (10, 9, 2));
    }

    #[test]
    fn flow_stacks_boxes_with_group_gaps() {
        let item = Measure {
            group: Group::List,
            group_gap: 4.,
            ..paragraph(1)
        };
        let first = place_flow(None, &item, 48.);
        assert_eq!((first.top, first.end), (48., 68.));
        let previous = Previous {
            measure: &item,
            end: first.end,
            end_page: 0,
        };
        assert_eq!(place_flow(Some(previous), &item, 48.).top, 72.);
        assert_eq!(place_flow(Some(previous), &paragraph(1), 48.).top, 78.);
    }

    #[test]
    fn geometry_scales_down_to_fit() {
        let setup = PageSetup::default();
        let (page_width, _) = setup.page_size();
        let wide = PageGeometry::new(
            &setup,
            Bounds::new(gpui::point(px(0.), px(0.)), gpui::size(px(1200.), px(800.))),
        );
        assert_eq!(wide.scale, 1.);
        assert_eq!(wide.sheet_width, px(page_width));
        let narrow = PageGeometry::new(
            &setup,
            Bounds::new(gpui::point(px(0.), px(0.)), gpui::size(px(400.), px(800.))),
        );
        assert!(narrow.scale < 1.);
        assert!(narrow.sheet_left >= px(DESK_SIDE_PADDING - 0.5));
        assert!(narrow.sheet_left + narrow.sheet_width <= px(400. - DESK_SIDE_PADDING + 0.5));
        assert!(!wide.same_flow(&narrow));
    }
}
