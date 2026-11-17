//! Placement: beside the text, inside the viewport.

use gpui::{Bounds, Pixels, point, px, size};
use innerword_assistant_quick::place::{
    Anchor, FULL_HEIGHT, GAP, MARGIN, MIN_HEIGHT, Side, WIDTH, origin, plan,
};

fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
}

/// A viewport like the editor's in a window of 1200 by 900.
fn viewport() -> Bounds<Pixels> {
    rect(200., 40., 800., 800.)
}

#[test]
fn under_the_text_where_there_is_room() {
    let anchor = Anchor::at(rect(300., 100., 2., 27.));
    let plan = plan(anchor, viewport());
    assert_eq!(plan.side, Side::Below);
    assert_eq!(plan.width, WIDTH);
    assert_eq!(plan.max_height, FULL_HEIGHT);
    assert!(plan.clear);
    let at = origin(anchor, plan.side, size(plan.width, px(120.)), viewport());
    assert_eq!(at, point(px(300.), px(127.) + GAP));
}

#[test]
fn over_the_text_at_the_bottom_edge() {
    let anchor = Anchor::at(rect(300., 780., 2., 27.));
    let plan = plan(anchor, viewport());
    assert_eq!(plan.side, Side::Above);
    assert_eq!(plan.max_height, FULL_HEIGHT);
    assert!(plan.clear);
    let popover = size(plan.width, px(120.));
    let at = origin(anchor, plan.side, popover, viewport());
    assert_eq!(at.y + popover.height + GAP, px(780.));
}

#[test]
fn the_side_does_not_depend_on_the_height_it_has_now() {
    // 300 of room below, 400 above: a small popover would fit below, but
    // it is going to grow, and it must not change sides when it does.
    let anchor = Anchor::at(rect(300., 500., 2., 27.));
    let plan = plan(anchor, viewport());
    assert_eq!(plan.side, Side::Above);
    for height in [60., 200., 440.] {
        let popover = size(plan.width, px(height).min(plan.max_height));
        let at = origin(anchor, plan.side, popover, viewport());
        assert!(
            at.y + popover.height <= px(500.),
            "over the text at {height}"
        );
        assert!(at.y >= viewport().top() + MARGIN);
    }
}

#[test]
fn the_room_on_its_side_is_the_limit_of_its_height() {
    // A lower viewport: 259 of room below and 246 above. It stays below,
    // and no taller than the room there.
    let low = rect(200., 40., 800., 560.);
    let anchor = Anchor::at(rect(300., 300., 2., 27.));
    let plan = plan(anchor, low);
    assert_eq!(plan.side, Side::Below);
    let room = low.bottom() - MARGIN - (px(327.) + GAP);
    assert_eq!(plan.max_height, room);
    assert!(plan.max_height >= MIN_HEIGHT && plan.max_height < FULL_HEIGHT);
    assert!(plan.clear);
}

#[test]
fn a_selection_of_several_rows_is_not_covered() {
    let rows = [
        rect(420., 300., 280., 27.),
        rect(300., 327., 400., 27.),
        rect(300., 354., 90., 27.),
    ];
    let anchor = Anchor::of_rows(&rows).unwrap();
    let plan = plan(anchor, viewport());
    assert_eq!(plan.side, Side::Below);
    let popover = size(plan.width, plan.max_height);
    let at = origin(anchor, plan.side, popover, viewport());
    // Under the last row, on its left edge.
    assert_eq!(at, point(px(300.), px(381.) + GAP));
    let bounds = Bounds::new(at, popover);
    for row in rows {
        assert!(!bounds.intersects(&row));
    }
}

#[test]
fn over_a_selection_it_stands_on_the_first_row() {
    let rows = [rect(420., 600., 280., 27.), rect(300., 627., 400., 27.)];
    let anchor = Anchor::of_rows(&rows).unwrap();
    let plan = plan(anchor, viewport());
    assert_eq!(plan.side, Side::Above);
    let popover = size(plan.width, px(300.));
    let at = origin(anchor, plan.side, popover, viewport());
    assert_eq!(at.x, px(420.));
    assert_eq!(at.y + popover.height + GAP, px(600.));
}

#[test]
fn inside_the_viewport_at_the_right_edge() {
    let anchor = Anchor::at(rect(950., 100., 2., 27.));
    let plan = plan(anchor, viewport());
    let popover = size(plan.width, px(120.));
    let at = origin(anchor, plan.side, popover, viewport());
    assert_eq!(at.x + popover.width, viewport().right() - MARGIN);
}

#[test]
fn a_narrow_editor_gets_a_narrow_popover() {
    let narrow = rect(200., 40., 300., 800.);
    let anchor = Anchor::at(rect(260., 100., 2., 27.));
    let plan = plan(anchor, narrow);
    assert_eq!(plan.width, px(300.) - MARGIN * 2.);
    let at = origin(anchor, plan.side, size(plan.width, px(120.)), narrow);
    assert_eq!(at.x, narrow.left() + MARGIN);
}

#[test]
fn a_selection_that_fills_the_viewport_is_covered_at_an_edge() {
    let rows: Vec<_> = (0..40)
        .map(|row| rect(300., 20. + 27. * row as f32, 400., 27.))
        .collect();
    let anchor = Anchor::of_rows(&rows).unwrap();
    let plan = plan(anchor, viewport());
    assert!(!plan.clear);
    assert_eq!(plan.max_height, FULL_HEIGHT);
    let popover = size(plan.width, px(300.));
    let at = origin(anchor, plan.side, popover, viewport());
    let bounds = Bounds::new(at, popover);
    assert!(bounds.top() >= viewport().top() + MARGIN);
    assert!(bounds.bottom() <= viewport().bottom() - MARGIN);
}

#[test]
fn text_scrolled_out_of_sight_keeps_the_popover_inside() {
    // Above the viewport: at the top edge.
    let above = Anchor::at(rect(300., -400., 2., 27.));
    let plan_above = plan(above, viewport());
    assert_eq!(plan_above.side, Side::Below);
    assert_eq!(plan_above.max_height, FULL_HEIGHT);
    let popover = size(plan_above.width, px(200.));
    let at = origin(above, plan_above.side, popover, viewport());
    assert_eq!(at.y, viewport().top() + MARGIN);

    // Below the viewport: at the bottom edge.
    let below = Anchor::at(rect(300., 2000., 2., 27.));
    let plan_below = plan(below, viewport());
    assert_eq!(plan_below.side, Side::Above);
    let at = origin(below, plan_below.side, popover, viewport());
    assert_eq!(at.y + popover.height, viewport().bottom() - MARGIN);
}

#[test]
fn a_small_viewport_never_gets_a_popover_larger_than_itself() {
    let small = rect(0., 0., 300., 150.);
    let anchor = Anchor::at(rect(20., 60., 2., 27.));
    let plan = plan(anchor, small);
    assert!(plan.max_height <= px(150.) - MARGIN * 2.);
    assert!(plan.width <= px(300.) - MARGIN * 2.);
    let at = origin(anchor, plan.side, size(plan.width, plan.max_height), small);
    assert!(at.x >= small.left() + MARGIN);
    assert!(at.y >= small.top() + MARGIN);
}
