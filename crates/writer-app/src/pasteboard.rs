//! The app's own look at the system pasteboard, for pictures GPUI's
//! clipboard does not show.
//!
//! GPUI's `read_from_clipboard` hands over the text of the pasteboard
//! whenever there is text, and a picture only when there is none and the
//! picture is of a kind it knows. That leaves out what people paste most:
//!
//! - **an image file copied in Finder**: the pasteboard has the file's
//!   address and, as text, its name;
//! - **"Copy Image" in a browser**: the picture, and beside it the address
//!   it came from as text;
//! - **a HEIC** (a photo copied in Photos), which GPUI does not know.
//!
//! [`image`] answers for those, and the editor asks it when its own look
//! at the clipboard found no picture
//! (`writer_editor::ImageProvider::pasteboard`, through
//! [`crate::images::LibraryImages`]).
//!
//! # What counts as a picture ([`decide`])
//!
//! A pure function of what the pasteboard offers, so that it can be
//! tested without a pasteboard:
//!
//! 1. a file address whose file has the extension of an image: that file;
//! 2. else picture data, **when there is no plain text beside it, or the
//!    plain text is one line that is an address** (`http:`, `https:`,
//!    `file:`, `data:`): the data. Cells copied in a spreadsheet carry a
//!    picture of themselves beside their text, and a paste of those must
//!    stay text;
//! 3. else nothing: the paste is text, as it always was.
//!
//! # Who reads the real pasteboard
//!
//! Only a run that was told to ([`use_system`]: `main`, when the run is
//! not scripted). The pasteboard is the user's: a scripted run and a test
//! get no answer from it, and a test says what is "on" it with
//! [`stand_in`]. The read itself (`NSPasteboard` from Rust) is checked by
//! hand; the decision is tested.

use std::path::{Path, PathBuf};

use gpui::{App, Global};
use writer_editor::NewImageSource;

/// The pasteboard type of a file's address.
pub const FILE_URL: &str = "public.file-url";
/// The pasteboard type of plain text.
pub const PLAIN_TEXT: &str = "public.utf8-plain-text";

/// The kinds of picture data that are read, in the order they are
/// preferred: a kind the library stores as it is before a kind that has to
/// be converted.
pub const PICTURE_TYPES: [&str; 6] = [
    "public.png",
    "public.jpeg",
    "com.compuserve.gif",
    "public.heic",
    "public.heif",
    "public.tiff",
];

/// The file extensions that make a copied file an image (lower case).
pub const IMAGE_EXTENSIONS: [&str; 11] = [
    "png", "jpg", "jpeg", "gif", "heic", "heif", "tif", "tiff", "bmp", "webp", "avif",
];

/// What a pasteboard offers, as far as [`decide`] needs it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Contents<'a> {
    /// The types it offers (uniform type identifiers).
    pub types: &'a [&'a str],
    /// The file its file address names, if it has one.
    pub file: Option<&'a Path>,
    /// Its plain text, if it has any.
    pub text: Option<&'a str>,
}

/// The picture to take from a pasteboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Take<'a> {
    /// This file.
    File(&'a Path),
    /// The data of this type (one of [`PICTURE_TYPES`]).
    Data(&'static str),
}

/// Whether `path` has the extension of an image file.
pub fn is_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            IMAGE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
}

/// Whether `text` is one line that is an address: what a browser puts
/// beside a picture it copies.
pub fn is_an_address(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() || text.chars().any(char::is_whitespace) {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    ["http:", "https:", "file:", "data:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}

/// Which picture a paste takes from a pasteboard with these contents, if
/// any (see the [module docs](self)).
pub fn decide<'a>(contents: &Contents<'a>) -> Option<Take<'a>> {
    if let Some(file) = contents.file.filter(|file| is_image_file(file)) {
        return Some(Take::File(file));
    }
    let text_allows = match contents.text {
        None => true,
        Some(text) => text.trim().is_empty() || is_an_address(text),
    };
    if !text_allows {
        return None;
    }
    PICTURE_TYPES
        .into_iter()
        .find(|kind| contents.types.contains(kind))
        .map(Take::Data)
}

/// Who is asked for the picture on the pasteboard.
#[derive(Default)]
enum Reader {
    /// Nobody: there is none (a scripted run, a test that said nothing).
    #[default]
    Nobody,
    /// The system pasteboard.
    System,
    /// What a test put there.
    StandIn(Option<NewImageSource>),
}

impl Global for Reader {}

/// From now on [`image`] reads the system pasteboard. Called by `main` in
/// a launch that is not scripted, and by nothing else.
pub fn use_system(cx: &mut App) {
    cx.set_global(Reader::System);
}

/// Says what picture is "on the pasteboard" from now on (`None`: no
/// picture). For tests: nothing is read from the system.
pub fn stand_in(source: Option<NewImageSource>, cx: &mut App) {
    cx.set_global(Reader::StandIn(source));
}

/// The picture on the pasteboard that GPUI's clipboard does not show, if
/// there is one and this run reads the pasteboard.
pub fn image(cx: &App) -> Option<NewImageSource> {
    match cx.try_global::<Reader>()? {
        Reader::Nobody => None,
        Reader::System => read_system(),
        Reader::StandIn(source) => source.clone(),
    }
}

/// Reads the general pasteboard: its types, its plain text and its file
/// address decide ([`decide`]), and only then is picture data copied.
#[cfg(target_os = "macos")]
// objc 0.2's msg_send! expands to a `cfg(feature = "cargo-clippy")` check.
#[allow(unexpected_cfgs)]
fn read_system() -> Option<NewImageSource> {
    use objc::{class, msg_send, runtime::Object, sel, sel_impl};
    use std::ffi::{CStr, CString, c_char, c_void};

    /// An `NSString` as text.
    unsafe fn text_of(string: *mut Object) -> Option<String> {
        if string.is_null() {
            return None;
        }
        // SAFETY: `string` is an NSString; `UTF8String` is valid until the
        // string is released, which is after this call.
        unsafe {
            let bytes: *const c_char = msg_send![string, UTF8String];
            (!bytes.is_null()).then(|| CStr::from_ptr(bytes).to_string_lossy().into_owned())
        }
    }

    /// Text as an autoreleased `NSString`.
    unsafe fn ns_string(text: &str) -> *mut Object {
        let Ok(text) = CString::new(text) else {
            return std::ptr::null_mut();
        };
        // SAFETY: a Foundation class method with a C string that lives
        // through the call.
        unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
    }

    // SAFETY: AppKit and Foundation calls on the main thread (every caller
    // is a GPUI handler). Every object is autoreleased or owned by the
    // pasteboard and only used within this function; bytes are copied out
    // before it returns.
    unsafe {
        let pasteboard: *mut Object = msg_send![class!(NSPasteboard), generalPasteboard];
        if pasteboard.is_null() {
            return None;
        }
        let offered: *mut Object = msg_send![pasteboard, types];
        let mut types: Vec<String> = Vec::new();
        if !offered.is_null() {
            let count: usize = msg_send![offered, count];
            for index in 0..count {
                let kind: *mut Object = msg_send![offered, objectAtIndex: index];
                types.extend(text_of(kind));
            }
        }
        let string_for = |kind: &str| -> Option<String> {
            if !types.iter().any(|offered| offered == kind) {
                return None;
            }
            let kind = ns_string(kind);
            if kind.is_null() {
                return None;
            }
            let string: *mut Object = msg_send![pasteboard, stringForType: kind];
            text_of(string)
        };
        let text = string_for(PLAIN_TEXT);
        // The address may be a file reference (`file:///.file/id=…`, what
        // Finder writes): `filePathURL` makes it the path.
        let file = string_for(FILE_URL).and_then(|address| {
            let address = ns_string(&address);
            if address.is_null() {
                return None;
            }
            let url: *mut Object = msg_send![class!(NSURL), URLWithString: address];
            if url.is_null() {
                return None;
            }
            let url: *mut Object = msg_send![url, filePathURL];
            if url.is_null() {
                return None;
            }
            let path: *mut Object = msg_send![url, path];
            text_of(path).map(PathBuf::from)
        });
        let types: Vec<&str> = types.iter().map(String::as_str).collect();
        let take = decide(&Contents {
            types: &types,
            file: file.as_deref(),
            text: text.as_deref(),
        })?;
        match take {
            Take::File(path) => Some(NewImageSource::File(path.to_path_buf())),
            Take::Data(kind) => {
                let kind = ns_string(kind);
                if kind.is_null() {
                    return None;
                }
                let data: *mut Object = msg_send![pasteboard, dataForType: kind];
                if data.is_null() {
                    return None;
                }
                let length: usize = msg_send![data, length];
                let bytes: *const c_void = msg_send![data, bytes];
                if bytes.is_null() || length == 0 {
                    return None;
                }
                let bytes = std::slice::from_raw_parts(bytes.cast::<u8>(), length);
                Some(NewImageSource::Bytes(bytes.to_vec()))
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn read_system() -> Option<NewImageSource> {
    None
}
