//! Pixels and PDFs through CoreGraphics: a snapshot's `NSImage` as BGRA bytes, and the
//! pages of a PDF WebKit wrote.

use std::ffi::c_void;

use objc::{msg_send, sel, sel_impl};

use crate::{
    Bitmap,
    ns::{Id, NIL},
};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CGRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGImageGetWidth(image: *const c_void) -> usize;
    fn CGImageGetHeight(image: *const c_void) -> usize;
    fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
    fn CGColorSpaceRelease(space: *mut c_void);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        bitmap_info: u32,
    ) -> *mut c_void;
    fn CGContextDrawImage(context: *mut c_void, rect: CGRect, image: *const c_void);
    fn CGContextRelease(context: *mut c_void);
    fn CGDataProviderCreateWithCFData(data: *const c_void) -> *mut c_void;
    fn CGDataProviderRelease(provider: *mut c_void);
    fn CGPDFDocumentCreateWithProvider(provider: *mut c_void) -> *mut c_void;
    fn CGPDFDocumentRelease(document: *mut c_void);
    fn CGPDFDocumentGetNumberOfPages(document: *mut c_void) -> usize;
    fn CGPDFDocumentGetPage(document: *mut c_void, number: usize) -> *mut c_void;
    fn CGPDFPageGetBoxRect(page: *mut c_void, box_kind: i32) -> CGRect;
}

/// `kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little`: B, G, R, A in memory.
const BGRA_PREMULTIPLIED: u32 = 2 | (2 << 12);

/// The pixels of an `NSImage` (a snapshot), as BGRA bytes at the size of its bitmap
/// (device pixels), alpha not premultiplied. `None` for an image without pixels.
pub(crate) unsafe fn bitmap_of(image: Id) -> Option<Bitmap> {
    if image.is_null() {
        return None;
    }
    // SAFETY: AppKit's and CoreGraphics' own; everything made here is released here.
    unsafe {
        let cg_image: *const c_void = msg_send![image,
            CGImageForProposedRect: std::ptr::null_mut::<c_void>()
            context: NIL
            hints: NIL];
        if cg_image.is_null() {
            return None;
        }
        let width = CGImageGetWidth(cg_image);
        let height = CGImageGetHeight(cg_image);
        if width == 0 || height == 0 {
            return None;
        }
        let mut bgra = vec![0u8; width * height * 4];
        let space = CGColorSpaceCreateDeviceRGB();
        let context = CGBitmapContextCreate(
            bgra.as_mut_ptr().cast(),
            width,
            height,
            8,
            width * 4,
            space,
            BGRA_PREMULTIPLIED,
        );
        CGColorSpaceRelease(space);
        if context.is_null() {
            return None;
        }
        CGContextDrawImage(
            context,
            CGRect {
                x: 0.,
                y: 0.,
                width: width as f64,
                height: height as f64,
            },
            cg_image,
        );
        CGContextRelease(context);
        unpremultiply(&mut bgra);
        Some(Bitmap {
            width: width as u32,
            height: height as u32,
            bgra,
        })
    }
}

/// Straight alpha from premultiplied (a page is opaque nearly everywhere, where the
/// two are the same).
fn unpremultiply(bgra: &mut [u8]) {
    for pixel in bgra.chunks_exact_mut(4) {
        let alpha = pixel[3] as u32;
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((*channel as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
}

/// The number of pages of a PDF and the size of its first page in points; `None` for
/// bytes that are no PDF.
pub(crate) fn pdf_pages(bytes: &[u8]) -> Option<(usize, (f64, f64))> {
    if !bytes.starts_with(b"%PDF") {
        return None;
    }
    // SAFETY: CoreFoundation's and CoreGraphics' own; everything made here is
    // released here.
    unsafe {
        let data: Id = msg_send![objc::class!(NSData),
            dataWithBytes: bytes.as_ptr() as *const c_void
            length: bytes.len()];
        if data.is_null() {
            return None;
        }
        let provider = CGDataProviderCreateWithCFData(data as *const c_void);
        if provider.is_null() {
            return None;
        }
        let document = CGPDFDocumentCreateWithProvider(provider);
        CGDataProviderRelease(provider);
        if document.is_null() {
            return None;
        }
        let pages = CGPDFDocumentGetNumberOfPages(document);
        let size = if pages > 0 {
            let page = CGPDFDocumentGetPage(document, 1);
            if page.is_null() {
                (0., 0.)
            } else {
                // kCGPDFMediaBox
                let rect = CGPDFPageGetBoxRect(page, 0);
                (rect.width, rect.height)
            }
        } else {
            (0., 0.)
        };
        CGPDFDocumentRelease(document);
        (pages > 0).then_some((pages, size))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premultiplied_pixels_come_out_straight() {
        let mut pixels = vec![10, 20, 30, 255, 0, 0, 0, 0, 64, 32, 0, 128];
        unpremultiply(&mut pixels);
        assert_eq!(pixels, vec![10, 20, 30, 255, 0, 0, 0, 0, 128, 64, 0, 128]);
    }

    #[test]
    fn what_is_no_pdf_has_no_pages() {
        assert_eq!(pdf_pages(b"no pdf at all"), None);
        assert_eq!(pdf_pages(b""), None);
    }
}
