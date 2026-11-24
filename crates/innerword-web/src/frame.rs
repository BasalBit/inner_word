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

/// The area the holes cover together, as rectangles that do not overlap: the mask is
/// filled with the even-odd rule, under which a point two holes share is not a hole (a
/// toast over a modal showed the covered view in a band where the two met). Bands from
/// one edge of a rectangle to the next, each cut where the holes in it begin and end;
/// empty rectangles are left out.
pub(crate) fn disjoint(holes: &[Rect]) -> Vec<Rect> {
    let holes: Vec<&Rect> = holes
        .iter()
        .filter(|hole| hole.width > 0. && hole.height > 0.)
        .collect();
    if holes.len() < 2 {
        return holes.into_iter().copied().collect();
    }
    let mut edges: Vec<f64> = holes
        .iter()
        .flat_map(|hole| [hole.y, hole.y + hole.height])
        .collect();
    edges.sort_by(f64::total_cmp);
    edges.dedup();
    let mut out: Vec<Rect> = Vec::new();
    for band in edges.windows(2) {
        let (top, bottom) = (band[0], band[1]);
        let mut spans: Vec<(f64, f64)> = holes
            .iter()
            .filter(|hole| hole.y <= top && hole.y + hole.height >= bottom)
            .map(|hole| (hole.x, hole.x + hole.width))
            .collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for (start, end) in spans {
            match merged.last_mut() {
                Some(last) if start <= last.1 => last.1 = last.1.max(end),
                _ => merged.push((start, end)),
            }
        }
        for (start, end) in merged {
            // The same span as the band above: one rectangle grows down.
            if let Some(above) = out
                .iter_mut()
                .find(|r| r.x == start && r.x + r.width == end && r.y + r.height == top)
            {
                above.height = bottom - above.y;
            } else {
                out.push(Rect {
                    x: start,
                    y: top,
                    width: end - start,
                    height: bottom - top,
                });
            }
        }
    }
    out
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

    /// How many of `rects` hold the point: the even-odd mask cuts where it is odd.
    fn count(rects: &[Rect], x: f64, y: f64) -> usize {
        rects
            .iter()
            .filter(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
            .count()
    }

    #[test]
    fn holes_that_overlap_are_cut_once_where_they_meet() {
        // A modal and a toast over its footer (the black band of 0.11.0's cover).
        let modal = rect(160., 90., 820., 560.);
        let toast = rect(270., 620., 500., 60.);
        let second = rect(270., 690., 500., 40.);
        let holes = [modal, toast, second, rect(5., 5., 0., 10.)];
        let cut = disjoint(&holes);
        for (x, y) in [
            (300., 630.), // the toast over the modal
            (300., 670.), // the toast below the modal
            (200., 100.), // the modal alone
            (300., 700.), // the second toast
        ] {
            assert_eq!(count(&cut, x, y), 1, "({x}, {y}) in {cut:?}");
            assert!(in_a_hole(&holes, x, y));
        }
        for (x, y) in [(100., 630.), (300., 685.), (5., 6.), (990., 300.)] {
            assert_eq!(count(&cut, x, y), 0, "({x}, {y}) in {cut:?}");
        }
        // No two rectangles share a point, and none is empty.
        for (i, a) in cut.iter().enumerate() {
            assert!(a.width > 0. && a.height > 0.);
            for b in &cut[i + 1..] {
                let apart = a.x + a.width <= b.x
                    || b.x + b.width <= a.x
                    || a.y + a.height <= b.y
                    || b.y + b.height <= a.y;
                assert!(apart, "{a:?} and {b:?}");
            }
        }
        // One hole, or holes apart, are as they came.
        assert_eq!(disjoint(&[toast]), vec![toast]);
        assert_eq!(disjoint(&[toast, second]).len(), 2);
    }
}
