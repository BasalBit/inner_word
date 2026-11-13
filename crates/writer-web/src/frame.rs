//! Rectangles between GPUI's way of counting (points from the top left) and AppKit's
//! (from the bottom left, unless a view is flipped).

use crate::{Rect, ns::NSRect};

/// `rect` (from the top left of a view `height` points high) in that view's own
/// coordinates.
pub(crate) fn to_appkit(rect: Rect, height: f64, flipped: bool) -> NSRect {
    let y = if flipped {
        rect.y
    } else {
        height - rect.y - rect.height
    };
    NSRect::new(rect.x, y, rect.width, rect.height)
}

/// A point of a view `height` points high, in its own coordinates, from its top left.
pub(crate) fn to_top_left(x: f64, y: f64, height: f64, flipped: bool) -> (f64, f64) {
    if flipped { (x, y) } else { (x, height - y) }
}

/// Whether a point (from the top left) lies inside one of the holes.
pub(crate) fn in_a_hole(holes: &[Rect], x: f64, y: f64) -> bool {
    holes.iter().any(|hole| {
        hole.width > 0.
            && hole.height > 0.
            && x >= hole.x
            && x < hole.x + hole.width
            && y >= hole.y
            && y < hole.y + hole.height
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn a_frame_from_the_top_left_is_counted_from_the_bottom_in_appkit() {
        // The page area of a window 800 high: 40 below the top, 700 high.
        let frame = to_appkit(rect(250., 40., 600., 700.), 800., false);
        assert_eq!(frame, NSRect::new(250., 60., 600., 700.));
        // A flipped view counts from the top already.
        let frame = to_appkit(rect(250., 40., 600., 700.), 800., true);
        assert_eq!(frame, NSRect::new(250., 40., 600., 700.));
        // Back again.
        assert_eq!(to_top_left(10., 60., 800., false), (10., 740.));
        assert_eq!(to_top_left(10., 60., 800., true), (10., 60.));
    }

    #[test]
    fn a_point_is_in_a_hole_up_to_its_edge() {
        let holes = [rect(100., 500., 300., 60.), rect(0., 0., 0., 0.)];
        assert!(in_a_hole(&holes, 100., 500.));
        assert!(in_a_hole(&holes, 399.9, 559.9));
        assert!(!in_a_hole(&holes, 400., 530.));
        assert!(!in_a_hole(&holes, 250., 499.));
        // An empty rectangle holds nothing.
        assert!(!in_a_hole(&holes, 0., 0.));
        assert!(!in_a_hole(&[], 1., 1.));
    }
}
