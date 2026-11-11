//! The export flow (`cmd-shift-s`).
//!
//! A picker offers the current document as a Markdown file, a Word document
//! or a plain text file, copying it as Markdown or plain text, and the
//! document's whole folder as one file. File exports then go through the
//! native save panel with a suggested name; the result (or the error) is
//! reported in a toast.
//!
//! This module is the only place that talks to `writer-export`.

use std::path::{Path, PathBuf};

use gpui::{App, AppContext as _, ClipboardItem, Context, SharedString, WeakEntity, Window};
use writer_core::Document;
use writer_export::{ExportFormat, ExportOptions, ExportSource, suggested_file_name};
use writer_library::{DocumentMeta, FolderNode};
use writer_ui::{IconName, Picker, PickerDelegate, PickerEntry, PickerItem, Toast};

use crate::{
    document_io::load_document,
    settings::keys,
    workspace::{Workspace, session_title},
};

/// The export picker as a modal view.
pub type ExportPicker = Picker<ExportDelegate>;

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
    /// How many documents it holds.
    pub documents: usize,
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Header(SharedString),
    Choice(ExportChoice),
}

/// The picker delegate behind the export picker.
pub struct ExportDelegate {
    workspace: WeakEntity<Workspace>,
    folder: String,
    rows: Vec<Row>,
}

impl ExportDelegate {
    /// `folder` names the open document's folder in the second group.
    pub fn new(workspace: WeakEntity<Workspace>, folder: String) -> Self {
        Self {
            workspace,
            folder,
            rows: Vec::new(),
        }
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
        let document = matching(ExportScope::Document);
        if !document.is_empty() {
            self.rows.push(Row::Header("This document".into()));
            self.rows.extend(document);
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
        let workspace = self.workspace.clone();
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |workspace, cx| workspace.export(choice, window, cx))
                .ok();
        });
        false
    }

    fn placeholder(&self) -> SharedString {
        "Export".into()
    }

    fn empty_text(&self, _query: &str) -> SharedString {
        "No export format matches".into()
    }
}

/// The name shown for the top level of the library.
pub const LIBRARY_ROOT_NAME: &str = "Library";

fn collect_folder(node: &FolderNode, out: &mut Vec<DocumentMeta>) {
    out.extend(node.documents.iter().cloned());
    for child in &node.folders {
        collect_folder(child, out);
    }
}

fn export_options(created: i64, modified: i64) -> ExportOptions {
    let mut options = ExportOptions::default();
    options.docx.created = created;
    options.docx.modified = modified;
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

    /// Builds the bytes of an export of the open document or its folder.
    /// The open document is saved first, so a folder export reads current
    /// text for every document.
    pub fn prepare_export(
        &mut self,
        scope: ExportScope,
        format: ExportFormat,
        cx: &mut Context<Self>,
    ) -> Result<PreparedExport, String> {
        let Some(id) = self.open_document_id() else {
            return Err("No document is open".to_string());
        };
        self.flush(cx);
        let library = self.library.read(cx);
        let meta = library
            .document_meta(id)
            .map_err(|error| error.to_string())?;

        match scope {
            ExportScope::Document => {
                // The live text, not the stored copy: correct even while a
                // save is failing.
                let document = self.editor.read(cx).document();
                let title = session_title(&meta.title);
                let source = ExportSource::single(title.clone(), document);
                let options = export_options(meta.created_at, meta.updated_at);
                Ok(PreparedExport {
                    file_name: suggested_file_name(source.title(), format),
                    title,
                    format,
                    bytes: writer_export::export(&source, format, &options),
                    documents: 1,
                })
            }
            ExportScope::Folder => {
                let tree = library.tree();
                let (name, metas) = match meta.folder_id {
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
                    let document = if part.id == id {
                        self.editor.read(cx).document().clone()
                    } else {
                        let stored = library
                            .document(part.id)
                            .map_err(|error| error.to_string())?;
                        load_document(&stored.content).document
                    };
                    parts.push((session_title(&part.title), document));
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
                );
                Ok(PreparedExport {
                    file_name: suggested_file_name(source.title(), format),
                    title: name,
                    format,
                    bytes: writer_export::export(
                        &source,
                        format,
                        &export_options(created, modified),
                    ),
                    documents: parts.len(),
                })
            }
        }
    }

    /// Runs one export choice: to the clipboard right away, or through the
    /// save panel into a file. The outcome is reported in a toast.
    pub fn export(&mut self, choice: ExportChoice, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_modal(window, cx);
        let prepared = match self.prepare_export(choice.scope, choice.format, cx) {
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
                    let bytes = prepared.bytes;
                    let written = cx
                        .background_spawn({
                            let path = path.clone();
                            async move { std::fs::write(&path, bytes) }
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
                            this.show_toast(
                                Toast::new(format!("Exported {what} to {}", display_path(&path)))
                                    .success(),
                                cx,
                            );
                        }
                        Err(error) => this.toast_error(
                            format!("Could not write {}: {error}", display_path(&path)),
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
    fn export_directory(&self, cx: &App) -> PathBuf {
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
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}
