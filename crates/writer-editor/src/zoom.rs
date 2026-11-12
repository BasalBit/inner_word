//! The zoom of the view: the text larger or smaller on screen, and nothing
//! else changed.
//!
//! A zoom is not a text size. The document keeps its sizes in points, its
//! lines break where they broke and its pages hold what they held; the
//! editor only draws all of it at another scale. That is guaranteed by
//! where the zoom lives: the layout (`crate::layout`) is computed in the
//! document's own points and never sees it, and one transform
//! (`layout::View`) turns layout coordinates into window coordinates when
//! a frame is planned, a geometry query is answered or a pointer position
//! comes in. A zoom in page view therefore lays nothing out again.
//!
//! - **Page view**: a true zoom of the sheets. A sheet that is wider than
//!   the room it has does not shrink: the view scrolls sideways (two
//!   fingers on a trackpad, `shift` with a wheel), and the caret is kept in
//!   view while typing. [`Zoom::FitWidth`] and [`Zoom::FitPage`] follow the
//!   window instead.
//! - **Flow view**: the type and the spacing zoom. The column grows with
//!   them up to the width there is; beyond that it keeps the width of the
//!   window and the text reflows. Flow view never scrolls sideways.
//! - **Compact editors** (a comment composer) do not zoom.
//!
//! Each of the two views has a zoom of its own ([`Editor::set_view_zooms`]):
//! the right zoom for a sheet is not the right one for a column of text.
//!
//! What 100 % is: one point of the document is one pixel of the window
//! (a logical pixel: two device pixels on a Retina display), in both
//! views. That is what Pages and TextEdit draw at 100 %, and by everything
//! that could be found Word for Mac too (not Word for Windows, which draws
//! 96 pixels to the inch and so a third larger): a sheet at 100 % is as
//! wide on screen here as it is there. [`PAGE_VIEW_SCALE`] is that number
//! for page view, in one place.

use gpui::{Context, Pixels, Point};
use writer_core::Position;

use crate::{editor::Editor, events::EditorEvent, scroll::PendingScroll};

/// The smallest and the largest zoom a view can be set to, in percent.
pub const ZOOM_MIN: u16 = 50;
/// See [`ZOOM_MIN`].
pub const ZOOM_MAX: u16 = 400;
/// The steps "zoom in" and "zoom out" go through, in percent.
pub const ZOOM_STEPS: [u16; 14] = [
    50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400,
];
/// A zoom that follows the window ([`Zoom::FitWidth`], [`Zoom::FitPage`])
/// may go below [`ZOOM_MIN`], down to this many percent: a whole sheet in
/// a small window is small.
pub const ZOOM_FIT_MIN: f32 = 25.;

/// Window pixels per point in page view at 100 %.
///
/// One, as in Pages, TextEdit and Word for Mac: an A4 sheet is 595 pixels
/// wide at 100 %. (Word for Windows draws 96 pixels to the inch, which
/// would be 96/72 here; a Mac does not.) On a dense display a sheet at
/// 100 % is much smaller than a sheet of paper: that is what the zoom is
/// for.
pub const PAGE_VIEW_SCALE: f32 = 1.;

/// What a view is zoomed to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Zoom {
    /// A fixed zoom in percent, [`ZOOM_MIN`] to [`ZOOM_MAX`].
    Percent(u16),
    /// As wide as the room there is: the sheet in page view, the text
    /// column in flow view. Follows the window.
    FitWidth,
    /// As wide as the room there is, but not above this many percent: a
    /// sheet that fills the window where the window is narrow and stops
    /// growing at a size that means something (the size of the paper it
    /// stands for, say). Follows the window.
    FitWidthUpTo(u16),
    /// Page view: one whole sheet in the window. Follows the window. In
    /// flow view, which has no sheet, this is 100 %.
    FitPage,
}

impl Default for Zoom {
    fn default() -> Self {
        Zoom::Percent(100)
    }
}

impl Zoom {
    /// A fixed zoom, brought into the range a view can have.
    pub fn percent(percent: f32) -> Self {
        let percent = if percent.is_finite() { percent } else { 100. };
        Zoom::Percent(
            percent
                .round()
                .clamp(f32::from(ZOOM_MIN), f32::from(ZOOM_MAX)) as u16,
        )
    }

    /// Whether the zoom follows the window.
    pub fn is_fit(&self) -> bool {
        !matches!(self, Zoom::Percent(_))
    }
}

/// The step after `percent` on the way in (`None` at the largest).
pub fn step_above(percent: f32) -> Option<u16> {
    ZOOM_STEPS
        .iter()
        .copied()
        .find(|step| f32::from(*step) > percent + 0.5)
}

/// The step before `percent` on the way out (`None` at the smallest).
pub fn step_below(percent: f32) -> Option<u16> {
    ZOOM_STEPS
        .iter()
        .rev()
        .copied()
        .find(|step| f32::from(*step) < percent - 0.5)
}

impl Editor {
    /// The zoom of the view that is showing (flow or page).
    pub fn zoom(&self) -> Zoom {
        self.zoom_of(self.page_view())
    }

    /// The zoom of page view (`true`) or of flow view (`false`).
    pub fn zoom_of(&self, page_view: bool) -> Zoom {
        if page_view {
            self.zoom_page
        } else {
            self.zoom_flow
        }
    }

    /// The zoom the view is drawn at, in percent. For a fixed zoom that is
    /// the number itself; for one that follows the window it is what the
    /// last frame worked out (100 before the first one).
    pub fn zoom_percent(&self) -> f32 {
        match self.zoom() {
            Zoom::Percent(percent) => f32::from(percent),
            _ => self.zoom_shown,
        }
    }

    /// Sets the zoom of both views without moving anything and without an
    /// event: for whoever restores what was stored. A compact editor
    /// ignores its zooms.
    pub fn set_view_zooms(&mut self, flow: Zoom, page: Zoom, cx: &mut Context<Self>) {
        let (flow, page) = (clamped(flow), clamped(page));
        if (self.zoom_flow, self.zoom_page) != (flow, page) {
            self.zoom_flow = flow;
            self.zoom_page = page;
            self.layout.borrow_mut().set_scroll_x(None);
            cx.notify();
        }
    }

    /// Zooms the view that is showing. The line the caret is on stays
    /// where it is on screen (when it is on screen; otherwise the text at
    /// the top of the viewport stays there). Emits
    /// [`EditorEvent::ZoomChanged`] when the zoom is another one. Nothing
    /// of the document changes.
    pub fn set_zoom(&mut self, zoom: Zoom, cx: &mut Context<Self>) {
        let zoom = clamped(zoom);
        if zoom == self.zoom() || self.config.mode.is_compact() {
            return;
        }
        let caret_y = self.caret_viewport_y();
        self.store_zoom(zoom);
        // Sideways the view goes back to where it rests; the caret is
        // brought into view with the request below.
        self.layout.borrow_mut().set_scroll_x(None);
        if let Some(viewport_y) = caret_y {
            // The layout takes the zoom with the next frame; the request
            // is applied then.
            self.defer_scroll(PendingScroll::CaretAt { viewport_y });
        }
        cx.emit(EditorEvent::ZoomChanged);
        cx.notify();
    }

    /// Zooms the view that is showing around a point of the window (the
    /// pointer): what is under the point stays under it. Emits
    /// [`EditorEvent::ZoomChanged`] when the zoom is another one.
    pub fn set_zoom_at(&mut self, zoom: Zoom, point: Point<Pixels>, cx: &mut Context<Self>) {
        let zoom = clamped(zoom);
        if zoom == self.zoom() || self.config.mode.is_compact() {
            return;
        }
        let anchor = self.anchor_at(point);
        self.store_zoom(zoom);
        if let Some(anchor) = anchor {
            self.defer_scroll(anchor);
        }
        cx.emit(EditorEvent::ZoomChanged);
        cx.notify();
    }

    /// One step in ([`ZOOM_STEPS`]) from the zoom the view is drawn at.
    pub fn zoom_in(&mut self, cx: &mut Context<Self>) {
        if let Some(step) = step_above(self.zoom_percent()) {
            self.set_zoom(Zoom::Percent(step), cx);
        }
    }

    /// One step out from the zoom the view is drawn at.
    pub fn zoom_out(&mut self, cx: &mut Context<Self>) {
        if let Some(step) = step_below(self.zoom_percent()) {
            self.set_zoom(Zoom::Percent(step), cx);
        }
    }

    /// The wheel turned with `cmd` held, or two fingers pinched: zooms by
    /// `factor` (above 1: in) around `point`, in whole percent.
    pub fn zoom_by(&mut self, factor: f32, point: Point<Pixels>, cx: &mut Context<Self>) {
        if !factor.is_finite() || factor <= 0. {
            return;
        }
        // The exact zoom a run of small steps has reached, so that many
        // steps of less than a percent add up.
        let from = match self.zoom_fine {
            Some((shown, exact)) if Zoom::percent(shown) == self.zoom() => exact,
            _ => self.zoom_percent(),
        };
        let exact = (from * factor).clamp(f32::from(ZOOM_MIN), f32::from(ZOOM_MAX));
        let zoom = Zoom::percent(exact);
        self.set_zoom_at(zoom, point, cx);
        if let Zoom::Percent(percent) = self.zoom() {
            self.zoom_fine = Some((f32::from(percent), exact));
        }
    }

    fn store_zoom(&mut self, zoom: Zoom) {
        if self.page_view() {
            self.zoom_page = zoom;
        } else {
            self.zoom_flow = zoom;
        }
        self.zoom_fine = None;
        if let Zoom::Percent(percent) = zoom {
            self.zoom_shown = f32::from(percent);
        }
        self.link_tip = None;
    }

    /// The scroll request that puts what is under `point` back under it
    /// once the zoom has changed.
    fn anchor_at(&self, point: Point<Pixels>) -> Option<PendingScroll> {
        let viewport = self.viewport_bounds()?;
        self.with_layout(|lay| {
            let view = lay.view();
            let index = lay.block_at(view.layout_y(point.y));
            let placed = lay.placed(index)?;
            // Shaping the block may have moved it.
            let view = lay.view();
            let (x, y) = (view.layout_x(point.x), view.layout_y(point.y));
            let row = placed.row_at(y);
            Some(PendingScroll::Anchor {
                position: Position::new(index, placed.layout.rows[row].range.start),
                dy: y - placed.row_top(row),
                x,
                at: point - viewport.origin,
            })
        })
        .flatten()
    }

    /// Window pixels per layout unit for a viewport: the zoom of the view
    /// that is showing, worked out for the window where it follows it.
    /// Also returns the zoom in percent.
    pub(crate) fn zoom_for(
        &self,
        paged: bool,
        room: gpui::Size<Pixels>,
        fit_width: Pixels,
        fit_height: Pixels,
    ) -> (f32, f32) {
        let scale = if paged { PAGE_VIEW_SCALE } else { 1. };
        let fit = |room: Pixels, length: Pixels| {
            let length = f32::from(length) * scale;
            if length > 0. {
                (f32::from(room) / length * 100.).clamp(ZOOM_FIT_MIN, f32::from(ZOOM_MAX))
            } else {
                100.
            }
        };
        let percent = match self.zoom_of(paged) {
            Zoom::Percent(percent) => f32::from(percent),
            Zoom::FitWidth => fit(room.width, fit_width),
            Zoom::FitWidthUpTo(limit) => fit(room.width, fit_width).min(f32::from(limit)),
            Zoom::FitPage if paged => fit(room.width, fit_width).min(fit(room.height, fit_height)),
            Zoom::FitPage => 100.,
        };
        (percent / 100. * scale, percent)
    }

    /// Records the zoom a frame was laid out at, and tells whoever shows
    /// it when a zoom that follows the window came out differently.
    pub(crate) fn note_zoom_shown(&mut self, percent: f32, cx: &mut Context<Self>) {
        if (self.zoom_shown - percent).abs() < 0.05 {
            return;
        }
        self.zoom_shown = percent;
        if self.zoom().is_fit() {
            cx.emit(EditorEvent::ZoomChanged);
        }
    }
}

fn clamped(zoom: Zoom) -> Zoom {
    match zoom {
        Zoom::Percent(percent) => Zoom::Percent(percent.clamp(ZOOM_MIN, ZOOM_MAX)),
        Zoom::FitWidthUpTo(limit) => Zoom::FitWidthUpTo(limit.clamp(ZOOM_MIN, ZOOM_MAX)),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_go_in_and_out_from_any_zoom() {
        assert_eq!(step_above(100.), Some(110));
        assert_eq!(step_below(100.), Some(90));
        // From a zoom between two steps (a fit, the wheel) the next step
        // is the nearest one in that direction.
        assert_eq!(step_above(83.4), Some(90));
        assert_eq!(step_below(83.4), Some(80));
        assert_eq!(step_above(66.2), Some(67));
        assert_eq!(step_above(66.7), Some(75));
        assert_eq!(step_below(67.2), Some(50));
        assert_eq!(step_above(400.), None);
        assert_eq!(step_below(50.), None);
        assert_eq!(step_below(30.), None);
        assert_eq!(step_above(30.), Some(50));
        // Every step is reached from its neighbour.
        for pair in ZOOM_STEPS.windows(2) {
            assert_eq!(step_above(f32::from(pair[0])), Some(pair[1]));
            assert_eq!(step_below(f32::from(pair[1])), Some(pair[0]));
        }
    }

    #[test]
    fn a_fixed_zoom_stays_in_its_range() {
        assert_eq!(Zoom::percent(10.), Zoom::Percent(ZOOM_MIN));
        assert_eq!(Zoom::percent(1000.), Zoom::Percent(ZOOM_MAX));
        assert_eq!(Zoom::percent(124.6), Zoom::Percent(125));
        assert_eq!(Zoom::percent(f32::NAN), Zoom::Percent(100));
        assert_eq!(clamped(Zoom::Percent(5)), Zoom::Percent(ZOOM_MIN));
        assert_eq!(clamped(Zoom::FitWidth), Zoom::FitWidth);
        assert_eq!(
            clamped(Zoom::FitWidthUpTo(9000)),
            Zoom::FitWidthUpTo(ZOOM_MAX)
        );
        assert!(Zoom::FitWidthUpTo(177).is_fit());
        assert!(Zoom::FitPage.is_fit() && !Zoom::default().is_fit());
    }
}
