//! The export of a page (`docs/pages-design.md`, section 7): "Export…" on a
//! page offers two rows instead of Markdown, DOCX and text.
//!
//! - **Web page folder (opens offline)**: the served files at their paths
//!   in a folder named after the title (`writer_page::write_folder`, the
//!   same as `writer export <page> <dir>`). A folder that exists gets the
//!   files written into it, the same names replaced.
//! - **PDF (as printed)**: the page loaded fresh, as it was made, in an
//!   offscreen view, and WebKit's print operation on the user's default
//!   paper (`writer_web::print_offscreen`); the fake engine writes its fixed
//!   page. Not offered without a page engine.
//!
//! The save panel asks where; a scripted run writes into
//! `THE_WRITER_EXPORT_DIR` instead. The bytes are read through a read-only
//! connection of their own to `images.db`, off the main thread.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

use gpui::{App, AppContext as _, Context, SharedString, WeakEntity, Window};
use writer_library::{
    DocumentId,
    store::{FileId, ImageFile},
};
use writer_page::{Manifest, Which};
use writer_ui::{IconName, Picker, PickerDelegate, PickerEntry, PickerItem, Toast};

use crate::{
    export::short_path,
    pages::{self, PageEngine},
    settings::keys,
    workspace::{Workspace, session_title},
};

/// The export picker of a page as a modal view.
pub type PageExportPicker = Picker<PageExportDelegate>;

/// One row of the export of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageExportChoice {
    /// The served files as a folder that opens offline.
    Folder,
    /// A PDF, as WebKit prints the page.
    Pdf,
}

impl PageExportChoice {
    /// The row's text.
    pub fn label(self) -> &'static str {
        match self {
            PageExportChoice::Folder => "Web page folder (opens offline)",
            PageExportChoice::Pdf => "PDF (as printed)",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            PageExportChoice::Folder => "Folder",
            PageExportChoice::Pdf => ".pdf",
        }
    }
}

/// The picker delegate of the export of a page.
pub struct PageExportDelegate {
    workspace: WeakEntity<Workspace>,
    page: DocumentId,
    heading: String,
    choices: Vec<PageExportChoice>,
    rows: Vec<PageExportChoice>,
}

impl PageExportDelegate {
    /// The picker for page `page` titled `title`; `pdf`: whether the PDF row
    /// is offered (a page engine is there).
    pub fn new(workspace: WeakEntity<Workspace>, page: DocumentId, title: &str, pdf: bool) -> Self {
        let mut choices = vec![PageExportChoice::Folder];
        if pdf {
            choices.push(PageExportChoice::Pdf);
        }
        Self {
            workspace,
            page,
            heading: format!("\u{201c}{title}\u{201d}"),
            rows: choices.clone(),
            choices,
        }
    }

    /// The listed choices, in order.
    pub fn choices(&self) -> Vec<PageExportChoice> {
        self.rows.clone()
    }

    /// The page it exports.
    pub fn page(&self) -> DocumentId {
        self.page
    }
}

impl PickerDelegate for PageExportDelegate {
    fn update_matches(&mut self, query: &str, _: &mut Window, _: &mut Context<Picker<Self>>) {
        let query = query.trim();
        self.rows = self
            .choices
            .iter()
            .copied()
            .filter(|choice| {
                query.is_empty()
                    || crate::palette::words_match(
                        query,
                        &format!("{} {}", choice.label(), choice.detail()),
                    )
            })
            .collect();
    }

    fn entry_count(&self) -> usize {
        self.rows.len() + usize::from(!self.rows.is_empty())
    }

    fn entry(&self, ix: usize, _: &App) -> PickerEntry {
        if ix == 0 {
            return PickerEntry::Header(self.heading.clone().into());
        }
        let choice = self.rows[ix - 1];
        PickerItem::new(choice.label())
            .detail(choice.detail())
            .icon(match choice {
                PageExportChoice::Folder => IconName::Folder,
                PageExportChoice::Pdf => IconName::Export,
            })
            .into()
    }

    fn is_selectable(&self, ix: usize, _: &App) -> bool {
        ix > 0 && ix <= self.rows.len()
    }

    fn confirm(
        &mut self,
        ix: usize,
        _secondary: bool,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> bool {
        let Some(choice) = ix.checked_sub(1).and_then(|ix| self.rows.get(ix)).copied() else {
            return false;
        };
        let page = self.page;
        let workspace = self.workspace.clone();
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |workspace, cx| {
                    workspace.export_page(page, choice, window, cx)
                })
                .ok();
        });
        false
    }

    fn placeholder(&self) -> SharedString {
        "Export page".into()
    }

    fn empty_text(&self, _query: &str) -> SharedString {
        "No export formats match".into()
    }
}

/// The name of the folder (or, with `.pdf`, the file) an export of a page
/// titled `title` suggests.
pub fn suggested_name(title: &str) -> String {
    let file = writer_export::suggested_file_name(title, writer_export::ExportFormat::PlainText);
    file.strip_suffix(".txt").unwrap_or(&file).to_string()
}

/// Where the bytes of a page's files come from while it is exported: a
/// read-only connection of its own to `images.db` (moved to the background
/// thread that writes), or, for a library in memory, the files read now.
enum Bytes {
    File(ImageFile),
    Read(HashMap<String, Vec<u8>>),
}

impl Bytes {
    fn of(library: &writer_library::Library, manifest: &Manifest) -> Bytes {
        match library.path() {
            Some(database) => Bytes::File(ImageFile::read_only(database)),
            None => Bytes::Read(
                manifest
                    .file_ids()
                    .into_iter()
                    .filter_map(|hex| {
                        let id = FileId::parse(&hex)?;
                        let bytes = library.store().file_bytes(&id).ok().flatten()?;
                        Some((hex, bytes))
                    })
                    .collect(),
            ),
        }
    }

    fn get(&mut self, hex: &str) -> std::io::Result<Option<Vec<u8>>> {
        match self {
            Bytes::File(file) => {
                let Some(id) = FileId::parse(hex) else {
                    return Ok(None);
                };
                file.file_bytes(&id)
                    .map_err(|error| std::io::Error::other(error.to_string()))
            }
            Bytes::Read(read) => Ok(read.get(hex).cloned()),
        }
    }
}

impl Workspace {
    /// Opens the export picker of page `page`.
    pub fn show_page_export(
        &mut self,
        page: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let meta = match self.library.read(cx).document_meta(page) {
            Ok(meta) => meta,
            Err(error) => {
                self.toast_error(format!("Could not export: {error}"), cx);
                return;
            }
        };
        let title = session_title(&meta.title);
        let pdf = pages::engine(cx).is_some();
        let workspace = cx.weak_entity();
        self.modal_layer.update(cx, |layer, cx| {
            let picker = cx.new(|cx| {
                let delegate = PageExportDelegate::new(workspace, page, &title, pdf);
                Picker::modal(delegate, window, cx)
                    .width(gpui::px(crate::workspace::MODAL_WIDTH))
                    .max_visible_rows(4)
            });
            layer.show(picker, window, cx);
        });
    }

    /// Exports page `page` as `choice` says: asks where (the save panel; a
    /// scripted run's export directory), then writes in the background and
    /// says what came of it in a toast.
    pub fn export_page(
        &mut self,
        page: DocumentId,
        choice: PageExportChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_modal(window, cx);
        let stored = match self.library.read(cx).document(page) {
            Ok(stored) => stored,
            Err(error) => {
                self.toast_error(format!("Could not export: {error}"), cx);
                return;
            }
        };
        let manifest = match Manifest::from_json(&stored.content) {
            Ok(manifest) => manifest,
            Err(error) => {
                self.toast_error(format!("Could not export: {}", error.sentence()), cx);
                return;
            }
        };
        let title = session_title(&stored.meta.title);
        let name = match choice {
            PageExportChoice::Folder => suggested_name(&title),
            PageExportChoice::Pdf => format!("{}.pdf", suggested_name(&title)),
        };
        let scripted = crate::automation::export_directory();
        let receiver = scripted.is_none().then(|| {
            let directory = self.export_directory(cx);
            cx.prompt_for_new_path(&directory, Some(&name))
        });
        let library = self.library.read(cx);
        let bytes = Bytes::of(library, &manifest);
        let engine = pages::engine(cx);
        let source = pages::LibraryPageSource::of(library, &manifest);
        let rules_dir = pages::rules_dir_of(library.path())
            .unwrap_or_else(|| std::env::temp_dir().join("the-writer-WebKit"));
        cx.spawn_in(window, async move |this, cx| {
            let chosen = match (scripted, receiver) {
                (Some(directory), _) => directory.join(&name),
                (None, Some(receiver)) => match receiver.await {
                    Ok(Ok(Some(path))) => path,
                    Ok(Ok(None)) | Err(_) => return,
                    Ok(Err(error)) => {
                        this.update(cx, |this, cx| {
                            this.toast_error(format!("Could not open the save panel: {error}"), cx)
                        })
                        .ok();
                        return;
                    }
                },
                (None, None) => return,
            };
            let said = match choice {
                PageExportChoice::Folder => {
                    let entry = manifest.entry.clone();
                    let written = cx
                        .background_spawn({
                            let chosen = chosen.clone();
                            async move {
                                let mut bytes = bytes;
                                writer_page::write_folder(
                                    &manifest,
                                    Which::Served,
                                    &chosen,
                                    true,
                                    &mut |hex| bytes.get(hex),
                                )
                            }
                        })
                        .await;
                    match written {
                        Ok(written) => Ok(folder_sentence(&title, &chosen, &entry, &written)),
                        Err(error) => Err(error.sentence()),
                    }
                }
                PageExportChoice::Pdf => {
                    let chosen = if chosen.extension().is_some_and(|ext| ext == "pdf") {
                        chosen.clone()
                    } else {
                        chosen.with_extension("pdf")
                    };
                    let printed = match engine {
                        Some(PageEngine::WebKit) => {
                            let (tx, rx) = pages::answer();
                            // From this task, outside any update.
                            writer_web::print_offscreen(
                                writer_web::PdfInput {
                                    source,
                                    entry: manifest.entry.clone(),
                                    settle_max: Duration::from_millis(writer_page::SETTLE_MAX_MS),
                                    limit: Duration::from_millis(writer_page::CHECK_LIMIT_MS),
                                    rules_dir,
                                },
                                move |printed| tx.send(printed),
                            );
                            rx.await
                                .unwrap_or_else(|| Err("the print was not finished".to_string()))
                        }
                        Some(PageEngine::Fake(_)) => Ok(crate::workspace::fake_printed()),
                        None => Err("pages are not shown in this run".to_string()),
                    };
                    match printed {
                        Ok(printed) => {
                            let path = chosen.clone();
                            let written = cx
                                .background_spawn(
                                    async move { std::fs::write(&path, &printed.bytes) },
                                )
                                .await;
                            match written {
                                Ok(()) => Ok(format!(
                                    "Exported \u{201c}{title}\u{201d} to {}.",
                                    short_path(&chosen)
                                )),
                                Err(error) => Err(format!(
                                    "{} could not be written: {error}",
                                    short_path(&chosen)
                                )),
                            }
                        }
                        Err(error) => Err(format!("the page could not be printed: {error}")),
                    }
                }
            };
            this.update(cx, |this, cx| match said {
                Ok(said) => {
                    if let Some(parent) = chosen.parent() {
                        let parent = parent.to_string_lossy().to_string();
                        this.library
                            .update(cx, |library, cx| {
                                library.set_setting(keys::EXPORT_DIRECTORY, &parent, cx)
                            })
                            .ok();
                    }
                    this.show_toast(
                        Toast::new(said).success().duration(Duration::from_secs(8)),
                        cx,
                    );
                }
                Err(error) => this.toast_error(format!("Could not export: {error}"), cx),
            })
            .ok();
        })
        .detach();
    }
}

/// What the toast says about a folder written for page `title`.
fn folder_sentence(title: &str, dir: &Path, entry: &str, written: &writer_page::Written) -> String {
    let files = crate::workspace::counted(written.files, "file", "files");
    let mut said = format!(
        "Exported \u{201c}{title}\u{201d} to {} ({files}). Open {entry} in a browser; a few \
         pages need to be served (python3 -m http.server).",
        short_path(dir)
    );
    if !written.missing.is_empty() {
        said.push_str(&format!(
            " {} the library does not have were left out.",
            crate::workspace::counted(written.missing.len(), "file", "files")
        ));
    }
    said
}

/// The folder an export of page `title` writes into, under `directory`.
pub fn folder_in(directory: &Path, title: &str) -> PathBuf {
    directory.join(suggested_name(title))
}
