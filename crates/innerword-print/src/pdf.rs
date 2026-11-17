//! The PDF writer: [`Page`]s in, a PDF with real text out.
//!
//! A [`PdfWriter`] is a CoreGraphics PDF context that writes into memory. Every
//! page gets its own media box; items are drawn in order, in the PDF's own
//! coordinates (origin at the bottom left, `y = page height - y from the top`), so
//! no context is flipped and text and pictures stand the right way up by
//! themselves.
//!
//! * Rectangles are rounded-rectangle paths, filled, then bordered inside their
//!   edge. Strokes are polylines with round joins and caps.
//! * Rows of text and labels are CoreText glyphs (see the module `text`).
//! * A picture is decoded by ImageIO once per key and drawn where the item says. A
//!   JPEG's own compressed data goes into the PDF as it is; nothing is decoded and
//!   kept (a 4096-pixel picture would be 64 MB of pixels).
//!
//! Colours are sRGB. A page has no transparency: [`crate::Color`] has no alpha.

use std::{
    collections::{BTreeSet, HashMap},
    ffi::c_void,
    fmt, ptr,
    sync::Arc,
};

use core_foundation::{
    base::{CFRelease, CFType, TCFType, kCFAllocatorDefault},
    boolean::CFBoolean,
    data::{CFData, CFDataCreateMutable, CFDataRef, CFMutableDataRef},
    dictionary::{CFDictionary, CFDictionaryRef},
    string::{CFString, CFStringRef},
};
use core_graphics::{
    base::CGFloat,
    color::{CGColor, SysCGColorRef},
    context::{CGContext, CGLineCap, CGLineJoin},
    geometry::{CGPoint, CGRect, CGSize},
};
use core_text::font::CTFont;

use crate::{
    fonts::Fonts,
    page::{Color, ImageItem, Item, Page, RectItem, StrokeItem},
};

type RawContext = *mut core_graphics::sys::CGContext;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGDataConsumerCreateWithCFData(data: CFMutableDataRef) -> *mut c_void;
    fn CGDataConsumerRelease(consumer: *mut c_void);
    fn CGPDFContextCreate(
        consumer: *mut c_void,
        media_box: *const CGRect,
        auxiliary_info: CFDictionaryRef,
    ) -> RawContext;
    fn CGPDFContextBeginPage(context: RawContext, page_info: CFDictionaryRef);
    fn CGPDFContextEndPage(context: RawContext);
    fn CGPDFContextClose(context: RawContext);
    fn CGContextRelease(context: RawContext);
    fn CGContextAddArcToPoint(
        context: RawContext,
        x1: CGFloat,
        y1: CGFloat,
        x2: CGFloat,
        y2: CGFloat,
        radius: CGFloat,
    );
    fn CGContextSetStrokeColorWithColor(context: RawContext, color: SysCGColorRef);
    fn CGContextDrawImage(context: RawContext, rect: CGRect, image: *mut c_void);
    fn CGColorCreateSRGB(
        red: CGFloat,
        green: CGFloat,
        blue: CGFloat,
        alpha: CGFloat,
    ) -> SysCGColorRef;
    fn CGImageRelease(image: *mut c_void);
    static kCGPDFContextMediaBox: CFStringRef;
    static kCGPDFContextTitle: CFStringRef;
    static kCGPDFContextCreator: CFStringRef;
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageSourceCreateWithData(data: CFDataRef, options: CFDictionaryRef) -> *mut c_void;
    fn CGImageSourceGetCount(source: *mut c_void) -> usize;
    fn CGImageSourceCreateImageAtIndex(
        source: *mut c_void,
        index: usize,
        options: CFDictionaryRef,
    ) -> *mut c_void;
    static kCGImageSourceShouldCache: CFStringRef;
}

/// What a print job is called and who made it. Both go into the PDF's document
/// information.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobInfo {
    /// The title of the document.
    pub title: String,
    /// The program that made the PDF, with its version: `"Inner Word 0.6.0"`.
    pub creator: String,
}

/// What a writer did, for whoever wants to know whether the PDF is the layout's
/// picture: counts, the fonts that were drawn with, and what could not be had.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// The number of pages.
    pub pages: usize,
    /// The number of rows of text ([`crate::TextRow`]) on them.
    pub rows: usize,
    /// Rows CoreText sets at other advances than the layout measured: another
    /// face than the layout's was resolved for them. Judged inside the row, pair
    /// of clusters by pair, leaving out what depends on the rest of the hard line
    /// the row was cut from (a tab, the row's first and last cluster) and what
    /// the layout widens or shrinks itself (spaces, inline code at its edges); a
    /// row off by more than the larger of half a point and half a percent of its
    /// width is counted. A row too short for that is judged by its width
    /// ([`crate::TextRow::width`]), unless it holds a tab.
    pub rows_off: usize,
    /// The PostScript names of the fonts that were drawn with, sorted.
    pub fonts: Vec<String>,
    /// The families that were asked for and are not installed, sorted.
    pub missing_families: Vec<String>,
    /// The number of distinct pictures that were put into the PDF.
    pub images: usize,
    /// The number of distinct pictures whose bytes are no picture: nothing was
    /// drawn for them.
    pub images_unreadable: usize,
}

impl Report {
    /// The report as one line, for a log: `12 pages, 595.28 x 841.89 pt, 5214
    /// rows, 0 rows off, 3 images, fonts: .SFNS-Regular, Charter-Bold,
    /// Charter-Roman`. `paper` is the size of the sheets in points.
    pub fn line(&self, paper: (f32, f32)) -> String {
        let counted = |count: usize, one: &str, many: &str| {
            format!("{count} {}", if count == 1 { one } else { many })
        };
        let mut line = format!(
            "{}, {:.2} x {:.2} pt, {}, {}, {}",
            counted(self.pages, "page", "pages"),
            paper.0,
            paper.1,
            counted(self.rows, "row", "rows"),
            counted(self.rows_off, "row off", "rows off"),
            counted(self.images, "image", "images"),
        );
        if self.images_unreadable > 0 {
            line.push_str(&format!(" ({} unreadable)", self.images_unreadable));
        }
        line.push_str(", fonts: ");
        if self.fonts.is_empty() {
            line.push_str("none");
        } else {
            line.push_str(&self.fonts.join(", "));
        }
        line
    }

    /// The families that are not installed as a line for a log (`missing fonts:
    /// Garamond Premier, Sabon`), or `None` when every family was there.
    pub fn missing_line(&self) -> Option<String> {
        (!self.missing_families.is_empty())
            .then(|| format!("missing fonts: {}", self.missing_families.join(", ")))
    }
}

/// A decoded picture: a `CGImage` that still holds the file's own bytes and no
/// pixels.
struct Picture(*mut c_void);

impl Drop for Picture {
    fn drop(&mut self) {
        // SAFETY: an image this struct owns one reference to.
        unsafe { CGImageRelease(self.0) };
    }
}

/// The PDF context and the raw pointer to it.
pub(crate) struct Context {
    pub(crate) raw: RawContext,
    pub(crate) wrapped: CGContext,
}

/// Makes a PDF out of [`Page`]s. Add the pages in order, then [`finish`].
///
/// Not `Send`: it holds CoreGraphics and CoreText objects.
///
/// [`finish`]: PdfWriter::finish
pub struct PdfWriter {
    /// What the context writes into.
    data: CFData,
    /// `None` when CoreGraphics could not make a PDF context: then nothing is
    /// written and the finished PDF is empty.
    pub(crate) context: Option<Context>,
    pub(crate) fonts: Fonts,
    /// One picture per key; `None` for bytes that are no picture.
    images: HashMap<[u8; 16], Option<Picture>>,
    colors: HashMap<[u32; 3], CGColor>,
    /// The fill colour the context has now.
    fill: Option<[u32; 3]>,
    /// The PostScript names of the fonts glyphs were drawn with.
    pub(crate) drawn_fonts: BTreeSet<String>,
    /// The fonts whose names are in `drawn_fonts`, by address; each is held, so
    /// an address cannot come to mean another font.
    pub(crate) named_fonts: HashMap<usize, CTFont>,
    pub(crate) report: Report,
}

impl PdfWriter {
    /// A writer for a new PDF.
    pub fn new(info: &JobInfo) -> PdfWriter {
        // SAFETY: plain CoreFoundation and CoreGraphics calls with valid
        // arguments; every object made here is released here or owned by the
        // writer.
        let (data, context) = unsafe {
            let data = CFDataCreateMutable(kCFAllocatorDefault, 0);
            let consumer = CGDataConsumerCreateWithCFData(data);
            let auxiliary: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[
                (
                    CFString::wrap_under_get_rule(kCGPDFContextTitle),
                    CFString::new(&info.title).as_CFType(),
                ),
                (
                    CFString::wrap_under_get_rule(kCGPDFContextCreator),
                    CFString::new(&info.creator).as_CFType(),
                ),
            ]);
            let raw = if consumer.is_null() {
                ptr::null_mut()
            } else {
                // No default media box: every page brings its own.
                let raw = CGPDFContextCreate(
                    consumer,
                    ptr::null(),
                    auxiliary.as_concrete_TypeRef() as CFDictionaryRef,
                );
                CGDataConsumerRelease(consumer);
                raw
            };
            let context = (!raw.is_null()).then(|| {
                // The wrapper takes a reference of its own; ours goes.
                let wrapped = CGContext::from_existing_context_ptr(raw);
                CGContextRelease(raw);
                Context { raw, wrapped }
            });
            (CFData::wrap_under_create_rule(data as CFDataRef), context)
        };
        PdfWriter {
            data,
            context,
            fonts: Fonts::new(),
            images: HashMap::new(),
            colors: HashMap::new(),
            fill: None,
            drawn_fonts: BTreeSet::new(),
            named_fonts: HashMap::new(),
            report: Report::default(),
        }
    }

    /// Adds a page. Its media box is the page's own size.
    pub fn add_page(&mut self, page: &Page) {
        let Some(raw) = self.context.as_ref().map(|context| context.raw) else {
            return;
        };
        let media_box = CGRect::new(
            &CGPoint::new(0., 0.),
            &CGSize::new(f64::from(page.width), f64::from(page.height)),
        );
        // SAFETY: the bytes of a `CGRect` (four doubles, `repr(C)`), copied
        // into a CFData; a valid context and a valid dictionary.
        unsafe {
            let bytes = std::slice::from_raw_parts(
                (&raw const media_box).cast::<u8>(),
                std::mem::size_of::<CGRect>(),
            );
            let info: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[(
                CFString::wrap_under_get_rule(kCGPDFContextMediaBox),
                CFData::from_buffer(bytes).as_CFType(),
            )]);
            CGPDFContextBeginPage(raw, info.as_concrete_TypeRef() as CFDictionaryRef);
        }
        // A page starts with a graphics state of its own.
        self.fill = None;
        for item in &page.items {
            match item {
                Item::Rect(rect) => self.draw_rect(rect, page.height),
                Item::Stroke(stroke) => self.draw_stroke(stroke, page.height),
                Item::Row(row) => self.draw_row(row, page.height),
                Item::Label(label) => self.draw_label(label, page.height),
                Item::Image(image) => self.draw_image(image, page.height),
            }
        }
        // SAFETY: the page that was begun above.
        unsafe { CGPDFContextEndPage(raw) };
        self.report.pages += 1;
    }

    /// The number of pages added so far.
    pub fn pages(&self) -> usize {
        self.report.pages
    }

    /// Ends the PDF and hands it out with the report.
    ///
    /// A writer that was given no page hands out a PDF without bytes
    /// ([`Pdf::is_empty`]): CoreGraphics would close it with one blank sheet of
    /// its own, which nobody asked to print.
    pub fn finish(mut self) -> Pdf {
        if let Some(context) = self.context.take() {
            // SAFETY: a PDF context that is closed once and then released by
            // its wrapper.
            unsafe { CGPDFContextClose(context.raw) };
            drop(context);
        }
        let mut report = std::mem::take(&mut self.report);
        report.fonts = self.drawn_fonts.iter().cloned().collect();
        report.missing_families = self.fonts.missing();
        let data = if report.pages == 0 {
            CFData::from_buffer(&[])
        } else {
            self.data.clone()
        };
        Pdf { data, report }
    }

    /// The CoreGraphics colour of a page colour.
    fn color(&mut self, color: Color) -> CGColor {
        let key = color_key(color);
        self.colors
            .entry(key)
            .or_insert_with(|| {
                let part = |value: f32| f64::from(value.clamp(0., 1.));
                // SAFETY: returns a colour we own.
                unsafe {
                    CGColor::wrap_under_create_rule(CGColorCreateSRGB(
                        part(color.r),
                        part(color.g),
                        part(color.b),
                        1.,
                    ))
                }
            })
            .clone()
    }

    /// Makes `color` the colour shapes and glyphs are filled with.
    pub(crate) fn set_fill(&mut self, color: Color) {
        let key = color_key(color);
        if self.fill == Some(key) {
            return;
        }
        let color = self.color(color);
        if let Some(context) = &self.context {
            context.wrapped.set_fill_color(&color);
            self.fill = Some(key);
        }
    }

    fn draw_rect(&mut self, rect: &RectItem, page_height: f32) {
        let (width, height) = (f64::from(rect.width), f64::from(rect.height));
        if !(width > 0. && height > 0.) {
            return;
        }
        let x = f64::from(rect.x);
        let y = f64::from(page_height) - f64::from(rect.y) - height;
        let radius = f64::from(rect.radius).clamp(0., width.min(height) / 2.);
        if let Some(fill) = rect.fill {
            self.set_fill(fill);
            if let Some(context) = &self.context {
                rounded_rect(context, x, y, width, height, radius);
                context.wrapped.fill_path();
            }
        }
        if let Some((border, color)) = rect.border {
            let border = f64::from(border);
            if border <= 0. {
                return;
            }
            if border * 2. >= width.min(height) {
                // Nothing is left inside the border: the rectangle is all
                // border.
                self.set_fill(color);
                if let Some(context) = &self.context {
                    rounded_rect(context, x, y, width, height, radius);
                    context.wrapped.fill_path();
                }
                return;
            }
            let color = self.color(color);
            if let Some(context) = &self.context {
                // A stroke is centred on its path: the path lies half the
                // border inside the edge.
                let inset = border / 2.;
                rounded_rect(
                    context,
                    x + inset,
                    y + inset,
                    width - border,
                    height - border,
                    (radius - inset).max(0.),
                );
                // SAFETY: a valid context and a valid colour.
                unsafe {
                    CGContextSetStrokeColorWithColor(context.raw, color.as_concrete_TypeRef());
                }
                context.wrapped.set_line_width(border);
                context.wrapped.set_line_join(CGLineJoin::CGLineJoinMiter);
                context.wrapped.stroke_path();
            }
        }
    }

    fn draw_stroke(&mut self, stroke: &StrokeItem, page_height: f32) {
        if stroke.points.len() < 2 || stroke.width.is_nan() || stroke.width <= 0. {
            return;
        }
        let color = self.color(stroke.color);
        let Some(context) = &self.context else {
            return;
        };
        let at = |(x, y): (f32, f32)| (f64::from(x), f64::from(page_height) - f64::from(y));
        context.wrapped.begin_path();
        let (x, y) = at(stroke.points[0]);
        context.wrapped.move_to_point(x, y);
        for point in &stroke.points[1..] {
            let (x, y) = at(*point);
            context.wrapped.add_line_to_point(x, y);
        }
        // SAFETY: a valid context and a valid colour.
        unsafe { CGContextSetStrokeColorWithColor(context.raw, color.as_concrete_TypeRef()) };
        context.wrapped.set_line_width(f64::from(stroke.width));
        context.wrapped.set_line_cap(CGLineCap::CGLineCapRound);
        context.wrapped.set_line_join(CGLineJoin::CGLineJoinRound);
        context.wrapped.stroke_path();
    }

    fn draw_image(&mut self, item: &ImageItem, page_height: f32) {
        if !self.images.contains_key(&item.key) {
            let picture = decode(&item.bytes);
            match picture {
                Some(_) => self.report.images += 1,
                None => self.report.images_unreadable += 1,
            }
            self.images.insert(item.key, picture);
        }
        let (width, height) = (f64::from(item.width), f64::from(item.height));
        if !(width > 0. && height > 0.) {
            return;
        }
        let (Some(Some(picture)), Some(context)) = (self.images.get(&item.key), &self.context)
        else {
            return;
        };
        let rect = CGRect::new(
            &CGPoint::new(
                f64::from(item.x),
                f64::from(page_height) - f64::from(item.y) - height,
            ),
            &CGSize::new(width, height),
        );
        // SAFETY: a valid context and an image the writer holds.
        unsafe { CGContextDrawImage(context.raw, rect, picture.0) };
    }
}

fn color_key(color: Color) -> [u32; 3] {
    [color.r.to_bits(), color.g.to_bits(), color.b.to_bits()]
}

/// Makes the path of a rectangle with rounded corners the context's path. The
/// radius is no larger than half the shorter side.
fn rounded_rect(context: &Context, x: f64, y: f64, width: f64, height: f64, radius: f64) {
    let c = &context.wrapped;
    c.begin_path();
    if radius <= 0. {
        c.move_to_point(x, y);
        c.add_line_to_point(x + width, y);
        c.add_line_to_point(x + width, y + height);
        c.add_line_to_point(x, y + height);
        c.close_path();
        return;
    }
    let (right, top) = (x + width, y + height);
    c.move_to_point(x + radius, y);
    // SAFETY: a valid context with a current point.
    unsafe {
        CGContextAddArcToPoint(context.raw, right, y, right, top, radius);
        CGContextAddArcToPoint(context.raw, right, top, x, top, radius);
        CGContextAddArcToPoint(context.raw, x, top, x, y, radius);
        CGContextAddArcToPoint(context.raw, x, y, right, y, radius);
    }
    c.close_path();
}

/// The first frame of a picture file, without its pixels: ImageIO reads them
/// from the file's bytes when the picture is drawn. `None` for bytes that are no
/// picture.
fn decode(bytes: &Arc<[u8]>) -> Option<Picture> {
    // The CFData points at the bytes and holds them; nothing is copied.
    let data = CFData::from_arc(Arc::new(bytes.clone()));
    // SAFETY: valid CoreFoundation objects; the source is released here, and
    // the image it made is owned by the `Picture`.
    unsafe {
        let source = CGImageSourceCreateWithData(data.as_concrete_TypeRef(), ptr::null());
        if source.is_null() {
            return None;
        }
        let options: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[(
            CFString::wrap_under_get_rule(kCGImageSourceShouldCache),
            CFBoolean::false_value().as_CFType(),
        )]);
        let image = if CGImageSourceGetCount(source) == 0 {
            ptr::null_mut()
        } else {
            CGImageSourceCreateImageAtIndex(
                source,
                0,
                options.as_concrete_TypeRef() as CFDictionaryRef,
            )
        };
        CFRelease(source as *const c_void);
        (!image.is_null()).then_some(Picture(image))
    }
}

/// A finished PDF.
///
/// It keeps the `CFData` the PDF context wrote into and hands that same object to
/// PDFKit when it is printed (`CFData` is `NSData`), so a book of photographs is
/// in memory once. A clone shares the data. Not `Send`.
#[derive(Clone)]
pub struct Pdf {
    data: CFData,
    /// What the writer did.
    pub report: Report,
}

impl Pdf {
    /// A PDF that was made elsewhere (WebKit's print operation, for a page),
    /// so that it goes to the same print target as one this crate wrote. The
    /// report has `pages`; its other counts are 0 and its lists empty. The
    /// bytes are taken as they are: nothing checks that they are a PDF.
    pub fn from_bytes(bytes: Vec<u8>, pages: usize) -> Pdf {
        Pdf {
            data: CFData::from_buffer(&bytes),
            report: Report {
                pages,
                ..Report::default()
            },
        }
    }

    /// The bytes of the PDF file.
    pub fn bytes(&self) -> &[u8] {
        self.data.bytes()
    }

    /// The size of the PDF file in bytes.
    pub fn len(&self) -> usize {
        usize::try_from(self.data.len()).unwrap_or(0)
    }

    /// Whether there is no PDF: the system could not make one.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The data as the `NSData *` it is.
    pub(crate) fn ns_data(&self) -> *mut c_void {
        self.data.as_concrete_TypeRef() as *mut c_void
    }
}

impl fmt::Debug for Pdf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pdf")
            .field("len", &self.len())
            .field("report", &self.report)
            .finish()
    }
}
