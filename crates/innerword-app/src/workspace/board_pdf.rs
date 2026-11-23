//! The PDF of a whiteboard (`docs/whiteboard-design.md`, section 5): one code
//! path for Export as PDF…, Print…, a folder's PDF and the scripted
//! `board:pdf:`, from the **stored** scene (never the live canvas, so the
//! zoom, the theme and a selection change nothing).
//!
//! WebKit loads the bundle's export page offscreen under its policy
//! (`innerword_web::board_export_policy`), the glue restores the scene,
//! makes Excalidraw's SVG at scale 1 in light colours with a 32 px margin,
//! and `createPDFWithConfiguration:` makes one page of exactly that size
//! (`innerword_web::run_script`, on the main thread, from a task between two
//! awaits). The fake engine answers a fixed page of the size Rust estimates
//! (`Scene::pdf_size`). An empty whiteboard has no PDF.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use gpui::{App, AsyncApp, Context, Task, WeakEntity};
use innerword_board::Scene;
use innerword_library::DocumentId;

use super::Workspace;
use crate::{boards, pages};

/// How long the export page has for the load, the SVG and the PDF.
pub const BOARD_PDF_LIMIT: Duration = Duration::from_secs(50);

/// What the app says of an empty whiteboard's PDF.
pub fn empty_sentence(title: &str, print: bool) -> String {
    format!(
        "\u{201c}{}\u{201d} is empty: there is nothing to {}.",
        super::session_title(title),
        if print { "print" } else { "export" }
    )
}

/// What a whiteboard's PDF is made from, read on the main thread.
pub(crate) struct BoardPdfInput {
    pub title: String,
    pub scene: Scene,
    /// The export page's source (WebKit); `None` on the fake engine.
    pub source: Option<Arc<dyn innerword_web::PageSource>>,
    pub rules_dir: std::path::PathBuf,
    pub engine: Option<pages::PageEngine>,
}

impl Workspace {
    /// What the PDF of whiteboard `id` is made from, as the library holds it
    /// now. `Err` says why there is none (an empty whiteboard, no engine).
    pub(crate) fn board_pdf_input(
        &mut self,
        id: DocumentId,
        print: bool,
        cx: &mut App,
    ) -> Result<BoardPdfInput, String> {
        let stored = self
            .library
            .read(cx)
            .document(id)
            .map_err(|error| error.to_string())?;
        let scene = Scene::from_stored(&stored.content)
            .map_err(|error| error.to_string())?
            .scene;
        if scene.is_empty() {
            return Err(empty_sentence(&stored.meta.title, print));
        }
        let engine = pages::engine(cx);
        let content: Arc<str> = Arc::from(stored.content.as_str());
        let source = match &engine {
            Some(pages::PageEngine::WebKit) => {
                let assets = boards::assets(cx)?;
                let cell = Arc::new(Mutex::new(boards::BoardCell {
                    stamp: String::new(),
                    scene: content.clone(),
                    view: Arc::from("{}"),
                    library: Arc::from("{}"),
                    files: scene
                        .stored_files()
                        .iter()
                        .map(|hex| hex.as_str().to_string())
                        .collect(),
                }));
                let source: Arc<dyn innerword_web::PageSource> = Arc::new(
                    boards::LibraryBoardSource::new(assets, self.library.read(cx), cell),
                );
                Some(source)
            }
            Some(pages::PageEngine::Fake(_)) => None,
            None => {
                return Err(format!(
                    "the whiteboard cannot be made a PDF here: {}",
                    boards::not_shown_sentence(cx)
                ));
            }
        };
        Ok(BoardPdfInput {
            title: stored.meta.title,
            scene,
            source,
            rules_dir: self.web_rules_dir(cx),
            engine,
        })
    }

    /// The PDF of the open whiteboard, after what its page holds is
    /// collected and saved (`board:pdf:`).
    pub fn board_pdf_now(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Task<Result<innerword_web::PrintedPdf, String>> {
        let Some(id) = self.open_board_id() else {
            return Task::ready(Err("no whiteboard is open".to_string()));
        };
        let collect = self.collect_board(id, cx);
        cx.spawn(async move |this, cx| {
            collect.await;
            board_pdf(this, id, false, cx).await
        })
    }

    /// The native web view of the open whiteboard (scripted runs:
    /// `board:press:`).
    pub fn board_native_view(&self) -> Option<*mut std::ffi::c_void> {
        let page = self.page.as_ref().filter(|page| page.is_board())?;
        let view = page.view.borrow();
        view.as_ref()?.as_webkit().map(|web| web.native_view())
    }

    /// The shape library as the app holds it, for `board:library`:
    /// `3 items, saved`.
    pub fn describe_shape_library(&mut self) -> String {
        let (count, saved) = match &self.shape_library.held {
            Some((_, items)) => (
                serde_json::from_str::<Vec<serde_json::Value>>(items)
                    .map(|items| items.len())
                    .unwrap_or(0),
                !self.shape_library.dirty(),
            ),
            None => (
                self.shape_library
                    .stored
                    .as_ref()
                    .and_then(|stored| innerword_board::ShapeLibrary::from_stored(stored).ok())
                    .map(|parsed| parsed.library.items().len())
                    .unwrap_or(0),
                true,
            ),
        };
        format!(
            "{count} item{}, {}",
            if count == 1 { "" } else { "s" },
            if saved { "saved" } else { "not saved" }
        )
    }
}

/// Makes the PDF of whiteboard `id` as the library holds it now. Called from
/// a task: WebKit's export page runs between two awaits, outside any update.
pub(crate) async fn board_pdf(
    this: WeakEntity<Workspace>,
    id: DocumentId,
    print: bool,
    cx: &mut AsyncApp,
) -> Result<innerword_web::PrintedPdf, String> {
    let input = this
        .update(cx, |this, cx| this.board_pdf_input(id, print, cx))
        .map_err(|_| "the window went".to_string())??;
    pdf_of(input).await
}

/// The PDF of `input`.
pub(crate) async fn pdf_of(input: BoardPdfInput) -> Result<innerword_web::PrintedPdf, String> {
    match (input.engine, input.source) {
        (Some(pages::PageEngine::Fake(fake)), _) => fake.make_board_pdf(&input.scene),
        (Some(pages::PageEngine::WebKit), Some(source)) => {
            let (tx, rx) = pages::answer();
            innerword_web::run_script(
                innerword_web::ScriptInput {
                    source,
                    entry: innerword_board_web::EXPORT_ENTRY.to_string(),
                    script: innerword_board_web::calls::EXPORT_LAYOUT.to_string(),
                    argument_json: innerword_board_web::calls::export_argument(),
                    pdf: true,
                    csp: innerword_web::board_export_policy(),
                    limit: BOARD_PDF_LIMIT,
                    rules_dir: input.rules_dir,
                },
                move |result| tx.send(result),
            );
            let output = rx
                .await
                .ok_or_else(|| "the export page did not answer".to_string())??;
            output
                .pdf
                .ok_or_else(|| "the export page made no PDF".to_string())
        }
        _ => Err("no page engine".to_string()),
    }
}
