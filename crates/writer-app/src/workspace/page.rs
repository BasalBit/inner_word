//! An open page: a second kind of open item beside the text session
//! (`docs/pages-design.md`, section 6).
//!
//! While a page is open `Workspace::session` is `None` (the editor holds an
//! empty, read-only document) and `Workspace::page` holds the
//! [`PageSession`]. What is here:
//!
//! - **Opening and closing** ([`Workspace::open_page`],
//!   [`Workspace::close_page`]): the manifest is read once, the sidebar marks
//!   the row, the page is the last open item.
//! - **The element** ([`Workspace::render_page`]): a canvas that fills the
//!   editor area (below the find strip while it is open). In its prepaint it
//!   makes the view at the first frame and keeps it on its bounds; in its
//!   paint it draws the snapshot that stands in for the view while a GPUI
//!   overlay is open. The view itself (`writer_web::WebPage`) is a native
//!   subview above GPUI's view; with the fake engine a GPUI element draws
//!   the page's text.
//! - **One focus, kept in step** ([`Workspace::sync_page_view`], every
//!   frame): the web view is AppKit's first responder exactly while the page
//!   element has GPUI's focus and no GPUI overlay that takes keys (a modal, a
//!   context menu) is open. A click into the page while the keyboard was
//!   elsewhere arrives as `wants_focus`, and the page element takes the
//!   focus.
//! - **AppKit's callbacks never enter GPUI**: the view's callbacks post to
//!   the page's inbox (`crate::pages`), and the task made at open drains it
//!   ([`Workspace::on_page_event`]).
//! - **Overlays**: a modal or a context menu over the page gets a snapshot
//!   of the page (the view is hidden once it arrives; the picture is handed
//!   to `Window::drop_image` when the overlay closes); toasts get holes in
//!   the view, where it draws nothing and takes no click.
//! - **The find strip**, the context menu of `ctrl-enter`, the title and the
//!   status items of a page, the zoom of the view, the theme, a crashed
//!   WebContent process.

use std::{
    cell::RefCell,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    Action, AnyElement, App, Bounds, Context, Corners, Entity, FocusHandle, Pixels, RenderImage,
    SharedString, Subscription, Task, Window, actions, canvas, div, point, prelude::*, px,
};
use writer_editor_tools::actions as tools;
use writer_library::{DocumentId, DocumentKind, LibraryItem};
use writer_page::Manifest;
use writer_ui::{
    ContextMenu, ContextMenuItem, TextField, TextFieldEvent, Toast,
    button::Button,
    theme::{ActiveTheme, SPACE_2, SPACE_3, SPACE_4, ThemeMode, UI_TEXT_SM},
};
use writer_web::{Appearance, EditCommand as WebEdit};

use super::{EditCommand, Workspace, WorkspaceEvent};
use crate::{
    pages::{
        self, FakeView, PAGE_VIEW, PageEngine, PageEvent, PageKeys, PageMenuItem, SharedInbox,
        ViewEvents,
    },
    settings::keys,
};

actions!(
    page,
    [
        /// Open the context menu of the open page (its items, with Copy and
        /// Select All first).
        OpenContextMenu,
    ]
);

/// The key context of the element of an open page.
pub const PAGE_VIEW_CONTEXT: &str = PAGE_VIEW;

/// The keys of an open page: its context menu (`shift-f10` first, so that
/// hints show `ctrl-enter`, as everywhere), and the four find keys once
/// more, with the defaults the tools give them in a text (a page is not on
/// the tools' path).
pub fn page_key_bindings() -> Vec<gpui::KeyBinding> {
    use gpui::KeyBinding;
    let page = Some(PAGE_VIEW);
    vec![
        KeyBinding::new("shift-f10", OpenContextMenu, page),
        KeyBinding::new("ctrl-enter", OpenContextMenu, page),
        KeyBinding::new("cmd-f", tools::Find, page),
        KeyBinding::new("cmd-alt-f", tools::FindAndReplace, page),
        KeyBinding::new("cmd-g", tools::FindNext, page),
        KeyBinding::new("cmd-shift-g", tools::FindPrevious, page),
    ]
}

/// The steps of the zoom of a page, in percent (the editor's steps up to
/// 300 %).
pub const PAGE_ZOOM_STEPS: &[u16] = &[50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300];

/// What the commands of a document that do not apply to a page say.
pub const NOT_FOR_A_PAGE: &str = "A page is shown as it was made: it has no document settings.";
/// "Spelling language" on a page.
pub const NO_SPELLING_ON_A_PAGE: &str = "A page is not checked for spelling.";
/// "Save now" on a page.
pub const NOTHING_TO_SAVE_ON_A_PAGE: &str =
    "A page is stored as it was added; there is nothing to save.";
/// The comment commands on a page.
pub const NO_COMMENTS_ON_A_PAGE: &str = "A page has no comments.";
/// The fits of the zoom on a page.
pub const NO_FIT_ON_A_PAGE: &str = "A page is drawn at its own size: it has no fit.";
/// What a page shows when its WebContent process ended twice in a row.
pub const PAGE_STOPPED: &str = "This page stopped working.";

/// How many frames in a row are asked for while the holes of the view do
/// not match the toasts (a toast coming in moves for a few frames).
const MAX_UNSETTLED_FRAMES: u8 = 30;

/// How soon after a crash a second one shows the placeholder instead of
/// loading the page again.
pub const CRASH_AGAIN_WITHIN: Duration = Duration::from_secs(10);

/// The state of the view of an open page.
#[derive(Clone, Debug, PartialEq)]
pub enum PageState {
    Loading,
    Loaded,
    Failed(String),
    /// The WebContent process ended `at`; `again`: a second time within
    /// [`CRASH_AGAIN_WITHIN`], the page shows that it stopped working.
    Crashed {
        at: Instant,
        again: bool,
    },
}

/// What stands in for the view while a GPUI overlay is over it.
#[derive(Clone, Default)]
pub enum Overlay {
    #[default]
    None,
    /// A snapshot is asked for; the view stays shown until it is there.
    Waiting,
    /// The snapshot is drawn, the view hidden.
    Snapshot(Arc<RenderImage>),
    /// Toasts are over the page: holes in the view at their bounds.
    Holes(Vec<Bounds<Pixels>>),
}

/// The view of the open page: WebKit's, or the fake engine's.
pub(crate) enum PageViewHandle {
    WebKit(writer_web::WebPage),
    Fake(FakeView),
}

impl PageViewHandle {
    fn set_frame(&self, bounds: Bounds<Pixels>) {
        if let PageViewHandle::WebKit(view) = self {
            view.set_frame(rect_of(bounds));
        }
    }

    fn set_hidden(&self, hidden: bool) {
        match self {
            PageViewHandle::WebKit(view) => view.set_hidden(hidden),
            PageViewHandle::Fake(view) => view.set_hidden(hidden),
        }
    }

    fn set_holes(&self, holes: &[writer_web::Rect]) {
        match self {
            PageViewHandle::WebKit(view) => view.set_holes(holes),
            PageViewHandle::Fake(view) => view.set_holes(holes),
        }
    }

    fn focus(&self) {
        match self {
            PageViewHandle::WebKit(view) => view.focus(),
            PageViewHandle::Fake(view) => view.focus(),
        }
    }

    fn unfocus(&self) {
        match self {
            PageViewHandle::WebKit(view) => view.unfocus(),
            PageViewHandle::Fake(view) => view.unfocus(),
        }
    }

    pub(crate) fn is_focused(&self) -> bool {
        match self {
            PageViewHandle::WebKit(view) => view.is_focused(),
            PageViewHandle::Fake(view) => view.is_focused(),
        }
    }

    fn snapshot(&self, inbox: &SharedInbox) {
        match self {
            PageViewHandle::WebKit(view) => {
                let inbox = inbox.clone();
                view.snapshot(move |bitmap| pages::post(&inbox, PageEvent::Snapshot(bitmap)));
            }
            PageViewHandle::Fake(view) => view.snapshot(),
        }
    }

    fn find(&self, query: &str, backwards: bool, case_sensitive: bool, inbox: &SharedInbox) {
        match self {
            PageViewHandle::WebKit(view) => {
                let inbox = inbox.clone();
                let asked = query.to_string();
                view.find_case(query, backwards, case_sensitive, move |found| {
                    pages::post(
                        &inbox,
                        PageEvent::Found {
                            query: asked,
                            found,
                        },
                    )
                });
            }
            PageViewHandle::Fake(view) => view.find(query, backwards, case_sensitive),
        }
    }

    fn clear_find(&self) {
        if let PageViewHandle::WebKit(view) = self {
            view.clear_find();
        }
    }

    fn set_zoom(&self, percent: u16) {
        match self {
            PageViewHandle::WebKit(view) => view.set_zoom(f64::from(percent) / 100.),
            PageViewHandle::Fake(view) => view.set_zoom(percent),
        }
    }

    fn set_appearance(&self, appearance: Appearance, background: (f64, f64, f64)) {
        match self {
            PageViewHandle::WebKit(view) => view.set_appearance(appearance, background),
            PageViewHandle::Fake(view) => view.set_appearance(appearance),
        }
    }

    fn edit(&self, command: WebEdit) {
        match self {
            PageViewHandle::WebKit(view) => view.edit(command),
            PageViewHandle::Fake(view) => view.edit(command),
        }
    }

    fn reload(&self) {
        match self {
            PageViewHandle::WebKit(view) => view.reload(),
            PageViewHandle::Fake(view) => view.reload(),
        }
    }

    /// Evaluates `script` in the page; `done` is called later (from inside
    /// AppKit for WebKit: it may only post).
    pub(crate) fn evaluate(
        &self,
        script: &str,
        done: impl FnOnce(Result<String, String>) + 'static,
    ) {
        match self {
            PageViewHandle::WebKit(view) => view.evaluate(script, done),
            PageViewHandle::Fake(view) => {
                view.evaluate(script);
                done(Ok(String::new()));
            }
        }
    }

    pub(crate) fn as_webkit(&self) -> Option<&writer_web::WebPage> {
        match self {
            PageViewHandle::WebKit(view) => Some(view),
            PageViewHandle::Fake(_) => None,
        }
    }
}

/// What makes the view of the open page at its first frame.
enum ViewSeed {
    WebKit(writer_web::ViewConfig, Rc<dyn writer_web::PageEvents>),
    Fake(FakeView),
}

/// The view of an open page, shared with its element (the element makes it
/// at the first frame, the session drops it with itself).
pub(crate) type SharedView = Rc<RefCell<Option<PageViewHandle>>>;

/// The find strip of an open page.
pub(crate) struct FindStrip {
    pub field: Entity<TextField>,
    /// Whether the last find of the query found it; `None` before one.
    pub found: Option<bool>,
    pub case_sensitive: bool,
    pub focus: FocusHandle,
    _subscription: Subscription,
}

/// An open page.
pub(crate) struct PageSession {
    pub id: DocumentId,
    pub title: String,
    pub source_url: Option<String>,
    /// Read once when the page opens.
    pub manifest: Manifest,
    pub words: u64,
    /// Made at the first prepaint.
    pub view: SharedView,
    seed: Rc<RefCell<Option<ViewSeed>>>,
    pub state: PageState,
    /// Percent, 100 at open; not stored.
    pub zoom: u16,
    pub find: Option<FindStrip>,
    pub overlay: Overlay,
    /// The page element's.
    pub focus: FocusHandle,
    /// Where the view's callbacks post.
    pub inbox: SharedInbox,
    /// What the view asks while AppKit waits.
    pub keys: Rc<RefCell<PageKeys>>,
    /// Snapshots to hand to `Window::drop_image` in the next prepaint.
    to_drop: Rc<RefCell<Vec<Arc<RenderImage>>>>,
    /// The appearance the view was given last.
    appearance: Option<(Appearance, (f64, f64, f64))>,
    /// Files of the manifest the library does not have.
    pub missing: usize,
    /// Who waits for the page to have loaded (`page:wait`).
    loaded_waiters: Vec<pages::AnswerTx<()>>,
    /// The bounds of the element in the last frame.
    pub bounds: Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
    /// The fake engine, when it shows the page.
    fake: Option<Rc<pages::FakePages>>,
    /// The version of the keymap `keys` was taken from.
    keymap_version: Option<gpui::KeymapVersion>,
    /// Frames asked for in a row while the holes did not match the toasts.
    unsettled_frames: u8,
    _events: Task<()>,
}

impl Drop for PageSession {
    fn drop(&mut self) {
        pages::close(&self.inbox);
        // The view goes with the session (its WebContent process with it).
        self.view.borrow_mut().take();
        self.seed.borrow_mut().take();
        for waiter in self.loaded_waiters.drain(..) {
            drop(waiter);
        }
    }
}

/// A rectangle of GPUI's window coordinates (logical pixels from the top
/// left of the content) as `writer_web` takes it.
fn rect_of(bounds: Bounds<Pixels>) -> writer_web::Rect {
    writer_web::Rect {
        x: f64::from(f32::from(bounds.origin.x)),
        y: f64::from(f32::from(bounds.origin.y)),
        width: f64::from(f32::from(bounds.size.width)),
        height: f64::from(f32::from(bounds.size.height)),
    }
}

/// The holes of `toasts` (window coordinates) in a view on `frame`, in the
/// view's own points from its top left; toasts outside the view are left
/// out.
pub fn holes_in(frame: Bounds<Pixels>, toasts: &[Bounds<Pixels>]) -> Vec<writer_web::Rect> {
    toasts
        .iter()
        .filter_map(|toast| {
            let hit = frame.intersect(toast);
            (f32::from(hit.size.width) > 0. && f32::from(hit.size.height) > 0.).then(|| {
                rect_of(Bounds {
                    origin: point(hit.origin.x - frame.origin.x, hit.origin.y - frame.origin.y),
                    size: hit.size,
                })
            })
        })
        .collect()
}

/// A snapshot of the view as GPUI paints it.
fn render_image_of(bitmap: writer_web::Bitmap) -> Option<Arc<RenderImage>> {
    if bitmap.width == 0 || bitmap.height == 0 {
        return None;
    }
    let pixels = image::RgbaImage::from_raw(bitmap.width, bitmap.height, bitmap.bgra)?;
    let frames: smallvec::SmallVec<[image::Frame; 1]> =
        smallvec::smallvec![image::Frame::new(pixels)];
    Some(Arc::new(RenderImage::new(frames)))
}

/// The appearance and the colour under the page for the app's theme.
fn appearance_for(mode: ThemeMode, cx: &App) -> (Appearance, (f64, f64, f64)) {
    let appearance = match mode {
        ThemeMode::Light => Appearance::Light,
        ThemeMode::Dark => Appearance::Dark,
        ThemeMode::System => Appearance::System,
    };
    let canvas = gpui::Rgba::from(cx.theme().colors.canvas);
    (
        appearance,
        (
            f64::from(canvas.r),
            f64::from(canvas.g),
            f64::from(canvas.b),
        ),
    )
}

impl Workspace {
    // ----- access ------------------------------------------------------------------

    /// The id of the open page, if a page is open.
    pub fn open_page_id(&self) -> Option<DocumentId> {
        self.page.as_ref().map(|page| page.id)
    }

    /// The id of the open item: the text or the page.
    pub fn open_item_id(&self) -> Option<DocumentId> {
        self.open_document_id().or(self.open_page_id())
    }

    /// The title of the open page.
    pub fn page_title(&self) -> Option<&str> {
        self.page.as_ref().map(|page| page.title.as_str())
    }

    /// The manifest of the open page.
    pub fn page_manifest(&self) -> Option<&Manifest> {
        self.page.as_ref().map(|page| &page.manifest)
    }

    /// The state of the view of the open page.
    pub fn page_state(&self) -> Option<PageState> {
        self.page.as_ref().map(|page| page.state.clone())
    }

    /// The zoom of the open page, in percent.
    pub fn page_zoom(&self) -> Option<u16> {
        self.page.as_ref().map(|page| page.zoom)
    }

    /// The focus handle of the open page's element.
    pub fn page_focus_handle(&self) -> Option<FocusHandle> {
        self.page.as_ref().map(|page| page.focus.clone())
    }

    /// Whether the page element has the keyboard.
    pub fn page_has_focus(&self, window: &Window) -> bool {
        self.page
            .as_ref()
            .is_some_and(|page| page.focus.is_focused(window))
    }

    /// The find strip's query and whether it was found, while it is open.
    pub fn page_find(&self, cx: &App) -> Option<(String, Option<bool>)> {
        let find = self.page.as_ref()?.find.as_ref()?;
        Some((find.field.read(cx).text().to_string(), find.found))
    }

    /// Whether the item `id` is a page, by the library's row.
    pub(crate) fn is_page(&self, id: DocumentId, cx: &App) -> bool {
        self.library
            .read(cx)
            .document_meta(id)
            .is_ok_and(|meta| meta.kind == DocumentKind::Page)
    }

    // ----- opening and closing -----------------------------------------------------

    /// Opens page `id`, closing the open text (saved first) or page. Returns
    /// whether it is open afterwards. With `focus_page` the page element
    /// takes the keyboard (and any modal closes).
    pub fn open_page(
        &mut self,
        id: DocumentId,
        focus_page: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.open_page_id() != Some(id) && !self.load_page(id, window, cx) {
            return false;
        }
        if focus_page {
            self.dismiss_modal(window, cx);
            if let Some(focus) = self.page_focus_handle() {
                window.focus(&focus);
            }
        }
        true
    }

    fn load_page(&mut self, id: DocumentId, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let stored = match self.library.read(cx).document(id) {
            Ok(stored) => stored,
            Err(error) => {
                self.toast_error(format!("Could not open the page: {error}"), cx);
                return false;
            }
        };
        let manifest = match Manifest::from_json(&stored.content) {
            Ok(manifest) => manifest,
            Err(error) => {
                self.toast_error(
                    format!(
                        "Could not open “{}”: {}",
                        super::session_title(&stored.meta.title),
                        error.sentence()
                    ),
                    cx,
                );
                return false;
            }
        };
        // The text that was open is saved and closed; the editor holds an
        // empty, read-only document while the page is open.
        let had_text_focus = self.editor.read(cx).is_focused(window);
        self.close_text_for_page(cx);
        self.page = None;
        let focus = cx.focus_handle();
        if had_text_focus {
            window.focus(&focus);
        }
        let missing = pages::missing_files(self.library.read(cx), &manifest);
        let inbox: SharedInbox = Rc::default();
        let keys = Rc::new(RefCell::new(PageKeys::default()));
        let events: Rc<dyn writer_web::PageEvents> = Rc::new(ViewEvents {
            inbox: inbox.clone(),
            keys: keys.clone(),
        });
        let theme = appearance_for(self.settings.theme, cx);
        let engine = pages::engine(cx);
        let seed = match &engine {
            Some(PageEngine::WebKit) => {
                let library = self.library.read(cx);
                let source = pages::LibraryPageSource::of(library, &manifest);
                let rules_dir = pages::rules_dir_of(library.path())
                    .unwrap_or_else(|| std::env::temp_dir().join("the-writer-WebKit"));
                Some(ViewSeed::WebKit(
                    writer_web::ViewConfig {
                        host: format!("page-{}", id.0),
                        entry: manifest.entry.clone(),
                        source,
                        rules_dir,
                        appearance: theme.0,
                        background: theme.1,
                    },
                    events,
                ))
            }
            Some(PageEngine::Fake(fake)) => Some(ViewSeed::Fake(FakeView::new(
                fake.clone(),
                id,
                &manifest.entry,
                &manifest.text,
                events,
                inbox.clone(),
            ))),
            None => None,
        };
        let fake = match &engine {
            Some(PageEngine::Fake(fake)) => Some(fake.clone()),
            _ => None,
        };
        let task_inbox = inbox.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            while let Some(events) = pages::next_events(task_inbox.clone()).await {
                for event in events {
                    let alive = match event {
                        // A key handed on is dispatched as GPUI's view would
                        // dispatch it: outside any update of the workspace,
                        // whose handlers it reaches.
                        PageEvent::KeyHandedOn(keystroke) => cx
                            .update(|window, cx| {
                                window.dispatch_keystroke(keystroke, cx);
                            })
                            .is_ok(),
                        event => this
                            .update_in(cx, |this, window, cx| {
                                this.on_page_event(id, event, window, cx)
                            })
                            .is_ok(),
                    };
                    if !alive {
                        return;
                    }
                }
            }
        });
        self.page = Some(PageSession {
            id,
            title: stored.meta.title.clone(),
            source_url: stored.meta.source_url.clone(),
            manifest,
            words: stored.meta.word_count,
            view: Rc::default(),
            seed: Rc::new(RefCell::new(seed)),
            state: if engine.is_some() {
                PageState::Loading
            } else {
                PageState::Failed(pages::not_shown_sentence(cx))
            },
            zoom: 100,
            find: None,
            overlay: Overlay::None,
            focus,
            inbox,
            keys,
            to_drop: Rc::default(),
            appearance: Some(theme),
            missing,
            loaded_waiters: Vec::new(),
            bounds: Rc::default(),
            fake,
            keymap_version: None,
            unsettled_frames: 0,
            _events: task,
        });
        self.refresh_page_keys(cx);
        self.panel.update(cx, |panel, cx| {
            panel.set_active_document(Some(id), cx);
            if !matches!(panel.selected(), Some(LibraryItem::Document(selected)) if selected == id)
            {
                panel.reveal_document(id, cx);
            }
        });
        self.library
            .update(cx, |library, cx| {
                library.set_setting(keys::LAST_DOCUMENT, &id, cx)
            })
            .ok();
        self.update_window_title(window);
        if missing > 0 {
            self.show_toast(
                Toast::new(format!(
                    "Some files of “{}” are missing from the library ({missing}). The page may \
                     not show as it did.",
                    super::session_title(&stored.meta.title)
                ))
                .duration(Duration::from_secs(10)),
                cx,
            );
        }
        self.assistant_document_changed(window, cx);
        cx.emit(WorkspaceEvent::DocumentOpened(id));
        cx.notify();
        true
    }

    /// The open text is saved and closed for a page: as `close_document`,
    /// without saying that nothing is open (the page is).
    fn close_text_for_page(&mut self, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        self.flush(cx);
        self.session = None;
        self.character_count = None;
        self.limit_counter = None;
        self.editor.update(cx, |editor, cx| {
            editor.set_document(writer_core::Document::new(), cx);
            editor.set_read_only(true, cx);
        });
    }

    /// Closes the open page (its view goes, and its WebContent process with
    /// it). The keyboard goes to the workspace when the page had it.
    pub fn close_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = self.page.take() else {
            return;
        };
        if page.focus.contains_focused(window, cx) {
            window.focus(&self.focus_handle);
        }
        drop(page);
        if self.session.is_none() {
            self.panel
                .update(cx, |panel, cx| panel.set_active_document(None, cx));
        }
        self.update_window_title(window);
        cx.notify();
    }

    /// The open page went to the trash or was deleted: as a document does,
    /// the most recently edited item that is left opens, or nothing.
    pub(crate) fn open_page_left(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let had_focus = self.page_has_focus(window);
        self.close_page(window, cx);
        self.assistant_document_changed(window, cx);
        cx.emit(WorkspaceEvent::DocumentClosed);
        let next = self
            .library
            .read(cx)
            .recent_documents(1)
            .ok()
            .and_then(|recent| recent.first().map(|meta| meta.id));
        if let Some(next) = next {
            self.open_document(next, had_focus, window, cx);
        }
    }

    /// The open page changed in the library (an agent's update, a restored
    /// version): its manifest is read again and the view loads the new
    /// page. The keyboard stays where it is.
    pub(crate) fn page_changed_in_library(
        &mut self,
        id: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(page) = self.page.as_ref().filter(|page| page.id == id) else {
            return;
        };
        let Ok(stored) = self.library.read(cx).document(id) else {
            return;
        };
        let title = stored.meta.title.clone();
        if Manifest::from_json(&stored.content).is_ok_and(|manifest| manifest == page.manifest) {
            // Only the row changed (a rename, a move).
            if let Some(page) = self.page.as_mut() {
                page.title = title;
                page.source_url = stored.meta.source_url.clone();
            }
            self.update_window_title(window);
            cx.notify();
            return;
        }
        let had_focus = self.page_has_focus(window);
        self.page = None;
        if self.load_page(id, window, cx)
            && had_focus
            && let Some(focus) = self.page_focus_handle()
        {
            window.focus(&focus);
        }
    }

    /// The open page was renamed in the library.
    pub(crate) fn page_renamed(
        &mut self,
        id: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(title) = self
            .library
            .read(cx)
            .document_meta(id)
            .ok()
            .map(|meta| meta.title)
        else {
            return;
        };
        if let Some(page) = self.page.as_mut().filter(|page| page.id == id) {
            page.title = title;
            self.update_window_title(window);
            cx.notify();
        }
    }

    // ----- what the view tells ------------------------------------------------------

    fn on_page_event(
        &mut self,
        id: DocumentId,
        event: PageEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_page_id() != Some(id) {
            return;
        }
        match event {
            PageEvent::Loaded => {
                if let Some(page) = self.page.as_mut() {
                    if !matches!(page.state, PageState::Crashed { again: true, .. }) {
                        page.state = PageState::Loaded;
                    }
                    for waiter in page.loaded_waiters.drain(..) {
                        waiter.send(());
                    }
                }
                // A query the strip was given before the view could be asked (a page
                // opened from the library search: the view is made at its first frame)
                // is looked for now that there is something to look in.
                let unasked = self.page.as_ref().and_then(|page| page.find.as_ref()).is_some_and(
                    |find| find.found.is_none() && !find.field.read(cx).text().is_empty(),
                );
                if unasked {
                    self.find_in_page(false, window, cx);
                }
            }
            PageEvent::Failed(message) => {
                if let Some(page) = self.page.as_mut() {
                    page.state = PageState::Failed(message);
                }
            }
            PageEvent::Crashed => self.page_crashed(),
            PageEvent::WantsFocus => {
                if let Some(focus) = self.page_focus_handle()
                    && !focus.is_focused(window)
                {
                    self.dismiss_modal(window, cx);
                    window.focus(&focus);
                }
            }
            PageEvent::OpenExternally(url) => cx.open_url(&url),
            PageEvent::Alert(message) => {
                self.show_toast(
                    Toast::new(format!("The page says: {message}"))
                        .duration(Duration::from_secs(8)),
                    cx,
                );
            }
            PageEvent::MenuChosen(item) => {
                let action = self
                    .page
                    .as_ref()
                    .and_then(|page| page.keys.borrow().menu_action(item));
                if let Some(action) = action {
                    if let Some(focus) = self.page_focus_handle() {
                        window.focus(&focus);
                    }
                    window.dispatch_action(action, cx);
                }
            }
            PageEvent::Snapshot(bitmap) => self.page_snapshot_arrived(bitmap),
            PageEvent::Found { query, found } => {
                if let Some(find) = self.page.as_mut().and_then(|page| page.find.as_mut())
                    && find.field.read(cx).text() == query
                {
                    find.found = Some(found);
                }
            }
            // Dispatched by the task, outside this update.
            PageEvent::KeyHandedOn(_) => {}
        }
        cx.notify();
    }

    fn page_crashed(&mut self) {
        let Some(page) = self.page.as_mut() else {
            return;
        };
        let now = Instant::now();
        let again = matches!(page.state, PageState::Crashed { at, .. } if now.duration_since(at) < CRASH_AGAIN_WITHIN);
        page.state = PageState::Crashed { at: now, again };
        if let Some(view) = page.view.borrow().as_ref() {
            if again {
                view.set_hidden(true);
            } else {
                // The first time the page loads again by itself.
                view.reload();
            }
        }
    }

    fn page_snapshot_arrived(&mut self, bitmap: Option<writer_web::Bitmap>) {
        let Some(page) = self.page.as_mut() else {
            return;
        };
        if !matches!(page.overlay, Overlay::Waiting) {
            // The overlay closed before the picture came: nothing to show.
            return;
        }
        let image = bitmap.and_then(render_image_of);
        let view = page.view.borrow();
        match image {
            Some(image) => {
                page.overlay = Overlay::Snapshot(image);
                if let Some(view) = view.as_ref() {
                    view.set_hidden(true);
                }
            }
            None => {
                // A failed snapshot: the view is hidden and the element
                // draws the canvas colour with the page's title.
                if let Some(view) = view.as_ref() {
                    view.set_hidden(true);
                }
                drop(view);
                page.overlay = Overlay::Snapshot(Arc::new(RenderImage::new(smallvec::SmallVec::<
                    [image::Frame; 1],
                >::new(
                ))));
            }
        }
    }

    /// Resolves once the open page reports that it loaded (at once when it
    /// has); `None` when no page is open or it went meanwhile.
    pub fn page_loaded(&mut self) -> Option<pages::Answer<()>> {
        let page = self.page.as_mut()?;
        let (tx, rx) = pages::answer();
        if page.state == PageState::Loaded {
            tx.send(());
        } else {
            page.loaded_waiters.push(tx);
        }
        Some(rx)
    }

    /// Makes version `snapshot` of the open page its current content, after
    /// keeping the page as it is now as a version; the view loads it.
    pub(crate) fn restore_page_version(
        &mut self,
        snapshot: writer_library::SnapshotId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(id) = self.open_page_id() else {
            return false;
        };
        let stored = match self.library.read(cx).snapshot(snapshot) {
            Ok(stored) if stored.meta.document_id == id => stored,
            Ok(_) => return false,
            Err(error) => {
                self.toast_error(format!("Could not read that version: {error}"), cx);
                return false;
            }
        };
        let manifest = match Manifest::from_json(&stored.content) {
            Ok(manifest) => manifest,
            Err(error) => {
                self.toast_error(
                    format!("Could not read that version: {}", error.sentence()),
                    cx,
                );
                return false;
            }
        };
        let Ok(meta) = self.library.read(cx).document_meta(id) else {
            return false;
        };
        let stats = writer_core::Fragment::from_plain_text(&manifest.text)
            .to_document()
            .stats();
        let restored = self.library.update(cx, |library, cx| {
            library.snapshot_if_stale(id, Duration::ZERO, cx)?;
            library.save_page(
                id,
                &writer_library::PageContent {
                    manifest: &stored.content,
                    title: &meta.title,
                    plain_text: &manifest.text,
                    word_count: stats.words as u64,
                    char_count: stats.characters as u64,
                    source_url: meta.source_url.as_deref(),
                },
                cx,
            )
        });
        match restored {
            Ok(_) => {
                let when = crate::version_history::version_time_label(
                    crate::version_history::LocalTime::from_unix_ms(stored.meta.created_at),
                    crate::version_history::LocalTime::from_unix_ms(
                        self.library.read(cx).clock().now_ms(),
                    ),
                );
                self.dismiss_modal(window, cx);
                if let Some(focus) = self.page_focus_handle() {
                    window.focus(&focus);
                }
                self.show_toast(
                    Toast::new(format!(
                        "Restored {when}. The page from before is kept as a version."
                    ))
                    .success()
                    .duration(Duration::from_secs(6)),
                    cx,
                );
                true
            }
            Err(error) => {
                self.toast_error(format!("Nothing was restored: {error}"), cx);
                false
            }
        }
    }

    /// The view of the open page, shared (for a job that prints it).
    pub(crate) fn page_view_handle(&self) -> Option<SharedView> {
        self.page.as_ref().map(|page| page.view.clone())
    }

    /// The PDF of the open page as it is now: WebKit's print operation
    /// into the user's default paper, or the fake engine's page. Call it
    /// from a task, outside any update.
    pub fn page_print_job(&self) -> Option<pages::Answer<Result<writer_web::PrintedPdf, String>>> {
        let view = self.page_view_handle()?;
        let (tx, rx) = pages::answer();
        start_printing(&view, tx);
        Some(rx)
    }

    /// Evaluates `script` in the open page (scripted runs: `page:js:`).
    pub fn evaluate_in_page(&self, script: &str) -> Option<pages::Answer<Result<String, String>>> {
        let page = self.page.as_ref()?;
        let view = page.view.borrow();
        let view = view.as_ref()?;
        let (tx, rx) = pages::answer();
        view.evaluate(script, move |result| tx.send(result));
        Some(rx)
    }

    // ----- keys ---------------------------------------------------------------------

    /// What the view of the open page asks while AppKit waits: the keys GPUI
    /// takes there and the app's items of WebKit's menu, from the keymap as
    /// it is installed now. Called when a page opens and whenever the keymap
    /// is installed.
    pub(crate) fn refresh_page_keys(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.page.as_ref() else {
            return;
        };
        let has_source = page.source_url.is_some();
        let item =
            |title: &str, action: Box<dyn Action>, enabled: bool, separator: bool| PageMenuItem {
                title: title.to_string(),
                key: pages::key_at_page(action.as_ref(), cx),
                enabled,
                separator_before: separator,
                action,
            };
        let menu = vec![
            item("Find in Page…", Box::new(tools::Find), true, true),
            item(
                "Reload Page",
                Box::new(crate::actions::ReloadPage),
                true,
                false,
            ),
            item(
                "Copy Page Address",
                Box::new(crate::actions::CopyPageAddress),
                has_source,
                false,
            ),
            item(
                "Open Page Address in Browser",
                Box::new(crate::actions::OpenPageAddress),
                has_source,
                false,
            ),
            item("Print…", Box::new(crate::actions::Print), true, true),
            item("Export…", Box::new(crate::actions::Export), true, false),
            item(
                "Version History…",
                Box::new(crate::actions::ShowVersionHistory),
                true,
                false,
            ),
        ];
        let version = cx.key_bindings().borrow().version();
        let mut keys = page.keys.borrow_mut();
        keys.refresh_keys(cx);
        keys.menu = menu;
        drop(keys);
        if let Some(page) = self.page.as_mut() {
            page.keymap_version = Some(version);
        }
    }

    // ----- the frame ----------------------------------------------------------------

    /// Keeps the view of the open page in step with GPUI, once a frame
    /// (called from `render`, after the focus is known to be on screen): the
    /// first responder, the overlays, the theme.
    pub(crate) fn sync_page_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let blocking = self.modal_layer.read(cx).has_modal() || ContextMenu::is_open(window, cx);
        let (toasts, toasts_shown) = if blocking {
            (Vec::new(), 0)
        } else {
            let layer = self.toast_layer.read(cx);
            (layer.toast_bounds(), layer.toasts().count())
        };
        let theme = appearance_for(self.settings.theme, cx);
        // The keys a page hands on follow every install of the keymap.
        let version = cx.key_bindings().borrow().version();
        if self
            .page
            .as_ref()
            .is_some_and(|page| page.keymap_version != Some(version))
        {
            self.refresh_page_keys(cx);
        }
        let Some(page) = self.page.as_mut() else {
            return;
        };
        // A click into the page that the task has not taken yet: the frame
        // must not take the first responder back from the page.
        if pages::take_wants_focus(&page.inbox) && !blocking {
            window.focus(&page.focus);
        }
        let view = page.view.borrow();
        let Some(view) = view.as_ref() else {
            return;
        };
        let wanted = page.focus.is_focused(window) && !blocking;
        if wanted && !view.is_focused() {
            view.focus();
        } else if !wanted && view.is_focused() {
            view.unfocus();
        }
        match (&page.overlay, blocking) {
            (Overlay::None | Overlay::Holes(_), true) => {
                view.set_holes(&[]);
                view.snapshot(&page.inbox);
                page.overlay = Overlay::Waiting;
            }
            (Overlay::Waiting | Overlay::Snapshot(_), true) => {}
            (Overlay::Snapshot(image), false) => {
                page.to_drop.borrow_mut().push(image.clone());
                if !matches!(page.state, PageState::Crashed { again: true, .. }) {
                    view.set_hidden(false);
                }
                page.overlay = Overlay::None;
            }
            (Overlay::Waiting, false) => page.overlay = Overlay::None,
            (Overlay::None | Overlay::Holes(_), false) => {}
        }
        if !blocking {
            // A toast's bounds are known once it was drawn, and change while
            // it comes in: another frame until the holes are where the
            // toasts are.
            let settled = toasts.len() == toasts_shown
                && match &page.overlay {
                    Overlay::Holes(holes) => *holes == toasts,
                    _ => toasts.is_empty(),
                };
            if settled {
                page.unsettled_frames = 0;
            } else if page.unsettled_frames < MAX_UNSETTLED_FRAMES {
                page.unsettled_frames += 1;
                window.request_animation_frame();
            }
            page.overlay = if toasts.is_empty() {
                Overlay::None
            } else {
                Overlay::Holes(toasts)
            };
        }
        if page.appearance != Some(theme) {
            view.set_appearance(theme.0, theme.1);
            page.appearance = Some(theme);
        }
    }

    /// The element of the open page: the find strip while it is open, then
    /// the page (the view's place, or what stands in for it).
    pub(crate) fn render_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors;
        let document_font = cx.theme().fonts.document.clone();
        let Some(page) = self.page.as_ref() else {
            return div().into_any_element();
        };
        let strip = page
            .find
            .as_ref()
            .map(|find| self.render_find_strip(find, cx));
        let view = page.view.clone();
        let seed = page.seed.clone();
        let to_drop = page.to_drop.clone();
        let fake = page.fake.clone();
        let bounds_cell = page.bounds.clone();
        let zoom = page.zoom;
        let holes = match &page.overlay {
            Overlay::Holes(toasts) => toasts.clone(),
            _ => Vec::new(),
        };
        let snapshot = match &page.overlay {
            Overlay::Snapshot(image) if image.frame_count() > 0 => Some(image.clone()),
            _ => None,
        };
        let failed_snapshot =
            matches!(&page.overlay, Overlay::Snapshot(image) if image.frame_count() == 0);
        // The native view behind the window, only for WebKit (a test's
        // window has none).
        let native = matches!(*seed.borrow(), Some(ViewSeed::WebKit(..)))
            .then(|| crate::screenshot::native_view(window).ok())
            .flatten();
        let draw_when_covered = pages::draws_when_covered(cx);
        let place = canvas(
            move |bounds, window, _cx| {
                for image in to_drop.borrow_mut().drain(..) {
                    if image.frame_count() > 0 {
                        window.drop_image(image).ok();
                    }
                    if let Some(fake) = &fake {
                        fake.note_dropped();
                    }
                }
                bounds_cell.set(Some(bounds));
                // A frame drawn while the view is lent out (AppKit can draw
                // the window while WebKit prints): it stays as it is.
                let Ok(mut view) = view.try_borrow_mut() else {
                    return;
                };
                if view.is_none()
                    && let Some(seed) = seed.borrow_mut().take()
                {
                    let made = match seed {
                        ViewSeed::WebKit(config, events) => native.map(|native| {
                            // SAFETY: the NSView behind this window, on the
                            // main thread; `pages::use_webkit` asked
                            // `writer_web::available` before installing.
                            let web = unsafe {
                                writer_web::WebPage::new(native.as_ptr(), config, events)
                            };
                            if draw_when_covered {
                                pages::keep_drawing_when_covered(web.native_view());
                            }
                            // Files dragged over the page are the app's to add.
                            pages::take_no_drops(web.native_view());
                            PageViewHandle::WebKit(web)
                        }),
                        ViewSeed::Fake(fake) => Some(PageViewHandle::Fake(fake)),
                    };
                    if let Some(made) = &made
                        && zoom != 100
                    {
                        made.set_zoom(zoom);
                    }
                    *view = made;
                    // The next frame puts the first responder in step.
                    window.refresh();
                }
                if let Some(view) = view.as_ref() {
                    view.set_frame(bounds);
                    view.set_holes(&holes_in(bounds, &holes));
                }
            },
            move |bounds, _, window, _cx| {
                if let Some(image) = snapshot {
                    window
                        .paint_image(bounds, Corners::default(), image, 0, false)
                        .ok();
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        let title = super::session_title(&page.title);
        let mut area = div()
            .id("page-area")
            .debug_selector(|| "page-area".into())
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            .bg(colors.canvas);
        // What stands in for the view, or the fake engine's drawing.
        let stand_in: Option<AnyElement> = match (&page.state, page.fake.is_some()) {
            (PageState::Crashed { again: true, .. }, _) => Some(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(SPACE_3)
                    .child(div().text_color(colors.muted).child(PAGE_STOPPED))
                    .child(
                        Button::new("page-reload", "Reload Page").on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(crate::actions::ReloadPage), cx);
                        }),
                    )
                    .into_any_element(),
            ),
            (PageState::Failed(message), false) => {
                Some(self.render_page_text_column(Some(message.clone()), &page.manifest.text, cx))
            }
            (_, true) => Some(self.render_page_text_column(None, &page.manifest.text, cx)),
            _ if failed_snapshot => Some(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_family(document_font)
                    .text_color(colors.muted)
                    .child(title)
                    .into_any_element(),
            ),
            _ => None,
        };
        area = area.child(place).children(stand_in);
        div()
            .id("page")
            .key_context(PAGE_VIEW)
            .track_focus(&page.focus)
            .size_full()
            .flex()
            .flex_col()
            .on_action(
                cx.listener(|this, _: &OpenContextMenu, window, cx| {
                    this.open_page_menu(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &tools::Find, window, cx| {
                this.open_find_strip(None, window, cx)
            }))
            .on_action(cx.listener(|this, _: &tools::FindAndReplace, window, cx| {
                this.open_find_strip(None, window, cx)
            }))
            .on_action(cx.listener(|this, _: &tools::FindNext, window, cx| {
                this.find_in_page(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &tools::FindPrevious, window, cx| {
                this.find_in_page(true, window, cx)
            }))
            // Files dropped on the page: the web view takes no drops, so
            // they reach GPUI's view, and the web pages among them are
            // added (`page_import.rs`).
            .on_drop::<gpui::ExternalPaths>(cx.listener(
                |this, paths: &gpui::ExternalPaths, window, cx| {
                    this.files_dropped_on_page(paths.paths().to_vec(), window, cx);
                },
            ))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, _| {
                    // A press on what stands in for the page (the view is
                    // hidden) gives it the keyboard, as a click into the
                    // page does.
                    if let Some(focus) = this.page_focus_handle() {
                        window.focus(&focus);
                    }
                }),
            )
            .children(strip)
            .child(area)
            .into_any_element()
    }

    /// The page's text in a quiet column: the fake engine's drawing, and
    /// what a run without an engine shows (with the sentence that says why).
    fn render_page_text_column(
        &self,
        why: Option<String>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors;
        let document_font = cx.theme().fonts.document.clone();
        let shown: String = text.chars().take(20_000).collect();
        div()
            .id("page-text")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .child(
                div()
                    .w(px(640.))
                    .max_w_full()
                    .py(SPACE_4)
                    .px(SPACE_4)
                    .flex()
                    .flex_col()
                    .gap(SPACE_3)
                    .children(why.map(|why| {
                        div()
                            .text_size(UI_TEXT_SM)
                            .text_color(colors.faint)
                            .child(why)
                    }))
                    .child(
                        div()
                            .font_family(document_font)
                            .text_color(colors.muted)
                            .whitespace_normal()
                            .child(shown),
                    ),
            )
            .into_any_element()
    }

    // ----- the context menu of the keyboard ---------------------------------------

    /// `ctrl-enter` on a page: the app's items as a GPUI context menu at the
    /// top left of the page, with Copy and Select All first, and "Open Link
    /// in Browser" first of all when the page's focused element is a link to
    /// another site.
    pub(crate) fn open_page_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = self.page.as_ref() else {
            return;
        };
        let view = page.view.borrow();
        match view.as_ref().and_then(PageViewHandle::as_webkit) {
            Some(web) => {
                let this = cx.weak_entity();
                let (tx, rx) = pages::answer::<Result<String, String>>();
                web.evaluate(
                    "(() => { const a = document.activeElement && document.activeElement.closest('a[href]'); return a ? a.href : ''; })()",
                    move |result| tx.send(result),
                );
                drop(view);
                cx.spawn_in(window, async move |_, cx| {
                    let link = rx.await.and_then(Result::ok).filter(|link| {
                        link.starts_with("http://")
                            || link.starts_with("https://")
                            || link.starts_with("mailto:")
                    });
                    this.update_in(cx, |this, window, cx| this.show_page_menu(link, window, cx))
                        .ok();
                })
                .detach();
            }
            None => {
                drop(view);
                self.show_page_menu(None, window, cx);
            }
        }
    }

    fn show_page_menu(
        &mut self,
        link: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(page) = self.page.as_ref() else {
            return;
        };
        let origin = page
            .bounds
            .get()
            .map(|bounds| bounds.origin + point(px(8.), px(8.)))
            .unwrap_or_default();
        let has_source = page.source_url.is_some();
        let mut items = Vec::new();
        if let Some(link) = link {
            items.push(ContextMenuItem::new(
                "Open Link in Browser",
                move |_, cx: &mut App| cx.open_url(&link),
            ));
            items.push(ContextMenuItem::separator());
        }
        items.push(ContextMenuItem::action(
            "Copy",
            writer_editor::actions::Copy,
        ));
        items.push(ContextMenuItem::action(
            "Select All",
            writer_editor::actions::SelectAll,
        ));
        items.push(ContextMenuItem::separator());
        items.push(ContextMenuItem::action("Find in Page…", tools::Find));
        items.push(ContextMenuItem::action(
            "Reload Page",
            crate::actions::ReloadPage,
        ));
        if has_source {
            items.push(ContextMenuItem::action(
                "Copy Page Address",
                crate::actions::CopyPageAddress,
            ));
            items.push(ContextMenuItem::action(
                "Open Page Address in Browser",
                crate::actions::OpenPageAddress,
            ));
        }
        items.push(ContextMenuItem::separator());
        items.push(ContextMenuItem::action("Print…", crate::actions::Print));
        items.push(ContextMenuItem::action("Export…", crate::actions::Export));
        items.push(ContextMenuItem::action(
            "Version History…",
            crate::actions::ShowVersionHistory,
        ));
        let focus = page.focus.clone();
        ContextMenu::open_for(&focus, origin, items, window, cx);
    }

    // ----- find ---------------------------------------------------------------------

    /// Opens the find strip of the open page (with `query`, when given) and
    /// puts the keyboard in its field, the text selected.
    pub fn open_find_strip(
        &mut self,
        query: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.page.is_none() {
            return;
        }
        if self.page.as_ref().is_some_and(|page| page.find.is_none()) {
            let field = cx.new(|cx| TextField::new(window, cx).with_placeholder("Find in page"));
            let subscription = cx.subscribe_in(&field, window, Self::on_find_field_event);
            let focus = cx.focus_handle();
            if let Some(page) = self.page.as_mut() {
                page.find = Some(FindStrip {
                    field,
                    found: None,
                    case_sensitive: false,
                    focus,
                    _subscription: subscription,
                });
            }
        }
        let Some(find) = self.page.as_ref().and_then(|page| page.find.as_ref()) else {
            return;
        };
        let field = find.field.clone();
        if let Some(query) = query {
            field.update(cx, |field, cx| field.set_text(query, cx));
        }
        field.update(cx, |field, cx| field.select_all(cx));
        field.read(cx).focus(window);
        if query.is_some() {
            self.find_in_page(false, window, cx);
        }
        cx.notify();
    }

    fn on_find_field_event(
        &mut self,
        _field: &Entity<TextField>,
        event: &TextFieldEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TextFieldEvent::Changed => self.find_in_page(false, window, cx),
            TextFieldEvent::Submitted => self.find_in_page(false, window, cx),
            TextFieldEvent::Cancelled => self.close_find_strip(window, cx),
            _ => {}
        }
    }

    /// The next (or the previous) place of the strip's query in the page.
    /// Without an open strip, opens it.
    pub(crate) fn find_in_page(
        &mut self,
        backwards: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(page) = self.page.as_ref() else {
            return;
        };
        let Some(find) = page.find.as_ref() else {
            self.open_find_strip(None, window, cx);
            return;
        };
        let query = find.field.read(cx).text().to_string();
        let case_sensitive = find.case_sensitive;
        let view = page.view.borrow();
        if let Some(view) = view.as_ref() {
            if query.is_empty() {
                view.clear_find();
            } else {
                view.find(&query, backwards, case_sensitive, &page.inbox);
            }
        }
        drop(view);
        if query.is_empty()
            && let Some(find) = self.page.as_mut().and_then(|page| page.find.as_mut())
        {
            find.found = None;
        }
        cx.notify();
    }

    /// Closes the find strip and gives the keyboard back to the page.
    pub(crate) fn close_find_strip(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = self.page.as_mut() else {
            return;
        };
        if page.find.take().is_some() {
            if let Some(view) = page.view.borrow().as_ref() {
                view.clear_find();
            }
            window.focus(&page.focus);
            cx.notify();
        }
    }

    fn toggle_find_case(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(find) = self.page.as_mut().and_then(|page| page.find.as_mut()) {
            find.case_sensitive = !find.case_sensitive;
        }
        self.find_in_page(false, window, cx);
    }

    fn render_find_strip(&self, find: &FindStrip, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors;
        let status = match find.found {
            Some(false) => Some("Not found"),
            _ => None,
        };
        div()
            .id("page-find")
            .debug_selector(|| "page-find".into())
            .key_context(tools::FIND_BAR_CONTEXT)
            .track_focus(&find.focus)
            .flex_none()
            .w_full()
            .h(px(36.))
            .px(SPACE_3)
            .flex()
            .items_center()
            .gap(SPACE_2)
            .border_b_1()
            .border_color(colors.hairline)
            .bg(colors.canvas)
            .on_action(cx.listener(|this, _: &tools::FindNext, window, cx| {
                this.find_in_page(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &tools::FindPrevious, window, cx| {
                this.find_in_page(true, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &tools::ToggleCaseSensitive, window, cx| {
                    this.toggle_find_case(window, cx)
                }),
            )
            .child(div().flex_1().min_w_0().child(find.field.clone()))
            .when(find.case_sensitive, |el| {
                el.child(
                    div()
                        .text_size(UI_TEXT_SM)
                        .text_color(colors.muted)
                        .child("Case sensitive"),
                )
            })
            .children(status.map(|status| {
                div()
                    .flex_none()
                    .text_size(UI_TEXT_SM)
                    .text_color(colors.muted)
                    .child(status)
            }))
            .into_any_element()
    }

    // ----- commands -----------------------------------------------------------------

    /// The Edit menu's items on a page: sent to the view (WebKit acts on its
    /// selection, or on a field of the page). Returns whether the page took
    /// the command (the page element has the keyboard).
    pub(crate) fn edit_on_page(&mut self, command: EditCommand, window: &Window) -> bool {
        let Some(page) = self.page.as_ref() else {
            return false;
        };
        if !page.focus.is_focused(window) {
            return false;
        }
        let command = match command {
            EditCommand::Undo => WebEdit::Undo,
            EditCommand::Redo => WebEdit::Redo,
            EditCommand::Cut => WebEdit::Cut,
            EditCommand::Copy => WebEdit::Copy,
            EditCommand::Paste => WebEdit::Paste,
            EditCommand::SelectAll => WebEdit::SelectAll,
        };
        if let Some(view) = page.view.borrow().as_ref() {
            view.edit(command);
        }
        true
    }

    /// "Reload page": the entry again, as it was stored.
    pub(crate) fn reload_page(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.page.as_mut() else {
            return;
        };
        if let Some(view) = page.view.borrow().as_ref() {
            view.set_hidden(false);
            view.reload();
        }
        page.state = PageState::Loading;
        cx.notify();
    }

    /// The zoom of the open page, in percent (50 to 300).
    pub fn set_page_zoom(&mut self, percent: u16, cx: &mut Context<Self>) {
        let Some(page) = self.page.as_mut() else {
            return;
        };
        let percent = percent.clamp(50, 300);
        page.zoom = percent;
        if let Some(view) = page.view.borrow().as_ref() {
            view.set_zoom(percent);
        }
        cx.notify();
    }

    /// One step of the page's zoom up or down.
    pub(crate) fn step_page_zoom(&mut self, up: bool, cx: &mut Context<Self>) {
        let Some(current) = self.page_zoom() else {
            return;
        };
        let next = if up {
            PAGE_ZOOM_STEPS.iter().copied().find(|step| *step > current)
        } else {
            PAGE_ZOOM_STEPS
                .iter()
                .rev()
                .copied()
                .find(|step| *step < current)
        };
        if let Some(next) = next {
            self.set_page_zoom(next, cx);
        }
    }

    /// The commands of an open page on the workspace root (reload, the
    /// page's address), and the commands of a document that would do
    /// something else on a page: they say what a page is, in one toast,
    /// instead (captured, so the handler of a text never sees them).
    pub(crate) fn page_actions(
        &self,
        root: gpui::Stateful<gpui::Div>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        use crate::actions::*;
        use writer_comments as comments;
        if self.page.is_none() {
            return root;
        }
        fn say<A: Action>(
            root: gpui::Stateful<gpui::Div>,
            sentence: &'static str,
            cx: &mut Context<Workspace>,
        ) -> gpui::Stateful<gpui::Div> {
            root.capture_action(cx.listener(move |this, _: &A, _, cx| {
                cx.stop_propagation();
                this.say_about_page(sentence, cx);
            }))
        }
        let mut root = root
            .on_action(cx.listener(|this, _: &ReloadPage, _, cx| this.reload_page(cx)))
            .on_action(cx.listener(|this, _: &CopyPageAddress, _, cx| this.copy_page_address(cx)))
            .on_action(cx.listener(|this, _: &OpenPageAddress, _, cx| this.open_page_address(cx)))
            // Find from the palette or the menu bar while the keyboard is
            // elsewhere in the window: the page's strip.
            .on_action(cx.listener(|this, _: &tools::Find, window, cx| {
                this.open_find_strip(None, window, cx)
            }))
            .on_action(cx.listener(|this, _: &tools::FindAndReplace, window, cx| {
                this.open_find_strip(None, window, cx)
            }))
            .on_action(cx.listener(|this, _: &tools::FindNext, window, cx| {
                this.find_in_page(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &tools::FindPrevious, window, cx| {
                this.find_in_page(true, window, cx)
            }));
        root = say::<OpenDocumentSettings>(root, NOT_FOR_A_PAGE, cx);
        root = say::<TogglePageView>(root, NOT_FOR_A_PAGE, cx);
        root = say::<TogglePageOrientation>(root, NOT_FOR_A_PAGE, cx);
        root = say::<IncreaseBodyTextSize>(root, NOT_FOR_A_PAGE, cx);
        root = say::<DecreaseBodyTextSize>(root, NOT_FOR_A_PAGE, cx);
        root = say::<IncreaseLineHeight>(root, NOT_FOR_A_PAGE, cx);
        root = say::<DecreaseLineHeight>(root, NOT_FOR_A_PAGE, cx);
        root = say::<UseTitleFromText>(root, NOT_FOR_A_PAGE, cx);
        root = say::<ChooseSpellingLanguage>(root, NO_SPELLING_ON_A_PAGE, cx);
        root = say::<SaveNow>(root, NOTHING_TO_SAVE_ON_A_PAGE, cx);
        root = say::<comments::AddComment>(root, NO_COMMENTS_ON_A_PAGE, cx);
        root = say::<comments::NextComment>(root, NO_COMMENTS_ON_A_PAGE, cx);
        root = say::<comments::PreviousComment>(root, NO_COMMENTS_ON_A_PAGE, cx);
        root = say::<comments::FocusComments>(root, NO_COMMENTS_ON_A_PAGE, cx);
        root.capture_action(cx.listener(|this, _: &AskAssistant, _, cx| {
            cx.stop_propagation();
            let key = crate::keymap::hint_in(&FocusAssistant, "Workspace > PageView", cx)
                .map(|hint| hint.text());
            let sentence = match key {
                Some(keys) => format!(
                    "The quick chat works in a text. Ask about this page in the side chat ({keys})."
                ),
                None => "The quick chat works in a text. Ask about this page in the side chat."
                    .to_string(),
            };
            this.show_toast(Toast::new(sentence).duration(Duration::from_secs(5)), cx);
        }))
    }

    /// "Copy page address": the address the page came from, on the clipboard.
    pub(crate) fn copy_page_address(&mut self, cx: &mut Context<Self>) {
        let Some(address) = self.page.as_ref().and_then(|page| page.source_url.clone()) else {
            self.say_about_page("This page has no address: it was added from a file.", cx);
            return;
        };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(address));
        self.show_toast(
            Toast::new("Copied the page's address.").duration(Duration::from_secs(3)),
            cx,
        );
    }

    /// "Open page address in browser": the address the page came from, in
    /// the default browser.
    pub(crate) fn open_page_address(&mut self, cx: &mut Context<Self>) {
        let Some(address) = self.page.as_ref().and_then(|page| page.source_url.clone()) else {
            self.say_about_page("This page has no address: it was added from a file.", cx);
            return;
        };
        cx.open_url(&address);
    }

    /// The names of the actions the palette leaves out while a page is open:
    /// the commands that act on a text (the tools, the image commands) and
    /// the spelling commands that act on a word. Empty without a page.
    pub(crate) fn commands_not_for_the_open_page(&self) -> Vec<&'static str> {
        if self.page.is_none() {
            return Vec::new();
        }
        use writer_spell::actions as spell;
        let mut names: Vec<&'static str> = self
            .text_commands
            .iter()
            .map(|action| action.name())
            .collect();
        let words: [&dyn Action; 6] = [
            &spell::NextMisspelling,
            &spell::PreviousMisspelling,
            &spell::ShowSuggestions,
            &spell::LearnSpelling,
            &spell::UnlearnSpelling,
            &spell::IgnoreSpelling,
        ];
        names.extend(words.iter().map(|action| action.name()));
        // Find works on a page: it has its own handlers there.
        let find: [&dyn Action; 4] = [
            &tools::Find,
            &tools::FindAndReplace,
            &tools::FindNext,
            &tools::FindPrevious,
        ];
        names.retain(|name| !find.iter().any(|action| action.name() == *name));
        names
    }

    /// A toast that says what a page is, for a command of a document.
    pub(crate) fn say_about_page(&mut self, sentence: &str, cx: &mut Context<Self>) {
        self.show_toast(
            Toast::new(SharedString::from(sentence.to_string())).duration(Duration::from_secs(4)),
            cx,
        );
    }

    /// One line about the open page, for the `state` step:
    /// `id=12 state=loaded zoom=100% overlay=none focus=page responder=web
    /// find="harbour":found missing=0`, or `none`.
    pub fn describe_page(&self, window: &Window, cx: &App) -> String {
        let Some(page) = self.page.as_ref() else {
            return "none".to_string();
        };
        let state = match &page.state {
            PageState::Loading => "loading".to_string(),
            PageState::Loaded => "loaded".to_string(),
            PageState::Failed(message) => format!("failed:{message:?}"),
            PageState::Crashed { again: false, .. } => "crashed".to_string(),
            PageState::Crashed { again: true, .. } => "stopped".to_string(),
        };
        let overlay = match &page.overlay {
            Overlay::None => "none".to_string(),
            Overlay::Waiting => "waiting".to_string(),
            Overlay::Snapshot(_) => "snapshot".to_string(),
            Overlay::Holes(holes) => format!("holes:{}", holes.len()),
        };
        let focus = if page.focus.is_focused(window) {
            "page"
        } else if page.focus.contains_focused(window, cx) {
            "find"
        } else {
            "elsewhere"
        };
        let responder = match page.view.borrow().as_ref() {
            Some(view) if view.is_focused() => "web",
            Some(_) => "gpui",
            None => "none",
        };
        let find = match &page.find {
            Some(find) => format!(
                " find={:?}:{}",
                find.field.read(cx).text(),
                match find.found {
                    Some(true) => "found",
                    Some(false) => "not-found",
                    None => "unasked",
                }
            ),
            None => String::new(),
        };
        format!(
            "id={} state={state} zoom={}% overlay={overlay} focus={focus} responder={responder}{find} \
             missing={}",
            page.id.0, page.zoom, page.missing
        )
    }

    /// The status bar's words of the open page ("4,212 words").
    pub(crate) fn page_words(&self) -> Option<String> {
        self.page
            .as_ref()
            .map(|page| super::counted(page.words as usize, "word", "words"))
    }
}

/// Starts the print of `view`; the answer goes to `tx` (from inside AppKit
/// for WebKit: it only notes it).
fn start_printing(view: &SharedView, tx: pages::AnswerTx<Result<writer_web::PrintedPdf, String>>) {
    let view = view.borrow();
    match view.as_ref() {
        Some(PageViewHandle::WebKit(web)) => web.print_to_pdf(move |printed| tx.send(printed)),
        Some(PageViewHandle::Fake(_)) => tx.send(Ok(fake_printed())),
        None => tx.send(Err("the page is not shown yet".to_string())),
    }
}

/// What the fake engine prints: one A4 page.
pub(crate) fn fake_printed() -> writer_web::PrintedPdf {
    writer_web::PrintedPdf {
        bytes: pages::fake_pdf(),
        pages: 1,
        paper: (595.28, 841.89),
    }
}

/// The PDF of a view as it is now (see [`Workspace::page_print_job`]).
pub(crate) async fn print_view(view: &SharedView) -> Result<writer_web::PrintedPdf, String> {
    let (tx, rx) = pages::answer();
    start_printing(view, tx);
    rx.await
        .unwrap_or_else(|| Err("the print was not finished".to_string()))
}
