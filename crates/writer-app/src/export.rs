//! The export flow (`cmd-shift-s`).
//!
//! A picker offers the current document as a Markdown file, a Word document
//! or a plain text file, copying it as Markdown or plain text, and the
//! document's whole folder as one file. File exports then go through the
//! native save panel with a suggested name; the result (or the error) is
//! reported in a toast.
//!
//! # Images
//!
//! The exporters get the pictures of the library
//! ([`crate::images::StoreImages`]). A Word document holds its pictures. A
//! **Markdown file** links them: they are written as files into a folder
//! beside it, named after the file as it was suggested
//! (`Chapter 2.md` and `Chapter 2 images/image-3f2a9c1b.png`;
//! [`PreparedExport::assets`]); when the file is given another name in
//! the save panel the folder keeps its own, and the links, which are
//! relative, stay right. **Markdown for the clipboard** names each image
//! as the library does (`![description](image:<id>)`): there is no place
//! to put a file. Plain text has the description of each image on a line
//! of its own. An export whose pictures cannot be read from the library
//! (`images.db` of a newer version, busy past its timeout) fails and
//! writes nothing ([`pictures_not_read`]); a picture the library does not
//! have is exported as its description.
//!
//! This module is the only place that exports with `writer-export`.

use std::path::{Path, PathBuf};

use gpui::{App, AppContext as _, ClipboardItem, Context, SharedString, WeakEntity, Window};
use writer_core::Document;
use writer_export::{
    ExportAsset, ExportFormat, ExportOptions, ExportSource, MarkdownImages, export_package,
    images_dir_name, suggested_file_name,
};
use writer_library::{DocumentId, DocumentMeta, FolderId, FolderNode};
use writer_ui::{IconName, Picker, PickerDelegate, PickerEntry, PickerItem, Toast};

use crate::{
    document_io::load_document,
    images::StoreImages,
    settings::keys,
    workspace::{Workspace, session_title},
};

/// The export picker as a modal view.
pub type ExportPicker = Picker<ExportDelegate>;

/// What the export picker was opened for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExportTarget {
    /// The open document and its folder (`cmd-shift-s`).
    #[default]
    Open,
    /// A document of the library and its folder: "Export…" on its row.
    Document(DocumentId),
    /// A folder of the library, as one file: "Export Folder…" on its row.
    Folder(FolderId),
}

/// What is exported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportScope {
    /// The open document.
    Document,
    /// Every document of the open document's folder (subfolders included),
    /// as one file. For a document at the top level: the top-level
    /// documents.
    Folder,
}

/// Where the export goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportDestination {
    /// A file chosen in the save panel.
    File,
    /// The clipboard (text formats only).
    Clipboard,
}

/// One row of the export picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportChoice {
    /// What is exported.
    pub scope: ExportScope,
    /// In which format.
    pub format: ExportFormat,
    /// Where it goes.
    pub destination: ExportDestination,
}

impl ExportChoice {
    /// The choices offered, in menu order.
    pub const ALL: [ExportChoice; 8] = [
        ExportChoice::new(
            ExportScope::Document,
            ExportFormat::Markdown,
            ExportDestination::File,
        ),
        ExportChoice::new(
            ExportScope::Document,
            ExportFormat::Docx,
            ExportDestination::File,
        ),
        ExportChoice::new(
            ExportScope::Document,
            ExportFormat::PlainText,
            ExportDestination::File,
        ),
        ExportChoice::new(
            ExportScope::Document,
            ExportFormat::Markdown,
            ExportDestination::Clipboard,
        ),
        ExportChoice::new(
            ExportScope::Document,
            ExportFormat::PlainText,
            ExportDestination::Clipboard,
        ),
        ExportChoice::new(
            ExportScope::Folder,
            ExportFormat::Markdown,
            ExportDestination::File,
        ),
        ExportChoice::new(
            ExportScope::Folder,
            ExportFormat::Docx,
            ExportDestination::File,
        ),
        ExportChoice::new(
            ExportScope::Folder,
            ExportFormat::PlainText,
            ExportDestination::File,
        ),
    ];

    /// A choice.
    pub const fn new(
        scope: ExportScope,
        format: ExportFormat,
        destination: ExportDestination,
    ) -> Self {
        Self {
            scope,
            format,
            destination,
        }
    }

    /// The row's text.
    pub fn label(&self) -> &'static str {
        match (self.destination, self.format) {
            (ExportDestination::File, ExportFormat::Markdown) => "Markdown file",
            (ExportDestination::File, ExportFormat::Docx) => "Word document",
            (ExportDestination::File, ExportFormat::PlainText) => "Plain text file",
            (ExportDestination::Clipboard, ExportFormat::Markdown) => "Copy as Markdown",
            (ExportDestination::Clipboard, _) => "Copy as plain text",
        }
    }

    fn detail(&self) -> &'static str {
        match self.destination {
            ExportDestination::File => match self.format {
                ExportFormat::Markdown => ".md",
                ExportFormat::Docx => ".docx",
                ExportFormat::PlainText => ".txt",
            },
            ExportDestination::Clipboard => "Clipboard",
        }
    }
}

/// An export ready to be written: the bytes, and what to call them.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedExport {
    /// The title of what was exported (document or folder).
    pub title: String,
    /// A safe file name with the format's extension.
    pub file_name: String,
    /// The format.
    pub format: ExportFormat,
    /// The file's bytes (UTF-8 text for Markdown and plain text).
    pub bytes: Vec<u8>,
    /// The files that go beside it: the pictures a Markdown file links,
    /// each at its path relative to the folder of the file
    /// (`Chapter 2 images/image-3f2a9c1b.png`). Empty for every other
    /// export, and for Markdown that goes to the clipboard.
    pub assets: Vec<ExportAsset>,
    /// How many documents it holds.
    pub documents: usize,
}

impl PreparedExport {
    /// The folder the pictures of this export go to, beside the file:
    /// `None` when it has none.
    pub fn pictures_folder(&self) -> Option<&str> {
        let first = self.assets.first()?;
        first.path.split('/').next()
    }
}

/// The sentence a toast adds for an export that wrote `pictures` picture
/// files into the folder `folder` beside the file, for an export of
/// `documents` documents.
pub fn pictures_sentence(pictures: usize, folder: &str, documents: usize) -> String {
    let whose = if documents == 1 { "Its" } else { "Their" };
    if pictures == 1 {
        format!("{whose} 1 picture is in \u{201c}{folder}\u{201d}.")
    } else {
        format!("{whose} {pictures} pictures are in \u{201c}{folder}\u{201d}.")
    }
}

/// Writes an exported file and, after it, the files that go beside it
/// (each folder is made when it is not there).
fn write_export(path: &Path, bytes: &[u8], assets: &[ExportAsset]) -> std::io::Result<()> {
    std::fs::write(path, bytes)?;
    let folder = path.parent().unwrap_or_else(|| Path::new("."));
    for asset in assets {
        // The path is relative and has `/` between its parts.
        let target = asset
            .path
            .split('/')
            .filter(|part| !part.is_empty() && *part != "." && *part != "..")
            .fold(folder.to_path_buf(), |target, part| target.join(part));
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, &asset.bytes)?;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Header(SharedString),
    Choice(ExportChoice),
}

/// The picker delegate behind the export picker.
pub struct ExportDelegate {
    workspace: WeakEntity<Workspace>,
    target: ExportTarget,
    /// The heading of the first group: the document. `None` when only a
    /// folder is on offer.
    document: Option<String>,
    folder: String,
    rows: Vec<Row>,
}

impl ExportDelegate {
    /// The picker for the open document; `folder` names its folder in the
    /// second group.
    pub fn new(workspace: WeakEntity<Workspace>, folder: String) -> Self {
        Self::for_target(
            workspace,
            ExportTarget::Open,
            Some("This document".to_string()),
            folder,
        )
    }

    /// The picker for `target`. `document` is the heading of the group
    /// that exports the document (`None`: the target is a folder, and only
    /// the group for the whole folder is listed); `folder` names the
    /// folder.
    pub fn for_target(
        workspace: WeakEntity<Workspace>,
        target: ExportTarget,
        document: Option<String>,
        folder: String,
    ) -> Self {
        Self {
            workspace,
            target,
            document,
            folder,
            rows: Vec::new(),
        }
    }

    /// What the picker exports.
    pub fn target(&self) -> ExportTarget {
        self.target
    }

    /// The listed choices, in order (headers excluded).
    pub fn choices(&self) -> Vec<ExportChoice> {
        self.rows
            .iter()
            .filter_map(|row| match row {
                Row::Choice(choice) => Some(*choice),
                Row::Header(_) => None,
            })
            .collect()
    }
}

impl PickerDelegate for ExportDelegate {
    fn update_matches(&mut self, query: &str, _: &mut Window, _: &mut Context<Picker<Self>>) {
        let query = query.trim();
        let matching = |scope: ExportScope| {
            ExportChoice::ALL
                .into_iter()
                .filter(move |choice| choice.scope == scope)
                .filter(|choice| {
                    query.is_empty()
                        || crate::palette::words_match(
                            query,
                            &format!("{} {}", choice.label(), choice.detail()),
                        )
                })
                .map(Row::Choice)
                .collect::<Vec<_>>()
        };
        self.rows.clear();
        if let Some(heading) = &self.document {
            let document = matching(ExportScope::Document);
            if !document.is_empty() {
                self.rows.push(Row::Header(heading.clone().into()));
                self.rows.extend(document);
            }
        }
        let folder = matching(ExportScope::Folder);
        if !folder.is_empty() {
            self.rows.push(Row::Header(
                format!("Whole folder “{}” as one file", self.folder).into(),
            ));
            self.rows.extend(folder);
        }
    }

    fn entry_count(&self) -> usize {
        self.rows.len()
    }

    fn entry(&self, ix: usize, _: &App) -> PickerEntry {
        match &self.rows[ix] {
            Row::Header(title) => PickerEntry::Header(title.clone()),
            Row::Choice(choice) => PickerItem::new(choice.label())
                .detail(choice.detail())
                .icon(match (choice.scope, choice.destination) {
                    (_, ExportDestination::Clipboard) => IconName::Page,
                    (ExportScope::Folder, _) => IconName::Folder,
                    (ExportScope::Document, _) => IconName::Export,
                })
                .into(),
        }
    }

    fn is_selectable(&self, ix: usize, _: &App) -> bool {
        matches!(self.rows.get(ix), Some(Row::Choice(_)))
    }

    fn confirm(
        &mut self,
        ix: usize,
        _secondary: bool,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> bool {
        let Some(Row::Choice(choice)) = self.rows.get(ix) else {
            return false;
        };
        let choice = *choice;
        let target = self.target;
        let workspace = self.workspace.clone();
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |workspace, cx| {
                    workspace.export_target(target, choice, window, cx)
                })
                .ok();
        });
        false
    }

    fn placeholder(&self) -> SharedString {
        "Export".into()
    }

    fn empty_text(&self, _query: &str) -> SharedString {
        "No export formats match".into()
    }
}

/// The name shown for the top level of the library.
pub const LIBRARY_ROOT_NAME: &str = "Library";

/// The words of an export whose pictures could not be read from the
/// library: nothing is written then, as `writer export` does
/// (`writer_agent::Error::Pictures`), so no file goes out without the
/// pictures the library has. After "Could not export: ".
pub fn pictures_not_read(error: &str) -> String {
    format!("its pictures could not be read from the library ({error}), so nothing was written")
}

/// Fails when a picture of the export met an error of the store.
fn pictures_were_read(images: &StoreImages<'_>) -> Result<(), String> {
    match images.failure() {
        Some(error) => Err(pictures_not_read(&error)),
        None => Ok(()),
    }
}

fn collect_folder(node: &FolderNode, out: &mut Vec<DocumentMeta>) {
    out.extend(node.documents.iter().cloned());
    for child in &node.folders {
        collect_folder(child, out);
    }
}

/// The options of an export that is called `file_name` and goes to
/// `destination`: a Markdown file links its pictures in the folder beside
/// it, everything else names them as the library does.
fn export_options(
    created: i64,
    modified: i64,
    file_name: &str,
    destination: ExportDestination,
) -> ExportOptions {
    let mut options = ExportOptions::default();
    options.docx.created = created;
    options.docx.modified = modified;
    if destination == ExportDestination::File {
        options.markdown.images = MarkdownImages::Files {
            dir: images_dir_name(file_name),
        };
    }
    options
}

impl Workspace {
    /// The name of the open document's folder ([`LIBRARY_ROOT_NAME`] at the
    /// top level).
    pub fn export_folder_name(&self, cx: &App) -> String {
        let library = self.library.read(cx);
        self.open_document_id()
            .and_then(|id| library.document_meta(id).ok())
            .and_then(|meta| meta.folder_id)
            .and_then(|folder| library.folder(folder).ok())
            .map(|folder| folder.name)
            .unwrap_or_else(|| LIBRARY_ROOT_NAME.to_string())
    }

    /// The name of folder `folder` ([`LIBRARY_ROOT_NAME`] for the top level
    /// and for a folder that is gone).
    fn folder_name(&self, folder: Option<FolderId>, cx: &App) -> String {
        let library = self.library.read(cx);
        folder
            .and_then(|folder| library.folder(folder).ok())
            .map(|folder| folder.name)
            .unwrap_or_else(|| LIBRARY_ROOT_NAME.to_string())
    }

    /// Opens the export picker for `target`: the open document
    /// (`cmd-shift-s`), another document of the library, or a folder as
    /// one file (the last two from the menu of a library row). The open
    /// document stays the open one.
    pub fn show_export(
        &mut self,
        target: ExportTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A row of the open document is the open document.
        let target = match target {
            ExportTarget::Document(id) if self.open_document_id() == Some(id) => ExportTarget::Open,
            other => other,
        };
        let (document, folder) = match target {
            ExportTarget::Open => {
                if self.session.is_none() {
                    self.show_toast(Toast::new("Open a document to export it"), cx);
                    return;
                }
                (
                    Some("This document".to_string()),
                    self.export_folder_name(cx),
                )
            }
            ExportTarget::Document(id) => {
                let meta = match self.library.read(cx).document_meta(id) {
                    Ok(meta) => meta,
                    Err(error) => {
                        self.toast_error(format!("Could not export: {error}"), cx);
                        return;
                    }
                };
                (
                    Some(format!(
                        "\u{201c}{}\u{201d}",
                        short_title(&session_title(&meta.title))
                    )),
                    self.folder_name(meta.folder_id, cx),
                )
            }
            ExportTarget::Folder(id) => (None, self.folder_name(Some(id), cx)),
        };
        let workspace = cx.weak_entity();
        self.modal_layer.update(cx, |layer, cx| {
            let picker = cx.new(|cx| {
                let delegate = ExportDelegate::for_target(workspace, target, document, folder);
                let rows = crate::workspace::picker_rows(window, 12);
                Picker::modal(delegate, window, cx)
                    .width(gpui::px(crate::workspace::MODAL_WIDTH))
                    .max_visible_rows(rows)
            });
            layer.show(picker, window, cx);
        });
    }

    /// Builds the bytes of an export of the open document or its folder.
    /// The open document is saved first, so a folder export reads current
    /// text for every document.
    pub fn prepare_export(
        &mut self,
        scope: ExportScope,
        format: ExportFormat,
        cx: &mut Context<Self>,
    ) -> Result<PreparedExport, String> {
        self.prepare_export_of(ExportTarget::Open, scope, format, cx)
    }

    /// Builds the bytes of an export of `target` as a file: the document
    /// it names (`scope` [`ExportScope::Document`]) or the folder it names
    /// or its document is in (`scope` [`ExportScope::Folder`]). The open
    /// document is saved first and exported as it is on screen; every
    /// other document is read from the library. The pictures of a
    /// Markdown file come with it ([`PreparedExport::assets`]).
    pub fn prepare_export_of(
        &mut self,
        target: ExportTarget,
        scope: ExportScope,
        format: ExportFormat,
        cx: &mut Context<Self>,
    ) -> Result<PreparedExport, String> {
        self.prepare_export_for(target, scope, format, ExportDestination::File, cx)
    }

    /// [`Workspace::prepare_export_of`] for where the export goes: for
    /// the clipboard Markdown names its images as the library does and
    /// nothing goes beside it.
    pub fn prepare_export_for(
        &mut self,
        target: ExportTarget,
        scope: ExportScope,
        format: ExportFormat,
        destination: ExportDestination,
        cx: &mut Context<Self>,
    ) -> Result<PreparedExport, String> {
        let open = self.open_document_id();
        let subject = match target {
            ExportTarget::Open => Some(open.ok_or_else(|| "No document is open".to_string())?),
            ExportTarget::Document(id) => Some(id),
            ExportTarget::Folder(_) => None,
        };
        self.flush(cx);
        let library = self.library.read(cx);
        let images = StoreImages::new(library.store());
        let meta = subject
            .map(|id| library.document_meta(id).map_err(|error| error.to_string()))
            .transpose()?;
        // The live text of the open document, not the stored copy: correct
        // even while a save is failing.
        let text_of = |id: DocumentId| -> Result<Document, String> {
            if Some(id) == open {
                Ok(self.editor.read(cx).document().clone())
            } else {
                let stored = library.document(id).map_err(|error| error.to_string())?;
                Ok(load_document(&stored.content).document)
            }
        };

        match (scope, meta) {
            (ExportScope::Document, None) => Err("A folder is exported as a whole".to_string()),
            (ExportScope::Document, Some(meta)) => {
                let document = text_of(meta.id)?;
                let title = session_title(&meta.title);
                let source = ExportSource::single(title.clone(), &document).with_images(&images);
                let file_name = suggested_file_name(source.title(), format);
                let options =
                    export_options(meta.created_at, meta.updated_at, &file_name, destination);
                let exported = export_package(&source, format, &options);
                pictures_were_read(&images)?;
                Ok(PreparedExport {
                    file_name,
                    title,
                    format,
                    bytes: exported.bytes,
                    assets: exported.assets,
                    documents: 1,
                })
            }
            (ExportScope::Folder, meta) => {
                let folder = match target {
                    ExportTarget::Folder(id) => Some(id),
                    _ => meta.and_then(|meta| meta.folder_id),
                };
                let tree = library.tree();
                let (name, metas) = match folder {
                    Some(folder) => {
                        let node = tree
                            .find_folder(folder)
                            .ok_or_else(|| "The folder no longer exists".to_string())?;
                        let mut metas = Vec::new();
                        collect_folder(node, &mut metas);
                        (node.folder.name.clone(), metas)
                    }
                    None => (LIBRARY_ROOT_NAME.to_string(), tree.documents.clone()),
                };
                let mut parts: Vec<(String, Document)> = Vec::with_capacity(metas.len());
                for part in &metas {
                    parts.push((session_title(&part.title), text_of(part.id)?));
                }
                if parts.is_empty() {
                    return Err("The folder has no documents".to_string());
                }
                let created = metas.iter().map(|m| m.created_at).min().unwrap_or(0);
                let modified = metas.iter().map(|m| m.updated_at).max().unwrap_or(0);
                let source = ExportSource::collection(
                    name.clone(),
                    parts
                        .iter()
                        .map(|(title, document)| (title.as_str(), document)),
                )
                .with_images(&images);
                let file_name = suggested_file_name(source.title(), format);
                let options = export_options(created, modified, &file_name, destination);
                let exported = export_package(&source, format, &options);
                pictures_were_read(&images)?;
                Ok(PreparedExport {
                    file_name,
                    title: name,
                    format,
                    bytes: exported.bytes,
                    assets: exported.assets,
                    documents: parts.len(),
                })
            }
        }
    }

    /// Runs one export choice: to the clipboard right away, or through the
    /// save panel into a file. The outcome is reported in a toast.
    pub fn export(&mut self, choice: ExportChoice, window: &mut Window, cx: &mut Context<Self>) {
        self.export_target(ExportTarget::Open, choice, window, cx);
    }

    /// Like [`Workspace::export`], for what the picker was opened for.
    pub fn export_target(
        &mut self,
        target: ExportTarget,
        choice: ExportChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_modal(window, cx);
        let prepared = match self.prepare_export_for(
            target,
            choice.scope,
            choice.format,
            choice.destination,
            cx,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.toast_error(format!("Could not export: {error}"), cx);
                return;
            }
        };

        match choice.destination {
            ExportDestination::Clipboard => match String::from_utf8(prepared.bytes) {
                Ok(text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    self.show_toast(
                        Toast::new(format!(
                            "Copied “{}” as {}",
                            short_title(&prepared.title),
                            match choice.format {
                                ExportFormat::Markdown => "Markdown",
                                _ => "plain text",
                            }
                        ))
                        .success(),
                        cx,
                    );
                }
                Err(_) => self.toast_error("Could not export: this format is not text", cx),
            },
            ExportDestination::File => {
                // A scripted run cannot drive the native save panel: it
                // names the directory instead and the suggested file name
                // is used as it is.
                let scripted = crate::automation::export_directory();
                let receiver = scripted.is_none().then(|| {
                    let directory = self.export_directory(cx);
                    cx.prompt_for_new_path(&directory, Some(&prepared.file_name))
                });
                cx.spawn_in(window, async move |this, cx| {
                    let chosen = match (scripted, receiver) {
                        (Some(directory), _) => directory.join(&prepared.file_name),
                        (None, Some(receiver)) => match receiver.await {
                            Ok(Ok(Some(path))) => path,
                            // Cancelled.
                            Ok(Ok(None)) | Err(_) => return,
                            Ok(Err(error)) => {
                                this.update(cx, |this, cx| {
                                    this.toast_error(
                                        format!("Could not open the save panel: {error}"),
                                        cx,
                                    );
                                })
                                .ok();
                                return;
                            }
                        },
                        (None, None) => return,
                    };
                    let path = with_extension(chosen, prepared.format);
                    // The pictures of a Markdown file: how many, and the
                    // folder beside the file they go to.
                    let pictures = prepared
                        .pictures_folder()
                        .map(|folder| (prepared.assets.len(), folder.to_string()));
                    let bytes = prepared.bytes;
                    let assets = prepared.assets;
                    let written = cx
                        .background_spawn({
                            let path = path.clone();
                            async move { write_export(&path, &bytes, &assets) }
                        })
                        .await;
                    this.update(cx, |this, cx| match written {
                        Ok(()) => {
                            if let Some(parent) = path.parent() {
                                let parent = parent.to_string_lossy().to_string();
                                this.library
                                    .update(cx, |library, cx| {
                                        library.set_setting(keys::EXPORT_DIRECTORY, &parent, cx)
                                    })
                                    .ok();
                            }
                            let what = if prepared.documents == 1 {
                                format!("“{}”", short_title(&prepared.title))
                            } else {
                                format!("{} documents", prepared.documents)
                            };
                            let mut said = format!("Exported {what} to {}", short_path(&path));
                            if let Some((count, folder)) = &pictures {
                                said.push_str(". ");
                                said.push_str(&pictures_sentence(
                                    *count,
                                    folder,
                                    prepared.documents,
                                ));
                            }
                            this.show_toast(Toast::new(said).success(), cx);
                        }
                        Err(error) => this.toast_error(
                            format!("Could not write {}: {error}", short_path(&path)),
                            cx,
                        ),
                    })
                    .ok();
                })
                .detach();
            }
        }
    }

    /// Where the save panel starts: the directory of the last export, else
    /// `~/Documents`, else the home directory.
    pub(crate) fn export_directory(&self, cx: &App) -> PathBuf {
        let stored: String = self
            .library
            .read(cx)
            .setting_or(keys::EXPORT_DIRECTORY, String::new());
        let home = std::env::var_os("HOME").map(PathBuf::from);
        [
            Some(PathBuf::from(stored)).filter(|path| !path.as_os_str().is_empty()),
            home.as_ref().map(|home| home.join("Documents")),
            home,
        ]
        .into_iter()
        .flatten()
        .find(|path| path.is_dir())
        .unwrap_or_else(|| PathBuf::from("/"))
    }
}

/// A title cut to fit a one-line notice.
fn short_title(title: &str) -> String {
    const MAX_CHARS: usize = 40;
    if title.chars().count() <= MAX_CHARS {
        return title.to_string();
    }
    let cut: String = title.chars().take(MAX_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

/// Makes sure the chosen path ends with the format's extension (the save
/// panel lets the user type any name).
pub fn with_extension(path: PathBuf, format: ExportFormat) -> PathBuf {
    let matches = path
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(ExportFormat::from_extension)
        == Some(format);
    if matches {
        path
    } else {
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(".");
        name.push(format.extension());
        path.with_file_name(name)
    }
}

/// The path with the home directory shortened to `~`.
pub fn display_path(path: &Path) -> String {
    display_path_from(path, std::env::var_os("HOME").map(PathBuf::from).as_deref())
}

/// [`display_path`] for the home directory `home`.
pub fn display_path_from(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home
        && let Ok(rest) = path.strip_prefix(home)
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

/// How long a path may be in a sentence that has to fit on the lines of a
/// toast, in characters: with the words before it ("Exported the keyboard
/// shortcuts to") it stays on two lines.
pub const SHORT_PATH_CHARS: usize = 56;

/// What stands for the folders a shortened path leaves out.
const LEFT_OUT: &str = "\u{2026}";

/// The path for a sentence that has to fit (a toast): the home directory
/// as `~`, and a path longer than [`SHORT_PATH_CHARS`] shortened in the
/// middle. The file name is always whole; before it stand the folders
/// nearest to the file, as many as fit, and before those the folder the
/// path begins with, so that both ends can be read:
/// `~/Documents/…/Exports/The Writer Shortcuts.json`. A file name that is
/// too long by itself is given with `…/` before it.
pub fn short_path(path: &Path) -> String {
    shorten_path(&display_path(path), SHORT_PATH_CHARS)
}

/// [`short_path`] for a path as text (`/` between its parts, `~` for the
/// home directory) and a limit in characters.
pub fn shorten_path(shown: &str, limit: usize) -> String {
    let length = |text: &str| text.chars().count();
    if length(shown) <= limit {
        return shown.to_string();
    }
    let mut parts: Vec<&str> = shown.split('/').collect();
    let Some(file) = parts.pop().filter(|file| !file.is_empty()) else {
        return shown.to_string();
    };
    // Where the path begins: `~` or the root, and the first folder.
    let head = match parts.as_slice() {
        [start, first, ..] => format!("{start}/{first}"),
        _ => {
            return shown.to_string();
        }
    };
    // The folders between the first one and the file: the ones nearest
    // to the file are kept, as many as fit.
    let folders = &parts[2..];
    let mut kept: Vec<&str> = Vec::new();
    for folder in folders.iter().rev() {
        let mut next = vec![*folder];
        next.extend(&kept);
        let candidate = format!("{head}/{LEFT_OUT}/{}/{file}", next.join("/"));
        if length(&candidate) > limit {
            break;
        }
        kept = next;
    }
    let shortened = if kept.is_empty() {
        format!("{head}/{LEFT_OUT}/{file}")
    } else {
        format!("{head}/{LEFT_OUT}/{}/{file}", kept.join("/"))
    };
    if !folders.is_empty() && length(&shortened) <= limit {
        shortened
    } else {
        // Nothing to leave out between the two ends, or the file name is
        // long by itself: the file name, whole.
        format!("{LEFT_OUT}/{file}")
    }
}
