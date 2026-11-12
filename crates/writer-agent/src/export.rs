//! Export of a document, or of a folder as one file, to a path on disk.

use std::path::{Path, PathBuf};

use writer_core::Document;
use writer_export::{ExportOptions, ExportSource};
use writer_store::{DocumentMeta, FolderNode};

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

fn store_format(format: ExportFormat) -> writer_export::ExportFormat {
    match format {
        ExportFormat::Markdown => writer_export::ExportFormat::Markdown,
        ExportFormat::Docx => writer_export::ExportFormat::Docx,
        ExportFormat::PlainText => writer_export::ExportFormat::PlainText,
    }
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
        let format = request
            .format
            .or_else(|| {
                request
                    .path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .and_then(ExportFormat::parse)
            })
            .unwrap_or_default();
        let path: PathBuf = if request.path.is_absolute() {
            request.path.clone()
        } else {
            std::env::current_dir()
                .map_err(io(&request.path))?
                .join(&request.path)
        };
        if path.is_dir() {
            return Err(Error::invalid(format!(
                "{} is a directory; name the file to write",
                path.display()
            )));
        }
        if path.exists() && !request.overwrite {
            return Err(Error::FileExists { path });
        }

        let session = self.reader()?;
        let store = &session.store;
        let index = Index::load(store)?;
        let (title, metas): (String, Vec<DocumentMeta>) = match targets {
            (Some(target), _) => {
                let meta = index.document(store, target)?;
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
                if metas.is_empty() {
                    return Err(Error::invalid(
                        "there is no document in that folder, so there is nothing to export",
                    ));
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
        let mut options = ExportOptions::default();
        options.docx.created = metas.iter().map(|meta| meta.created_at).min().unwrap_or(0);
        options.docx.modified = metas.iter().map(|meta| meta.updated_at).max().unwrap_or(0);
        let bytes = writer_export::export(&source, store_format(format), &options);

        if !request.dry_run {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent).map_err(io(parent))?;
            }
            // Beside the file first, then in its place: a reader never sees half of it.
            let mut partial = path.clone().into_os_string();
            partial.push(format!(".partial-{}", std::process::id()));
            let partial = PathBuf::from(partial);
            std::fs::write(&partial, &bytes).map_err(io(&partial))?;
            if let Err(error) = std::fs::rename(&partial, &path) {
                std::fs::remove_file(&partial).ok();
                return Err(io(&path)(error));
            }
        }
        Ok(Exported {
            path,
            format,
            bytes: bytes.len() as u64,
            title,
            documents: metas
                .iter()
                .map(|meta| index.document_summary(store, meta))
                .collect(),
            dry_run: request.dry_run,
        })
    }
}
