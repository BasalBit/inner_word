//! Tests of the PDF writer with real fonts (CoreText) and without GPUI.
//!
//! Pages are made by hand here, the way the editor's print pass makes them, and
//! what the writer made of them is read back with [`PdfFile`]: page count, media
//! boxes, the text PDFKit finds, where a word stands, and the pixels of a page.
//!
//! Rows whose glyphs are 0.6 em apart (what GPUI's test text system gives every
//! glyph) are set in Courier New, whose own advance is 0.6001 em: in any other
//! font PDFKit sees spaces inside the words.

use std::sync::Arc;

use crate::{
    Anchor, Color, FontSpec, GlyphStart, ImageItem, Item, JobInfo, Label, Page, Pdf, PdfFile,
    PdfWriter, RectItem, Report, RowRun, StrokeItem, TextRow, check_selectors, fonts::Fonts,
    read::picture_file,
};

const A4: (f32, f32) = (595.276, 841.89);
const LETTER_LANDSCAPE: (f32, f32) = (792., 612.);
const INK: Color = Color {
    r: 0.1,
    g: 0.1,
    b: 0.1,
};
const RED: Color = Color {
    r: 0.8,
    g: 0.1,
    b: 0.1,
};
const BLUE: Color = Color {
    r: 0.1,
    g: 0.2,
    b: 0.9,
};

fn font(family: &str, weight: f32, italic: bool) -> Arc<FontSpec> {
    Arc::new(FontSpec {
        family: family.to_string(),
        weight,
        italic,
        fallbacks: Vec::new(),
    })
}

fn courier() -> Arc<FontSpec> {
    font("Courier New", 400., false)
}

fn charter() -> Arc<FontSpec> {
    Arc::new(FontSpec {
        family: "Charter".to_string(),
        weight: 400.,
        italic: false,
        fallbacks: vec!["Iowan Old Style".to_string(), "Georgia".to_string()],
    })
}

/// A row whose glyphs start 0.6 em apart, one per character: what the layout
/// hands over in a test of the editor.
fn fixed_row(text: &str, font: Arc<FontSpec>, size: f32, x: f32, baseline: f32) -> TextRow {
    let glyphs: Vec<GlyphStart> = text
        .char_indices()
        .enumerate()
        .map(|(index, (byte, _))| GlyphStart {
            byte: byte as u32,
            x: index as f32 * 0.6 * size,
            scale: 1.,
        })
        .collect();
    TextRow {
        text: text.to_string(),
        x,
        baseline,
        size,
        runs: vec![RowRun {
            end: text.len(),
            font,
            color: INK,
        }],
        width: glyphs.len() as f32 * 0.6 * size,
        glyphs,
    }
}

fn page(size: (f32, f32), items: Vec<Item>) -> Page {
    Page {
        width: size.0,
        height: size.1,
        items,
    }
}

fn write(pages: &[Page]) -> (Pdf, PdfFile) {
    let mut writer = PdfWriter::new(&JobInfo {
        title: "A test".to_string(),
        creator: "Inner Word 0.0.0".to_string(),
    });
    for page in pages {
        writer.add_page(page);
    }
    assert_eq!(writer.pages(), pages.len());
    let pdf = writer.finish();
    let file = PdfFile::open(pdf.bytes()).expect("the writer made a PDF");
    (pdf, file)
}

/// The text of a page without anything a reader of PDFs adds or leaves out
/// between glyphs.
fn squeezed(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn close(a: f32, b: f32, slack: f32) -> bool {
    (a - b).abs() <= slack
}

/// The colour of the pixel at a point of a page (points from its top left).
fn pixel(file: &PdfFile, page: usize, x: f32, y: f32) -> [u8; 3] {
    const SCALE: f32 = 4.;
    let (width, height, pixels) = file.page_pixels(page, SCALE).expect("the page as pixels");
    let (column, row) = ((x * SCALE) as usize, (y * SCALE) as usize);
    assert!(column < width && row < height, "({x}, {y}) is on the page");
    let at = (row * width + column) * 4;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}

fn is(pixel: [u8; 3], color: Color) -> bool {
    let wanted = [color.r, color.g, color.b].map(|part| (part * 255.).round() as i32);
    pixel
        .iter()
        .zip(wanted)
        .all(|(have, want)| (i32::from(*have) - want).abs() <= 6)
}

const WHITE: Color = Color {
    r: 1.,
    g: 1.,
    b: 1.,
};

#[test]
fn two_pages_of_different_sizes_keep_their_boxes_and_their_text() {
    let (pdf, file) = write(&[
        page(
            A4,
            vec![
                Item::Row(fixed_row(
                    "The first page, on A4.",
                    courier(),
                    12.,
                    72.,
                    100.,
                )),
                Item::Row(fixed_row("Its second row.", courier(), 12., 72., 120.)),
            ],
        ),
        page(
            LETTER_LANDSCAPE,
            vec![Item::Row(fixed_row(
                "The second page, on its side.",
                courier(),
                12.,
                72.,
                100.,
            ))],
        ),
    ]);
    assert_eq!(file.page_count(), 2);
    let (width, height) = file.page_size(0);
    assert!(
        close(width, A4.0, 0.01) && close(height, A4.1, 0.01),
        "{width} x {height}"
    );
    assert_eq!(file.page_size(1), LETTER_LANDSCAPE);
    assert_eq!(
        file.page_text(0).trim(),
        "The first page, on A4.\nIts second row."
    );
    assert_eq!(file.page_text(1).trim(), "The second page, on its side.");
    // A page the file does not have is empty, not a panic.
    assert_eq!(file.page_size(2), (0., 0.));
    assert_eq!(file.page_text(2), "");
    assert_eq!(file.find(2, "The"), None);
    assert!(file.page_png(2, 1.).is_none());

    assert_eq!(pdf.report.pages, 2);
    assert_eq!(pdf.report.rows, 3);
    assert_eq!(pdf.report.rows_off, 0);
    assert_eq!(pdf.report.fonts, vec!["CourierNewPSMT".to_string()]);
    assert!(pdf.report.missing_families.is_empty());
    assert_eq!(
        file.title_and_creator(),
        ("A test".to_string(), "Inner Word 0.0.0".to_string())
    );
}

#[test]
fn a_word_stands_where_the_layout_put_it() {
    let (size, x, baseline) = (12., 90.5, 233.25);
    let text = "Fixed pitch: still ill at ease, illicit lilt.";
    let (pdf, file) = write(&[page(
        A4,
        vec![Item::Row(fixed_row(text, courier(), size, x, baseline))],
    )]);
    assert_eq!(file.page_text(0).trim(), text);
    assert_eq!(pdf.report.rows_off, 0);

    // PDFKit's box for a word starts at the origin of its first glyph and is as
    // high as the font: its bottom is the baseline plus the font's descent.
    let descent = Fonts::new().font(&courier(), size).descent() as f32;
    for word in ["Fixed", "still", "illicit", "lilt."] {
        let chars_before = text[..text.find(word).unwrap()].chars().count();
        let (found_x, top, width, height) = file.find(0, word).expect(word);
        let wanted_x = x + chars_before as f32 * 0.6 * size;
        assert!(
            close(found_x, wanted_x, 0.05),
            "{word}: x {found_x}, wanted {wanted_x}"
        );
        let found_baseline = top + height - descent;
        assert!(
            close(found_baseline, baseline, 0.05),
            "{word}: baseline {found_baseline}, wanted {baseline}"
        );
        // As wide as its characters at the layout's pitch (the last one at
        // the font's own advance).
        let wanted_width = word.chars().count() as f32 * 0.6 * size;
        assert!(close(width, wanted_width, 0.1), "{word}: width {width}");
    }
    assert_eq!(file.find(0, "not on the page"), None);
    assert_eq!(file.find(0, ""), None);
}

#[test]
fn bold_and_italic_are_the_bold_and_italic_faces() {
    let row = |text: &str, font: Arc<FontSpec>, baseline: f32| {
        Item::Row(fixed_row(text, font, 12., 72., baseline))
    };
    let (pdf, file) = write(&[page(
        A4,
        vec![
            row("regular", font("Courier New", 400., false), 100.),
            row("bold", font("Courier New", 700., false), 120.),
            row("italic", font("Courier New", 400., true), 140.),
            row("bold italic", font("Courier New", 700., true), 160.),
            row("serif", font("Charter", 400., false), 180.),
            row("serif bold", font("Charter", 700., false), 200.),
            row("serif italic", font("Charter", 400., true), 220.),
            Item::Label(Label {
                text: "7".to_string(),
                font: font(".SystemUIFont", 400., false),
                size: 11.,
                color: INK,
                x: A4.0 / 2.,
                baseline: 800.,
                anchor: Anchor::Middle,
            }),
        ],
    )]);
    let fonts = &pdf.report.fonts;
    for name in [
        "CourierNewPSMT",
        "CourierNewPS-BoldMT",
        "CourierNewPS-ItalicMT",
        "CourierNewPS-BoldItalicMT",
        "Charter-Roman",
        "Charter-Bold",
        "Charter-Italic",
    ] {
        assert!(
            fonts.iter().any(|font| font == name),
            "{name} is not among {fonts:?}"
        );
    }
    // The interface font is the system's own, found by its family and never by
    // its PostScript name (which CoreText answers with Times New Roman).
    assert!(
        fonts.iter().any(|font| font.starts_with(".SF")),
        "no system font among {fonts:?}"
    );
    assert!(
        !fonts.iter().any(|font| font.contains("Times")),
        "{fonts:?}"
    );
    assert_eq!(fonts.len(), 8, "{fonts:?}");
    let mut sorted = fonts.clone();
    sorted.sort();
    assert_eq!(&sorted, fonts);
    assert!(pdf.report.missing_families.is_empty());
    assert!(file.page_text(0).contains("bold italic"));
    assert!(file.page_text(0).trim_end().ends_with('7'));
}

#[test]
fn a_row_of_other_advances_than_the_layout_measured_is_counted() {
    // A row placed at CoreText's own advances: Courier New's is 1229 units of
    // 2048.
    let advance = 1229. / 2048. * 14.;
    let text = "placed at the font's own advances";
    let mut row = fixed_row(text, courier(), 14., 72., 100.);
    for (index, glyph) in row.glyphs.iter_mut().enumerate() {
        glyph.x = index as f32 * advance;
    }
    row.width = text.len() as f32 * advance;
    // The layout measured every glyph twice as wide. The row starts further
    // left so that every glyph of it is on the page: a glyph past the edge of
    // the sheet is in the page text of some versions of PDFKit and not of
    // others (macOS 15 leaves it out).
    let mut doubled = row.clone();
    doubled.x = 20.;
    doubled.baseline = 130.;
    doubled.width *= 2.;
    for glyph in &mut doubled.glyphs {
        glyph.x *= 2.;
    }
    let last = doubled.glyphs.last().expect("glyphs").x;
    assert!(doubled.x + last + advance < A4.0, "{last}");
    // Only the width differs, with every glyph inside the row where CoreText
    // has it: what the row's ends do in its line (a tab's stop, a contextual
    // form) and not another face.
    let mut only_wider = row.clone();
    only_wider.baseline = 160.;
    only_wider.width += 30.;
    // A row without text is as wide as nothing.
    let empty = TextRow {
        text: String::new(),
        runs: Vec::new(),
        glyphs: Vec::new(),
        width: 0.,
        ..row.clone()
    };
    let mut empty_but_wide = empty.clone();
    empty_but_wide.width = 40.;

    let (pdf, _) = write(&[page(A4, vec![Item::Row(row.clone())])]);
    assert_eq!((pdf.report.rows, pdf.report.rows_off), (1, 0));
    let (pdf, file) = write(&[page(
        A4,
        vec![
            Item::Row(row),
            Item::Row(doubled),
            Item::Row(only_wider),
            Item::Row(empty),
            Item::Row(empty_but_wide),
        ],
    )]);
    assert_eq!((pdf.report.rows, pdf.report.rows_off), (5, 2));
    // The rows with text are whole all the same. PDFKit reads the doubled one
    // as letters with gaps between them, and where it puts those among the
    // other rows differs between versions of macOS (macOS 15 reads half of it
    // into the line of the row below), so the letters are compared as letters:
    // every one of the three rows, no more and no fewer.
    let page_text = file.page_text(0);
    assert_eq!(page_text.trim().lines().next(), Some(text), "{page_text}");
    let letters = |text: &str| {
        let mut letters: Vec<char> = squeezed(text).chars().collect();
        letters.sort_unstable();
        letters
    };
    assert_eq!(letters(&page_text), letters(&text.repeat(3)), "{page_text}");

    // A row too short to have clusters inside it is judged by its width,
    // unless it holds a tab, whose width depends on where the row stands.
    let short = |text: &str, wider: f32| {
        let mut row = fixed_row(text, courier(), 14., 72., 100.);
        for (index, glyph) in row.glyphs.iter_mut().enumerate() {
            glyph.x = index as f32 * advance;
        }
        row.width = text.len() as f32 * advance + wider;
        Item::Row(row)
    };
    let (pdf, _) = write(&[page(
        A4,
        vec![short("ab", 0.), short("ab", 20.), short("a\tb", 20.)],
    )]);
    assert_eq!((pdf.report.rows, pdf.report.rows_off), (3, 1));

    // The same text in a proportional font at the fixed pitch: the text is
    // whole, every glyph starts where the layout said, and the row is counted.
    let (pdf, file) = write(&[page(
        A4,
        vec![Item::Row(fixed_row(text, charter(), 14., 72., 100.))],
    )]);
    assert_eq!(pdf.report.rows_off, 1);
    assert_eq!(squeezed(&file.page_text(0)), squeezed(text));
    let (x, ..) = file.find(0, "p").expect("the first letter");
    assert!(close(x, 72., 0.05), "{x}");
}

#[test]
fn glyphs_the_font_does_not_have_come_from_other_fonts() {
    let text = "Emoji and CJK in Charter: 😀 漢字 → done.";
    let (pdf, file) = write(&[page(
        A4,
        vec![Item::Row(fixed_row(text, charter(), 12., 72., 100.))],
    )]);
    let found = file.page_text(0);
    assert_eq!(squeezed(&found), squeezed(text), "{found:?}");
    let fonts = &pdf.report.fonts;
    assert!(
        fonts.iter().any(|font| font == "Charter-Roman"),
        "{fonts:?}"
    );
    assert!(fonts.iter().any(|font| font.contains("Emoji")), "{fonts:?}");
    assert!(
        fonts.len() >= 3,
        "a font for the emoji and one for the CJK characters: {fonts:?}"
    );
}

#[test]
fn bytes_and_utf16_units_are_told_apart() {
    // Two-byte characters, a three-byte one and a four-byte one (two UTF-16
    // units) stand before the word: its glyph starts are named by byte, and
    // CoreText names its glyphs by UTF-16 unit.
    let size = 12.;
    for (text, word) in [
        ("ééé naïve word", "word"),
        ("漢字 then a word", "word"),
        ("😀 after an emoji: word", "word"),
        ("😀😀 é 漢 mixed: word", "word"),
    ] {
        let (_, file) = write(&[page(
            A4,
            vec![Item::Row(fixed_row(text, courier(), size, 72., 100.))],
        )]);
        let chars_before = text[..text.find(word).unwrap()].chars().count();
        let wanted = 72. + chars_before as f32 * 0.6 * size;
        let (x, ..) = file
            .find(0, word)
            .unwrap_or_else(|| panic!("{word:?} in {:?}", file.page_text(0)));
        assert!(close(x, wanted, 0.05), "{text:?}: x {x}, wanted {wanted}");
    }
}

#[test]
fn runs_have_their_fonts_colours_and_sizes() {
    // "plain bold code": the middle word bold and red, the last one drawn at
    // 0.88 of the size, as inline code is.
    let size = 20.;
    let text = "plain bold code";
    let mut row = fixed_row(text, courier(), size, 72., 100.);
    row.runs = vec![
        RowRun {
            end: 6,
            font: courier(),
            color: INK,
        },
        RowRun {
            end: 11,
            font: font("Courier New", 700., false),
            color: RED,
        },
        RowRun {
            end: text.len(),
            font: courier(),
            color: BLUE,
        },
    ];
    for glyph in &mut row.glyphs[11..] {
        glyph.scale = 0.88;
        glyph.x = 11. * 0.6 * size + (glyph.byte as f32 - 11.) * 0.6 * size * 0.88;
    }
    let (pdf, file) = write(&[page(A4, vec![Item::Row(row)])]);
    assert_eq!(file.page_text(0).trim(), text);
    assert_eq!(
        pdf.report.fonts,
        vec![
            "CourierNewPS-BoldMT".to_string(),
            "CourierNewPSMT".to_string()
        ]
    );
    // The smaller word is as wide as its characters at the smaller size.
    let (x, _, width, height) = file.find(0, "code").expect("code");
    assert!(close(x, 72. + 11. * 0.6 * size, 0.05), "{x}");
    assert!(close(width, 4. * 0.6 * size * 0.88, 0.2), "{width}");
    // (PDFKit's box is as high as the row, whatever the size of a word in it.)
    let (_, _, _, full_height) = file.find(0, "plain").expect("plain");
    assert!(
        close(height, full_height, 0.05),
        "{height} of {full_height}"
    );

    // The colours: somewhere in each word there is ink of its run's colour and
    // none of the others'.
    let colours_in = |word: &str| {
        let (x, top, width, height) = file.find(0, word).expect("the word");
        let (page_width, _, pixels) = file.page_pixels(0, 4.).expect("pixels");
        let mut seen = [false; 3];
        for row in (top * 4.) as usize..((top + height) * 4.) as usize {
            for column in (x * 4.) as usize..((x + width) * 4.) as usize {
                let at = (row * page_width + column) * 4;
                let pixel = [pixels[at], pixels[at + 1], pixels[at + 2]];
                for (index, color) in [INK, RED, BLUE].into_iter().enumerate() {
                    seen[index] |= is(pixel, color);
                }
            }
        }
        seen
    };
    assert_eq!(colours_in("plain"), [true, false, false]);
    assert_eq!(colours_in("bold"), [false, true, false]);
    assert_eq!(colours_in("code"), [false, false, true]);
}

#[test]
fn a_label_is_placed_by_its_anchor() {
    let label = |anchor: Anchor, baseline: f32| {
        Item::Label(Label {
            text: "12.".to_string(),
            font: courier(),
            size: 10.,
            color: INK,
            x: 300.,
            baseline,
            anchor,
        })
    };
    let (pdf, file) = write(&[
        page(A4, vec![label(Anchor::Start, 100.)]),
        page(A4, vec![label(Anchor::Middle, 100.)]),
        page(A4, vec![label(Anchor::End, 100.)]),
    ]);
    // Three characters of Courier New at 10 points.
    let width = 3. * 1229. / 2048. * 10.;
    for (page, wanted) in [(0, 300.), (1, 300. - width / 2.), (2, 300. - width)] {
        let (x, _, found_width, _) = file.find(page, "12.").expect("the label");
        assert!(
            close(x, wanted, 0.05),
            "page {page}: x {x}, wanted {wanted}"
        );
        assert!(close(found_width, width, 0.05), "{found_width}");
    }
    // A label is not a row of the document.
    assert_eq!(pdf.report.rows, 0);
    assert_eq!(pdf.report.rows_off, 0);
}

#[test]
fn shapes_are_drawn_where_they_are_said_to_be() {
    let (_, file) = write(&[page(
        A4,
        vec![
            // A filled panel with a border inside its edge.
            Item::Rect(RectItem {
                x: 100.,
                y: 100.,
                width: 200.,
                height: 60.,
                radius: 0.,
                fill: Some(BLUE),
                border: Some((4., RED)),
            }),
            // A ring: a border and no fill.
            Item::Rect(RectItem {
                x: 100.,
                y: 200.,
                width: 40.,
                height: 40.,
                radius: 20.,
                fill: None,
                border: Some((4., INK)),
            }),
            // A disc.
            Item::Rect(RectItem {
                x: 200.,
                y: 200.,
                width: 40.,
                height: 40.,
                radius: 20.,
                fill: Some(RED),
                border: None,
            }),
            // A tick.
            Item::Stroke(StrokeItem {
                points: vec![(100., 320.), (120., 340.), (160., 300.)],
                width: 4.,
                color: INK,
            }),
            // Later items lie over earlier ones.
            Item::Rect(RectItem {
                x: 400.,
                y: 100.,
                width: 60.,
                height: 60.,
                radius: 0.,
                fill: Some(RED),
                border: None,
            }),
            Item::Rect(RectItem {
                x: 420.,
                y: 120.,
                width: 20.,
                height: 20.,
                radius: 0.,
                fill: Some(BLUE),
                border: None,
            }),
            // Nothing: no size, and neither fill nor border.
            Item::Rect(RectItem {
                x: 500.,
                y: 500.,
                width: 0.,
                height: 30.,
                radius: 0.,
                fill: Some(RED),
                border: Some((2., RED)),
            }),
        ],
    )]);
    let at = |x: f32, y: f32| pixel(&file, 0, x, y);
    // The panel: its border inside its edge, its fill inside the border, the
    // paper outside.
    assert!(is(at(200., 130.), BLUE));
    assert!(
        is(at(102., 130.), RED),
        "the border is inside the left edge"
    );
    assert!(is(at(298., 130.), RED));
    assert!(is(at(200., 102.), RED));
    assert!(is(at(200., 158.), RED));
    assert!(is(at(98., 130.), WHITE), "nothing outside the edge");
    assert!(is(at(302., 130.), WHITE));
    assert!(is(at(200., 98.), WHITE));
    assert!(is(at(106., 130.), BLUE), "the border is four points wide");
    // The ring: ink on the circle, paper in its middle and in the corners of
    // its box.
    assert!(is(at(102., 220.), INK));
    assert!(is(at(120., 220.), WHITE));
    assert!(is(at(101., 201.), WHITE));
    // The disc: filled to its middle, round at its corners.
    assert!(is(at(220., 220.), RED));
    assert!(is(at(201., 201.), WHITE));
    // The tick goes through its three points, and y grows downwards: the
    // middle point is the lowest.
    assert!(is(at(100., 320.), INK));
    assert!(is(at(120., 340.), INK));
    assert!(is(at(160., 300.), INK));
    assert!(is(at(140., 320.), INK));
    assert!(is(at(120., 300.), WHITE));
    // In order.
    assert!(is(at(410., 110.), RED));
    assert!(is(at(430., 130.), BLUE));
    assert!(is(at(500., 510.), WHITE));
}

/// A picture with something in every part of it, so that its compressed data is
/// long enough to be looked for: red at the left running to blue at the right,
/// with a pattern on top.
fn picture(width: usize, height: usize) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let ramp = (x * 255 / width.max(1)) as u8;
            let pattern = ((x * 7 + y * 13) % 64) as u8;
            pixels.extend_from_slice(&[255 - ramp, pattern * 2, ramp, 255]);
        }
    }
    pixels
}

fn count_of(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

#[test]
fn a_jpeg_goes_into_the_pdf_as_it_is_and_once() {
    let jpeg: Arc<[u8]> = Arc::from(picture_file(160, 120, &picture(160, 120), true));
    assert_eq!(&jpeg[..3], &[0xff, 0xd8, 0xff], "a JPEG file");
    // A stretch from the middle of the file: compressed picture data, which
    // stands in the PDF only if the file's own data was taken over.
    let middle = &jpeg[jpeg.len() / 2..jpeg.len() / 2 + 48];
    let item = |key: u8, bytes: &Arc<[u8]>, x: f32, y: f32| {
        Item::Image(ImageItem {
            key: [key; 16],
            bytes: bytes.clone(),
            x,
            y,
            width: 160.,
            height: 120.,
        })
    };
    let (once, file) = write(&[page(A4, vec![item(1, &jpeg, 100., 100.)])]);
    assert_eq!(once.report.images, 1);
    assert_eq!(once.report.images_unreadable, 0);
    assert_eq!(
        count_of(once.bytes(), middle),
        1,
        "the JPEG's own data is in the PDF"
    );
    // Drawn where it was put, the right way up and round: red at its left,
    // blue at its right, paper around it.
    let at = |x: f32, y: f32| pixel(&file, 0, x, y);
    let (left, right) = (at(104., 160.), at(256., 160.));
    assert!(left[0] > 180 && left[2] < 80, "red at the left: {left:?}");
    assert!(
        right[2] > 180 && right[0] < 80,
        "blue at the right: {right:?}"
    );
    assert!(is(at(98., 160.), WHITE));
    assert!(is(at(262., 160.), WHITE));
    assert!(is(at(180., 98.), WHITE));
    assert!(is(at(180., 222.), WHITE));

    // The same picture three times, on two pages: in the PDF once.
    let (thrice, _) = write(&[
        page(
            A4,
            vec![item(1, &jpeg, 100., 100.), item(1, &jpeg, 100., 300.)],
        ),
        page(A4, vec![item(1, &jpeg, 100., 100.)]),
    ]);
    assert_eq!(thrice.report.images, 1);
    assert_eq!(count_of(thrice.bytes(), middle), 1);
    assert!(
        thrice.len() < once.len() + jpeg.len() / 2,
        "{} bytes with three, {} with one, the picture is {}",
        thrice.len(),
        once.len(),
        jpeg.len()
    );
    // Another key is another picture, whatever its bytes.
    let (two, _) = write(&[page(
        A4,
        vec![item(1, &jpeg, 100., 100.), item(2, &jpeg, 100., 300.)],
    )]);
    assert_eq!(two.report.images, 2);
}

#[test]
fn a_png_is_drawn_and_bytes_that_are_no_picture_are_counted() {
    let png: Arc<[u8]> = Arc::from(picture_file(80, 60, &picture(80, 60), false));
    assert_eq!(&png[1..4], b"PNG");
    let not_a_picture: Arc<[u8]> = Arc::from(&b"these bytes are no picture at all"[..]);
    let nothing: Arc<[u8]> = Arc::from(&b""[..]);
    let item = |key: u8, bytes: &Arc<[u8]>, y: f32| {
        Item::Image(ImageItem {
            key: [key; 16],
            bytes: bytes.clone(),
            x: 100.,
            y,
            width: 80.,
            height: 60.,
        })
    };
    let (pdf, file) = write(&[page(
        A4,
        vec![
            item(1, &png, 100.),
            item(2, &not_a_picture, 200.),
            item(2, &not_a_picture, 300.),
            item(3, &nothing, 400.),
        ],
    )]);
    assert_eq!(pdf.report.images, 1);
    assert_eq!(
        pdf.report.images_unreadable, 2,
        "two keys, one of them twice"
    );
    let at = |x: f32, y: f32| pixel(&file, 0, x, y);
    let left = at(102., 130.);
    assert!(left[0] > 180 && left[2] < 80, "the PNG: {left:?}");
    // Nothing where the others would be.
    for y in [230., 330., 430.] {
        assert!(is(at(140., y), WHITE), "something was drawn at {y}");
    }
}

#[test]
fn a_family_that_is_not_installed_is_named_and_the_text_is_whole() {
    let text = "Set in a font nobody has.";
    let (pdf, file) = write(&[page(
        A4,
        vec![
            Item::Row(fixed_row(
                text,
                font("No Such Family Anywhere", 700., true),
                12.,
                72.,
                100.,
            )),
            Item::Row(fixed_row(
                "And one that is there.",
                courier(),
                12.,
                72.,
                120.,
            )),
        ],
    )]);
    assert_eq!(
        pdf.report.missing_families,
        vec!["No Such Family Anywhere".to_string()]
    );
    assert_eq!(
        squeezed(&file.page_text(0)),
        squeezed(&format!("{text}And one that is there."))
    );
    // GPUI's first last resort a Mac has, in its regular face.
    assert!(
        pdf.report.fonts.iter().any(|font| font == "Helvetica"),
        "{:?}",
        pdf.report.fonts
    );
    assert_eq!(
        pdf.report.missing_line().as_deref(),
        Some("missing fonts: No Such Family Anywhere")
    );
}

#[test]
fn every_selector_of_the_print_operation_is_there() {
    assert_eq!(check_selectors(), Ok(()));
}

#[test]
fn a_finished_pdf_is_a_pdf_and_a_clone_shares_it() {
    let (pdf, file) = write(&[page(
        A4,
        vec![Item::Row(fixed_row("One page.", courier(), 12., 72., 100.))],
    )]);
    assert_eq!(&pdf.bytes()[..4], b"%PDF");
    assert_eq!(pdf.len(), pdf.bytes().len());
    assert!(!pdf.is_empty());
    assert_eq!(file.page_count(), 1);
    let copy = pdf.clone();
    assert_eq!(copy.bytes().as_ptr(), pdf.bytes().as_ptr());
    assert_eq!(copy.report, pdf.report);
    assert_eq!(copy.ns_data(), pdf.ns_data());
    // The picture of a page is a PNG.
    let png = file.page_png(0, 1.).expect("the page as a picture");
    assert_eq!(&png[1..4], b"PNG");
    // What is no PDF is not opened.
    assert!(PdfFile::open(b"not a PDF").is_none());
    assert!(PdfFile::open(b"").is_none());
    // A writer that was given no page has made nothing that can be printed
    // (not the blank sheet CoreGraphics closes such a PDF with).
    let empty = PdfWriter::new(&JobInfo {
        title: String::new(),
        creator: String::new(),
    })
    .finish();
    assert_eq!(empty.report.pages, 0);
    assert!(empty.is_empty());
    assert!(PdfFile::open(empty.bytes()).is_none());
    assert!(format!("{pdf:?}").starts_with("Pdf { len: "));
}

/// A PDF made elsewhere (WebKit's, for a page) gives its bytes back as they
/// came, with the page count it was given and nothing else in the report.
#[test]
fn a_pdf_from_bytes_gives_them_back() {
    let (made, _) = write(&[
        page(
            A4,
            vec![Item::Row(fixed_row("One.", courier(), 12., 72., 100.))],
        ),
        page(
            A4,
            vec![Item::Row(fixed_row("Two.", courier(), 12., 72., 100.))],
        ),
    ]);
    let bytes = made.bytes().to_vec();
    let pdf = Pdf::from_bytes(bytes.clone(), 2);
    assert_eq!(pdf.bytes(), &bytes[..]);
    assert_eq!(pdf.len(), bytes.len());
    assert!(!pdf.is_empty());
    assert_eq!(
        pdf.report,
        Report {
            pages: 2,
            ..Report::default()
        }
    );
    assert_eq!(PdfFile::open(pdf.bytes()).unwrap().page_count(), 2);
    // a clone shares them
    assert_eq!(pdf.clone().bytes().as_ptr(), pdf.bytes().as_ptr());
    // taken as they are
    let odd = Pdf::from_bytes(b"not a PDF".to_vec(), 1);
    assert_eq!(odd.bytes(), b"not a PDF");
    assert_eq!(odd.report.pages, 1);
    assert!(Pdf::from_bytes(Vec::new(), 0).is_empty());
}

#[test]
fn a_page_with_broken_rows_is_still_written() {
    // What the layout never hands over, and what must not bring a print job
    // down all the same: runs that end inside a character or past the text,
    // glyph starts past the text, no glyph starts at all, sizes of nothing.
    let text = "naïve text";
    let mut inside_a_character = fixed_row(text, courier(), 12., 72., 100.);
    inside_a_character.runs = vec![
        RowRun {
            end: 3,
            font: courier(),
            color: INK,
        },
        RowRun {
            end: 999,
            font: font("Courier New", 700., false),
            color: RED,
        },
    ];
    let mut starts_past_the_text = fixed_row(text, courier(), 12., 72., 120.);
    starts_past_the_text.glyphs.push(GlyphStart {
        byte: 500,
        x: 400.,
        scale: 1.,
    });
    let mut no_starts = fixed_row(text, courier(), 12., 72., 140.);
    no_starts.glyphs.clear();
    let mut no_size = fixed_row(text, courier(), 0., 72., 160.);
    no_size.width = 0.;
    let mut short_run = fixed_row(text, courier(), 12., 72., 180.);
    short_run.runs[0].end = 2;
    let (pdf, file) = write(&[page(
        A4,
        vec![
            Item::Row(inside_a_character),
            Item::Row(starts_past_the_text),
            Item::Row(no_starts),
            Item::Row(no_size),
            Item::Row(short_run),
            Item::Stroke(StrokeItem {
                points: vec![(10., 10.)],
                width: 2.,
                color: INK,
            }),
            Item::Label(Label {
                text: String::new(),
                font: courier(),
                size: 10.,
                color: INK,
                x: 10.,
                baseline: 10.,
                anchor: Anchor::End,
            }),
            Item::Rect(RectItem {
                x: 10.,
                y: 10.,
                width: 4.,
                height: 4.,
                radius: 400.,
                fill: Some(INK),
                border: Some((9., RED)),
            }),
        ],
    )]);
    assert_eq!(pdf.report.rows, 5);
    let found = file.page_text(0);
    assert!(found.matches("text").count() >= 4, "{found:?}");
    // Without glyph starts a row stands where CoreText puts it, from its x.
    let (_, file) = write(&[page(
        A4,
        vec![Item::Row({
            let mut row = fixed_row(text, courier(), 12., 72., 140.);
            row.glyphs.clear();
            row
        })],
    )]);
    let (x, ..) = file.find(0, "text").expect("the word");
    assert!(close(x, 72. + 6. * 1229. / 2048. * 12., 0.05), "{x}");
}

#[test]
fn the_report_is_one_line() {
    let report = Report {
        pages: 12,
        rows: 5214,
        rows_off: 0,
        fonts: vec![
            ".SFNS-Regular".to_string(),
            "Charter-Bold".to_string(),
            "Charter-Roman".to_string(),
        ],
        missing_families: Vec::new(),
        images: 3,
        images_unreadable: 0,
    };
    assert_eq!(
        report.line(A4),
        "12 pages, 595.28 x 841.89 pt, 5214 rows, 0 rows off, 3 images, \
         fonts: .SFNS-Regular, Charter-Bold, Charter-Roman"
    );
    assert_eq!(report.missing_line(), None);
    let one = Report {
        pages: 1,
        rows: 1,
        rows_off: 1,
        fonts: Vec::new(),
        missing_families: vec!["Sabon".to_string(), "Scala".to_string()],
        images: 1,
        images_unreadable: 2,
    };
    assert_eq!(
        one.line((612., 792.)),
        "1 page, 612.00 x 792.00 pt, 1 row, 1 row off, 1 image (2 unreadable), fonts: none"
    );
    assert_eq!(
        one.missing_line().as_deref(),
        Some("missing fonts: Sabon, Scala")
    );
}
