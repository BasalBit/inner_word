//! The assistant in the workspace: whether it is shown, and what the
//! workspace supplies to its two chats.
//!
//! # Whether it is shown
//!
//! The assistant is shown when three things hold ([`Workspace::assistant_shown`]):
//! the app has an engine for it ([`crate::assistant::host`]; `main`
//! installs Claude Code, a test the fake, and without one there is no
//! assistant at all), the engine's program is installed on this Mac, and
//! the setting "Assistant" is on. Whether it is installed is found out in
//! the background, at launch and whenever the settings are opened, and the
//! app never waits for it. Until the first answer of a launch is in, the
//! answer of the last launch stands (`assistant.engine_seen`): on a Mac
//! where the engine was there yesterday the keys work from the first
//! moment, and the side chat that was open is open, without the window
//! jumping when the answer arrives. On a Mac where it was never found the
//! assistant is not shown until it is.
//!
//! While it is not shown, nothing of it is: its keys do nothing, its
//! commands are not in the palette or the list of shortcuts
//! ([`crate::commands::set_category_hidden`]), the menu bar has none of
//! its items ([`crate::menus::set_assistant_shown`]), the text menu and the
//! format bar have no "Ask the Assistant", and the side chat is not drawn.
//! The settings then say in one line what it needs.
//!
//! Installed but not signed in, the assistant is shown, and its chats say
//! how to sign in.
//!
//! # The two chats
//!
//! | Chat | View | Where | Opened by |
//! |---|---|---|---|
//! | Quick chat | `innerword_assistant_quick::QuickChat` | a popover at the selection or the caret, drawn over the editor area | [`AskAssistant`] (`cmd-j`, the palette, "Ask the Assistant" in the text menu, the button of the format bar, Edit in the menu bar) |
//! | Side chat | `innerword_assistant_ui::AssistantPanel` | the slot at the right of the text, shared with the comments (`right_panel.rs`) | [`FocusAssistant`] (`cmd-3`), [`ToggleAssistant`], View in the menu bar, any of the side chat's own commands |
//!
//! Both views are made once per window, when the app has an engine, and
//! start nothing until a message is sent.
//!
//! # What the workspace supplies
//!
//! - **The quick chat** needs nothing but the editor: it captures the
//!   selection itself, and the selected text and the question are all it
//!   sends. It applies its answers itself.
//! - **The side chat** asks, each time a message is sent, for the open
//!   document as it is in the editor at that moment (not as it was last
//!   saved), with its title and its path in the library
//!   ([`Workspace::assistant_open_document`]); for the documents and
//!   folders of the library when its picker opens
//!   ([`library_attachments`]); and is given the library's own MCP server
//!   as its tools ([`library_tool_server`]: the `innerword` tool beside the
//!   app, `mcp`, on the data directory of the open library; read-only, so
//!   it works while the app has the library open).
//! - **Answers** are applied by the workspace when the side chat asks
//!   ([`crate::assistant::apply`]): Insert at Caret and Replace Selection
//!   are one undo step each, and the Markdown of the answer becomes
//!   formatted text. Which of the two is possible follows the editor
//!   ([`Workspace::follow_the_text`]).
//! - **Conversations**: each document has its own
//!   ([`crate::assistant::conversations`]). The side chat reports every
//!   change and the workspace stores it; opening a document hands the
//!   stored one over.
//! - **The model and effort** of each chat are settings; the pickers in
//!   the chats report a choice and the workspace stores it.
//! - **The note about where the text goes** is shown in both chats until
//!   it was acknowledged in either (`assistant.notice_accepted`).

use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

use gpui::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, IntoElement,
    ParentElement as _, Styled as _, Subscription, Window, div,
};
use innerword_assistant::{
    Attachment, Availability, Mode, OpenDocument, OpenKind, Selection, ToolServer,
};
use innerword_assistant_quick::{QuickChat, QuickChatEvent};
use innerword_assistant_ui::{
    ApplyKind, AssistantPanel, AssistantPanelEvent, AssistantPanelOptions, DocumentRef,
};
use innerword_editor::Editor;
use innerword_library::{DocumentId, Library};
use innerword_ui::Toast;

use super::{RightPanel, Workspace, saver::StoredPictures};
use crate::{
    actions::{AskAssistant, CATEGORY_ASSISTANT, FocusAssistant, ToggleAssistant},
    assistant::{Host, HostEvent, apply, choices, conversations},
    settings::{Settings, keys},
};

/// What the side chat is told about the open document, kept where its
/// source can read it without asking the workspace (which may be in the
/// middle of an update when a message is sent).
#[derive(Clone, Debug, PartialEq, Eq)]
struct OpenRef {
    id: DocumentId,
    title: String,
    /// The open item is a page: its address and the text it showed (a page
    /// does not change while it is open, so the text is taken once).
    page: Option<OpenPage>,
}

/// What the side chat is told about an open page.
#[derive(Clone, Debug, PartialEq, Eq)]
struct OpenPage {
    source_url: Option<String>,
    text: Rc<str>,
    /// A whiteboard: the scene the app holds of it, whose listing of
    /// elements is sent (made when a message is sent, from what the app
    /// holds then).
    board: Option<BoardFeed>,
}

/// The open whiteboard as the side chat's source reads it, kept up to date
/// by [`Workspace::feed_the_side_chat`] (as a frame is built, and as the
/// page posts).
#[derive(Clone, Debug, PartialEq, Eq)]
struct BoardFeed {
    /// The newest scene the page posted, else the stored scene it descends
    /// from.
    scene: Arc<str>,
    /// The pictures the app has stored, for the listing ("picture stored").
    pictures: Rc<StoredPictures>,
}

impl BoardFeed {
    /// The listing of the whiteboard's elements, one per line, with the
    /// header the agents' `read` writes.
    fn listing(&self, open: &OpenRef, folder: Option<&str>) -> String {
        let pictures = &self.pictures;
        let scene = innerword_board::Scene::from_view(&self.scene, &|id| pictures.get(id).cloned())
            .or_else(|_| innerword_board::Scene::from_stored(&self.scene))
            .map(|parsed| parsed.scene)
            .unwrap_or_else(|_| innerword_board::Scene::empty());
        scene.listing(&innerword_board::ListingHeader {
            reference: &format!("board:{}", open.id.0),
            title: &open.title,
            folder,
        })
    }
}

/// The assistant as one workspace holds it.
pub(crate) struct AssistantState {
    /// The engine; `None` in an app without one.
    pub host: Option<Entity<Host>>,
    /// Whether the assistant is shown right now.
    pub shown: bool,
    /// The quick chat; `None` in an app without an engine.
    pub quick: Option<Entity<QuickChat>>,
    /// The side chat; `None` in an app without an engine.
    pub side: Option<Entity<AssistantPanel>>,
    /// The open document, for the side chat's source.
    open: Rc<RefCell<Option<OpenRef>>>,
    /// What the side chat was last told can be done with an answer.
    apply_state: (bool, bool),
    /// Whether the keyboard was in the side chat when the last frame was
    /// built ([`Workspace::collect_as_the_side_chat_takes_the_keyboard`]).
    side_had_keyboard: bool,
    _subscriptions: Vec<Subscription>,
}

impl AssistantState {
    /// The state of a workspace that is being made: the engine the app
    /// has and, with one, the two chats over `editor`. Nothing is shown
    /// yet.
    pub fn new(
        editor: &Entity<Editor>,
        library: &Entity<Library>,
        settings: &Settings,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Self {
        let open: Rc<RefCell<Option<OpenRef>>> = Rc::default();
        let Some(host) = crate::assistant::host(cx) else {
            return Self {
                host: None,
                shown: false,
                quick: None,
                side: None,
                open,
                apply_state: (false, false),
                side_had_keyboard: false,
                _subscriptions: Vec::new(),
            };
        };
        let (backend, catalog, availability) = {
            let host = host.read(cx);
            (
                host.backend().clone(),
                host.catalog().clone(),
                host.availability().cloned(),
            )
        };
        let notice = !notice_accepted(library.read(cx));

        let quick = cx.new(|cx| {
            let mut quick = QuickChat::new(editor.clone(), backend.clone(), window, cx);
            quick.set_selection(
                choices::selection_for(Mode::Quick, &settings.assistant, &catalog),
                cx,
            );
            quick.set_privacy_notice(notice, cx);
            quick
        });
        let tools = library_tool_server(library.read(cx), cx);
        let workspace = cx.weak_entity();
        let side = cx.new(|cx| {
            let mut side = AssistantPanel::new(
                backend,
                AssistantPanelOptions {
                    tools,
                    selection: Some(choices::selection_for(
                        Mode::Side,
                        &settings.assistant,
                        &catalog,
                    )),
                    privacy_notice: notice,
                    availability,
                },
                window,
                cx,
            );
            // The open document as it is at the moment a message is sent.
            let (source_editor, source_library, source_open) =
                (editor.clone(), library.clone(), open.clone());
            side.set_document_source(move |cx| {
                let open = source_open.borrow().clone()?;
                Some(open_document(
                    &open,
                    source_editor.read(cx),
                    source_library.read(cx),
                ))
            });
            // Over a whiteboard a message waits for what its page holds and
            // has not posted (a gesture, a text being edited), at most
            // `VIEW_COLLECT_PATIENCE`, and then goes with what the app holds
            // (section 6.6). The source cannot ask the workspace (it may be
            // in the middle of an update), so the hook only looks at what is
            // open and asks the workspace a moment later.
            let hook_open = open.clone();
            side.set_before_send(move |_, cx| {
                let over_a_board = matches!(
                    &*hook_open.borrow(),
                    Some(OpenRef {
                        page: Some(OpenPage { board: Some(_), .. }),
                        ..
                    })
                );
                if !over_a_board {
                    return None;
                }
                let workspace = workspace.clone();
                Some(cx.spawn(async move |cx| {
                    let collect = workspace
                        .update(cx, |this, cx| this.collect_for_a_message(cx))
                        .ok()
                        .flatten();
                    if let Some(collect) = collect {
                        let timer = cx
                            .background_executor()
                            .timer(crate::boards::VIEW_COLLECT_PATIENCE);
                        crate::pages::within(collect, timer).await;
                    }
                    workspace
                        .update(cx, |this, _| this.feed_the_side_chat())
                        .ok();
                }))
            });
            let picker_library = library.clone();
            side.set_library_source(move |cx| library_attachments(picker_library.read(cx)));
            // Not on screen until the workspace puts it there.
            side.set_shown(false, cx);
            side
        });

        let subscriptions = vec![
            cx.subscribe(
                &host,
                |this: &mut Workspace, _, event: &HostEvent, cx| match event {
                    HostEvent::Checked => this.apply_assistant(cx),
                },
            ),
            cx.subscribe_in(&quick, window, Workspace::on_quick_chat_event),
            cx.subscribe_in(&side, window, Workspace::on_side_chat_event),
        ];
        Self {
            host: Some(host),
            shown: false,
            quick: Some(quick),
            side: Some(side),
            open,
            apply_state: (false, false),
            side_had_keyboard: false,
            _subscriptions: subscriptions,
        }
    }
}

/// Whether the note about where the text goes was acknowledged in this
/// library.
fn notice_accepted(library: &Library) -> bool {
    library.setting_or(keys::ASSISTANT_NOTICE_ACCEPTED, false)
}

/// The open document as the side chat sends it: its Markdown as it is in
/// `editor` right now, its title and its path in the library. For an open
/// page: the text the page showed, and what it is (`OpenKind::Page`), so
/// that the assistant is told it cannot be changed.
fn open_document(open: &OpenRef, editor: &Editor, library: &Library) -> OpenDocument {
    let folder = library
        .document_meta(open.id)
        .ok()
        .and_then(|meta| meta.folder_id);
    let mut path = library.folder_path(folder);
    let folder_path = path.join("/");
    path.push(open.title.clone());
    let (markdown, kind) = match &open.page {
        Some(OpenPage {
            board: Some(feed), ..
        }) => (
            feed.listing(open, Some(folder_path.as_str()).filter(|f| !f.is_empty())),
            OpenKind::Board,
        ),
        Some(page) => (
            page.text.to_string(),
            OpenKind::Page {
                source_url: page.source_url.clone(),
            },
        ),
        None => (apply::document_markdown(editor), OpenKind::Text),
    };
    OpenDocument {
        id: Some(open.id.0),
        title: open.title.clone(),
        path: path.join("/"),
        markdown,
        kind,
    }
}

/// The documents and folders of `library` the side chat can be given as
/// context: every live folder and every live document, each with its path
/// in the library ("The Salt Road/Part one/Chapter 1"). Folders first, in
/// the order of the sidebar; then the documents, the most recently edited
/// first.
pub fn library_attachments(library: &Library) -> Vec<Attachment> {
    fn folders(
        nodes: &[innerword_library::FolderNode],
        path: &mut Vec<String>,
        out: &mut Vec<Attachment>,
    ) {
        for node in nodes {
            path.push(node.folder.name.clone());
            out.push(Attachment::folder(
                node.folder.id.0,
                node.folder.name.clone(),
                path.join("/"),
            ));
            folders(&node.folders, path, out);
            path.pop();
        }
    }
    let mut out = Vec::new();
    folders(&library.tree().folders, &mut Vec::new(), &mut out);
    for entry in library.all_documents().unwrap_or_default() {
        let title = super::session_title(&entry.meta.title);
        let mut path = entry.folder_path.clone();
        path.push(title.clone());
        out.push(Attachment::document(entry.meta.id.0, title, path.join("/")));
    }
    out
}

/// The library's own MCP server as the side chat's tools: the `innerword`
/// tool that belongs to this copy of the app, as `innerword --library <data
/// directory> mcp`, with only its read-only tools allowed (the engine
/// crate's `library_tools`). `None` when the tool is not there (a debug
/// build without `cargo build -p innerword-cli`) or the library is not on
/// disk: the side chat then works on the open document alone.
pub fn library_tool_server(library: &Library, cx: &App) -> Option<ToolServer> {
    let tool = assistant_tool_path(cx)?;
    let data_dir = library.path()?.parent()?.to_path_buf();
    Some(innerword_assistant::library_tools(tool, data_dir))
}

/// The `innerword` tool the side chat starts as its MCP server: the one of
/// the agents environment when the app has one (the tool beside the
/// app's executable; a test names its own), else the one beside the
/// running executable. `None` when there is no such file.
fn assistant_tool_path(cx: &App) -> Option<PathBuf> {
    crate::agents::environment(cx)
        .map(|environment| environment.tool)
        .or_else(crate::agents::bundled_tool_path)
        .filter(|tool| tool.is_file())
}

impl Workspace {
    /// Whether the assistant is shown: the app has an engine, the engine
    /// is installed on this Mac and the setting is on (the rules are in the
    /// docs of `workspace/assistant.rs`).
    pub fn assistant_shown(&self) -> bool {
        self.assistant.shown
    }

    /// The engine of the assistant, when the app has one.
    pub fn assistant_host(&self) -> Option<&Entity<Host>> {
        self.assistant.host.as_ref()
    }

    /// The quick chat, in an app with an engine. It is only drawn (and
    /// only opens) while the assistant is shown.
    pub fn quick_chat(&self) -> Option<&Entity<QuickChat>> {
        self.assistant.quick.as_ref()
    }

    /// The side chat, in an app with an engine. It sits in the slot at the
    /// right of the text while the assistant is shown, the user has it
    /// open and it is the panel in front.
    pub fn side_chat(&self) -> Option<&Entity<AssistantPanel>> {
        self.assistant.side.as_ref()
    }

    /// Looks for the assistant's engine in the background. Called once at
    /// launch; the settings ask again every time they are opened.
    pub(crate) fn look_for_assistant(&mut self, cx: &mut Context<Self>) {
        if let Some(host) = self.assistant.host.clone() {
            host.update(cx, |host, cx| host.check(cx));
        }
        self.apply_assistant(cx);
    }

    /// Shows or hides the assistant as things are now: after a check of
    /// the engine, and after a change of its settings.
    pub(crate) fn apply_assistant(&mut self, cx: &mut Context<Self>) {
        let (installed, availability, catalog) = match &self.assistant.host {
            Some(host) => {
                let host = host.read(cx);
                let seen_before = self
                    .library
                    .read(cx)
                    .setting_or(keys::ASSISTANT_ENGINE_SEEN, false);
                let installed = match host.availability() {
                    Some(availability) => availability.is_installed(),
                    // Not looked yet in this launch: as it was found last.
                    None => seen_before,
                };
                (
                    installed,
                    host.availability().cloned(),
                    Some(host.catalog().clone()),
                )
            }
            None => (false, None, None),
        };
        // What this look found is what the next launch starts from.
        if availability.is_some() {
            let seen_before = self
                .library
                .read(cx)
                .setting_or(keys::ASSISTANT_ENGINE_SEEN, false);
            if seen_before != installed {
                self.library
                    .update(cx, |library, cx| {
                        library.set_setting(keys::ASSISTANT_ENGINE_SEEN, &installed, cx)
                    })
                    .ok();
            }
        }
        let shown = installed && self.settings.assistant.enabled;
        let was_shown = std::mem::replace(&mut self.assistant.shown, shown);
        crate::commands::set_category_hidden(CATEGORY_ASSISTANT, !shown, cx);
        crate::menus::set_assistant_shown(shown, cx);
        if was_shown != shown {
            // An open list of keyboard shortcuts has other rows now, and
            // says other things about a key.
            self.shortcuts_rows_changed(cx);
        }

        // What the chats are told: what was found, and what each of them
        // asks with.
        let settings = self.settings.assistant.clone();
        if let Some(quick) = self.assistant.quick.clone() {
            quick.update(cx, |quick, cx| {
                if let Some(availability) = &availability {
                    quick.set_availability(availability.clone(), cx);
                }
                if let Some(catalog) = &catalog {
                    let selection = choices::selection_for(Mode::Quick, &settings, catalog);
                    if *quick.selection() != selection {
                        quick.set_selection(selection, cx);
                    }
                }
            });
        }
        if let Some(side) = self.assistant.side.clone() {
            side.update(cx, |side, cx| {
                if let Some(availability) = &availability {
                    side.set_availability(availability.clone(), cx);
                }
                if let Some(catalog) = &catalog {
                    let selection = choices::selection_for(Mode::Side, &settings, catalog);
                    if *side.selection() != selection {
                        side.set_selection(selection, cx);
                    }
                }
            });
        }

        self.right.assistant = shown.then(|| self.assistant.side.clone()).flatten();
        self.sync_right_panel(cx);
        cx.notify();
    }

    /// The slot at the right of the text was filled or emptied: the side
    /// chat is told whether it is on screen (its window-wide commands
    /// leave it alone while it is not), and a side chat that is put away
    /// ends the engine's idle programs.
    pub(crate) fn assistant_panel_shown_changed(&mut self, cx: &mut Context<Self>) {
        let Some(side) = self.assistant.side.clone() else {
            return;
        };
        let on_screen = self.right.shown == Some(RightPanel::Assistant);
        let open = self.assistant.shown && self.right.assistant_open;
        side.update(cx, |side, cx| {
            if side.is_shown() != on_screen {
                side.set_shown(on_screen, cx);
            }
            if !open {
                side.end_idle_processes(cx);
            }
        });
    }

    // ----- the open document ---------------------------------------------------------

    /// The open document as the side chat sends it with a message: its
    /// text as Markdown as it is in the editor right now, its title and
    /// its path in the library; for an open page the text it showed. `None`
    /// without an open item.
    ///
    /// An image of the document is one line of that Markdown,
    /// `![description](image:<id>)`: the assistant is told what the image
    /// shows, in the writer's words, and which image it is. The picture
    /// itself is never sent, in neither chat.
    pub fn assistant_open_document(&self, cx: &App) -> Option<OpenDocument> {
        let open = self.assistant.open.borrow().clone()?;
        Some(open_document(
            &open,
            self.editor.read(cx),
            self.library.read(cx),
        ))
    }

    /// Another document is open, or none: the side chat shows that
    /// document's conversation (the stored one), and the quick chat, which
    /// was about a place in the other text, closes.
    pub(crate) fn assistant_document_changed(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open = match (&self.session, &self.page) {
            (Some(session), _) => Some(OpenRef {
                id: session.id,
                title: self.display_title(),
                page: None,
            }),
            (None, Some(page)) => Some(OpenRef {
                id: page.id,
                title: self.display_title(),
                page: Some(match page.page_item() {
                    Some(item) => OpenPage {
                        source_url: item.source_url.clone(),
                        text: Rc::from(item.manifest.text.as_str()),
                        board: None,
                    },
                    // A whiteboard: the listing of its elements as the app
                    // holds them when a message is sent.
                    None => OpenPage {
                        source_url: None,
                        text: Rc::from(""),
                        board: Some(BoardFeed {
                            scene: self.board_scene_held().unwrap_or_else(|| Arc::from("{}")),
                            pictures: Rc::new(self.board_stored_pictures()),
                        }),
                    },
                }),
            }),
            (None, None) => None,
        };
        *self.assistant.open.borrow_mut() = open.clone();
        if let Some(quick) = self.assistant.quick.clone()
            && quick.read(cx).is_open()
        {
            quick.update(cx, |quick, cx| quick.close(window, cx));
        }
        if let Some(side) = self.assistant.side.clone() {
            let document = open
                .as_ref()
                .map(|open| DocumentRef::new(open.id.0, open.title.clone()));
            let stored = open
                .as_ref()
                .map(|open| conversations::load(self.library.read(cx), open.id));
            side.update(cx, |side, cx| {
                side.set_document(document, stored, window, cx)
            });
        }
        self.sync_right_panel(cx);
    }

    /// Keeps the side chat in step with the text, as a frame is built:
    /// the title of the open document, and what can be done with an
    /// answer (a caret to insert at, a selection to replace).
    pub(crate) fn follow_the_text(&mut self, cx: &mut Context<Self>) {
        let Some(side) = self.assistant.side.clone() else {
            return;
        };
        self.feed_the_side_chat();
        // The title of the open item, a text or a page (a page has no
        // caret and no selection: nothing can be put into it).
        let title = self.open_item_id().map(|_| self.display_title());
        let renamed = {
            let mut open = self.assistant.open.borrow_mut();
            match (open.as_mut(), title) {
                (Some(open), Some(title)) if open.title != title => {
                    open.title = title.clone();
                    Some(title)
                }
                _ => None,
            }
        };
        let apply_state = {
            let editor = self.editor.read(cx);
            let can_edit = self.session.is_some() && !editor.is_read_only();
            (can_edit, can_edit && !editor.selection().is_collapsed())
        };
        let changed = self.assistant.apply_state != apply_state;
        if renamed.is_none() && !changed {
            return;
        }
        self.assistant.apply_state = apply_state;
        side.update(cx, |side, cx| {
            if let Some(title) = renamed {
                side.set_document_title(title, cx);
            }
            side.set_apply_state(apply_state.0, apply_state.1, cx);
        });
    }

    /// Gives the side chat's source the scene of the open whiteboard as the
    /// app holds it now (a pointer is compared; the pictures are read again
    /// only when the scene or the number of stored pictures changed), so
    /// that a message is sent with the listing of what the writer last drew.
    pub(crate) fn feed_the_side_chat(&mut self) {
        let Some(id) = self.open_board_id() else {
            return;
        };
        let Some(scene) = self.board_scene_held() else {
            return;
        };
        let stored = self.board_stored_picture_count();
        let mut open = self.assistant.open.borrow_mut();
        let Some(OpenRef {
            id: open_id,
            page: Some(OpenPage {
                board: Some(feed), ..
            }),
            ..
        }) = open.as_mut()
        else {
            return;
        };
        if *open_id != id || (Arc::ptr_eq(&feed.scene, &scene) && feed.pictures.len() == stored) {
            return;
        }
        feed.scene = scene;
        drop(open);
        let pictures = Rc::new(self.board_stored_pictures());
        if let Some(OpenRef {
            page: Some(OpenPage {
                board: Some(feed), ..
            }),
            ..
        }) = self.assistant.open.borrow_mut().as_mut()
        {
            feed.pictures = pictures;
        }
    }

    /// Stores the side chat's conversation about the open document as it
    /// is right now. The side chat reports its changes as they happen (a
    /// message sent, an answer ended); this is for the moment the window
    /// closes or the app quits in the middle of an answer, so that what
    /// had been written of it is there after the relaunch (as an answer
    /// that was stopped).
    pub(crate) fn store_assistant_conversation(&mut self, cx: &mut Context<Self>) {
        let (Some(side), Some(id)) = (self.assistant.side.clone(), self.open_item_id()) else {
            return;
        };
        let transcript = side.read(cx).transcript().clone();
        let stored = conversations::load(self.library.read(cx), id);
        if conversations::bounded(&transcript) != stored
            && (!transcript.is_empty() || conversations::is_stored(self.library.read(cx), id))
        {
            conversations::store(&self.library, id, &transcript, cx);
        }
    }

    /// A document was deleted for good: its conversation goes with it.
    pub(crate) fn forget_assistant_conversation(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        conversations::forget(&self.library, id, cx);
    }

    // ----- the chats report ----------------------------------------------------------

    fn on_quick_chat_event(
        &mut self,
        _quick: &Entity<QuickChat>,
        event: &QuickChatEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            QuickChatEvent::SelectionChanged(selection) => {
                self.remember_assistant_selection(Mode::Quick, selection, cx)
            }
            QuickChatEvent::PrivacyNoticeAcknowledged => self.accept_assistant_notice(cx),
            QuickChatEvent::AvailabilityChanged(found) => self.assistant_found(found.clone(), cx),
            _ => cx.notify(),
        }
    }

    fn on_side_chat_event(
        &mut self,
        _side: &Entity<AssistantPanel>,
        event: &AssistantPanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AssistantPanelEvent::Apply { kind, markdown, .. } => {
                self.apply_answer(*kind, markdown, window, cx)
            }
            AssistantPanelEvent::CopyText(text) => {
                self.copy_from_assistant(ClipboardItem::new_string(text.clone()), cx)
            }
            AssistantPanelEvent::TranscriptChanged {
                document,
                transcript,
            } => {
                if let Some(id) = document {
                    conversations::store(&self.library, DocumentId(*id), transcript, cx);
                }
            }
            AssistantPanelEvent::SelectionChanged(selection) => {
                self.remember_assistant_selection(Mode::Side, selection, cx)
            }
            AssistantPanelEvent::FocusTextRequested => self.focus_editor(window, cx),
            AssistantPanelEvent::CloseRequested => self.set_assistant_panel_open(false, window, cx),
            AssistantPanelEvent::PrivacyNoticeAcknowledged => self.accept_assistant_notice(cx),
            AssistantPanelEvent::AvailabilityChanged(found) => {
                self.assistant_found(found.clone(), cx)
            }
            _ => cx.notify(),
        }
    }

    /// Does what a button under an answer of the side chat asks for. An
    /// answer that went into the text leaves the keyboard in the text, at
    /// what was put in: `cmd-z` right after it takes the answer out again
    /// (in the chat's composer the key would have undone typing there).
    fn apply_answer(
        &mut self,
        kind: ApplyKind,
        markdown: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if kind == ApplyKind::Copy {
            // The Markdown for other apps, the formatted text for a
            // document of this one.
            let text = markdown.trim().to_string();
            let fragment = apply::answer_fragment(&self.editor, Some(&self.library), markdown, cx);
            let item = if fragment.is_empty() {
                ClipboardItem::new_string(text)
            } else {
                ClipboardItem::new_string_with_metadata(text, fragment.to_json())
            };
            self.copy_from_assistant(item, cx);
            return;
        }
        if self.session.is_none() || self.editor.read(cx).is_read_only() {
            self.show_toast(Toast::new("Open a document to put the answer into"), cx);
            return;
        }
        let editor = self.editor.clone();
        let library = self.library.clone();
        let done = match kind {
            ApplyKind::InsertAtCaret => {
                apply::insert_at_caret_from(&editor, &library, markdown, cx)
            }
            ApplyKind::ReplaceSelection => {
                if !apply::has_selection(&editor, cx) {
                    self.show_toast(
                        Toast::new("Select the text the answer is to replace first"),
                        cx,
                    );
                    return;
                }
                apply::replace_selection_from(&editor, &library, markdown, cx)
            }
            ApplyKind::Copy => false,
        };
        if done {
            self.focus_editor(window, cx);
        } else {
            self.show_toast(Toast::new("The answer has no text to put in"), cx);
        }
    }

    /// Puts `item` on the clipboard and says so. A scripted run leaves the
    /// clipboard alone: it is the clipboard of whoever is at the machine.
    fn copy_from_assistant(&mut self, item: ClipboardItem, cx: &mut Context<Self>) {
        let scripted = std::env::var_os(crate::automation::AUTOMATION_ENV)
            .is_some_and(|script| !script.is_empty());
        if !scripted {
            cx.write_to_clipboard(item);
        }
        self.show_toast(
            Toast::new("Copied").duration(std::time::Duration::from_secs(2)),
            cx,
        );
    }

    /// A picker in one of the chats chose a model or an effort: that chat
    /// asks with it from now on, here and after a relaunch.
    fn remember_assistant_selection(
        &mut self,
        mode: Mode,
        selection: &Selection,
        cx: &mut Context<Self>,
    ) {
        let (model_key, effort_key) = choices::keys_for(mode);
        let model = choices::stored_model(&selection.model);
        let effort = choices::stored_effort(&selection.effort);
        self.write_setting(model_key, &model, cx);
        self.write_setting(effort_key, &effort, cx);
    }

    /// The note about where the text goes was read in one of the chats:
    /// neither shows it again, in this library.
    fn accept_assistant_notice(&mut self, cx: &mut Context<Self>) {
        if !notice_accepted(self.library.read(cx)) {
            self.library
                .update(cx, |library, cx| {
                    library.set_setting(keys::ASSISTANT_NOTICE_ACCEPTED, &true, cx)
                })
                .ok();
        }
        if let Some(quick) = self.assistant.quick.clone() {
            quick.update(cx, |quick, cx| quick.set_privacy_notice(false, cx));
        }
        if let Some(side) = self.assistant.side.clone() {
            side.update(cx, |side, cx| side.set_privacy_notice(false, cx));
        }
    }

    /// Whether the note about where the text goes is still to be shown
    /// before the first message.
    pub fn assistant_notice_pending(&self, cx: &App) -> bool {
        !notice_accepted(self.library.read(cx))
    }

    /// One of the chats looked for the engine itself and found this.
    fn assistant_found(&mut self, found: Availability, cx: &mut Context<Self>) {
        if let Some(host) = self.assistant.host.clone() {
            host.update(cx, |host, cx| host.found(found, cx));
        }
    }

    // ----- opening the chats ---------------------------------------------------------

    /// Opens the quick chat at the selection (or the caret), or closes it
    /// when it is open. What `cmd-j`, "Ask the Assistant" in the text menu
    /// and the button of the format bar do.
    pub fn toggle_quick_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.assistant.shown {
            return;
        }
        let Some(quick) = self.assistant.quick.clone() else {
            return;
        };
        if self.session.is_none() {
            self.show_toast(Toast::new("Open a document to ask about its text"), cx);
            return;
        }
        self.close_modal_first(window, cx);
        quick.update(cx, |quick, cx| quick.toggle(window, cx));
    }

    /// Goes to the side chat: opens it when it is closed, brings it in
    /// front of the comments, and puts the keyboard in its composer. What
    /// `cmd-3` does.
    pub fn focus_assistant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.assistant.shown {
            return;
        }
        // A page is asked about as a text is (its text is sent).
        if self.open_item_id().is_none() {
            self.show_toast(
                Toast::new("Open a document to ask the assistant about it"),
                cx,
            );
            return;
        }
        self.close_modal_first(window, cx);
        self.set_assistant_panel_open(true, window, cx);
        // Over a whiteboard whose page holds more than it posted (a gesture,
        // a text being edited), the page is asked for it now, so that the
        // listing the next message is sent with has it.
        if let Some(id) = self.open_board_id()
            && self.board_holds_more()
        {
            self.collect_board(id, cx).detach();
        }
        if let Some(side) = self.assistant.side.clone() {
            side.update(cx, |side, cx| side.focus(window, cx));
        }
    }

    /// The keyboard came into the side chat, by whatever way (`cmd-3`, a
    /// click in the composer, `tab`, a command), over a whiteboard whose
    /// page holds more than it posted (a gesture, a text being edited):
    /// the page is asked for it now, as `cmd-3` asks, so that the listing
    /// the next message is sent with has it (section 6.6). Called as a
    /// frame is built; the collect starts after the frame. A collect that
    /// is already on its way (the one `cmd-3` started) is not asked twice.
    pub(crate) fn collect_as_the_side_chat_takes_the_keyboard(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let has = self.assistant_panel_has_focus(window, cx);
        let came = has && !self.assistant.side_had_keyboard;
        self.assistant.side_had_keyboard = has;
        if !came || self.open_board_id().is_none() {
            return;
        }
        cx.defer_in(window, |this, _, cx| {
            let Some(id) = this.open_board_id() else {
                return;
            };
            let asked = this.board().is_some_and(|board| board.collecting > 0);
            if this.board_holds_more() && !asked {
                this.collect_board(id, cx).detach();
            }
        });
    }

    /// A message of the side chat is about to be sent over the open
    /// whiteboard: when its page holds more than it posted, the page is
    /// asked for it. The answer resolves when what it handed over is taken
    /// in and saved; whoever waits sets the limit (the side chat: at most
    /// [`crate::boards::VIEW_COLLECT_PATIENCE`]). The collect itself goes on
    /// when nobody waits any more. `None`: nothing to wait for.
    pub(crate) fn collect_for_a_message(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<crate::pages::Answer<bool>> {
        let id = self.open_board_id()?;
        let holds_more = self
            .board()
            .is_some_and(|board| board.ready && board.status.holds_more());
        if !holds_more {
            return None;
        }
        let collect = self.collect_board(id, cx);
        let (done, answer) = crate::pages::answer();
        cx.spawn(async move |_, _| done.send(collect.await)).detach();
        Some(answer)
    }

    /// Shows the side chat (with the keyboard in it) or hides it.
    pub fn toggle_assistant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.assistant.shown {
            return;
        }
        if self.right.shown == Some(RightPanel::Assistant) {
            self.close_modal_first(window, cx);
            self.set_assistant_panel_open(false, window, cx);
        } else {
            self.focus_assistant(window, cx);
        }
    }

    /// One of the side chat's own commands (new chat, clear, the pickers,
    /// add context) is on its way to the side chat from somewhere else in
    /// the window: the side chat is opened and brought to the front first,
    /// so that the command has something to act on and the user sees what
    /// it did. Nothing happens while the keyboard is in the quick chat,
    /// which has the same pickers under the same keys.
    pub(crate) fn bring_the_side_chat_for_its_command(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.assistant.shown || self.open_item_id().is_none() {
            return;
        }
        let in_quick = self
            .assistant
            .quick
            .as_ref()
            .is_some_and(|quick| quick.read(cx).has_keyboard(window, cx));
        if in_quick || self.right.shown == Some(RightPanel::Assistant) {
            return;
        }
        self.close_modal_first(window, cx);
        self.set_assistant_panel_open(true, window, cx);
    }

    pub(crate) fn on_ask_assistant(
        &mut self,
        _: &AskAssistant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_quick_chat(window, cx);
    }

    pub(crate) fn on_focus_assistant(
        &mut self,
        _: &FocusAssistant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_assistant(window, cx);
    }

    pub(crate) fn on_toggle_assistant(
        &mut self,
        _: &ToggleAssistant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_assistant(window, cx);
    }

    // ----- drawing -------------------------------------------------------------------

    /// The editor area with the quick chat over it. The popover takes no
    /// room; it is mounted after the editor so that it is placed with the
    /// geometry of the same frame.
    pub(crate) fn with_quick_chat(&self, editor_area: AnyElement) -> AnyElement {
        match (&self.assistant.quick, self.assistant.shown) {
            (Some(quick), true) => div()
                .relative()
                .size_full()
                .child(editor_area)
                .child(quick.clone())
                .into_any_element(),
            _ => editor_area,
        }
    }

    /// A quick chat that is open while the assistant is not shown any
    /// more (the switch was turned off under it) closes. Called as a frame
    /// is built.
    pub(crate) fn close_a_hidden_quick_chat(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.assistant.shown {
            return;
        }
        if let Some(quick) = self.assistant.quick.clone()
            && quick.read(cx).is_open()
        {
            quick.update(cx, |quick, cx| quick.close(window, cx));
        }
    }

    /// One part of the `state` line of a scripted run: whether the
    /// assistant is shown, its engine, the command of the side chat's
    /// tools, the quick chat while it is open, the side chat while it is
    /// in the slot.
    pub fn describe_assistant(&self, window: &Window, cx: &App) -> Option<String> {
        let host = self.assistant.host.as_ref()?;
        // The command the side chat's tools are started with.
        let tools = match library_tool_server(self.library.read(cx), cx) {
            Some(server) => format!("{} {}", server.command.display(), server.args.join(" ")),
            None => "none".to_string(),
        };
        let mut line = format!(
            "shown={} engine=[{}] tools=[{tools}]",
            self.assistant.shown,
            host.read(cx).status_line()
        );
        if let Some(quick) = &self.assistant.quick
            && let Some(quick) = quick.read(cx).describe(cx)
        {
            line.push_str(&format!(" quick=[{quick}]"));
        }
        if let Some(side) = &self.assistant.side
            && self.right.shown == Some(RightPanel::Assistant)
        {
            line.push_str(&format!(
                " side=[{}]",
                side.read(cx).debug_state(window, cx)
            ));
        }
        Some(line)
    }
}
