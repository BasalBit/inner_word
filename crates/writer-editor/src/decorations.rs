//! Block decorations: what a block kind paints besides its text.
//!
//! List markers (bullets by indent level, numbers, checkboxes), the quote
//! bar, the code block panel, the divider rule and the page break marker.
//! Nothing here affects layout (the gutters the markers sit in come from
//! `layout::style`). Sizes and positions are worked out in the pixels of
//! the view a block is planned for, from the layout's points through that
//! view's [`View`]: a marker is as large as the text it belongs to is
//! drawn.
//!
//! A block that continues on the next sheet (page view) is decorated per
//! segment: the code panel and the quote bar are drawn once for each run of
//! rows on a sheet.
//!
//! # One geometry for the screen and for paper
//!
//! [`plan_block`] does not paint and does not know what it plans for. It
//! works out neutral shapes (a rectangle with a fill, a radius and a
//! border; a stroked line; a single shaped line of text with the point it
//! is placed by) and hands them to a [`DecorationSink`]. The screen's sink
//! ([`ScreenSink`]) turns them into what a frame paints: quads, paths and
//! shaped lines of the frame plan, and the places a click toggles a to-do.
//! The print pass has a sink of its own (`crate::print`) that turns the
//! same shapes into the items of a printed page. So a bullet, a number or
//! a code panel is in one place on screen and on paper because one piece
//! of code put it there.
//!
//! What is on screen and never on paper (the page break marker) is marked
//! [`screen only`](RectShape::screen_only); a fill that only repeats the
//! surface the block lies on (the inside of an open to-do box) is a
//! [`ShapeFill::Surface`], which paper does not need.

use gpui::{
    Bounds, Font, Hsla, PathBuilder, Pixels, Point, ShapedLine, SharedString, TextRun,
    WindowTextSystem, fill, point, px, size,
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

/// What a rectangle is filled with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ShapeFill {
    /// Nothing: only its border is drawn.
    None,
    /// A colour of its own.
    Color(Hsla),
    /// The colour of the surface it lies on, so that nothing shows
    /// through it on screen. Paper is that surface already.
    Surface(Hsla),
}

/// A rectangle of a decoration, in the pixels of the view it was planned
/// for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RectShape {
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) fill: ShapeFill,
    /// The radius of its corners.
    pub(crate) radius: Pixels,
    /// A border inside its edge: its width and its colour.
    pub(crate) border: Option<(Pixels, Hsla)>,
    /// Part of something that is shown on screen and not printed.
    pub(crate) screen_only: bool,
}

impl RectShape {
    /// A filled rectangle with square corners and no border.
    fn filled(bounds: Bounds<Pixels>, color: Hsla) -> Self {
        Self {
            bounds,
            fill: ShapeFill::Color(color),
            radius: px(0.),
            border: None,
            screen_only: false,
        }
    }

    fn radius(mut self, radius: Pixels) -> Self {
        self.radius = radius;
        self
    }
}

/// A stroked open line through several points, with round joins and caps
/// (the tick of a to-do box).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StrokeShape {
    pub(crate) points: Vec<Point<Pixels>>,
    pub(crate) width: Pixels,
    pub(crate) color: Hsla,
}

/// Which point of a label's line is the one its place was worked out
/// from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LabelAnchor {
    /// The line is centred on it.
    Middle,
    /// The line ends at it.
    End,
}

/// A single line of text beside or over a block: a list number, the
/// language of a code block, the words of the page break marker.
///
/// It comes shaped, at the place it is painted at on screen (`origin`,
/// `line_height`). A sink that does not paint GPUI's glyphs has what the
/// line says and what it is set in, and the point it is placed by, so that
/// it can set the line again with its own measure.
pub(crate) struct LabelShape {
    pub(crate) line: ShapedLine,
    /// The top left corner of the line's box.
    pub(crate) origin: Point<Pixels>,
    /// The height of the box the line is centred in.
    pub(crate) line_height: Pixels,
    pub(crate) text: SharedString,
    pub(crate) font: Font,
    pub(crate) font_size: Pixels,
    pub(crate) color: Hsla,
    pub(crate) anchor: LabelAnchor,
    /// The x of the anchor.
    pub(crate) anchor_x: Pixels,
    /// Part of something that is shown on screen and not printed.
    pub(crate) screen_only: bool,
}

impl LabelShape {
    /// The y of the line's baseline: where GPUI's line painter puts it
    /// inside the box.
    pub(crate) fn baseline(&self) -> Pixels {
        self.origin.y
            + (self.line_height - self.line.ascent - self.line.descent) / 2.
            + self.line.ascent
    }
}

/// Whoever takes the shapes [`plan_block`] works out.
pub(crate) trait DecorationSink {
    fn rect(&mut self, rect: RectShape);
    fn stroke(&mut self, stroke: StrokeShape);
    fn label(&mut self, label: LabelShape);
    /// A to-do checkbox was planned: a click inside toggles the block.
    fn checkbox(&mut self, hit: CheckboxHit);
}

/// The sink of a frame: shapes become what the frame plan paints.
pub(crate) struct ScreenSink<'a> {
    pub(crate) plan: &'a mut FramePlan,
    pub(crate) checkboxes: &'a mut Vec<CheckboxHit>,
}

impl DecorationSink for ScreenSink<'_> {
    fn rect(&mut self, rect: RectShape) {
        let background = match rect.fill {
            ShapeFill::None => gpui::transparent_black(),
            ShapeFill::Color(color) | ShapeFill::Surface(color) => color,
        };
        let mut quad = fill(rect.bounds, background).corner_radii(rect.radius);
        if let Some((width, color)) = rect.border {
            quad = quad.border_widths(width).border_color(color);
        }
        self.plan.back.push(quad);
    }

    fn stroke(&mut self, stroke: StrokeShape) {
        let mut points = stroke.points.into_iter();
        let Some(first) = points.next() else {
            return;
        };
        let mut path = PathBuilder::stroke(stroke.width);
        path.move_to(first);
        for point in points {
            path.line_to(point);
        }
        if let Ok(path) = path.build() {
            self.plan.paths.push((path, stroke.color));
        }
    }

    fn label(&mut self, label: LabelShape) {
        self.plan.markers.push(PlannedMarker {
            origin: label.origin,
            line_height: label.line_height,
            line: label.line,
        });
    }

    fn checkbox(&mut self, hit: CheckboxHit) {
        self.checkboxes.push(hit);
    }
}

/// The number of a sheet in page view: how large it is set and where it
/// stands, on screen and on paper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PageNumber {
    /// The text size.
    pub(crate) font_size: Pixels,
    /// The height of the box the number is centred in.
    pub(crate) line_height: Pixels,
}

impl PageNumber {
    /// The number for a bottom margin `margin` high, where the interface's
    /// small text is `text_size` (both as they are drawn): in the lower
    /// part of the margin, so that the text above it keeps its air, and
    /// smaller when the margin is small. `None` in a margin that has no
    /// room for a number at all.
    pub(crate) fn in_margin(margin: Pixels, text_size: Pixels) -> Option<Self> {
        let font_size = text_size.max(px(8.)).min((margin * 0.5).max(px(6.)));
        let line_height = (font_size * 1.4).round();
        (margin >= line_height + px(2.)).then_some(Self {
            font_size,
            line_height,
        })
    }

    /// The top of the number's box on a sheet: its middle two fifths of
    /// the way up the bottom margin.
    pub(crate) fn top(&self, sheet: Bounds<Pixels>, margin: Pixels) -> Pixels {
        (sheet.bottom() - margin * 0.4 - self.line_height / 2.)
            .min(sheet.bottom() - self.line_height - px(1.))
            .round()
    }
}

/// Shapes a single line of interface or marker text.
pub(crate) fn shape_label(
    text_system: &WindowTextSystem,
    text: SharedString,
    font: Font,
    font_size: Pixels,
    color: Hsla,
) -> ShapedLine {
    let run = TextRun {
        len: text.len(),
        font,
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    text_system.shape_line(text, font_size, &[run], None)
}

/// Works out the decorations of one block and hands them to `sink`.
pub(crate) fn plan_block(sink: &mut impl DecorationSink, cx: &BlockContext) {
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
    let text_system = &*env.text_system;
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
            sink.rect(match level {
                // Disc, ring, square: the three levels repeat.
                0 => RectShape::filled(bounds, colors.muted).radius(side / 2.),
                1 => RectShape {
                    bounds,
                    fill: ShapeFill::None,
                    radius: side / 2.,
                    border: Some((px(1.), colors.muted)),
                    screen_only: false,
                },
                _ => RectShape::filled(bounds, colors.muted),
            });
        }
        BlockKind::NumberedItem => {
            let text: SharedString = format!("{}.", cx.ordinal.unwrap_or(1)).into();
            let line = shape_label(
                text_system,
                text.clone(),
                style.body_font.clone(),
                font_size,
                colors.muted,
            );
            // Right-aligned in the gutter so the dots line up. The gutter
            // is as wide as the widest number of the list needs.
            sink.label(LabelShape {
                origin: point(text_left - units(MARKER_GAP) - line.width, first_row_y),
                line_height,
                line,
                text,
                font: style.body_font.clone(),
                font_size,
                color: colors.muted,
                anchor: LabelAnchor::End,
                anchor_x: text_left - units(MARKER_GAP),
                screen_only: false,
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
            sink.rect(if *checked {
                RectShape::filled(bounds, colors.accent).radius(radius)
            } else {
                RectShape {
                    bounds,
                    fill: ShapeFill::Surface(style.surface()),
                    radius,
                    border: Some((px(1.5), colors.border_strong)),
                    screen_only: false,
                }
            });
            if *checked {
                let at = |x: f32, y: f32| {
                    point(
                        bounds.left() + px(f32::from(side) * x),
                        bounds.top() + px(f32::from(side) * y),
                    )
                };
                sink.stroke(StrokeShape {
                    points: vec![at(0.25, 0.53), at(0.43, 0.70), at(0.76, 0.31)],
                    width: px((f32::from(side) * 0.12).max(1.4)),
                    color: colors.on_accent,
                });
            }
            sink.checkbox(CheckboxHit {
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
                sink.rect(
                    RectShape::filled(
                        Bounds::new(
                            point(text_left - units(QUOTE_GUTTER) + units(1.), top),
                            size(units(3.).max(px(2.)), bottom - top),
                        ),
                        colors.border_strong,
                    )
                    .radius(px(1.5)),
                );
            }
        }
        BlockKind::CodeBlock { language } => {
            let padding = units(CODE_PADDING);
            let left = text_left - padding;
            let width = view.len(layout.text_width) + padding * 2.;
            for segment in placed.segments() {
                let (top, bottom) = (view.y(segment.top), view.y(segment.bottom));
                sink.rect(
                    RectShape::filled(
                        Bounds::new(point(left, top), size(width, bottom - top)),
                        colors.hover,
                    )
                    .radius(theme::RADIUS_MD),
                );
            }
            if let Some(language) = language.as_ref().filter(|language| !language.is_empty()) {
                let text: SharedString = language.clone().into();
                let font_size = units(f32::from(theme::UI_TEXT_XS));
                let label = shape_label(
                    text_system,
                    text.clone(),
                    style.ui_font.clone(),
                    font_size,
                    colors.faint,
                );
                let label_height = units(16.);
                sink.label(LabelShape {
                    origin: point(
                        left + width - label.width - units(8.),
                        view.y(placed.top) + units(4.),
                    ),
                    line_height: label_height,
                    line: label,
                    text,
                    font: style.ui_font.clone(),
                    font_size,
                    color: colors.faint,
                    anchor: LabelAnchor::End,
                    anchor_x: left + width - units(8.),
                    screen_only: false,
                });
            }
        }
        BlockKind::Divider => {
            let y = snap(first_row_y + line_height / 2.);
            sink.rect(RectShape::filled(
                Bounds::new(point(column_left, y), size(column_width, px(1.))),
                colors.border_strong,
            ));
        }
        BlockKind::PageBreak => {
            // A quiet marker: a hairline interrupted by a label. In page
            // view the rest of the sheet below it stays empty. It is for
            // whoever edits the text: paper has the break itself.
            let y = snap(first_row_y + line_height / 2.);
            let text: SharedString = "Page break".into();
            let font_size = units(f32::from(theme::UI_TEXT_XS));
            let label = shape_label(
                text_system,
                text.clone(),
                style.ui_font.clone(),
                font_size,
                colors.faint,
            );
            let gap = units(10.);
            let label_x = (column_left + (column_width - label.width) / 2.).round();
            let left_width = label_x - gap - column_left;
            let hairline = |bounds: Bounds<Pixels>| RectShape {
                screen_only: true,
                ..RectShape::filled(bounds, colors.hairline)
            };
            if left_width > px(0.) {
                sink.rect(hairline(Bounds::new(
                    point(column_left, y),
                    size(left_width, px(1.)),
                )));
                let right = label_x + label.width + gap;
                sink.rect(hairline(Bounds::new(
                    point(right, y),
                    size(column_left + column_width - right, px(1.)),
                )));
            }
            sink.label(LabelShape {
                origin: point(label_x, first_row_y),
                line_height,
                line: label,
                text,
                font: style.ui_font.clone(),
                font_size,
                color: colors.faint,
                anchor: LabelAnchor::Middle,
                anchor_x: column_left + column_width / 2.,
                screen_only: true,
            });
        }
        BlockKind::Paragraph | BlockKind::Heading { .. } => {}
        // An image has no decoration: the picture itself is painted elsewhere.
        BlockKind::Image(_) => {}
    }
}
