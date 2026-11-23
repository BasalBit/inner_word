//! Geometry of elements: bounds, the outlines of shapes, where a line meets them, and
//! Excalidraw's focus of a binding (`determineFocusDistance`, ported).

use crate::{Element, ElementType};

pub(crate) type Point = (f64, f64);

pub(crate) fn rotate(p: Point, center: Point, angle: f64) -> Point {
    if angle == 0.0 {
        return p;
    }
    let (s, c) = angle.sin_cos();
    let (dx, dy) = (p.0 - center.0, p.1 - center.1);
    (center.0 + dx * c - dy * s, center.1 + dx * s + dy * c)
}

fn distance(a: Point, b: Point) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// The box of an element that is not linear: its position and size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shape {
    pub kind: ShapeKind,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub angle: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShapeKind {
    Rectangle,
    Ellipse,
    Diamond,
}

impl Shape {
    pub(crate) fn of(element: &Element) -> Shape {
        let kind = match element.kind() {
            ElementType::Ellipse => ShapeKind::Ellipse,
            ElementType::Diamond => ShapeKind::Diamond,
            _ => ShapeKind::Rectangle,
        };
        Shape {
            kind,
            x: element.x(),
            y: element.y(),
            width: element.width(),
            height: element.height(),
            angle: element.angle(),
        }
    }

    pub(crate) fn center(&self) -> Point {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// Where the infinite line through `a` and `b` crosses the outline, as parameters `t`
    /// of `a + t (b - a)`, sorted.
    pub(crate) fn crossings(&self, a: Point, b: Point) -> Vec<f64> {
        let center = self.center();
        // In the shape's own frame: unrotated, centred on the origin.
        let la = rotate(a, center, -self.angle);
        let lb = rotate(b, center, -self.angle);
        let (ax, ay) = (la.0 - center.0, la.1 - center.1);
        let (dx, dy) = (lb.0 - la.0, lb.1 - la.1);
        let (hw, hh) = (self.width / 2.0, self.height / 2.0);
        let mut ts = Vec::new();
        match self.kind {
            ShapeKind::Ellipse => {
                if hw <= 0.0 || hh <= 0.0 {
                    return ts;
                }
                let qa = (dx / hw).powi(2) + (dy / hh).powi(2);
                let qb = 2.0 * (ax * dx / (hw * hw) + ay * dy / (hh * hh));
                let qc = (ax / hw).powi(2) + (ay / hh).powi(2) - 1.0;
                if qa == 0.0 {
                    return ts;
                }
                let disc = qb * qb - 4.0 * qa * qc;
                if disc < 0.0 {
                    return ts;
                }
                let root = disc.sqrt();
                ts.push((-qb - root) / (2.0 * qa));
                ts.push((-qb + root) / (2.0 * qa));
            }
            ShapeKind::Rectangle | ShapeKind::Diamond => {
                let corners: Vec<Point> = if self.kind == ShapeKind::Rectangle {
                    vec![(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)]
                } else {
                    vec![(0.0, -hh), (hw, 0.0), (0.0, hh), (-hw, 0.0)]
                };
                for n in 0..4 {
                    let p = corners[n];
                    let q = corners[(n + 1) % 4];
                    if let Some((t, u)) = line_segment(ax, ay, dx, dy, p, q)
                        && (-1e-9..=1.0 + 1e-9).contains(&u)
                    {
                        ts.push(t);
                    }
                }
            }
        }
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
        ts
    }

    /// The four (or, for an ellipse, the extreme) points of the outline, in scene
    /// coordinates: what the shape's bounds are made of.
    pub(crate) fn extremes(&self) -> Vec<Point> {
        let center = self.center();
        let (hw, hh) = (self.width / 2.0, self.height / 2.0);
        match self.kind {
            ShapeKind::Ellipse => {
                // The bounds of a rotated ellipse.
                let (s, c) = self.angle.sin_cos();
                let ex = ((hw * c).powi(2) + (hh * s).powi(2)).sqrt();
                let ey = ((hw * s).powi(2) + (hh * c).powi(2)).sqrt();
                vec![
                    (center.0 - ex, center.1 - ey),
                    (center.0 + ex, center.1 + ey),
                ]
            }
            ShapeKind::Rectangle => [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)]
                .iter()
                .map(|(dx, dy)| rotate((center.0 + dx, center.1 + dy), center, self.angle))
                .collect(),
            ShapeKind::Diamond => [(0.0, -hh), (hw, 0.0), (0.0, hh), (-hw, 0.0)]
                .iter()
                .map(|(dx, dy)| rotate((center.0 + dx, center.1 + dy), center, self.angle))
                .collect(),
        }
    }
}

/// Where the line `(ax, ay) + t (dx, dy)` meets the segment `p`–`q`: `(t, u)` with `u` the
/// parameter along the segment; `None` when they are parallel.
fn line_segment(ax: f64, ay: f64, dx: f64, dy: f64, p: Point, q: Point) -> Option<(f64, f64)> {
    let (ex, ey) = (q.0 - p.0, q.1 - p.1);
    let denominator = dx * ey - dy * ex;
    if denominator.abs() < 1e-12 {
        return None;
    }
    let (fx, fy) = (p.0 - ax, p.1 - ay);
    let t = (fx * ey - fy * ex) / denominator;
    let u = (fx * dy - fy * dx) / denominator;
    Some((t, u))
}

/// The bounds of an element in scene coordinates, `[x1, y1, x2, y2]`, rotation included.
pub(crate) fn element_bounds(element: &Element) -> Option<[f64; 4]> {
    let kind = element.kind();
    let points: Vec<Point> = match kind {
        ElementType::Arrow | ElementType::Line | ElementType::Freedraw => {
            let pts = element.points();
            if pts.is_empty() {
                return None;
            }
            let (x, y) = (element.x(), element.y());
            let min_x = pts.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
            let max_x = pts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
            let min_y = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
            let max_y = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
            let center = (x + (min_x + max_x) / 2.0, y + (min_y + max_y) / 2.0);
            pts.iter()
                .map(|p| rotate((x + p.0, y + p.1), center, element.angle()))
                .collect()
        }
        _ => Shape::of(element).extremes(),
    };
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for (px, py) in points {
        if !px.is_finite() || !py.is_finite() {
            continue;
        }
        b[0] = b[0].min(px);
        b[1] = b[1].min(py);
        b[2] = b[2].max(px);
        b[3] = b[3].max(py);
    }
    if b[0].is_finite() { Some(b) } else { None }
}

/// Excalidraw 0.18.1's `determineFocusDistance(element, a, b)`: where the line from `a`
/// (the point next to the end) through `b` (the end) passes the shape's centre, as a
/// signed share of its half diagonal (0: through the centre).
pub(crate) fn focus(shape: &Shape, a: Point, b: Point) -> f64 {
    let center = shape.center();
    if a == b {
        return 0.0;
    }
    let ra = rotate(a, center, -shape.angle);
    let rb = rotate(b, center, -shape.angle);
    // sign = Math.sign(cross(b→a, b→center)) * -1, with the unrotated a, as Excalidraw has it.
    let v1 = (a.0 - rb.0, a.1 - rb.1);
    let v2 = (center.0 - rb.0, center.1 - rb.1);
    let cross = v1.0 * v2.1 - v1.1 * v2.0;
    let sign = -(if cross > 0.0 {
        1.0
    } else if cross < 0.0 {
        -1.0
    } else {
        0.0
    });
    let (x, y, w, h) = (shape.x, shape.y, shape.width, shape.height);
    // From b on, away from a: into the shape.
    let direction = (rb.0 - ra.0, rb.1 - ra.1);
    let length = (direction.0.powi(2) + direction.1.powi(2)).sqrt();
    if length == 0.0 {
        return 0.0;
    }
    let reach = (w * 2.0).max(h * 2.0);
    let end = (
        rb.0 + direction.0 / length * reach,
        rb.1 + direction.1 / length * reach,
    );
    let diamond = shape.kind == ShapeKind::Diamond;
    let axes: [(Point, Point); 2] = if diamond {
        [
            ((x + w / 2.0, y), (x + w / 2.0, y + h)),
            ((x, y + h / 2.0), (x + w, y + h / 2.0)),
        ]
    } else {
        [((x, y), (x + w, y + h)), ((x + w, y), (x, y + h))]
    };
    let interceptees: [(Point, Point); 2] = if diamond {
        [
            ((x + w / 2.0, y - h), (x + w / 2.0, y + h * 2.0)),
            ((x - w, y + h / 2.0), (x + w * 2.0, y + h / 2.0)),
        ]
    } else {
        [
            ((x - w, y - h), (x + w * 2.0, y + h * 2.0)),
            ((x + w * 2.0, y - h), (x - w, y + h * 2.0)),
        ]
    };
    let mut hits: Vec<(f64, usize, Point)> = Vec::new();
    for (n, (p, q)) in interceptees.iter().enumerate() {
        if let Some(point) = segment_intersection((rb, end), (*p, *q)) {
            let d2 = (point.0 - b.0).powi(2) + (point.1 - b.1).powi(2);
            hits.push((d2, n, point));
        }
    }
    // Sorted by the distance to b; each mapped with the axis of its place in that order,
    // as Excalidraw maps them (`axes[idx]`).
    hits.sort_by(|g, h| g.0.total_cmp(&h.0));
    let mut ratios: Vec<f64> = hits
        .iter()
        .enumerate()
        .map(|(idx, (_, _, p))| {
            let half = if diamond {
                distance(axes[idx].0, axes[idx].1) / 2.0
            } else {
                (w * w + h * h).sqrt() / 2.0
            };
            if half == 0.0 {
                0.0
            } else {
                sign * distance(center, *p) / half
            }
        })
        .collect();
    ratios.sort_by(|g, h| g.abs().total_cmp(&h.abs()));
    ratios.first().copied().unwrap_or(0.0)
}

/// Excalidraw's `lineSegmentIntersectionPoints`: the point where two segments cross.
fn segment_intersection(s: (Point, Point), t: (Point, Point)) -> Option<Point> {
    let (dx, dy) = (s.1.0 - s.0.0, s.1.1 - s.0.1);
    let (t_param, u) = line_segment(s.0.0, s.0.1, dx, dy, t.0, t.1)?;
    if (0.0..=1.0).contains(&t_param) && (0.0..=1.0).contains(&u) {
        Some((s.0.0 + t_param * dx, s.0.1 + t_param * dy))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(kind: ShapeKind) -> Shape {
        Shape {
            kind,
            x: 100.0,
            y: 100.0,
            width: 200.0,
            height: 100.0,
            angle: 0.0,
        }
    }

    #[test]
    fn a_line_meets_each_outline() {
        let r = shape(ShapeKind::Rectangle);
        assert_eq!(r.crossings((0.0, 150.0), (400.0, 150.0)), vec![0.25, 0.75]);
        let e = shape(ShapeKind::Ellipse);
        assert_eq!(e.crossings((0.0, 150.0), (400.0, 150.0)), vec![0.25, 0.75]);
        let d = shape(ShapeKind::Diamond);
        let ts = d.crossings((200.0, 0.0), (200.0, 400.0));
        assert_eq!(ts, vec![0.25, 0.5]);
        assert!(r.crossings((0.0, 0.0), (400.0, 0.0)).is_empty());
    }

    #[test]
    fn focus_through_the_centre_is_zero() {
        let e = shape(ShapeKind::Ellipse);
        assert_eq!(focus(&e, (0.0, 150.0), (100.0, 150.0)), 0.0);
        let off = focus(&e, (0.0, 140.0), (100.0, 140.0));
        assert!((off.abs() - 0.2).abs() < 1e-9, "{off}");
    }

    #[test]
    fn focus_as_excalidraw_computed_it_for_the_scout() {
        // An arrow at y 155 ending on an ellipse centred at (590, 160): Excalidraw wrote 0.1.
        let e = Shape {
            kind: ShapeKind::Ellipse,
            x: 500.0,
            y: 110.0,
            width: 180.0,
            height: 100.0,
            angle: 0.0,
        };
        let f = focus(&e, (320.0, 155.0), (500.0, 155.0));
        assert!((f - 0.1).abs() < 1e-9, "{f}");
        let r = Shape {
            kind: ShapeKind::Rectangle,
            x: 100.0,
            y: 100.0,
            width: 220.0,
            height: 110.0,
            angle: 0.0,
        };
        assert_eq!(focus(&r, (500.0, 155.0), (320.0, 155.0)), 0.0);
    }
}
