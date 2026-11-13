//! What a printed page is, as data.
//!
//! The editor's layout fills these; the PDF writer draws them. Nothing here knows a
//! framework: the types are plain data and `Send`, so pages can be made on one thread
//! and written on another, compared in tests, and counted.
//!
//! All lengths are points (1/72 inch). A page's origin is its **top left corner, y
//! down** (the way a layout thinks; the PDF writer turns that into PDF coordinates).
//! The items of a page are painted in order, so a later one lies over an earlier one.

use std::sync::Arc;

/// An opaque colour in sRGB, each part `0.0..=1.0`. There is no alpha: paper has
/// none, and whoever fills a page blends a translucent colour onto white (or onto
/// what lies under it) first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    /// Red.
    pub r: f32,
    /// Green.
    pub g: f32,
    /// Blue.
    pub b: f32,
}

/// A font as the layout asked for it: the family by the name the app uses, a weight
/// and a style. The PDF writer finds the face.
#[derive(Clone, Debug, PartialEq)]
pub struct FontSpec {
    /// The family as the app names it; `".SystemUIFont"` is the interface font.
    pub family: String,
    /// The weight, `100.0..=900.0` (400 regular, 700 bold).
    pub weight: f32,
    /// Whether the face is italic.
    pub italic: bool,
    /// Families to take a glyph from when this one has none, in order.
    pub fallbacks: Vec<String>,
}

/// One printed page: its size and what is on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    /// The width of the sheet in points.
    pub width: f32,
    /// The height of the sheet in points.
    pub height: f32,
    /// What is on the page, painted in this order.
    pub items: Vec<Item>,
}

/// One thing on a page.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A rectangle, filled, bordered or both.
    Rect(RectItem),
    /// A stroked open line through several points.
    Stroke(StrokeItem),
    /// One visual row of document text.
    Row(TextRow),
    /// A single line the writer measures itself.
    Label(Label),
    /// A picture.
    Image(ImageItem),
}

/// A rectangle with optional rounded corners: the panel of a code block, the bar of
/// a quote, a highlight behind text, a rule, an underline, the box of a to-do item.
#[derive(Clone, Debug, PartialEq)]
pub struct RectItem {
    /// The left edge.
    pub x: f32,
    /// The top edge.
    pub y: f32,
    /// The width.
    pub width: f32,
    /// The height.
    pub height: f32,
    /// The radius of the corners; `0.0` for square ones.
    pub radius: f32,
    /// The colour it is filled with, if it is filled.
    pub fill: Option<Color>,
    /// The border, if it has one: its width and its colour. It is drawn inside the
    /// rectangle's edge.
    pub border: Option<(f32, Color)>,
}

/// A stroked open polyline with round joins and caps (the tick of a to-do box).
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeItem {
    /// The points the line goes through, in order.
    pub points: Vec<(f32, f32)>,
    /// The width of the line.
    pub width: f32,
    /// The colour of the line.
    pub color: Color,
}

/// One visual row of document text. The editor's layout says where every glyph
/// starts; the writer supplies the outlines.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRow {
    /// The row's text, UTF-8.
    pub text: String,
    /// The left edge of the row's text box on the page.
    pub x: f32,
    /// The y of the baseline on the page.
    pub baseline: f32,
    /// The block's font size.
    pub size: f32,
    /// The fonts and colours of the row: consecutive runs that cover `text`.
    pub runs: Vec<RowRun>,
    /// Where each shaped glyph starts: byte offset in `text`, x from
    /// [`TextRow::x`], and the fraction of [`TextRow::size`] it is drawn at (inline
    /// code is smaller). Ascending by byte within a run of one direction.
    pub glyphs: Vec<GlyphStart>,
    /// The row's shaped advance width as the layout has it. The writer compares its
    /// own with it: a row that comes out another width was set in another face.
    pub width: f32,
}

/// A stretch of a [`TextRow`] in one font and one colour.
#[derive(Clone, Debug, PartialEq)]
pub struct RowRun {
    /// Where the run ends: a byte offset in the row's text, exclusive. It starts
    /// where the run before it ends (the first at 0).
    pub end: usize,
    /// The font. Shared: the rows of a document name a handful of fonts.
    pub font: Arc<FontSpec>,
    /// The colour of the text.
    pub color: Color,
}

/// Where one shaped glyph of a [`TextRow`] starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphStart {
    /// The byte offset in the row's text of the cluster the glyph belongs to.
    pub byte: u32,
    /// The x of the glyph's origin, from the row's left edge.
    pub x: f32,
    /// The fraction of the row's font size the glyph is drawn at (`1.0` for body
    /// text).
    pub scale: f32,
}

/// A single line the writer measures itself: the number of a list item, a page
/// number, the label of a code block.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    /// The text.
    pub text: String,
    /// The font.
    pub font: Arc<FontSpec>,
    /// The font size.
    pub size: f32,
    /// The colour.
    pub color: Color,
    /// Where the line stands: the x of the point `anchor` names.
    pub x: f32,
    /// The y of the baseline on the page.
    pub baseline: f32,
    /// Which point of the line `x` is.
    pub anchor: Anchor,
}

/// Which point of a [`Label`]'s line its `x` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// The line starts at `x` and runs to the right of it.
    Start,
    /// The line is centred on `x`.
    Middle,
    /// The line ends at `x`.
    End,
}

/// A picture on a page.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageItem {
    /// Which picture this is. Items with the same key show the same picture, and it
    /// is put into the PDF once.
    pub key: [u8; 16],
    /// The picture as a file: PNG, JPEG or GIF. Shared between the items of one
    /// picture.
    pub bytes: Arc<[u8]>,
    /// The left edge.
    pub x: f32,
    /// The top edge.
    pub y: f32,
    /// The width it is drawn at.
    pub width: f32,
    /// The height it is drawn at.
    pub height: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    const INK: Color = Color {
        r: 0.11,
        g: 0.1,
        b: 0.09,
    };
    const MUTED: Color = Color {
        r: 0.5,
        g: 0.48,
        b: 0.45,
    };

    fn font(family: &str, weight: f32, italic: bool) -> Arc<FontSpec> {
        Arc::new(FontSpec {
            family: family.to_string(),
            weight,
            italic,
            fallbacks: vec!["Iowan Old Style".to_string(), "Georgia".to_string()],
        })
    }

    /// An A4 sheet with one of everything on it: a panel, a tick, a row of text in
    /// two fonts, a page number and a picture.
    fn a_page() -> Page {
        let body = font("Charter", 400.0, false);
        let bold = font("Charter", 700.0, false);
        let picture: Arc<[u8]> = Arc::from(&b"\x89PNG\r\n\x1a\n not a whole picture"[..]);
        Page {
            width: 595.276,
            height: 841.89,
            items: vec![
                Item::Rect(RectItem {
                    x: 72.0,
                    y: 72.0,
                    width: 451.276,
                    height: 40.0,
                    radius: 6.0,
                    fill: Some(Color {
                        r: 0.96,
                        g: 0.95,
                        b: 0.93,
                    }),
                    border: Some((1.0, MUTED)),
                }),
                Item::Stroke(StrokeItem {
                    points: vec![(75.0, 130.0), (78.0, 134.0), (84.0, 126.0)],
                    width: 1.5,
                    color: INK,
                }),
                Item::Row(TextRow {
                    text: "Hello, wörld".to_string(),
                    x: 72.0,
                    baseline: 160.0,
                    size: 17.0,
                    runs: vec![
                        RowRun {
                            end: 7,
                            font: body.clone(),
                            color: INK,
                        },
                        RowRun {
                            end: 13,
                            font: bold,
                            color: INK,
                        },
                    ],
                    glyphs: vec![
                        GlyphStart {
                            byte: 0,
                            x: 0.0,
                            scale: 1.0,
                        },
                        GlyphStart {
                            byte: 1,
                            x: 11.4,
                            scale: 1.0,
                        },
                        GlyphStart {
                            byte: 8,
                            x: 60.2,
                            scale: 0.9,
                        },
                    ],
                    width: 96.5,
                }),
                Item::Label(Label {
                    text: "1".to_string(),
                    font: body,
                    size: 11.0,
                    color: MUTED,
                    x: 297.638,
                    baseline: 805.0,
                    anchor: Anchor::Middle,
                }),
                Item::Image(ImageItem {
                    key: [7; 16],
                    bytes: picture,
                    x: 72.0,
                    y: 200.0,
                    width: 225.638,
                    height: 150.0,
                }),
            ],
        }
    }

    #[test]
    fn a_page_is_data_that_can_be_built_copied_and_compared() {
        let page = a_page();
        assert_eq!(page.items.len(), 5);
        // Built twice, it is the same page; a copy is the same page.
        assert_eq!(page, a_page());
        let copy = page.clone();
        assert_eq!(copy, page);
        // The row's runs cover its text, and every glyph starts inside it, on a
        // character.
        let Item::Row(row) = &page.items[2] else {
            panic!("the third item is the row");
        };
        assert_eq!(row.runs.last().map(|run| run.end), Some(row.text.len()));
        assert!(row.runs.windows(2).all(|pair| pair[0].end < pair[1].end));
        assert!(
            row.glyphs
                .iter()
                .all(|glyph| row.text.is_char_boundary(glyph.byte as usize))
        );
        // A copy shares the picture's bytes and the fonts: nothing of a book of
        // photographs is in memory twice because its pages were copied.
        let (Item::Image(image), Item::Image(copied)) = (&page.items[4], &copy.items[4]) else {
            panic!("the fifth item is the picture");
        };
        assert!(Arc::ptr_eq(&image.bytes, &copied.bytes));
        let (Item::Label(label), Item::Label(copied)) = (&page.items[3], &copy.items[3]) else {
            panic!("the fourth item is the page number");
        };
        assert!(Arc::ptr_eq(&label.font, &copied.font));
        // Two fonts that say the same are the same font, shared or not.
        assert_eq!(font("Charter", 400.0, false), font("Charter", 400.0, false));
        assert_ne!(font("Charter", 400.0, false), font("Charter", 400.0, true));
        assert_ne!(font("Charter", 400.0, false), font("Charter", 700.0, false));
        assert_ne!(font("Charter", 400.0, false), font("Georgia", 400.0, false));
    }

    #[test]
    fn a_page_that_differs_in_one_thing_is_another_page() {
        let page = a_page();
        let changed = |change: &dyn Fn(&mut Page)| {
            let mut other = a_page();
            change(&mut other);
            other
        };
        let others = [
            changed(&|p| p.width = 612.0),
            changed(&|p| p.height = 792.0),
            changed(&|p| {
                p.items.pop();
            }),
            // The order is the order of painting.
            changed(&|p| p.items.swap(0, 1)),
            changed(&|p| {
                if let Item::Rect(rect) = &mut p.items[0] {
                    rect.radius = 0.0;
                }
            }),
            changed(&|p| {
                if let Item::Rect(rect) = &mut p.items[0] {
                    rect.fill = None;
                }
            }),
            changed(&|p| {
                if let Item::Rect(rect) = &mut p.items[0] {
                    rect.border = Some((2.0, MUTED));
                }
            }),
            changed(&|p| {
                if let Item::Stroke(stroke) = &mut p.items[1] {
                    stroke.points.push((90.0, 120.0));
                }
            }),
            changed(&|p| {
                if let Item::Row(row) = &mut p.items[2] {
                    row.baseline += 0.5;
                }
            }),
            changed(&|p| {
                if let Item::Row(row) = &mut p.items[2] {
                    row.glyphs[1].x += 0.25;
                }
            }),
            changed(&|p| {
                if let Item::Row(row) = &mut p.items[2] {
                    row.runs[0].color = MUTED;
                }
            }),
            changed(&|p| {
                if let Item::Row(row) = &mut p.items[2] {
                    row.runs[1].font = font("Charter", 700.0, true);
                }
            }),
            changed(&|p| {
                if let Item::Label(label) = &mut p.items[3] {
                    label.anchor = Anchor::End;
                }
            }),
            changed(&|p| {
                if let Item::Label(label) = &mut p.items[3] {
                    label.text = "2".to_string();
                }
            }),
            changed(&|p| {
                if let Item::Image(image) = &mut p.items[4] {
                    image.key = [8; 16];
                }
            }),
            changed(&|p| {
                if let Item::Image(image) = &mut p.items[4] {
                    image.bytes = Arc::from(&b"other bytes"[..]);
                }
            }),
            changed(&|p| {
                if let Item::Image(image) = &mut p.items[4] {
                    image.width = 451.276;
                }
            }),
        ];
        for (i, other) in others.iter().enumerate() {
            assert_ne!(other, &page, "change {i} was not seen");
        }
    }

    /// Pages are made on one thread and may be written on another.
    #[test]
    fn pages_can_go_to_another_thread() {
        fn sendable<T: Send + Sync + 'static>() {}
        sendable::<Page>();
        sendable::<Item>();
        sendable::<FontSpec>();
        let page = a_page();
        let from_the_other_thread = std::thread::spawn(move || {
            let count = page.items.len();
            (page, count)
        })
        .join()
        .expect("the thread ended");
        assert_eq!(from_the_other_thread, (a_page(), 5));
    }
}
