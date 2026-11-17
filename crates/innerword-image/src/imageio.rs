//! ImageIO and the little of CoreGraphics that goes with it, declared as the C it is,
//! and the few safe steps the rest of the crate is written in: read a file's facts,
//! decode it upright at a size, write a picture as a file, copy a file without its
//! location.
//!
//! Every object here follows CoreFoundation's ownership: what a function with `Create`
//! or `Copy` in its name returns is owned by the caller and released when its wrapper is
//! dropped ([`CFType`] does that); what a `Get` returns is not.

use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr;

use core_foundation::base::{CFType, CFTypeRef, TCFType, kCFAllocatorDefault, kCFAllocatorNull};
use core_foundation::boolean::CFBoolean;
use core_foundation::data::{
    CFData, CFDataCreateMutable, CFDataCreateWithBytesNoCopy, CFDataGetBytePtr, CFDataGetLength,
    CFDataRef,
};
use core_foundation::dictionary::{CFDictionary, CFDictionaryGetValue, CFDictionaryRef};
use core_foundation::error::CFErrorRef;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::color_space::kCGColorSpaceSRGB;

/// A CoreFoundation object of a type this crate has no name for (an image source, an
/// image destination, an image, a colour space, a data provider).
type Object = *const c_void;

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageSourceCreateWithData(data: CFDataRef, options: CFDictionaryRef) -> Object;
    fn CGImageSourceGetType(source: Object) -> CFStringRef;
    fn CGImageSourceGetCount(source: Object) -> usize;
    fn CGImageSourceCopyPropertiesAtIndex(
        source: Object,
        index: usize,
        options: CFDictionaryRef,
    ) -> CFDictionaryRef;
    fn CGImageSourceCreateThumbnailAtIndex(
        source: Object,
        index: usize,
        options: CFDictionaryRef,
    ) -> Object;
    fn CGImageDestinationCreateWithData(
        data: *mut c_void,
        kind: CFStringRef,
        count: usize,
        options: CFDictionaryRef,
    ) -> Object;
    fn CGImageDestinationAddImage(destination: Object, image: Object, properties: CFDictionaryRef);
    fn CGImageDestinationCopyImageSource(
        destination: Object,
        source: Object,
        options: CFDictionaryRef,
        error: *mut CFErrorRef,
    ) -> bool;
    fn CGImageDestinationFinalize(destination: Object) -> bool;

    static kCGImagePropertyPixelWidth: CFStringRef;
    static kCGImagePropertyPixelHeight: CFStringRef;
    static kCGImagePropertyOrientation: CFStringRef;
    static kCGImagePropertyDPIWidth: CFStringRef;
    static kCGImagePropertyDPIHeight: CFStringRef;
    static kCGImagePropertyHasAlpha: CFStringRef;
    static kCGImagePropertyGPSDictionary: CFStringRef;
    static kCGImagePropertyGPSLatitude: CFStringRef;
    static kCGImagePropertyGPSLatitudeRef: CFStringRef;
    static kCGImagePropertyGPSLongitude: CFStringRef;
    static kCGImagePropertyGPSLongitudeRef: CFStringRef;

    static kCGImageSourceCreateThumbnailFromImageAlways: CFStringRef;
    static kCGImageSourceCreateThumbnailWithTransform: CFStringRef;
    static kCGImageSourceThumbnailMaxPixelSize: CFStringRef;

    static kCGImageDestinationLossyCompressionQuality: CFStringRef;
    static kCGImageMetadataShouldExcludeGPS: CFStringRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGImageGetWidth(image: Object) -> usize;
    fn CGImageGetHeight(image: Object) -> usize;
    fn CGImageGetAlphaInfo(image: Object) -> u32;
    fn CGImageGetBitmapInfo(image: Object) -> u32;
    fn CGImageGetBitsPerComponent(image: Object) -> usize;
    fn CGImageGetBitsPerPixel(image: Object) -> usize;
    fn CGImageGetBytesPerRow(image: Object) -> usize;
    fn CGImageGetDataProvider(image: Object) -> Object;
    fn CGDataProviderCopyData(provider: Object) -> CFDataRef;
    #[allow(clippy::too_many_arguments)]
    fn CGImageCreate(
        width: usize,
        height: usize,
        bits_per_component: usize,
        bits_per_pixel: usize,
        bytes_per_row: usize,
        space: Object,
        bitmap_info: u32,
        provider: Object,
        decode: *const f64,
        should_interpolate: bool,
        intent: i32,
    ) -> Object;
    fn CGDataProviderCreateWithCFData(data: CFDataRef) -> Object;
    fn CGColorSpaceCreateWithName(name: CFStringRef) -> Object;
}

/// The uniform type identifiers of the kinds of file the crate names.
pub(crate) mod uti {
    pub(crate) const PNG: &str = "public.png";
    pub(crate) const JPEG: &str = "public.jpeg";
    pub(crate) const GIF: &str = "com.compuserve.gif";
    #[cfg(test)]
    pub(crate) const TIFF: &str = "public.tiff";
    #[cfg(test)]
    pub(crate) const HEIC: &str = "public.heic";
    #[cfg(test)]
    pub(crate) const BMP: &str = "com.microsoft.bmp";
}

/// Takes ownership of an object a `Create` or `Copy` function returned; `None` for null.
fn owned(object: Object) -> Option<CFType> {
    if object.is_null() {
        None
    } else {
        // SAFETY: the caller got `object` from a function that follows the create rule,
        // so this wrapper holds the one reference there is to release.
        Some(unsafe { CFType::wrap_under_create_rule(object as CFTypeRef) })
    }
}

/// A dictionary of options or properties, from keys ImageIO exports and values.
fn dictionary(pairs: &[(CFStringRef, CFType)]) -> CFDictionary<CFString, CFType> {
    let pairs: Vec<(CFString, CFType)> = pairs
        .iter()
        .map(|(key, value)| {
            // SAFETY: the keys are constants of ImageIO, alive for as long as the
            // framework is loaded; the wrapper retains for itself.
            (
                unsafe { CFString::wrap_under_get_rule(*key) },
                value.clone(),
            )
        })
        .collect();
    CFDictionary::from_CFType_pairs(&pairs)
}

fn boolean(value: bool) -> CFType {
    if value {
        CFBoolean::true_value().as_CFType()
    } else {
        CFBoolean::false_value().as_CFType()
    }
}

/// The value under `key` in a dictionary ImageIO returned.
fn value(properties: &CFDictionary, key: CFStringRef) -> Option<CFType> {
    // SAFETY: both are live CoreFoundation objects; the value belongs to the dictionary
    // (get rule), so the wrapper retains it.
    unsafe {
        let found = CFDictionaryGetValue(properties.as_concrete_TypeRef(), key as *const c_void);
        (!found.is_null()).then(|| CFType::wrap_under_get_rule(found as CFTypeRef))
    }
}

fn number(properties: &CFDictionary, key: CFStringRef) -> Option<f64> {
    value(properties, key)?.downcast::<CFNumber>()?.to_f64()
}

/// What a file says about its first picture.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Facts {
    /// The kind of file, as a uniform type identifier (`public.png`).
    pub(crate) kind: String,
    /// How many pictures the file holds: more than one for an animation.
    pub(crate) count: usize,
    /// Width and height in pixels as they are stored, before the orientation is applied.
    pub(crate) stored_pixels: (u32, u32),
    /// The EXIF orientation, 1 to 8; 1 when the file names none.
    pub(crate) orientation: u8,
    /// Pixels per inch across, when the file says.
    pub(crate) dpi: Option<f64>,
    /// Whether the picture has transparency, when the file says.
    pub(crate) has_alpha: Option<bool>,
    /// Whether the file carries a position (a GPS dictionary).
    pub(crate) has_location: bool,
}

impl Facts {
    /// Width and height as the picture is meant to be seen: swapped for the four
    /// orientations that lie on their side.
    pub(crate) fn upright_pixels(&self) -> (u32, u32) {
        let (width, height) = self.stored_pixels;
        if (5..=8).contains(&self.orientation) {
            (height, width)
        } else {
            (width, height)
        }
    }
}

/// An image file that ImageIO can read, over bytes that are borrowed and not copied.
pub(crate) struct Source<'a> {
    source: CFType,
    _bytes: PhantomData<&'a [u8]>,
}

impl<'a> Source<'a> {
    /// `None` when ImageIO does not take the bytes for an image.
    pub(crate) fn new(bytes: &'a [u8]) -> Option<Source<'a>> {
        if bytes.is_empty() {
            return None;
        }
        // SAFETY: the data object points into `bytes` without owning them
        // (`kCFAllocatorNull` frees nothing), and `Source` cannot outlive `bytes`.
        // Everything made from the source in this crate is finished (decoded, written)
        // before the source is dropped.
        let data = unsafe {
            let data = CFDataCreateWithBytesNoCopy(
                kCFAllocatorDefault,
                bytes.as_ptr(),
                bytes.len() as isize,
                kCFAllocatorNull,
            );
            if data.is_null() {
                return None;
            }
            CFData::wrap_under_create_rule(data)
        };
        // SAFETY: a live data object and no options.
        let source =
            owned(unsafe { CGImageSourceCreateWithData(data.as_concrete_TypeRef(), ptr::null()) })?;
        Some(Source {
            source,
            _bytes: PhantomData,
        })
    }

    fn object(&self) -> Object {
        self.source.as_CFTypeRef() as Object
    }

    /// What the file says about its first picture; `None` when it holds none that can
    /// be read (a source is made of any bytes; this is where bytes that are no image
    /// are found out).
    pub(crate) fn facts(&self) -> Option<Facts> {
        // SAFETY: a live source. The type is not owned by the caller (get rule).
        let (kind, count) = unsafe {
            let kind = CGImageSourceGetType(self.object());
            if kind.is_null() {
                return None;
            }
            (
                CFString::wrap_under_get_rule(kind).to_string(),
                CGImageSourceGetCount(self.object()),
            )
        };
        if count == 0 {
            return None;
        }
        let properties = self.properties()?;
        let pixels = |key| {
            let size = number(&properties, key)?;
            (size >= 1.0 && size <= f64::from(u32::MAX)).then_some(size as u32)
        };
        // SAFETY (here and below): the keys are constants ImageIO exports.
        let stored_pixels = unsafe {
            (
                pixels(kCGImagePropertyPixelWidth)?,
                pixels(kCGImagePropertyPixelHeight)?,
            )
        };
        let orientation = unsafe { number(&properties, kCGImagePropertyOrientation) }
            .filter(|o| (1.0..=8.0).contains(o))
            .map_or(1, |o| o as u8);
        let dpi = unsafe { number(&properties, kCGImagePropertyDPIWidth) };
        let has_alpha = unsafe { value(&properties, kCGImagePropertyHasAlpha) }
            .and_then(|v| v.downcast::<CFBoolean>())
            .map(bool::from);
        let has_location = unsafe { value(&properties, kCGImagePropertyGPSDictionary) }.is_some();
        Some(Facts {
            kind,
            count,
            stored_pixels,
            orientation,
            dpi,
            has_alpha,
            has_location,
        })
    }

    fn properties(&self) -> Option<CFDictionary> {
        // SAFETY: a live source; the dictionary is owned by the caller (copy rule).
        unsafe {
            let properties = CGImageSourceCopyPropertiesAtIndex(self.object(), 0, ptr::null());
            (!properties.is_null()).then(|| CFDictionary::wrap_under_create_rule(properties))
        }
    }

    /// The first picture decoded: upright (the orientation applied) and at most
    /// `long_edge` pixels on its long edge, in one step.
    pub(crate) fn decode(&self, long_edge: u32) -> Option<Picture> {
        // SAFETY: the keys are constants ImageIO exports.
        let options = unsafe {
            dictionary(&[
                (kCGImageSourceCreateThumbnailFromImageAlways, boolean(true)),
                (kCGImageSourceCreateThumbnailWithTransform, boolean(true)),
                (
                    kCGImageSourceThumbnailMaxPixelSize,
                    CFNumber::from(i64::from(long_edge)).as_CFType(),
                ),
            ])
        };
        // SAFETY: a live source and a live dictionary; the image is owned by the caller.
        owned(unsafe {
            CGImageSourceCreateThumbnailAtIndex(
                self.object(),
                0,
                options.as_concrete_TypeRef() as CFDictionaryRef,
            )
        })
        .map(Picture)
    }

    /// The file again as it is, without re-encoding the picture, and without its
    /// position. `None` when ImageIO cannot do that for this kind of file.
    pub(crate) fn copy_without_location(&self, facts: &Facts) -> Option<Vec<u8>> {
        let (data, destination) = destination(&facts.kind, facts.count)?;
        // SAFETY: the key is a constant ImageIO exports.
        let options = unsafe { dictionary(&[(kCGImageMetadataShouldExcludeGPS, boolean(true))]) };
        let mut error: CFErrorRef = ptr::null_mut();
        // SAFETY: a live destination that nothing was added to, a live source, a live
        // dictionary. The call finishes the destination itself.
        let copied = unsafe {
            CGImageDestinationCopyImageSource(
                destination.as_CFTypeRef() as Object,
                self.object(),
                options.as_concrete_TypeRef() as CFDictionaryRef,
                &mut error,
            )
        };
        // The error, when there is one, is owned by the caller.
        drop(owned(error as Object));
        copied.then(|| bytes_of(&data)).filter(|b| !b.is_empty())
    }
}

/// A decoded picture (a `CGImage`).
pub(crate) struct Picture(CFType);

impl Picture {
    fn object(&self) -> Object {
        self.0.as_CFTypeRef() as Object
    }

    /// Width and height in pixels.
    pub(crate) fn pixels(&self) -> (u32, u32) {
        // SAFETY: a live image.
        unsafe {
            (
                CGImageGetWidth(self.object()) as u32,
                CGImageGetHeight(self.object()) as u32,
            )
        }
    }

    /// Whether anything of the picture is transparent: it has an alpha channel, and at
    /// least one pixel is not fully opaque. (ImageIO decodes some kinds of file, a HEIC
    /// among them, into pixels with an alpha channel whether or not the file has one, so
    /// the channel alone says nothing.) A picture whose pixels cannot be looked at in
    /// the one layout this reads counts as transparent when it has the channel.
    pub(crate) fn has_transparency(&self) -> bool {
        let image = self.object();
        // SAFETY: a live image, here and in the block below.
        let alpha = unsafe { CGImageGetAlphaInfo(image) };
        // kCGImageAlphaNone, NoneSkipLast and NoneSkipFirst (0, 5, 6) have no channel.
        if matches!(alpha, 0 | 5 | 6) {
            return false;
        }
        let (width, height) = self.pixels();
        // SAFETY: the provider belongs to the image (get rule); its data is owned by the
        // caller (copy rule) and holds `bytes_per_row` bytes for each of `height` rows.
        unsafe {
            let eight_bit_rgba = CGImageGetBitsPerComponent(image) == 8
                && CGImageGetBitsPerPixel(image) == 32
                && matches!(alpha, 1..=4);
            let provider = CGImageGetDataProvider(image);
            if !eight_bit_rgba || provider.is_null() {
                return true;
            }
            let Some(data) = owned(CGDataProviderCopyData(provider) as Object) else {
                return true;
            };
            let data = data.as_CFTypeRef() as CFDataRef;
            let start = CFDataGetBytePtr(data);
            let len = CFDataGetLength(data).max(0) as usize;
            let bytes_per_row = CGImageGetBytesPerRow(image);
            let (width, height) = (width as usize, height as usize);
            if start.is_null()
                || bytes_per_row < width * 4
                || len < bytes_per_row * (height - 1) + width * 4
            {
                return true;
            }
            let pixels = std::slice::from_raw_parts(start, len);
            // Where the opacity stands in the four bytes of a pixel: first or last by
            // the alpha info, and the other way round when the pixel is little-endian
            // (kCGBitmapByteOrder32Little).
            let alpha_first = matches!(alpha, 2 | 4);
            let little_endian = CGImageGetBitmapInfo(image) & 0x7000 == 0x2000;
            let at = if alpha_first != little_endian { 0 } else { 3 };
            (0..height).any(|row| {
                let row = &pixels[row * bytes_per_row..row * bytes_per_row + width * 4];
                row.chunks_exact(4).any(|pixel| pixel[at] != 255)
            })
        }
    }

    /// How light the pixel at `x`, `y` is, 0 to 255 (the mean of its colours), for a
    /// test that has to know where something in a picture went. `None` for a picture
    /// that is not stored in four bytes a pixel.
    #[cfg(test)]
    pub(crate) fn lightness_at(&self, x: u32, y: u32) -> Option<u8> {
        let image = self.object();
        // SAFETY: as in `has_transparency`.
        unsafe {
            if CGImageGetBitsPerComponent(image) != 8 || CGImageGetBitsPerPixel(image) != 32 {
                return None;
            }
            let data = owned(CGDataProviderCopyData(CGImageGetDataProvider(image)) as Object)?;
            let data = data.as_CFTypeRef() as CFDataRef;
            let pixels =
                std::slice::from_raw_parts(CFDataGetBytePtr(data), CFDataGetLength(data) as usize);
            let at = y as usize * CGImageGetBytesPerRow(image) + x as usize * 4;
            let pixel = pixels.get(at..at + 4)?;
            // The fourth byte (opacity, or nothing) stands first or last.
            let alpha_first = matches!(CGImageGetAlphaInfo(image), 2 | 4 | 6);
            let little_endian = CGImageGetBitmapInfo(image) & 0x7000 == 0x2000;
            let colours = if alpha_first != little_endian {
                &pixel[1..]
            } else {
                &pixel[..3]
            };
            Some((colours.iter().map(|c| u16::from(*c)).sum::<u16>() / 3) as u8)
        }
    }

    /// A picture of one colour, `width` by `height` pixels, in sRGB. With `alpha` every
    /// pixel has that much opacity (and the picture an alpha channel); without, it is
    /// opaque and has none.
    pub(crate) fn flat(width: u32, height: u32, rgb: [u8; 3], alpha: Option<u8>) -> Picture {
        let pixel = match alpha {
            // Premultiplied, as `from_pixels` takes them.
            Some(alpha) => {
                let scaled = |c: u8| (u16::from(c) * u16::from(alpha) / 255) as u8;
                [scaled(rgb[0]), scaled(rgb[1]), scaled(rgb[2]), alpha]
            }
            None => [rgb[0], rgb[1], rgb[2], 255],
        };
        let pixels: Vec<u8> = pixel
            .iter()
            .copied()
            .cycle()
            .take(width as usize * height as usize * 4)
            .collect();
        Picture::from_pixels(width, height, &pixels, alpha.is_some())
    }

    /// A picture from its pixels in sRGB: four bytes each, red, green, blue and a
    /// fourth, row after row from the top. With `alpha` the fourth byte is the opacity
    /// and the colours are premultiplied by it; without, it is skipped and the picture
    /// has no alpha channel.
    pub(crate) fn from_pixels(width: u32, height: u32, pixels: &[u8], alpha: bool) -> Picture {
        assert!(width > 0 && height > 0, "a picture has at least one pixel");
        assert_eq!(
            pixels.len(),
            width as usize * height as usize * 4,
            "four bytes for every pixel"
        );
        let data = CFData::from_buffer(pixels);
        // kCGImageAlphaPremultipliedLast, or kCGImageAlphaNoneSkipLast.
        let bitmap_info = if alpha { 1 } else { 5 };
        // SAFETY: the data object owns a copy of the pixels and is retained by the
        // provider, the provider and the colour space by the image; 8 bits per
        // component, 32 per pixel and `width * 4` bytes per row is what the buffer is.
        let image = unsafe {
            let provider = owned(CGDataProviderCreateWithCFData(data.as_concrete_TypeRef()))
                .expect("a data provider over a buffer");
            let space = owned(CGColorSpaceCreateWithName(kCGColorSpaceSRGB)).expect("sRGB");
            owned(CGImageCreate(
                width as usize,
                height as usize,
                8,
                32,
                width as usize * 4,
                space.as_CFTypeRef() as Object,
                bitmap_info,
                provider.as_CFTypeRef() as Object,
                ptr::null(),
                false,
                0,
            ))
        };
        Picture(image.expect("an image over a buffer of its size"))
    }
}

/// What a written file is given besides its picture.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Written {
    /// The quality of a lossy file, 0 to 1.
    pub(crate) quality: Option<f64>,
    /// Pixels per inch the file declares.
    pub(crate) dpi: Option<u32>,
    /// The EXIF orientation the file declares (the picture itself is written as it is).
    pub(crate) orientation: Option<u8>,
    /// A position the file declares, as latitude north and longitude east.
    pub(crate) location: Option<(f64, f64)>,
}

/// Writes pictures as one file of the kind `kind` names (several pictures: the frames of
/// an animation). `None` when this Mac's ImageIO cannot write that kind, or fails.
pub(crate) fn write(pictures: &[&Picture], kind: &str, with: &Written) -> Option<Vec<u8>> {
    let (data, destination) = destination(kind, pictures.len())?;
    let mut properties: Vec<(CFStringRef, CFType)> = Vec::new();
    // SAFETY: the keys are constants ImageIO exports.
    unsafe {
        if let Some(quality) = with.quality {
            properties.push((
                kCGImageDestinationLossyCompressionQuality,
                CFNumber::from(quality).as_CFType(),
            ));
        }
        if let Some(dpi) = with.dpi {
            let dpi = CFNumber::from(f64::from(dpi)).as_CFType();
            properties.push((kCGImagePropertyDPIWidth, dpi.clone()));
            properties.push((kCGImagePropertyDPIHeight, dpi));
        }
        if let Some(orientation) = with.orientation {
            properties.push((
                kCGImagePropertyOrientation,
                CFNumber::from(i32::from(orientation)).as_CFType(),
            ));
        }
        if let Some((latitude, longitude)) = with.location {
            let half = |value: f64, positive: &'static str, negative: &'static str| {
                CFString::from_static_string(if value < 0.0 { negative } else { positive })
                    .as_CFType()
            };
            let position = dictionary(&[
                (
                    kCGImagePropertyGPSLatitude,
                    CFNumber::from(latitude.abs()).as_CFType(),
                ),
                (kCGImagePropertyGPSLatitudeRef, half(latitude, "N", "S")),
                (
                    kCGImagePropertyGPSLongitude,
                    CFNumber::from(longitude.abs()).as_CFType(),
                ),
                (kCGImagePropertyGPSLongitudeRef, half(longitude, "E", "W")),
            ]);
            properties.push((kCGImagePropertyGPSDictionary, position.as_CFType()));
        }
    }
    let properties = dictionary(&properties);
    for picture in pictures {
        // SAFETY: a live destination, a live image, a live dictionary.
        unsafe {
            CGImageDestinationAddImage(
                destination.as_CFTypeRef() as Object,
                picture.object(),
                properties.as_concrete_TypeRef() as CFDictionaryRef,
            );
        }
    }
    // SAFETY: a live destination with as many pictures as it was made for.
    let finished = unsafe { CGImageDestinationFinalize(destination.as_CFTypeRef() as Object) };
    finished.then(|| bytes_of(&data)).filter(|b| !b.is_empty())
}

/// A destination that writes `count` pictures as a file of kind `kind` into the data
/// object beside it. `None` when ImageIO cannot write that kind.
fn destination(kind: &str, count: usize) -> Option<(CFType, CFType)> {
    // SAFETY: a new, empty, growable data object, owned here.
    let data = owned(unsafe { CFDataCreateMutable(kCFAllocatorDefault, 0) } as Object)?;
    let kind = CFString::new(kind);
    // SAFETY: a live data object and a live string, no options.
    let destination = owned(unsafe {
        CGImageDestinationCreateWithData(
            data.as_CFTypeRef() as *mut c_void,
            kind.as_concrete_TypeRef(),
            count,
            ptr::null(),
        )
    })?;
    Some((data, destination))
}

/// The bytes a destination wrote into its data object.
fn bytes_of(data: &CFType) -> Vec<u8> {
    // SAFETY: a live data object; the bytes are copied out before it can change.
    unsafe {
        let data = data.as_CFTypeRef() as CFDataRef;
        let len = CFDataGetLength(data);
        let start = CFDataGetBytePtr(data);
        if start.is_null() || len <= 0 {
            return Vec::new();
        }
        std::slice::from_raw_parts(start, len as usize).to_vec()
    }
}
