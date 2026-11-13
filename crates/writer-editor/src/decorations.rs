//! Block decorations: what a block kind paints besides its text.
//!
//! List markers (bullets by indent level, numbers, checkboxes), the quote
//! bar, the code block panel, the divider rule and the page break marker.
//! Everything is added to the frame plan as quads, paths and single shaped
//! lines; nothing here affects layout (the gutters the markers sit in come
//! from `layout::style`). Sizes and positions are worked out in window
//! pixels, from the layout's points through the frame's `View`: a marker
//! is as large as the text it belongs to is drawn.
//!
//! A block that continues on the next sheet (page view) is decorated per
//! segment: the code panel and the quote bar are drawn once for each run of
//! rows on a sheet.

use gpui::{
    Bounds, Font, Hsla, PathBuilder, Pixels, SharedString, TextRun, Window, fill, point, px, size,
};
use writer_core::{Block, BlockKind};
use writer_ui::theme;

use crate::{
    element::{FramePlan, PlannedMarker},
    layout::{
        LayoutEnv, Placed, View,
        style::{CODE_PADDING, MARKER_GAP, MARKER_GUTTER, QUOTE_GUTTER},
    },
    paint::snap,
};

/// What `plan_block` needs to know about one block.
pub(crate) struct BlockContext<'a> {
    pub(crate) index: usize,
    pub(crate) block: &'a Block,
    pub(crate) placed: &'a Placed,
    /// From layout coordinates to the window.
    pub(crate) view: View,
    /// Number of a numbered list item.
    pub(crate) ordinal: Option<usize>,
    /// The next block is a quote too: the bar runs on to it.
    pub(crate) quote_continues: bool,
    pub(crate) env: &'a LayoutEnv,
}

/// A to-do checkbox on screen: clicking inside `bounds` toggles the block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CheckboxHit {
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) block: usize,
    pub(crate) checked: bool,
}

/// Shapes a single line of interface or marker text.
pub(crate) fn shape_label(
    window: &Window,
    text: SharedString,
    font: Font,
    font_size: Pixels,
    color: Hsla,
) -> gpui::ShapedLine {
    let run = TextRun {
        len: text.len(),
        font,
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text, font_size, &[run], None)
}

/// Adds the decorations of one block to the plan.
pub(crate) fn plan_block(
    plan: &mut FramePlan,
    checkboxes: &mut Vec<CheckboxHit>,
    cx: &BlockContext,
    window: &Window,
) {
    let BlockContext {
        block,
        placed,
        view,
        env,
        ..
    } = cx;
    let layout = &placed.layout;
    let style = &env.style;
    let colors = style.colors;
    // Points as they are drawn.
    let units = |points: f32| px(points * view.zoom);
    let font_size = view.len(layout.font_size);
    let baseline = view.len(layout.baseline);
    let x_height = view.len(layout.x_height);
    let line_height = view.len(layout.line_height);
    let column_left = view.x(px(0.));
    let column_width = view.len(env.column_width);
    let first_row_y = view.y(placed.row_top(0));
    let text_left = view.x(layout.text_left);
    // The optical middle of lowercase text on the first row.
    let middle_y = first_row_y + baseline - x_height * 0.55;

    match block.kind() {
        BlockKind::BulletItem => {
            let level = block.indent() % 3;
            let diameter = px((f32::from(font_size) * 0.3).round().max(4.));
            let side = if level == 2 {
                diameter - px(1.)
            } else {
                diameter
            };
            let bounds = Bounds::new(
                point(
                    snap(text_left - units(MARKER_GUTTER * 0.5) - side / 2.),
                    snap(middle_y - side / 2.),
                ),
                size(side, side),
            );
            let quad = match level {
                // Disc, ring, square: the three levels repeat.
                0 => fill(bounds, colors.muted).corner_radii(side / 2.),
                1 => fill(bounds, gpui::transparent_black())
                    .corner_radii(side / 2.)
                    .border_widths(px(1.))
                    .border_color(colors.muted),
                _ => fill(bounds, colors.muted),
            };
            plan.back.push(quad);
        }
        BlockKind::NumberedItem => {
            let label = format!("{}.", cx.ordinal.unwrap_or(1));
            let line = shape_label(
                window,
                label.into(),
                style.body_font.clone(),
                font_size,
                colors.muted,
            );
            // Right-aligned in the gutter so the dots line up. The gutter
            // is as wide as the widest number of the list needs.
            plan.markers.push(PlannedMarker {
                origin: point(text_left - units(MARKER_GAP) - line.width, first_row_y),
                line_height,
                line,
            });
        }
        BlockKind::TodoItem { checked } => {
            let side = px((f32::from(font_size) * 0.86)
                .round()
                .clamp(10. * view.zoom.min(1.), 28. * view.zoom.max(1.)));
            let bounds = Bounds::new(
                point(
                    snap(text_left - units(MARKER_GUTTER) + units(3.)),
                    snap(first_row_y + baseline - x_height * 0.72 - side / 2.),
                ),
                size(side, side),
            );
            let radius = px(f32::from(side) * 0.24);
            let quad = if *checked {
                fill(bounds, colors.accent).corner_radii(radius)
            } else {
                fill(bounds, style.surface())
                    .corner_radii(radius)
                    .border_widths(px(1.5))
                    .border_color(colors.border_strong)
            };
            plan.back.push(quad);
            if *checked {
                let at = |x: f32, y: f32| {
                    point(
                        bounds.left() + px(f32::from(side) * x),
                        bounds.top() + px(f32::from(side) * y),
                    )
                };
                let mut tick = PathBuilder::stroke(px((f32::from(side) * 0.12).max(1.4)));
                tick.move_to(at(0.25, 0.53));
                tick.line_to(at(0.43, 0.70));
                tick.line_to(at(0.76, 0.31));
                if let Ok(path) = tick.build() {
                    plan.paths.push((path, colors.on_accent));
                }
            }
            checkboxes.push(CheckboxHit {
                bounds: bounds.dilate(px(3.)),
                block: cx.index,
                checked: *checked,
            });
        }
        BlockKind::Quote => {
            let segments = placed.segments();
            let last = segments.len() - 1;
            for (ix, segment) in segments.iter().enumerate() {
                let mut bottom = segment.bottom;
                if ix == last && cx.quote_continues {
                    bottom += px(layout.measure.group_gap);
                }
                let (top, bottom) = (view.y(segment.top), view.y(bottom));
                plan.back.push(
                    fill(
                        Bounds::new(
                            point(text_left - units(QUOTE_GUTTER) + units(1.), top),
                            size(units(3.).max(px(2.)), bottom - top),
                        ),
                        colors.border_strong,
                    )
                    .corner_radii(px(1.5)),
                );
            }
        }
        BlockKind::CodeBlock { language } => {
            let padding = units(CODE_PADDING);
            let left = text_left - padding;
            let width = view.len(layout.text_width) + padding * 2.;
            for segment in placed.segments() {
                let (top, bottom) = (view.y(segment.top), view.y(segment.bottom));
                plan.back.push(
                    fill(
                        Bounds::new(point(left, top), size(width, bottom - top)),
                        colors.hover,
                    )
                    .corner_radii(theme::RADIUS_MD),
                );
            }
            if let Some(language) = language.as_ref().filter(|language| !language.is_empty()) {
                let label = shape_label(
                    window,
                    language.clone().into(),
                    style.ui_font.clone(),
                    units(f32::from(theme::UI_TEXT_XS)),
                    colors.faint,
                );
                let label_height = units(16.);
                plan.markers.push(PlannedMarker {
                    origin: point(
                        left + width - label.width - units(8.),
                        view.y(placed.top) + units(4.),
                    ),
                    line_height: label_height,
                    line: label,
                });
            }
        }
        BlockKind::Divider => {
            let y = snap(first_row_y + line_height / 2.);
            plan.back.push(fill(
                Bounds::new(point(column_left, y), size(column_width, px(1.))),
                colors.border_strong,
            ));
        }
        BlockKind::PageBreak => {
            // A quiet marker: a hairline interrupted by a label. In page
            // view the rest of the sheet below it stays empty.
            let y = snap(first_row_y + line_height / 2.);
            let label = shape_label(
                window,
                "Page break".into(),
                style.ui_font.clone(),
                units(f32::from(theme::UI_TEXT_XS)),
                colors.faint,
            );
            let gap = units(10.);
            let label_x = (column_left + (column_width - label.width) / 2.).round();
            let left_width = label_x - gap - column_left;
            if left_width > px(0.) {
                plan.back.push(fill(
                    Bounds::new(point(column_left, y), size(left_width, px(1.))),
                    colors.hairline,
                ));
                let right = label_x + label.width + gap;
                plan.back.push(fill(
                    Bounds::new(
                        point(right, y),
                        size(column_left + column_width - right, px(1.)),
                    ),
                    colors.hairline,
                ));
            }
            plan.markers.push(PlannedMarker {
                origin: point(label_x, first_row_y),
                line_height,
                line: label,
            });
        }
        BlockKind::Paragraph | BlockKind::Heading { .. } => {}
        // An image has no decoration: the picture itself is painted elsewhere.
        BlockKind::Image(_) => {}
    }
}
