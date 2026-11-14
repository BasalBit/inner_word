//! Export of a document, or of a folder as one file, to a path on disk.
//!
//! Images go with the text. A DOCX holds its pictures; a Markdown file links them, and
//! they are written into a folder beside it (`chapter.md` and `chapter images/`), which
//! is also how an agent gets to look at a picture: the files are there to open. Plain
//! text has the description of each image on a line of its own.

use std::borrow::Cow;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

use writer_core::{Document, ImageId};
use writer_export::{
    ExportOptions, ExportSource, ImageData, ImageFileFormat, ImageSource, MarkdownImages,
    export_package, images_dir_name,
};
use writer_store::{DocumentKind, DocumentMeta, FolderNode, ImageFormat, Store};

use crate::content;
use crate::error::{Error, Result};
use crate::library::Library;
use crate::resolve::{Index, title_of};
use crate::types::{ExportFormat, ExportRequest, Exported};

/// The name of an export of the whole library.
const LIBRARY_NAME: &str = "Library";

/// The documents of a folder in the order of an export: its own first, then those of its
/// folders, as the app exports a folder.
fn collect(node: &FolderNode, documents: &mut Vec<DocumentMeta>) {
    documents.extend(node.documents.iter().cloned());
    for child in &node.folders {
        collect(child, documents);
    }
}

/// The exporter's format for a format of a text; `None` for the formats of a page.
fn store_format(format: ExportFormat) -> Option<writer_export::ExportFormat> {
    match format {
        ExportFormat::Markdown => Some(writer_export::ExportFormat::Markdown),
        ExportFormat::Docx => Some(writer_export::ExportFormat::Docx),
        ExportFormat::PlainText => Some(writer_export::ExportFormat::PlainText),
        ExportFormat::Html | ExportFormat::Pdf => None,
    }
}

/// "2 pages were left out: a page is exported on its own, as a folder."
fn pages_left_out(count: usize) -> String {
    if count == 1 {
        "1 page was left out: a page is exported on its own, as a folder.".to_string()
    } else {
        format!("{count} pages were left out: a page is exported on its own, as a folder.")
    }
}

/// The pictures of the library, for the exporters. One the library does not have is
/// exported as a missing one (its description stands in for it). One that could not be
/// read (`images.db` damaged, busy past its timeout, of a newer version) is not a missing
/// one: the first such error is kept, and the export fails with it instead of writing a
/// file without its pictures.
struct Pictures<'a> {
    store: &'a Store,
    failed: RefCell<Option<writer_store::StoreError>>,
}

impl Pictures<'_> {
    fn keep<T>(&self, read: writer_store::Result<Option<T>>) -> Option<T> {
        match read {
            Ok(found) => found,
            Err(error) => {
                self.failed.borrow_mut().get_or_insert(error);
                None
            }
        }
    }
}

impl ImageSource for Pictures<'_> {
    fn image(&self, id: ImageId) -> Option<ImageData<'_>> {
        let name = writer_store::ImageId(*id.as_bytes());
        let info = self.keep(self.store.image_info(&name))?;
        let bytes = self.keep(self.store.image_bytes(&name))?;
        Some(ImageData {
            bytes: Cow::Owned(bytes),
            format: match info.format {
                ImageFormat::Png => ImageFileFormat::Png,
                ImageFormat::Jpeg => ImageFileFormat::Jpeg,
                ImageFormat::Gif => ImageFileFormat::Gif,
            },
        })
    }
}

/// Writes `bytes` to `path` so that a reader never sees half of them: beside the file
/// first, then in its place.
pub(crate) fn write_whole(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut partial = path.to_path_buf().into_os_string();
    partial.push(format!(".partial-{}", std::process::id()));
    let partial = PathBuf::from(partial);
    std::fs::write(&partial, bytes).map_err(io(&partial))?;
    if let Err(error) = std::fs::rename(&partial, path) {
        std::fs::remove_file(&partial).ok();
        return Err(io(path)(error));
    }
    Ok(())
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

impl Library {
    /// Writes a document, or a folder as one file, as Markdown, DOCX or plain text. Only
    /// reads the library, so it works while the app is open. See [`ExportRequest`].
    ///
    /// The images of a Markdown export are written as files into the folder
    /// `<name> images` beside the file, and the Markdown links them there
    /// ([`Exported::assets`] lists them); a DOCX holds its pictures.
    ///
    /// A page is written as a folder of web files that opens offline (its served files,
    /// or with `original` the files it was added from), as a PDF the way WebKit prints
    /// it (`.pdf`; it needs the page checker, [`Library::with_page_checker`]), or as its
    /// text (`.txt`). Markdown and DOCX are formats of a text, and a page in a folder
    /// that is exported as one file is left out (a warning says so).
    pub fn export(&self, request: &ExportRequest) -> Result<Exported> {
        let targets = (request.document.as_ref(), request.folder.as_ref());
        if matches!(targets, (Some(_), Some(_)) | (None, None)) {
            return Err(Error::invalid(
                "name what to export: a document, or a folder (not both)",
            ));
        }
        if request.path.as_os_str().is_empty() {
            return Err(Error::invalid("name the file to write"));
        }
        let absolute = |path: &Path| -> Result<PathBuf> {
            Ok(if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir().map_err(io(path))?.join(path)
            })
        };
        let path: PathBuf = absolute(&request.path)?;
        // From the path: `.md`, `.docx`, `.txt`, and `.pdf` for a page (a folder has no
        // ending, so `html` is only taken when it is named).
        let guessed = request.format.or_else(|| {
            request
                .path
                .extension()
                .and_then(|extension| extension.to_str())
                .and_then(ExportFormat::parse)
                .filter(|format| *format != ExportFormat::Html)
        });

        let session = self.reader()?;
        let store = &session.store;
        let index = Index::load(store)?;
        let named = match targets.0 {
            Some(target) => Some(index.document(store, target)?),
            None => None,
        };
        // A page has formats of its own.
        if let Some(meta) = &named
            && meta.kind == DocumentKind::Page
        {
            let format = crate::pages::export_format(meta, request)?;
            return crate::pages::export(self, store, &index, meta, request, format, path);
        }
        if guessed.is_some_and(ExportFormat::is_for_pages) {
            return Err(match &named {
                Some(meta) => crate::content::is_a_text(meta),
                None => Error::invalid(
                    "a folder is exported as one file of text (Markdown, DOCX or plain text); \
                     a web page folder and a PDF are made of a page",
                ),
            });
        }
        let format = guessed.unwrap_or_default();
        if path.is_dir() {
            return Err(Error::invalid(format!(
                "{} is a directory; name the file to write",
                path.display()
            )));
        }
        if path.exists() && !request.overwrite {
            return Err(Error::FileExists { path });
        }

        let mut warnings = Vec::new();
        let (title, metas): (String, Vec<DocumentMeta>) = match targets {
            (Some(_), _) => {
                let meta = named.expect("the document was found above");
                (title_of(&meta).to_string(), vec![meta])
            }
            (_, Some(target)) => {
                let folder = index.live_folder_or_root(store, target)?;
                let mut metas = Vec::new();
                match &folder {
                    Some(folder) => {
                        if let Some(node) = index.node(folder.id) {
                            collect(node, &mut metas);
                        }
                    }
                    None => {
                        metas.extend(index.tree.documents.iter().cloned());
                        for node in &index.tree.folders {
                            collect(node, &mut metas);
                        }
                    }
                }
                let before = metas.len();
                metas.retain(|meta| meta.kind != DocumentKind::Page);
                let pages = before - metas.len();
                if pages > 0 {
                    warnings.push(pages_left_out(pages));
                }
                if metas.is_empty() {
                    return Err(Error::invalid(if pages > 0 {
                        "there is no document in that folder, only pages, so there is nothing \
                         to export as one file: a page is exported on its own, as a folder"
                    } else {
                        "there is no document in that folder, so there is nothing to export"
                    }));
                }
                (
                    folder.map_or_else(|| LIBRARY_NAME.to_string(), |folder| folder.name),
                    metas,
                )
            }
            (None, None) => unreachable!("checked above"),
        };
        let mut documents: Vec<Document> = Vec::with_capacity(metas.len());
        for meta in &metas {
            documents.push(content::load(store, meta)?.document);
        }
        let source = if request.document.is_some() {
            ExportSource::single(title.clone(), &documents[0])
        } else {
            ExportSource::collection(
                title.clone(),
                metas
                    .iter()
                    .zip(&documents)
                    .map(|(meta, document)| (title_of(meta).to_string(), document)),
            )
        };
        let pictures = Pictures {
            store,
            failed: RefCell::new(None),
        };
        let source = source.with_images(&pictures);
        let mut options = ExportOptions::default();
        options.docx.created = metas.iter().map(|meta| meta.created_at).min().unwrap_or(0);
        options.docx.modified = metas.iter().map(|meta| meta.updated_at).max().unwrap_or(0);
        // A Markdown file links its pictures in a folder of its own beside it.
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        options.markdown.images = MarkdownImages::Files {
            dir: images_dir_name(&file_name),
        };
        let Some(store_format) = store_format(format) else {
            unreachable!("the formats of a page are taken apart above");
        };
        let exported = export_package(&source, store_format, &options);
        if let Some(source) = pictures.failed.into_inner() {
            return Err(Error::Pictures { source });
        }
        let bytes = exported.bytes;
        let beside = path.parent().unwrap_or(Path::new("")).to_path_buf();
        let assets: Vec<PathBuf> = exported
            .assets
            .iter()
            .map(|asset| beside.join(&asset.path))
            .collect();

        if !request.dry_run {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent).map_err(io(parent))?;
            }
            // The pictures first: a file that is there links nothing that is not. They
            // are named by what they are, so one that is there already is the same one.
            for (asset, file) in exported.assets.iter().zip(&assets) {
                if let Some(folder) = file.parent() {
                    std::fs::create_dir_all(folder).map_err(io(folder))?;
                }
                write_whole(file, &asset.bytes)?;
            }
            write_whole(&path, &bytes)?;
        }
        Ok(Exported {
            path,
            format,
            bytes: bytes.len() as u64,
            assets,
            title,
            documents: metas
                .iter()
                .map(|meta| index.document_summary(store, meta))
                .collect(),
            files: 0,
            warnings,
            dry_run: request.dry_run,
        })
    }
}
