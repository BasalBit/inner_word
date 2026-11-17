//! A PDF read back, for tests and for looking at what was made.
//!
//! Page count and page sizes come from CoreGraphics (`CGPDFDocument`, plain C
//! calls); the text of a page and where a word stands come from PDFKit, which is
//! what Preview reads a PDF with. A page can also be drawn into a PNG, which is
//! how a worker without a screen looks at a page.
//!
//! Everything here may be called from any thread: each Objective-C call runs
//! inside an autorelease pool of its own.

// `objc`'s macros ask for a cfg this crate does not have.
#![allow(unexpected_cfgs)]

use std::{ffi::c_void, ptr};

use core_foundation::{
    base::{CFRelease, TCFType},
    data::{CFData, CFDataCreateMutable, CFDataRef, CFMutableDataRef},
    dictionary::CFDictionaryRef,
    string::{CFString, CFStringRef},
};
use core_graphics::{
    base::CGFloat,
    geometry::{CGAffineTransform, CGPoint, CGRect, CGSize},
};
use objc::{
    msg_send,
    rc::autoreleasepool,
    runtime::{Class, Object},
    sel, sel_impl,
};

use crate::system::{object, string};

type RawContext = *mut core_graphics::sys::CGContext;

/// `kCGPDFMediaBox`.
const MEDIA_BOX: i32 = 0;
/// `kCGImageAlphaPremultipliedLast`.
const ALPHA_PREMULTIPLIED_LAST: u32 = 1;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGDataProviderCreateWithCFData(data: CFDataRef) -> *mut c_void;
    fn CGDataProviderRelease(provider: *mut c_void);
    fn CGPDFDocumentCreateWithProvider(provider: *mut c_void) -> *mut c_void;
    fn CGPDFDocumentRelease(document: *mut c_void);
    fn CGPDFDocumentGetNumberOfPages(document: *mut c_void) -> usize;
    fn CGPDFDocumentGetPage(document: *mut c_void, number: usize) -> *mut c_void;
    fn CGPDFPageGetBoxRect(page: *mut c_void, kind: i32) -> CGRect;
    fn CGColorSpaceCreateWithName(name: CFStringRef) -> *mut c_void;
    fn CGColorSpaceRelease(space: *mut c_void);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        bitmap_info: u32,
    ) -> RawContext;
    fn CGBitmapContextCreateImage(context: RawContext) -> *mut c_void;
    fn CGBitmapContextGetData(context: RawContext) -> *mut c_void;
    fn CGBitmapContextGetBytesPerRow(context: RawContext) -> usize;
    fn CGContextRelease(context: RawContext);
    fn CGContextSetRGBFillColor(
        context: RawContext,
        red: CGFloat,
        green: CGFloat,
        blue: CGFloat,
        alpha: CGFloat,
    );
    fn CGContextFillRect(context: RawContext, rect: CGRect);
    fn CGContextConcatCTM(context: RawContext, transform: CGAffineTransform);
    fn CGContextDrawPDFPage(context: RawContext, page: *mut c_void);
    fn CGImageRelease(image: *mut c_void);
    static kCGColorSpaceSRGB: CFStringRef;
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageDestinationCreateWithData(
        data: CFMutableDataRef,
        kind: CFStringRef,
        count: usize,
        options: CFDictionaryRef,
    ) -> *mut c_void;
    fn CGImageDestinationAddImage(
        destination: *mut c_void,
        image: *mut c_void,
        properties: CFDictionaryRef,
    );
    fn CGImageDestinationFinalize(destination: *mut c_void) -> bool;
}

// PDFKit's classes are looked up by name at run time; the framework has to be
// part of the program for them to be there.
#[link(name = "PDFKit", kind = "framework")]
unsafe extern "C" {}

/// `NSRange`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Range {
    location: usize,
    length: usize,
}

// SAFETY: the Objective-C type encoding of `NSRange` on 64-bit macOS.
unsafe impl objc::Encode for Range {
    fn encode() -> objc::Encoding {
        unsafe { objc::Encoding::from_str("{_NSRange=QQ}") }
    }
}

/// A PDF that was opened to be looked at.
///
/// Pages are numbered from 0.
pub struct PdfFile {
    /// The bytes; both documents below read from them.
    _data: CFData,
    /// A `CGPDFDocumentRef`.
    document: *mut c_void,
    /// A `PDFDocument *`.
    kit: *mut Object,
}

impl PdfFile {
    /// Opens the bytes of a PDF file. `None` when they are no PDF, or one
    /// without a page.
    pub fn open(bytes: &[u8]) -> Option<PdfFile> {
        let data = CFData::from_buffer(bytes);
        // SAFETY: valid CoreFoundation objects; the provider is released here
        // and the document is owned by the `PdfFile`.
        let document = unsafe {
            let provider = CGDataProviderCreateWithCFData(data.as_concrete_TypeRef());
            if provider.is_null() {
                return None;
            }
            let document = CGPDFDocumentCreateWithProvider(provider);
            CGDataProviderRelease(provider);
            document
        };
        if document.is_null() {
            return None;
        }
        // SAFETY: a document that was just made.
        if unsafe { CGPDFDocumentGetNumberOfPages(document) } == 0 {
            // SAFETY: the reference made above.
            unsafe { CGPDFDocumentRelease(document) };
            return None;
        }
        let kit = autoreleasepool(|| {
            let Some(class) = Class::get("PDFDocument") else {
                return ptr::null_mut();
            };
            // SAFETY: `initWithData:` takes an NSData, which a CFData is, and
            // returns an object we own, or nil.
            unsafe {
                let allocated: *mut Object = msg_send![class, alloc];
                let document: *mut Object =
                    msg_send![allocated, initWithData: data.as_concrete_TypeRef() as *mut Object];
                document
            }
        });
        if kit.is_null() {
            // SAFETY: the reference made above.
            unsafe { CGPDFDocumentRelease(document) };
            return None;
        }
        Some(PdfFile {
            _data: data,
            document,
            kit,
        })
    }

    /// The number of pages.
    pub fn page_count(&self) -> usize {
        // SAFETY: the document this file owns.
        unsafe { CGPDFDocumentGetNumberOfPages(self.document) }
    }

    /// The size of page `page` in points: its media box. `(0, 0)` for a page the
    /// file does not have.
    pub fn page_size(&self, page: usize) -> (f32, f32) {
        let media_box = self.media_box(page);
        (media_box.size.width as f32, media_box.size.height as f32)
    }

    fn media_box(&self, page: usize) -> CGRect {
        let empty = CGRect::new(&CGPoint::new(0., 0.), &CGSize::new(0., 0.));
        if page >= self.page_count() {
            return empty;
        }
        // SAFETY: the document this file owns; CoreGraphics numbers pages
        // from 1, and a page it hands out belongs to the document.
        unsafe {
            let page = CGPDFDocumentGetPage(self.document, page + 1);
            if page.is_null() {
                empty
            } else {
                CGPDFPageGetBoxRect(page, MEDIA_BOX)
            }
        }
    }

    /// The `PDFPage *` of a page, or nil.
    fn kit_page(&self, page: usize) -> *mut Object {
        if page >= self.page_count() {
            return ptr::null_mut();
        }
        // SAFETY: an index inside the document.
        unsafe { msg_send![self.kit, pageAtIndex: page] }
    }

    /// The text of page `page` as PDFKit reads it (what Preview selects and
    /// copies): rows from top to bottom, one per line. Empty for a page without
    /// text and for a page the file does not have.
    ///
    /// The text of a PDF this crate wrote is what CoreGraphics maps the drawn
    /// glyphs back to. That is the text that was set, with two kinds of
    /// exceptions a comparison has to know: a ligature comes back as its own
    /// character ("ﬃ", U+FB03), and a glyph a font has for several characters
    /// as one of them (日 from a CJK fallback font as the Kangxi radical
    /// U+2F47). Unicode's compatibility folding (NFKC) undoes both.
    pub fn page_text(&self, page: usize) -> String {
        autoreleasepool(|| {
            let page = self.kit_page(page);
            if page.is_null() {
                return String::new();
            }
            // SAFETY: `string` returns an NSString or nil.
            let text: *mut Object = unsafe { msg_send![page, string] };
            string_of(text)
        })
    }

    /// Where the first occurrence of `needle` stands on page `page`, from the top
    /// left corner of the page: `(x, y, width, height)` of the box PDFKit selects
    /// for it. The box is as high as the font's ascent and descent together, so
    /// the baseline of the text is `y + height` minus the font's descent.
    /// `None` when the page does not hold the text.
    pub fn find(&self, page: usize, needle: &str) -> Option<(f32, f32, f32, f32)> {
        if needle.is_empty() {
            return None;
        }
        let text = self.page_text(page);
        let at = text.find(needle)?;
        let range = Range {
            location: text[..at].encode_utf16().count(),
            length: needle.encode_utf16().count(),
        };
        let page_height = self.media_box(page).size.height;
        autoreleasepool(|| {
            let page = self.kit_page(page);
            if page.is_null() {
                return None;
            }
            // SAFETY: a range inside the page's string; `selectionForRange:`
            // returns a PDFSelection or nil, `boundsForPage:` an NSRect, which
            // is a CGRect.
            let bounds: CGRect = unsafe {
                let selection: *mut Object = msg_send![page, selectionForRange: range];
                if selection.is_null() {
                    return None;
                }
                msg_send![selection, boundsForPage: page]
            };
            Some((
                bounds.origin.x as f32,
                (page_height - bounds.origin.y - bounds.size.height) as f32,
                bounds.size.width as f32,
                bounds.size.height as f32,
            ))
        })
    }

    /// Page `page` drawn on white as a PNG file, `scale` pixels to the point (2
    /// is what a Retina display shows at 100 %). `None` for a page the file does
    /// not have, or when the picture cannot be made.
    pub fn page_png(&self, page: usize, scale: f32) -> Option<Vec<u8>> {
        self.render(page, scale)?.png()
    }

    /// Page `page` drawn on white as pixels, `scale` pixels to the point: the
    /// width and the height in pixels, and four bytes for each pixel (red, green,
    /// blue, alpha) in rows from the top. `None` for a page the file does not
    /// have.
    pub fn page_pixels(&self, page: usize, scale: f32) -> Option<(usize, usize, Vec<u8>)> {
        let bitmap = self.render(page, scale)?;
        // SAFETY: the context's own buffer, `height` rows of `bytes_per_row`
        // bytes, alive as long as the bitmap.
        unsafe {
            let data = CGBitmapContextGetData(bitmap.context).cast::<u8>();
            let stride = CGBitmapContextGetBytesPerRow(bitmap.context);
            if data.is_null() || stride < bitmap.width * 4 {
                return None;
            }
            let mut pixels = Vec::with_capacity(bitmap.width * bitmap.height * 4);
            for row in 0..bitmap.height {
                let line = std::slice::from_raw_parts(data.add(row * stride), bitmap.width * 4);
                pixels.extend_from_slice(line);
            }
            Some((bitmap.width, bitmap.height, pixels))
        }
    }

    /// Draws a page into a bitmap, on white.
    fn render(&self, page: usize, scale: f32) -> Option<Bitmap> {
        if page >= self.page_count() || scale.is_nan() || scale <= 0. {
            return None;
        }
        let media_box = self.media_box(page);
        let scale = f64::from(scale);
        let width = (media_box.size.width * scale).ceil().max(1.) as usize;
        let height = (media_box.size.height * scale).ceil().max(1.) as usize;
        // SAFETY: plain CoreGraphics calls; the colour space is released here,
        // the context is owned by the `Bitmap`, the page belongs to the document.
        unsafe {
            let pdf_page = CGPDFDocumentGetPage(self.document, page + 1);
            if pdf_page.is_null() {
                return None;
            }
            let bitmap = Bitmap::new(width, height, ptr::null_mut())?;
            CGContextSetRGBFillColor(bitmap.context, 1., 1., 1., 1.);
            CGContextFillRect(
                bitmap.context,
                CGRect::new(
                    &CGPoint::new(0., 0.),
                    &CGSize::new(width as f64, height as f64),
                ),
            );
            CGContextConcatCTM(
                bitmap.context,
                CGAffineTransform::new(
                    scale,
                    0.,
                    0.,
                    scale,
                    -media_box.origin.x * scale,
                    -media_box.origin.y * scale,
                ),
            );
            CGContextDrawPDFPage(bitmap.context, pdf_page);
            Some(bitmap)
        }
    }

    /// The title and the creator the PDF says it has (its document
    /// information), each empty when it names none.
    pub fn title_and_creator(&self) -> (String, String) {
        autoreleasepool(|| {
            // SAFETY: `documentAttributes` returns an NSDictionary or nil;
            // its keys are the strings PDFKit documents ("Title", "Creator").
            unsafe {
                let attributes: *mut Object = msg_send![self.kit, documentAttributes];
                if attributes.is_null() {
                    return (String::new(), String::new());
                }
                let read = |key: &str| {
                    let key = string(key);
                    let value: *mut Object = msg_send![attributes, objectForKey: object(&key)];
                    string_of(value)
                };
                (read("Title"), read("Creator"))
            }
        })
    }
}

impl Drop for PdfFile {
    fn drop(&mut self) {
        // SAFETY: the two references this file owns.
        unsafe {
            CGPDFDocumentRelease(self.document);
            autoreleasepool(|| {
                let _: () = msg_send![self.kit, release];
            });
        }
    }
}

/// Pixels to draw into: eight bits each of red, green, blue and alpha, sRGB.
struct Bitmap {
    context: RawContext,
    width: usize,
    height: usize,
}

impl Bitmap {
    /// A bitmap of that size. With `pixels` null it has a buffer of its own;
    /// otherwise it draws into (and reads from) `pixels`, which must be
    /// `width * height * 4` bytes and outlive it.
    unsafe fn new(width: usize, height: usize, pixels: *mut c_void) -> Option<Bitmap> {
        // SAFETY: a named colour space, released once the context holds it; the
        // caller's promise about `pixels`.
        unsafe {
            let space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
            if space.is_null() {
                return None;
            }
            let stride = if pixels.is_null() { 0 } else { width * 4 };
            let context = CGBitmapContextCreate(
                pixels,
                width,
                height,
                8,
                stride,
                space,
                ALPHA_PREMULTIPLIED_LAST,
            );
            CGColorSpaceRelease(space);
            (!context.is_null()).then_some(Bitmap {
                context,
                width,
                height,
            })
        }
    }

    /// The bitmap as a PNG file.
    fn png(&self) -> Option<Vec<u8>> {
        self.file("public.png")
    }

    /// The bitmap as a picture file of a kind ImageIO names (`public.png`,
    /// `public.jpeg`).
    fn file(&self, kind: &'static str) -> Option<Vec<u8>> {
        // SAFETY: plain CoreGraphics and ImageIO calls; what is made here is
        // released here.
        unsafe {
            let image = CGBitmapContextCreateImage(self.context);
            if image.is_null() {
                return None;
            }
            let data = CFDataCreateMutable(ptr::null(), 0);
            let file = CFData::wrap_under_create_rule(data as CFDataRef);
            let kind = CFString::from_static_string(kind);
            let destination =
                CGImageDestinationCreateWithData(data, kind.as_concrete_TypeRef(), 1, ptr::null());
            if destination.is_null() {
                CGImageRelease(image);
                return None;
            }
            CGImageDestinationAddImage(destination, image, ptr::null());
            let written = CGImageDestinationFinalize(destination);
            CFRelease(destination as *const c_void);
            CGImageRelease(image);
            written.then(|| file.bytes().to_vec())
        }
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        // SAFETY: the context this bitmap owns.
        unsafe { CGContextRelease(self.context) };
    }
}

/// A picture file made of pixels (four bytes each: red, green, blue, alpha; rows
/// from the top), for tests that need a real JPEG or PNG.
#[cfg(test)]
pub(crate) fn picture_file(width: usize, height: usize, pixels: &[u8], jpeg: bool) -> Vec<u8> {
    assert_eq!(pixels.len(), width * height * 4);
    let mut pixels = pixels.to_vec();
    // SAFETY: the buffer is as large as the bitmap and outlives it.
    let bitmap = unsafe { Bitmap::new(width, height, pixels.as_mut_ptr().cast()) }
        .expect("a bitmap of the pixels");
    bitmap
        .file(if jpeg { "public.jpeg" } else { "public.png" })
        .expect("the picture as a file")
}

/// The text of an `NSString *`; empty for nil.
fn string_of(string: *mut Object) -> String {
    if string.is_null() {
        return String::new();
    }
    // SAFETY: an NSString; `UTF8String` returns a C string that lives as long
    // as the autorelease pool the caller is in.
    unsafe {
        let bytes: *const std::os::raw::c_char = msg_send![string, UTF8String];
        if bytes.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(bytes)
                .to_string_lossy()
                .into_owned()
        }
    }
}
