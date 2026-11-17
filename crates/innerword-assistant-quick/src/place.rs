//! Where the popover goes.
//!
//! The popover hangs under the text it was opened on (the last row of the
//! selection, or the caret), its left edge on that row's left edge. Where
//! there is not enough room under the text and more above it, it stands
//! on top of the first row instead. It always stays inside the editor's
//! viewport, also while its text is scrolled out of sight.
//!
//! The side is decided from the room there is and from the height the
//! popover may reach, not from the height it has now: an answer that
//! streams in makes the popover grow, and it must not jump to the other
//! side of the text while it does. The room on the chosen side is also the
//! limit of its height, so it never grows over the text it was opened on.
//! Only where neither side has room for a usable popover (a selection that
//! fills the viewport) does it lie over the text, at the edge of the
//! viewport.

use gpui::{Bounds, Pixels, Point, Size, point, px};

/// The width of the popover where the editor is wide enough.
pub const WIDTH: Pixels = px(420.);
/// The height the popover grows to before its conversation scrolls.
pub const FULL_HEIGHT: Pixels = px(440.);
/// The least height worth having beside the text. With less room than
/// this on both sides the popover lies over the text.
pub const MIN_HEIGHT: Pixels = px(200.);
/// Distance between the text and the popover.
pub const GAP: Pixels = px(6.);
/// Distance kept from the edges of the viewport.
pub const MARGIN: Pixels = px(8.);

/// The side of its text the popover is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Under the last row.
    Below,
    /// Over the first row.
    Above,
}

/// What is known before the popover is laid out: its side and the size it
/// may take.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    /// The side of the text.
    pub side: Side,
    /// The width of the popover.
    pub width: Pixels,
    /// The height it may grow to.
    pub max_height: Pixels,
    /// Whether it stays clear of the text it was opened on. `false` only
    /// when neither side has [`MIN_HEIGHT`] of room.
    pub clear: bool,
}

/// The text the popover is attached to, in window coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    /// The first row of the selection (the caret, for a caret).
    pub first: Bounds<Pixels>,
    /// The last row of the selection (the caret, for a caret).
    pub last: Bounds<Pixels>,
}

impl Anchor {
    /// An anchor of one rectangle (a caret).
    pub fn at(bounds: Bounds<Pixels>) -> Anchor {
        Anchor {
            first: bounds,
            last: bounds,
        }
    }

    /// The anchor of the rows of a selection, in reading order.
    pub fn of_rows(rows: &[Bounds<Pixels>]) -> Option<Anchor> {
        let first = *rows.first()?;
        let last = *rows.last()?;
        // Rows of one block come in order; be safe about the rest.
        let top = rows.iter().fold(
            first,
            |top, row| {
                if row.top() < top.top() { *row } else { top }
            },
        );
        let bottom = rows.iter().fold(last, |bottom, row| {
            if row.bottom() > bottom.bottom() {
                *row
            } else {
                bottom
            }
        });
        Some(Anchor {
            first: top,
            last: bottom,
        })
    }
}

/// Decides the side and the size the popover may take.
pub fn plan(anchor: Anchor, viewport: Bounds<Pixels>) -> Plan {
    let most = (viewport.size.height - MARGIN * 2.).max(px(0.));
    let room_below = (viewport.bottom() - MARGIN - (anchor.last.bottom() + GAP)).min(most);
    let room_above = (anchor.first.top() - GAP - (viewport.top() + MARGIN)).min(most);
    let side = if room_below >= FULL_HEIGHT || room_below >= room_above {
        Side::Below
    } else {
        Side::Above
    };
    let room = match side {
        Side::Below => room_below,
        Side::Above => room_above,
    };
    let clear = room >= MIN_HEIGHT.min(most);
    let max_height = if clear { room } else { most }.min(FULL_HEIGHT);
    let width = WIDTH.min((viewport.size.width - MARGIN * 2.).max(px(0.)));
    Plan {
        side,
        width,
        max_height,
        clear,
    }
}

/// The top left corner of a popover of `size` on `side` of `anchor`,
/// inside `viewport`.
pub fn origin(
    anchor: Anchor,
    side: Side,
    size: Size<Pixels>,
    viewport: Bounds<Pixels>,
) -> Point<Pixels> {
    let (left, y) = match side {
        Side::Below => (anchor.last.left(), anchor.last.bottom() + GAP),
        Side::Above => (anchor.first.left(), anchor.first.top() - GAP - size.height),
    };
    // Inside the viewport; where the popover is larger than the viewport
    // its top left corner wins.
    let min_x = viewport.left() + MARGIN;
    let min_y = viewport.top() + MARGIN;
    let max_x = (viewport.right() - MARGIN - size.width).max(min_x);
    let max_y = (viewport.bottom() - MARGIN - size.height).max(min_y);
    point(left.clamp(min_x, max_x), y.clamp(min_y, max_y))
}
