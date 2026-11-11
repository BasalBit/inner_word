//! Block decorations: what a block kind paints besides its text.
//!
//! List markers (bullet, number, checkbox), the quote bar, the code block
//! panel, the divider rule and the page break label. Everything is added to
//! the frame plan as quads and single shaped lines; nothing here affects
//! layout (the gutters the markers sit in come from `layout::style`).
//!
//! Status: skeleton for polish. The shapes are plain and the metrics are
//! first guesses; nested bullet styles, the checkbox tick as a path, code
//! language labels and page view sheets are the implementer's.

use gpui::{
    Bounds, Font, Hsla, Pixels, Point, SharedString, TextRun, Window, fill, point, px, size,
};
use writer_core::{Block, BlockKind};
use writer_ui::theme;

use crate::{
    element::{FramePlan, PlannedMarker},
    layout::{
        BlockLayout, LayoutEnv,
        style::{CODE_PADDING, MARKER_GUTTER, QUOTE_GUTTER},
    },
};

/// Side of a to-do checkbox.
const CHECKBOX_SIZE: Pixels = px(14.);

/// Adds the decorations of one block to the plan. `origin` is the window
/// position of the block's layout box (x: the column's left edge).
pub(crate) fn plan_block(
    plan: &mut FramePlan,
    block: &Block,
    layout: &BlockLayout,
    origin: Point<Pixels>,
    ordinal: Option<usize>,
    env: &LayoutEnv,
    window: &Window,
) {
    let colors = env.style.colors;
    let first_row_y = origin.y + layout.rows[0].y;
    let last_row = &layout.rows[layout.rows.len() - 1];
    let text_bottom = origin.y + last_row.y + layout.line_height;
    let text_left = origin.x + layout.text_left;

    let shape = |text: SharedString, font: Font, font_size: Pixels, color: Hsla| {
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
    };

    match block.kind() {
        BlockKind::BulletItem => {
            let line = shape(
                "•".into(),
                env.style.body_font.clone(),
                layout.font_size,
                colors.muted,
            );
            plan.markers.push(PlannedMarker {
                origin: point(text_left - MARKER_GUTTER + px(10.), first_row_y),
                line_height: layout.line_height,
                line,
            });
        }
        BlockKind::NumberedItem => {
            let label = format!("{}.", ordinal.unwrap_or(1));
            let line = shape(
                label.into(),
                env.style.body_font.clone(),
                layout.font_size,
                colors.muted,
            );
            // Right-aligned in the gutter so the dots line up.
            plan.markers.push(PlannedMarker {
                origin: point(text_left - px(8.) - line.width, first_row_y),
                line_height: layout.line_height,
                line,
            });
        }
        BlockKind::TodoItem { checked } => {
            let top = first_row_y + ((layout.line_height - CHECKBOX_SIZE) / 2.).round();
            let bounds = Bounds::new(
                point(text_left - MARKER_GUTTER + px(6.), top),
                size(CHECKBOX_SIZE, CHECKBOX_SIZE),
            );
            let quad = if *checked {
                fill(bounds, colors.accent)
            } else {
                fill(bounds, colors.canvas)
                    .border_widths(px(1.5))
                    .border_color(colors.border_strong)
            };
            plan.back.push(quad.corner_radii(px(3.)));
            if *checked {
                let tick = shape(
                    "✓".into(),
                    env.style.ui_font.clone(),
                    theme::UI_TEXT_XS,
                    colors.on_accent,
                );
                plan.markers.push(PlannedMarker {
                    origin: point(
                        bounds.left() + ((CHECKBOX_SIZE - tick.width) / 2.).round(),
                        bounds.top(),
                    ),
                    line_height: CHECKBOX_SIZE,
                    line: tick,
                });
            }
        }
        BlockKind::Quote => {
            plan.back.push(
                fill(
                    Bounds::new(
                        point(text_left - QUOTE_GUTTER + px(4.), first_row_y),
                        size(px(3.), text_bottom - first_row_y),
                    ),
                    colors.border_strong,
                )
                .corner_radii(px(1.5)),
            );
        }
        BlockKind::CodeBlock { .. } => {
            plan.back.push(
                fill(
                    Bounds::new(
                        point(text_left - CODE_PADDING, first_row_y - CODE_PADDING),
                        size(
                            layout.text_width + CODE_PADDING * 2.,
                            text_bottom - first_row_y + CODE_PADDING * 2.,
                        ),
                    ),
                    colors.hover,
                )
                .corner_radii(theme::RADIUS_MD),
            );
        }
        BlockKind::Divider => {
            let y = first_row_y + (layout.line_height / 2.).round();
            plan.back.push(fill(
                Bounds::new(point(origin.x, y), size(env.column_width, px(1.))),
                colors.border_strong,
            ));
        }
        BlockKind::PageBreak => {
            let y = first_row_y + (layout.line_height / 2.).round();
            plan.back.push(fill(
                Bounds::new(point(origin.x, y), size(env.column_width, px(1.))),
                colors.hairline,
            ));
            let label = shape(
                "Page break".into(),
                env.style.ui_font.clone(),
                theme::UI_TEXT_XS,
                colors.faint,
            );
            let label_x = origin.x + ((env.column_width - label.width) / 2.).round();
            // The label interrupts the rule.
            plan.back.push(fill(
                Bounds::new(
                    point(label_x - px(8.), first_row_y),
                    size(label.width + px(16.), layout.line_height),
                ),
                colors.canvas,
            ));
            plan.markers.push(PlannedMarker {
                origin: point(label_x, first_row_y),
                line_height: layout.line_height,
                line: label,
            });
        }
        BlockKind::Paragraph | BlockKind::Heading { .. } => {}
    }
}
