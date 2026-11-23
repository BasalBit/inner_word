//! Image files a request names: found, read and made into what the library stores, in
//! the process that was asked.
//!
//! A text an agent writes may name an image file (`![The harbour](/Users/ada/h.jpg)`),
//! and `add_image` names one outright. The file is read **here, before the operation is
//! handed anywhere**: while the app is open the operation runs on the app's main thread
//! ([`crate::live::execute`]), and the app must never open a path an agent named (its
//! permissions are not the agent's, and a privacy prompt of macOS would hold the main
//! thread while the tool's patience runs out). So every method of
//! [`Library`] that takes a request with images first calls [`Prepare::prepared`] on it
//! (the table in `operations.rs` does), which fills `images` with the bytes; from then on
//! the request is carried out from what it brings, by the tool or by the app.

use std::borrow::Cow;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use innerword_core::ImageId;
use innerword_export::{image_reference, markdown_image_sources};

use crate::error::{Error, Result, quoted_excerpt};
use crate::library::Library;
use crate::live::MAX_ATTACHED_BYTES;
use crate::types::{
    AddCommentRequest, AddImageRequest, AttachedImage, CommentRequest, CreateDocumentRequest,
    CreateFolderRequest, DeleteCommentRequest, DeleteDocumentRequest, DeleteFolderRequest,
    DuplicateDocumentRequest, EditRequest, EmptyTrashRequest, InsertRequest, MoveDocumentRequest,
    MoveFolderRequest, RemoveFromBoardRequest, RenameDocumentRequest, RenameFolderRequest,
    ReplaceDocumentRequest, ReplaceSectionRequest, ReplyRequest, RestoreDocumentRequest,
    RestoreFolderRequest, RestoreVersionRequest, TrashDocumentRequest, TrashFolderRequest,
    UpdateBoardRequest,
};

/// What is said when the images of one request weigh more than
/// [`MAX_ATTACHED_BYTES`] together.
pub(crate) const TOO_MUCH: &str =
    "the images of one request can be 32 MB together at most; add them in several requests";

/// What the destination of an image in Markdown is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Destination {
    /// `image:<id>`: a stored image.
    Reference(ImageId),
    /// `image:` followed by something that is no id.
    BadReference,
    /// An address on the network (anything with a scheme other than `file:`), a place
    /// in the page (`#top`), or nothing at all: not for this crate to read.
    Network,
    /// A file on this Mac: an absolute path, `~/...`, a `file:` address, or a path
    /// relative to where the text came from.
    Local,
}

/// Sorts a destination as it stands in the Markdown.
pub(crate) fn destination(source: &str) -> Destination {
    let source = source.trim();
    if let Some(id) = image_reference(source) {
        return Destination::Reference(id);
    }
    if source.is_empty() || source.starts_with("//") || source.starts_with('#') {
        return Destination::Network;
    }
    match scheme(source) {
        Some(scheme) if scheme.eq_ignore_ascii_case("file") => Destination::Local,
        Some(scheme) if scheme.eq_ignore_ascii_case("image") => Destination::BadReference,
        Some(_) => Destination::Network,
        None => Destination::Local,
    }
}

/// The scheme an address begins with (`https` of `https://…`), when it begins with one:
/// a letter, then letters, digits, `+`, `-` or `.`, at least two characters in all, and
/// a colon.
fn scheme(source: &str) -> Option<&str> {
    let (scheme, _) = source.split_once(':')?;
    let mut chars = scheme.chars();
    let sound = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    (sound && scheme.len() >= 2).then_some(scheme)
}

/// `text` with every `%XX` read as the byte it stands for.
fn percent_decoded(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let hex = |byte: u8| (byte as char).to_digit(16).map(|digit| digit as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && at + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[at + 1]), hex(bytes[at + 2]))
        {
            out.push(high << 4 | low);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    out
}

/// The path a local destination names, as far as that can be said without looking at
/// the disk. `Err` is why it cannot be followed, as the end of a sentence whose subject
/// is the destination.
fn path_of(source: &str, base_dir: Option<&Path>) -> std::result::Result<PathBuf, &'static str> {
    let source = source.trim();
    const RELATIVE: &str = "is a relative path, and nothing says what it is relative to: give the absolute path \
         of the image file.";
    let path = if scheme(source).is_some_and(|scheme| scheme.eq_ignore_ascii_case("file")) {
        // file:///Users/ada/h.jpg, file://localhost/Users/ada/h.jpg, file:/Users/ada/h.jpg
        let rest = &source[5..];
        let rest = match rest.strip_prefix("//") {
            Some(after) => {
                let (host, path) = match after.find('/') {
                    Some(slash) => after.split_at(slash),
                    None => (after, ""),
                };
                if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
                    return Err("names a file on another computer.");
                }
                path
            }
            None => rest,
        };
        PathBuf::from(OsString::from_vec(percent_decoded(rest)))
    } else if source == "~" || source.starts_with("~/") {
        let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) else {
            return Err("starts at the home folder, and this process does not know where that is.");
        };
        PathBuf::from(home).join(source[1..].trim_start_matches('/'))
    } else {
        PathBuf::from(source)
    };
    if path.is_absolute() {
        return Ok(path);
    }
    if path.as_os_str().is_empty() {
        return Err("names no file.");
    }
    base_dir.map(|base| base.join(&path)).ok_or(RELATIVE)
}

/// The file a local destination names, when there is one. A destination that is not
/// there as written and has `%20` and the like in it is tried with those read as the
/// characters they stand for (Markdown has no other way to write a space in a path
/// without angle brackets).
pub(crate) fn file_of(
    source: &str,
    base_dir: Option<&Path>,
) -> std::result::Result<Option<PathBuf>, &'static str> {
    let path = path_of(source, base_dir)?;
    if path.exists() {
        return Ok(Some(path));
    }
    if source.contains('%')
        && !source
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("file:")
    {
        let decoded = String::from_utf8_lossy(&percent_decoded(source.trim())).into_owned();
        if let Ok(path) = path_of(&decoded, base_dir)
            && path.exists()
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// `source` as the subject of a sentence: in quotation marks, shortened when long.
pub(crate) fn subject(source: &str) -> String {
    quoted_excerpt(source.trim())
}

/// The attachment for the local destination `source`: the file as the library stores
/// it, or the sentence that says why it cannot be used. `None` when there is no such
/// file.
fn attachment(source: &str, base_dir: Option<&Path>) -> Option<AttachedImage> {
    let problem = |sentence: String| AttachedImage {
        source: source.to_string(),
        problem: Some(sentence),
        ..AttachedImage::default()
    };
    let path = match file_of(source, base_dir) {
        Ok(Some(path)) => path,
        Ok(None) => return None,
        Err(why) => return Some(problem(format!("{} {why}", subject(source)))),
    };
    Some(match innerword_image::prepare_file(&path) {
        Ok(prepared) => AttachedImage {
            source: source.to_string(),
            format: prepared.format.as_str().to_string(),
            pixels: prepared.pixels,
            density: prepared.density,
            data: prepared.bytes,
            problem: None,
        },
        Err(error) => problem(error.sentence(&subject(source))),
    })
}

/// Fails when `images` weigh more together than one request may bring.
pub(crate) fn weigh(images: &[AttachedImage]) -> Result<()> {
    let bytes: usize = images.iter().map(|image| image.data.len()).sum();
    if bytes > MAX_ATTACHED_BYTES {
        return Err(Error::invalid(TOO_MUCH));
    }
    Ok(())
}

/// The attachments for the image files `markdown` names, without those `have` holds
/// already: one for each local destination that is there, in the order of the text.
fn attachments(
    markdown: &str,
    have: &[AttachedImage],
    base_dir: Option<&Path>,
) -> Result<Vec<AttachedImage>> {
    // Every image of a Markdown text is written with these two characters.
    if !markdown.contains("![") {
        return Ok(Vec::new());
    }
    let mut found: Vec<AttachedImage> = Vec::new();
    let mut bytes: usize = have.iter().map(|image| image.data.len()).sum();
    for source in markdown_image_sources(markdown) {
        if destination(&source) != Destination::Local
            || have
                .iter()
                .chain(&found)
                .any(|image| image.source == source)
        {
            continue;
        }
        if let Some(image) = attachment(&source, base_dir) {
            bytes += image.data.len();
            if bytes > MAX_ATTACHED_BYTES {
                return Err(Error::invalid(TOO_MUCH));
            }
            found.push(image);
        }
    }
    Ok(found)
}

/// What a request does before it is carried out, in the process that was asked: it
/// brings the image files it names. A request without any is as it is.
pub(crate) trait Prepare: Clone {
    /// The request with the images its text names attached; itself when there is
    /// nothing to bring. `library` gives the directory a relative path is taken from
    /// when the request names none.
    fn prepared(&self, _library: &Library) -> Result<Cow<'_, Self>> {
        Ok(Cow::Borrowed(self))
    }
}

macro_rules! brings_nothing {
    ($($request:ty),* $(,)?) => {
        $(impl Prepare for $request {})*
    };
}

brings_nothing!(
    RenameDocumentRequest,
    MoveDocumentRequest,
    DuplicateDocumentRequest,
    TrashDocumentRequest,
    RestoreDocumentRequest,
    DeleteDocumentRequest,
    CreateFolderRequest,
    RenameFolderRequest,
    MoveFolderRequest,
    TrashFolderRequest,
    RestoreFolderRequest,
    DeleteFolderRequest,
    EmptyTrashRequest,
    AddCommentRequest,
    ReplyRequest,
    CommentRequest,
    DeleteCommentRequest,
    RestoreVersionRequest,
    UpdateBoardRequest,
    RemoveFromBoardRequest,
);

macro_rules! brings_the_images_of {
    ($($request:ty: $markdown:expr;)*) => {
        $(impl Prepare for $request {
            fn prepared(&self, library: &Library) -> Result<Cow<'_, Self>> {
                let markdown: fn(&$request) -> Option<&str> = $markdown;
                let Some(markdown) = markdown(self) else {
                    return Ok(Cow::Borrowed(self));
                };
                let base_dir = self.base_dir.as_deref().or(library.base_dir.as_deref());
                let found = attachments(markdown, &self.images, base_dir)?;
                if found.is_empty() {
                    return Ok(Cow::Borrowed(self));
                }
                let mut request = self.clone();
                request.images.extend(found);
                Ok(Cow::Owned(request))
            }
        })*
    };
}

brings_the_images_of! {
    CreateDocumentRequest: |request| Some(&request.markdown);
    ReplaceDocumentRequest: |request| Some(&request.markdown);
    // Taken literally, the new text is the characters it is: it names no image.
    EditRequest: |request| (!request.literal).then_some(request.new_text.as_str());
    InsertRequest: |request| Some(&request.markdown);
    ReplaceSectionRequest: |request| Some(&request.markdown);
}

impl Prepare for AddImageRequest {
    fn prepared(&self, library: &Library) -> Result<Cow<'_, Self>> {
        // Whoever calls with the image in hand has read the file already.
        if !self.images.is_empty() {
            return Ok(Cow::Borrowed(self));
        }
        let path = self.path.trim();
        if path.is_empty() {
            return Err(Error::invalid("name the image file to add"));
        }
        if destination(path) != Destination::Local {
            return Err(Error::invalid(format!(
                "{} is not a file on this Mac; give the path of an image file",
                subject(path)
            )));
        }
        let base_dir = self.base_dir.as_deref().or(library.base_dir.as_deref());
        let file = match file_of(path, base_dir) {
            Ok(Some(file)) => file,
            Ok(None) => {
                return Err(Error::invalid(format!(
                    "there is no file {}; give the path of an image file",
                    subject(path)
                )));
            }
            Err(_) if base_dir.is_none() && !Path::new(path).is_absolute() => {
                return Err(Error::invalid("give the absolute path of the image file"));
            }
            Err(why) => return Err(Error::invalid(format!("{} {why}", subject(path)))),
        };
        let prepared = innerword_image::prepare_file(&file)
            .map_err(|error| Error::invalid(error.sentence(&subject(path))))?;
        let mut request = self.clone();
        request.images = vec![AttachedImage {
            source: self.path.clone(),
            format: prepared.format.as_str().to_string(),
            pixels: prepared.pixels,
            density: prepared.density,
            data: prepared.bytes,
            problem: None,
        }];
        Ok(Cow::Owned(request))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> ImageId {
        ImageId::parse("3f2a9c1b5d6e7f80a1b2c3d4e5f60718").unwrap()
    }

    #[test]
    fn destinations_are_told_apart() {
        assert_eq!(
            destination("image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718"),
            Destination::Reference(id())
        );
        for bad in ["image:harbour", "image:", "IMAGE:3f2a"] {
            assert_eq!(destination(bad), Destination::BadReference, "{bad}");
        }
        for network in [
            "https://example.com/harbour.jpg",
            "http://example.com/a.png",
            "//example.com/a.png",
            "data:image/png;base64,AAAA",
            "mailto:ada@example.com",
            "ftp://example.com/a.png",
            "#harbour",
            "",
            "  ",
        ] {
            assert_eq!(destination(network), Destination::Network, "{network}");
        }
        for local in [
            "/Users/ada/harbour.jpg",
            "~/Pictures/harbour.jpg",
            "~",
            "harbour.jpg",
            "pictures/harbour.jpg",
            "./harbour.jpg",
            "../harbour.jpg",
            "file:///Users/ada/harbour.jpg",
            "FILE:/Users/ada/harbour.jpg",
            // A colon in a name is no scheme: one letter is a drive, and a name with a
            // space or a slash before the colon is a name.
            "a:b.png",
            "my pictures: one.png",
            "pictures/12:30.png",
            "harbour.jpg?version=2",
        ] {
            assert_eq!(destination(local), Destination::Local, "{local}");
        }
    }

    #[test]
    fn paths_are_followed_from_where_they_start() {
        let base = Path::new("/base/dir");
        let path = |source: &str, base: Option<&Path>| path_of(source, base);
        assert_eq!(
            path("/Users/ada/h.jpg", None),
            Ok(PathBuf::from("/Users/ada/h.jpg"))
        );
        assert_eq!(
            path("pictures/h.jpg", Some(base)),
            Ok(PathBuf::from("/base/dir/pictures/h.jpg"))
        );
        assert!(
            path("pictures/h.jpg", None)
                .unwrap_err()
                .contains("relative")
        );
        assert_eq!(
            path("file:///Users/ada/my%20harbour.jpg", None),
            Ok(PathBuf::from("/Users/ada/my harbour.jpg"))
        );
        assert_eq!(
            path("file://localhost/Users/ada/h.jpg", None),
            Ok(PathBuf::from("/Users/ada/h.jpg"))
        );
        assert_eq!(
            path("file:/Users/ada/h.jpg", None),
            Ok(PathBuf::from("/Users/ada/h.jpg"))
        );
        assert!(
            path("file://server/share/h.jpg", None)
                .unwrap_err()
                .contains("another computer")
        );
        assert_eq!(
            path("file:h.jpg", Some(base)),
            Ok(PathBuf::from("/base/dir/h.jpg"))
        );
        let home = PathBuf::from(std::env::var_os("HOME").expect("a home directory"));
        assert_eq!(
            path("~/Pictures/h.jpg", None),
            Ok(home.join("Pictures/h.jpg"))
        );
        assert_eq!(path("~", None), Ok(home));
        assert_eq!(percent_decoded("a%20b%2Fc%zz%2"), b"a b/c%zz%2");
        assert_eq!(percent_decoded("100%"), b"100%");
        assert_eq!(percent_decoded("%41"), b"A");
    }

    #[test]
    fn a_file_is_found_as_written_or_with_its_percent_signs_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("my harbour.png"), b"x").unwrap();
        std::fs::write(dir.path().join("100%.png"), b"x").unwrap();
        let found = |source: &str| file_of(source, Some(dir.path())).unwrap();
        assert_eq!(
            found("my harbour.png"),
            Some(dir.path().join("my harbour.png"))
        );
        assert_eq!(
            found("my%20harbour.png"),
            Some(dir.path().join("my harbour.png"))
        );
        assert_eq!(found("100%.png"), Some(dir.path().join("100%.png")));
        assert_eq!(found("other.png"), None);
        assert_eq!(found("other%20one.png"), None);
    }

    #[test]
    fn what_a_text_names_is_attached_once_and_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let png = innerword_image::fixtures::png(8, 6, [200, 30, 30]);
        std::fs::write(dir.path().join("a.png"), &png).unwrap();
        std::fs::write(dir.path().join("notes.pdf"), b"%PDF-1.4\n").unwrap();
        let absolute = dir.path().join("a.png");
        let markdown = format!(
            "![one](a.png)\n\n![again](a.png) and ![far](https://example.com/x.png)\n\n\
             ![kept](image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718)\n\n![full]({})\n\n\
             ![none](missing.png)\n\n![doc](notes.pdf)\n",
            absolute.display()
        );
        let found = attachments(&markdown, &[], Some(dir.path())).unwrap();
        let sources: Vec<&str> = found.iter().map(|image| image.source.as_str()).collect();
        assert_eq!(
            sources,
            ["a.png", absolute.to_str().unwrap(), "notes.pdf"],
            "{found:?}"
        );
        assert_eq!(found[0].data, png, "a PNG is stored as it came");
        assert_eq!(
            (found[0].format.as_str(), found[0].pixels, found[0].density),
            ("png", (8, 6), 72)
        );
        assert_eq!(found[0].problem, None);
        let problem = found[2].problem.as_deref().unwrap();
        assert_eq!(
            problem,
            "\u{201c}notes.pdf\u{201d} is a PDF file. Images can be PNG, JPEG, GIF, HEIC, TIFF, WebP or BMP."
        );
        assert!(found[2].data.is_empty() && found[2].format.is_empty());
        // What is there already is not read again.
        let again = attachments(&markdown, &found, Some(dir.path())).unwrap();
        assert!(again.is_empty(), "{again:?}");
        // Without a directory a relative path is not followed, and says so.
        let nowhere = attachments("![one](a.png)", &[], None).unwrap();
        assert_eq!(nowhere.len(), 1);
        assert!(
            nowhere[0]
                .problem
                .as_deref()
                .unwrap()
                .starts_with("\u{201c}a.png\u{201d} is a relative path"),
            "{nowhere:?}"
        );
        // A text without an image is not even parsed.
        assert!(
            attachments("No image [here](a.png).", &[], Some(dir.path()))
                .unwrap()
                .is_empty()
        );
        // The debug form of megabytes of picture is a number.
        assert!(format!("{:?}", found[0]).contains(&format!("{} bytes", png.len())));
    }
}
