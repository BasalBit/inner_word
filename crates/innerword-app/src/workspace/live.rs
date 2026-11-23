//! What the workspace does with a request of an agent while the app is
//! open (see [`crate::live`] for the channel the request comes in on).
//!
//! [`Workspace::carry_out`] is one turn of the main thread, so no key press
//! comes between its steps:
//!
//! 1. **Save.** What is not saved in the open document is saved, with the
//!    synchronous flush that also runs before another document is opened.
//!    It settles the background autosave first, so nothing older can be
//!    written after the agent's change. A save that fails refuses the
//!    request: nothing is changed underneath text that is only in memory.
//! 2. **Carry out.** `innerword_agent::live::execute` works on the library's
//!    own store, through [`innerword_library::Library::change_outside`], which
//!    turns the list of what changed into the events the library's own
//!    methods emit: the sidebar, the trash and whatever else listens follow
//!    as if the writer had made the change.
//! 3. **The open item.** The open whiteboard, when its scene is among what
//!    changed, takes the change in as one step of its undo, by the base
//!    rule (`Workspace::take_over_open_board`); the note waits for the
//!    view's answer to name the changes it did not take and to offer Undo.
//!    The open document, when it is among what changed, is read
//!    back and taken over by the editor
//!    ([`innerword_editor::Editor::adopt_document`]): the caret, the selection
//!    and the view stay, and the change is one more step of the undo
//!    history, on top of the writer's own. Its title and the flag of a
//!    chosen title are read again. A document that went to the trash or
//!    was deleted closes the way it does when the writer trashes it (the
//!    library's event does that). The keyboard stays where it is, and an
//!    open dialog stays open.
//! 4. **Tell the writer.** One note at the bottom ([`AgentChanges`]); a
//!    change that comes in while the note of the one before is still up
//!    joins it, and the note counts them.
//! 5. The answer goes back (the caller sends it).
//!
//! # Nothing to save after an agent's change
//!
//! The editor reports the text it took over as an edit, like any other.
//! It is not one that has to be saved: the editor shows exactly what the
//! library holds. [`Workspace::text_is_what_an_agent_stored`] tells the two
//! apart by the revision of the document, so the session stays "Saved" and
//! nothing is written a second time. The first edit of the writer's own
//! (an undo of the agent's change is one) is saved like every other.

use std::{
    cell::Cell,
    rc::Rc,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

use gpui::{App, Context, Window};
use innerword_agent::live::{self as channel, Answer, Change, Refusal, Request};
use innerword_core::{BlockId, Document, Position};
use innerword_editor::Autoscroll;
use innerword_library::{
    DocumentId, FolderId, LibraryItem, LibraryTree, OutsideChange, StoreError, TrashEntry,
};
use innerword_ui::{Toast, toast::ToastId};

use super::{Workspace, session_title};
use crate::{
    actions::ShowVersionHistory,
    boards,
    document_io::load_document,
    live::{ANONYMOUS, ShowAgentChange, TOAST_DURATION, UndoAgentChange},
    settings::keys,
    view_memory::TitleFlag,
};

/// The longest name of a document or a folder in a note about a change; a
/// longer one ends in an ellipsis, so the note stays one or two lines.
const LONGEST_NAME: usize = 48;

/// What a run of changes is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    /// One document.
    Document(DocumentId),
    /// One folder.
    Folder(FolderId),
    /// More than one thing, or the trash.
    Library,
}

/// How a run of changes is taken back in one step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Undo {
    /// The newest `steps` steps of the undo history of the open document
    /// `document`, for as long as the text is at `revision`: the same
    /// steps the undo key takes out.
    Text {
        /// The document.
        document: DocumentId,
        /// How many steps the run added.
        steps: usize,
        /// The revision of the text after the last of them.
        revision: u64,
    },
    /// Bring this back from the trash.
    Trash(LibraryItem),
    /// The newest `steps` steps of the undo of the open whiteboard `board`
    /// (Excalidraw's own), for as long as its canvas is at `rev`, the rev
    /// the last adopt of an agent's change answered: the writer has not
    /// drawn since.
    Board {
        /// The whiteboard.
        board: DocumentId,
        /// How many steps the run added.
        steps: usize,
        /// The rev of the canvas after the last of them.
        rev: u64,
    },
}

/// What the note at the bottom offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    /// "Undo": [`Workspace::undo_agent_change`].
    Undo,
    /// "Open": [`Workspace::show_agent_change`], for a document that is
    /// not the open one.
    Open,
}

impl Offer {
    /// The label of the button.
    pub fn label(self) -> &'static str {
        match self {
            Offer::Undo => "Undo",
            Offer::Open => "Open",
        }
    }
}

/// The changes agents made that the note at the bottom is about: one, or a
/// run of several that came in while the note was up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentChanges {
    /// Who made them: the name the agent gave (`--author`), or
    /// [`ANONYMOUS`].
    pub who: String,
    /// How many requests changed something.
    pub count: usize,
    /// What the note says.
    pub message: String,
    /// What the note says before the changes that were not taken.
    pub said: String,
    /// The elements of a whiteboard whose change by the agent was not
    /// taken, because the writer had changed them meanwhile (the base rule
    /// kept the writer's copy): the note names them after its sentence.
    pub conflicts: Vec<String>,
    /// What they are about.
    pub subject: Subject,
    /// How all of them are taken back in one step, when one step does it.
    pub undo: Option<Undo>,
    /// The first block that changed in the open document, for "Go to the
    /// last change by an agent".
    pub place: Option<(DocumentId, BlockId)>,
}

/// What carrying out one request of an agent cost on the main thread,
/// where it delays typing and frames. Kept for the last request that
/// changed something, for measuring (the `state` step of a scripted run
/// prints it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChangeCost {
    /// Saving what was not saved in the open document.
    pub save: Duration,
    /// The change itself, in the library (`innerword_agent::live::execute`),
    /// and the library's tree read again.
    pub change: Duration,
    /// Bringing the window up to date: the open document read back and
    /// taken over by the editor, the note.
    pub window: Duration,
}

/// What the workspace keeps about the changes of agents.
#[derive(Default)]
pub(crate) struct LiveState {
    /// The revision of the open document at which the editor holds exactly
    /// what an agent's change left in the library.
    as_stored: Option<u64>,
    /// What the note is about.
    last: Option<AgentChanges>,
    /// The note, while it is up.
    toast: Option<ToastId>,
    /// How many requests of agents changed something in this window.
    total: usize,
    /// What the last of them cost.
    cost: Option<ChangeCost>,
    /// What the request at the head of the queue waits for.
    wait: RequestWait,
    /// The adopt an agent's change to the open whiteboard started, whose
    /// answer the note waits for (its conflicts, its Undo).
    board_adopt: Option<BoardAdopt>,
}

/// What a request of an agent has waited for so far (section 6.7 of
/// `docs/whiteboard-design.md`), each wait with its own patience, measured
/// on the app's clock (a test's clock in a test).
#[derive(Default)]
pub(crate) struct RequestWait {
    /// Since when it waits for a whiteboard that is closing to be saved.
    closing_since: Option<Instant>,
    /// Since when it waits for the open whiteboard to answer the adopt of
    /// an earlier outside write.
    adopt_since: Option<Instant>,
    /// The collect it asked of the open whiteboard: since when, and whether
    /// what it handed over is saved.
    collect: Option<(Instant, Rc<Cell<bool>>)>,
    /// Since when it waits for a composition to end.
    composing_since: Option<Instant>,
}

/// An adopt of an agent's change to the open whiteboard, not answered yet.
#[derive(Clone, Copy, Debug)]
struct BoardAdopt {
    board: DocumentId,
    /// The number of the adopt (`BoardState::adopt_in_flight`).
    seq: u64,
    /// The steps of the run before this one, when this change joins them
    /// (the writer has not drawn since the last of them).
    steps_before: usize,
}

/// The names of the things one request changed, for the note.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Names {
    /// The document, as it is called now.
    pub document: Option<String>,
    /// The document, as it was called before.
    pub document_before: Option<String>,
    /// The folder, as it is called now.
    pub folder: Option<String>,
    /// The folder, as it was called before.
    pub folder_before: Option<String>,
}

/// `name` in the quotation marks of a note, cut to [`LONGEST_NAME`].
fn quoted(name: &str) -> String {
    let name = session_title(name);
    if name.chars().count() <= LONGEST_NAME {
        return format!("\u{201c}{name}\u{201d}");
    }
    let cut: String = name.chars().take(LONGEST_NAME - 1).collect();
    format!("\u{201c}{}\u{2026}\u{201d}", cut.trim_end())
}

/// What the note says about one request that changed something:
/// "Claude changed “Chapter 2”". `operation` is the name of the operation
/// on the channel (`innerword_agent::live::Request::name`).
pub fn sentence(who: &str, operation: &str, names: &Names) -> String {
    let document = names
        .document
        .as_deref()
        .or(names.document_before.as_deref())
        .map(quoted)
        .unwrap_or_else(|| "a document".to_string());
    let folder = names
        .folder
        .as_deref()
        .or(names.folder_before.as_deref())
        .map(quoted)
        .map(|name| format!("the folder {name}"))
        .unwrap_or_else(|| "a folder".to_string());
    let whiteboard = names
        .document
        .as_deref()
        .or(names.document_before.as_deref())
        .map(quoted);
    let whiteboard_or_a = || {
        whiteboard
            .clone()
            .unwrap_or_else(|| "a whiteboard".to_string())
    };
    let did = match operation {
        "create_document" | "duplicate_document" => format!("created {document}"),
        "replace_document" => format!("rewrote {document}"),
        "edit" | "insert" | "replace_section" => format!("changed {document}"),
        "add_image" => format!("added an image to {document}"),
        // A page: added, or brought up to date from its address (the note
        // tells the two apart by what changed: `operation_words`).
        "add_page" => format!("added {document}"),
        UPDATED_PAGE => format!("updated {document}"),
        "rename_document" => match (&names.document_before, &names.document) {
            (Some(before), Some(now)) if before != now => {
                format!("renamed {} to {}", quoted(before), quoted(now))
            }
            _ => format!("renamed {document}"),
        },
        "move_document" => format!("moved {document}"),
        "trash_document" => format!("moved {document} to the trash"),
        "restore_document" => format!("brought {document} back from the trash"),
        "delete_document" => format!("deleted {document} for good"),
        "create_folder" => format!("created {folder}"),
        "rename_folder" => match (&names.folder_before, &names.folder) {
            (Some(before), Some(now)) if before != now => {
                format!("renamed the folder {} to {}", quoted(before), quoted(now))
            }
            _ => format!("renamed {folder}"),
        },
        "move_folder" => format!("moved {folder}"),
        "trash_folder" => format!("moved {folder} to the trash"),
        "restore_folder" => format!("brought {folder} back from the trash"),
        "delete_folder" => format!("deleted {folder} for good"),
        "empty_trash" => "emptied the trash".to_string(),
        "add_comment" => format!("commented on {document}"),
        "reply" => format!("replied to a comment on {document}"),
        "resolve_comment" => format!("resolved a comment on {document}"),
        "reopen_comment" => format!("reopened a comment on {document}"),
        "delete_comment" => format!("deleted a comment on {document}"),
        "restore_version" => format!("brought back an earlier version of {document}"),
        // A whiteboard.
        "create_board" => match &whiteboard {
            Some(name) => format!("made the whiteboard {name}"),
            None => "made a whiteboard".to_string(),
        },
        "add_to_board" => format!("drew on {}", whiteboard_or_a()),
        "update_board" => format!("changed {}", whiteboard_or_a()),
        "remove_from_board" => format!("removed elements from {}", whiteboard_or_a()),
        // An operation a later version of the engine has.
        _ if names.document.is_some() || names.document_before.is_some() => {
            format!("changed {document}")
        }
        _ if names.folder.is_some() || names.folder_before.is_some() => {
            format!("changed {folder}")
        }
        _ => "changed the library".to_string(),
    };
    format!("{who} {did}")
}

/// The words [`sentence`] takes for an `add_page` that updated a page
/// (the old one was kept as a version) instead of adding one: not an
/// operation of the channel, only what the note says of it.
pub const UPDATED_PAGE: &str = "add_page:updated";

/// The name [`sentence`] is given for `request`, which made `changes`: the
/// operation's own, except for an `add_page` that created nothing (it
/// updated the page with that address).
fn operation_words<'a>(request: &'a Request, changes: &[Change]) -> &'a str {
    let created = changes
        .iter()
        .any(|change| matches!(change, Change::DocumentCreated { .. }));
    match request.name() {
        "add_page" if !created => UPDATED_PAGE,
        name => name,
    }
}

/// What the note says about a run of `count` changes: "Claude made 20
/// changes to “Chapter 2”", with `name` what they have in common (as
/// [`sentence`] names it), or nothing when they are about several things.
pub fn run_sentence(who: &str, count: usize, name: Option<&str>) -> String {
    match name {
        Some(name) => format!("{who} made {count} changes to {name}"),
        None => format!("{who} made {count} changes in the library"),
    }
}

/// The most element ids a note names of the changes that were not taken.
const NAMED_CONFLICTS: usize = 5;

/// What the note says after its sentence when the open whiteboard kept the
/// writer's copy of elements an agent changed (the base rule: the writer
/// changed them meanwhile): "2 of its changes were not taken, because you
/// changed those elements meanwhile: api, reads." `None` for none.
pub fn conflicts_sentence(ids: &[String]) -> Option<String> {
    match ids {
        [] => None,
        [one] => Some(format!(
            "1 of its changes was not taken, because you changed that element meanwhile: {one}."
        )),
        _ => {
            let named = ids
                .iter()
                .take(NAMED_CONFLICTS)
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            let more = match ids.len().saturating_sub(NAMED_CONFLICTS) {
                0 => String::new(),
                more => format!(" and {more} more"),
            };
            Some(format!(
                "{} of its changes were not taken, because you changed those elements meanwhile: \
                 {named}{more}.",
                ids.len()
            ))
        }
    }
}

/// The note's whole message: `said`, then what was not taken.
fn with_conflicts(said: &str, conflicts: &[String]) -> String {
    match conflicts_sentence(conflicts) {
        Some(sentence) => format!("{said}. {sentence}"),
        None => said.to_string(),
    }
}

/// What a run of changes is about: the first document among them, else
/// the first folder, else the library.
fn subject_of(changes: &[Change]) -> Subject {
    let document = changes.iter().find_map(|change| match change {
        Change::DocumentCreated { id }
        | Change::DocumentText { id }
        | Change::DocumentComments { id }
        | Change::DocumentSettings { id }
        | Change::DocumentRenamed { id }
        | Change::DocumentMoved { id }
        | Change::DocumentTrashed { id }
        | Change::DocumentRestored { id }
        | Change::DocumentDeleted { id } => Some(*id),
        Change::VersionKept { document } => Some(*document),
        _ => None,
    });
    let folder = changes.iter().rev().find_map(|change| match change {
        Change::FolderCreated { id }
        | Change::FolderRenamed { id }
        | Change::FolderMoved { id }
        | Change::FolderTrashed { id }
        | Change::FolderRestored { id }
        | Change::FolderDeleted { id } => Some(*id),
        _ => None,
    });
    match (document, folder) {
        (Some(id), _) => Subject::Document(id),
        (None, Some(id)) => Subject::Folder(id),
        (None, None) => Subject::Library,
    }
}

/// What the library's own events are for a list of changes by an agent.
fn outside_changes(changes: &[Change]) -> Vec<OutsideChange> {
    let mut outside: Vec<OutsideChange> = Vec::with_capacity(changes.len());
    for change in changes {
        let one = match *change {
            Change::DocumentCreated { id } => OutsideChange::DocumentCreated(id),
            // A name an agent chose is not the writer naming the document
            // in the sidebar (which is what the library's "renamed" means to
            // the workspace): the flag of a chosen title is the engine's to
            // set or to remove, and the open document reads it back.
            Change::DocumentText { id }
            | Change::DocumentComments { id }
            | Change::DocumentSettings { id }
            | Change::DocumentRenamed { id } => OutsideChange::DocumentSaved(id),
            Change::DocumentMoved { id } => OutsideChange::DocumentMoved(id),
            Change::DocumentTrashed { id } => OutsideChange::DocumentTrashed(id),
            Change::DocumentRestored { id } => OutsideChange::DocumentRestored(id),
            Change::DocumentDeleted { id } => OutsideChange::DocumentPurged(id),
            Change::FolderCreated { id } => OutsideChange::FolderCreated(id),
            Change::FolderRenamed { id } => OutsideChange::FolderRenamed(id),
            Change::FolderMoved { id } => OutsideChange::FolderMoved(id),
            Change::FolderTrashed { id } => OutsideChange::FolderTrashed(id),
            Change::FolderRestored { id } => OutsideChange::FolderRestored(id),
            Change::FolderDeleted { id } => OutsideChange::FolderPurged(id),
            Change::TrashEmptied => OutsideChange::TrashEmptied,
            Change::VersionKept { document } => OutsideChange::SnapshotsChanged(document),
        };
        if !outside.contains(&one) {
            outside.push(one);
        }
    }
    outside
}

/// The first block of `new` that is not what `old` has in its place.
fn first_difference(old: &Document, new: &Document) -> Option<BlockId> {
    let index = old
        .blocks()
        .iter()
        .zip(new.blocks())
        .position(|(old, new)| old != new)
        .or_else(|| {
            (old.block_count() != new.block_count())
                .then(|| old.block_count().min(new.block_count()))
        })?;
    new.blocks()
        .get(index)
        .or(new.blocks().last())
        .map(|block| block.id())
}

/// The library as it was before a request, for the names of what the
/// request takes away.
struct Before {
    tree: Arc<LibraryTree>,
    trash: Vec<TrashEntry>,
}

impl Before {
    fn document(&self, id: DocumentId) -> Option<String> {
        self.tree
            .find_document(id)
            .map(|meta| meta.title.clone())
            .or_else(|| {
                self.trash.iter().find_map(|entry| match entry {
                    TrashEntry::Document(meta) if meta.id == id => Some(meta.title.clone()),
                    _ => None,
                })
            })
    }

    fn folder(&self, id: FolderId) -> Option<String> {
        self.tree
            .find_folder(id)
            .map(|node| node.folder.name.clone())
            .or_else(|| {
                self.trash.iter().find_map(|entry| match entry {
                    TrashEntry::Folder { folder, .. } if folder.id == id => {
                        Some(folder.name.clone())
                    }
                    _ => None,
                })
            })
    }
}

impl Workspace {
    // ----- the switch -----------------------------------------------------------------

    /// Whether the owner lets agents make changes while the app is open:
    /// the setting `agents.changes_while_open` of the library, on unless
    /// it was switched off.
    pub fn agents_may_change(&self, cx: &App) -> bool {
        channel::changes_allowed(self.library.read(cx).store())
    }

    /// Switches "Agents can make changes while Inner Word is open".
    pub fn set_agents_may_change(&mut self, on: bool, cx: &mut Context<Self>) {
        let stored = self.library.update(cx, |library, cx| {
            library.set_setting(channel::CHANGES_SETTING, &on, cx)
        });
        if let Err(error) = stored {
            self.toast_error(format!("Could not save the setting: {error}"), cx);
        }
        // What the thread that accepts requests goes by (the library's
        // event says the same; this holds when the setting did not change).
        crate::live::set_switch(self.agents_may_change(cx), cx);
    }

    // ----- a request ------------------------------------------------------------------

    /// Whether `request` has to wait before it is carried out (`first`: this
    /// is the first look at it). In this order, each with its own patience
    /// (section 6.7 of `docs/whiteboard-design.md`):
    ///
    /// 1. A whiteboard is closing (its view parked while its last gesture
    ///    is collected): every request waits for that save, at most
    ///    [`boards::VIEW_COLLECT_PATIENCE`], after which it is saved as the
    ///    app holds it. A late collect never saves the scene from before
    ///    the agent's change over it.
    /// 2. The open whiteboard has not answered the adopt of an earlier
    ///    outside write: every request waits for it (at most until the app
    ///    gave it up and merged in Rust), so that the next change is merged
    ///    against what the canvas holds after the last one.
    /// 3. The open whiteboard's page holds more than it posted (a gesture
    ///    under way, a text being edited, a change not posted yet): the
    ///    page is asked once for this request, and the request waits until
    ///    what it handed over is saved, at most a second; then it goes on
    ///    with what the app holds, a read and a write alike. What the page
    ///    still holds is newer than the base the agent's change is merged
    ///    against, and the base rule keeps it.
    /// 4. It would change the library while text is being composed with an
    ///    input method (in the open document, or in a text element of the
    ///    open whiteboard): taking a change over would end the composition
    ///    where it stands, so it waits, at most
    ///    [`crate::live::COMPOSITION_PATIENCE`], and is then refused by
    ///    [`Workspace::carry_out`]. A read does not wait for a composition.
    pub(crate) fn must_wait(
        &mut self,
        request: &Request,
        first: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if first {
            self.live.wait = RequestWait::default();
        }
        let now = cx.background_executor().now();
        let waited = |since: Instant| now.saturating_duration_since(since);

        if !self.closing_boards.is_empty() {
            let since = *self.live.wait.closing_since.get_or_insert(now);
            if waited(since) < boards::VIEW_COLLECT_PATIENCE {
                return true;
            }
            self.finish_closing_board_now(window, cx);
        }

        if self.board().is_some_and(|board| board.adopting) {
            let since = *self.live.wait.adopt_since.get_or_insert(now);
            if waited(since) < boards::ADOPT_PATIENCE + boards::VIEW_COLLECT_PATIENCE {
                return true;
            }
        }

        if let Some(id) = self.open_board_id() {
            match &self.live.wait.collect {
                None if self
                    .board()
                    .is_some_and(|board| board.ready && board.status.holds_more()) =>
                {
                    let done = Rc::new(Cell::new(false));
                    let collect = self.collect_board(id, cx);
                    let saved = done.clone();
                    cx.spawn(async move |_, _| {
                        collect.await;
                        saved.set(true);
                    })
                    .detach();
                    self.live.wait.collect = Some((now, done));
                    return true;
                }
                Some((since, done))
                    if !done.get() && waited(*since) < boards::VIEW_COLLECT_PATIENCE =>
                {
                    return true;
                }
                _ => {}
            }
        }

        if request.changes_library() && self.is_composing(cx) {
            let since = *self.live.wait.composing_since.get_or_insert(now);
            return waited(since) < crate::live::COMPOSITION_PATIENCE;
        }
        false
    }

    /// Whether text is being composed with an input method: in the open
    /// document, or in a text element of the open whiteboard.
    fn is_composing(&self, cx: &App) -> bool {
        (self.session.is_some() && self.editor.read(cx).is_composing())
            || self.board().is_some_and(|board| board.status.composing)
    }

    /// Carries `request` out and returns what goes back to the agent. See
    /// the [module docs](self) for the steps.
    pub(crate) fn carry_out(
        &mut self,
        request: &Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Answer {
        if !request.changes_library() {
            // "Save what is not saved yet", so that a read sees the text on
            // screen; also for the dry run of a change. A save that fails
            // is the writer's to hear of (it is retried), not the agent's:
            // the read goes ahead on what is saved.
            if request.is_flush() || request.is_dry_run() {
                self.flush(cx);
            }
            let executed = self.library.update(cx, |library, cx| {
                library
                    .change_outside(|store| (channel::execute(store, request), Vec::new()), cx)
                    .0
            });
            return executed.answer;
        }
        // Refused before anything is saved or looked up.
        if !self.agents_may_change(cx) {
            return Answer::refused(Refusal::SwitchedOff);
        }
        // The composition outlasted the patience of whoever called.
        if self.is_composing(cx) {
            return Answer::refused(Refusal::Composing);
        }
        let started = Instant::now();
        if !self.flush(cx) {
            let message = self
                .session
                .as_ref()
                .and_then(|session| session.failure.clone())
                .or_else(|| self.board().and_then(|board| board.failure.clone()))
                .unwrap_or_else(|| "the save failed".to_string());
            return Answer::refused(Refusal::CouldNotSave { message });
        }

        let saved = Instant::now();
        let before = {
            let library = self.library.read(cx);
            Before {
                tree: library.tree(),
                // Only what leaves the trash for good has its name there.
                trash: if matches!(
                    request.name(),
                    "delete_document" | "delete_folder" | "restore_document" | "restore_folder"
                ) {
                    library.trash().unwrap_or_default()
                } else {
                    Vec::new()
                },
            }
        };
        let (executed, refreshed) = self.library.update(cx, |library, cx| {
            library.change_outside(
                |store| {
                    let executed = channel::execute(store, request);
                    let outside = outside_changes(&executed.changes);
                    (executed, outside)
                },
                cx,
            )
        });
        if let Err(error) = refreshed {
            eprintln!("Inner Word: could not re-read the library after an agent's change: {error}");
        }
        if executed.changes.is_empty() {
            return executed.answer;
        }
        let changed = Instant::now();
        // The request may have brought the bytes of an image the editor
        // had found missing (with a change to this document or to another
        // one): it asks again.
        self.editor
            .update(cx, |editor, cx| editor.images_changed(cx));
        let adopted = self.take_over_open_document(&executed.changes, window, cx);
        let board = self.take_over_open_board(&executed.changes, cx);
        self.note_agent_change(request, &executed.changes, &before, adopted, board, cx);
        self.live.cost = Some(ChangeCost {
            save: saved - started,
            change: changed - saved,
            window: changed.elapsed(),
        });
        cx.notify();
        executed.answer
    }

    /// Brings the open document up to date when it is among `changes`:
    /// its text, comments and settings come into the editor, its title
    /// and the flag of a chosen title are read again. Returns what the
    /// editor took over.
    fn take_over_open_document(
        &mut self,
        changes: &[Change],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<TakenOver> {
        let open = self.open_document_id()?;
        let touched = changes.iter().any(|change| {
            matches!(
                change,
                Change::DocumentText { id }
                    | Change::DocumentComments { id }
                    | Change::DocumentSettings { id }
                    | Change::DocumentRenamed { id } if *id == open
            )
        });
        if !touched {
            return None;
        }
        let stored = match self.library.read(cx).document(open) {
            Ok(stored) => stored,
            // Deleted by the same request: the library's event closes it.
            Err(StoreError::NotFound { .. }) => return None,
            Err(error) => {
                self.toast_error(
                    format!("Could not read the document an agent changed: {error}"),
                    cx,
                );
                return None;
            }
        };
        let loaded = load_document(&stored.content);
        if let Some(error) = &loaded.recovered_from {
            // What was stored cannot be read as a document. The editor
            // keeps the text it has (the version from before the change is
            // in the version history), and the next edit saves it again.
            eprintln!(
                "Inner Word: \u{201c}{}\u{201d} could not be read after an agent's change: {error}",
                session_title(&stored.meta.title)
            );
            self.toast_error(
                "An agent stored a text that cannot be read. The text on screen is kept.",
                cx,
            );
            return None;
        }
        let revision_before = self.editor.read(cx).state().revision();
        let place = first_difference(self.editor.read(cx).document(), &loaded.document);
        let adopted = self
            .editor
            .update(cx, |editor, cx| editor.adopt_document(loaded.document, cx));
        // Vim mode hears of it here, before the editor's events of the
        // change reach it: this was not the writer's doing, so a command
        // they are in the middle of (an operator waiting for its motion,
        // a search being typed after "/") stays theirs.
        self.vim.update(cx, |vim, cx| vim.changed_elsewhere(cx));
        let revision = self.editor.read(cx).state().revision();
        // From here to the writer's next edit the editor holds what the
        // library holds.
        self.live.as_stored = Some(revision);

        let chosen = self
            .library
            .read(cx)
            .setting::<TitleFlag>(&keys::custom_title(open))
            .ok()
            .flatten()
            .is_some_and(|flag| flag.holds_for(&stored.meta));
        if let Some(session) = self.session.as_mut() {
            session.custom_title.store(chosen, Ordering::SeqCst);
            session.title = stored.meta.title.clone();
        }
        // Counted per block of the text as it was.
        self.character_count = None;
        self.limit_counter = None;
        self.update_window_title(window);
        Some(TakenOver {
            document: open,
            undo_step: adopted.undo_step,
            revision_before,
            revision,
            place,
        })
    }

    /// Brings the open whiteboard up to date when its scene is among
    /// `changes` (section 6.7): the stored scene after the change is taken
    /// into the view by the base rule, as one step of its undo
    /// ([`Workspace::adopt_outside_write`]: the delta from the stored scene
    /// the view descends from, which the flush before the change made the
    /// writer's last gesture, to the stored scene now; from here until the
    /// view answers, its posts are held). A rename changes the title only
    /// (the library's event does that); a whiteboard that went to the trash
    /// or was deleted closes as a page does. Returns the adopt that was
    /// started, for the note (its conflicts, its Undo).
    fn take_over_open_board(
        &mut self,
        changes: &[Change],
        cx: &mut Context<Self>,
    ) -> Option<(DocumentId, u64)> {
        let open = self.open_board_id()?;
        let touched = changes
            .iter()
            .any(|change| matches!(change, Change::DocumentText { id } if *id == open));
        if !touched {
            return None;
        }
        self.adopt_outside_write(open, cx);
        let seq = self.board()?.adopt_in_flight()?;
        Some((open, seq))
    }

    /// The open whiteboard answered an adopt (`seq`): when it was the one an
    /// agent's change started, the note says which of the agent's changes
    /// were not taken, and offers Undo while the writer has not drawn since.
    pub(crate) fn agent_adopt_answered(
        &mut self,
        board: DocumentId,
        seq: u64,
        rev: u64,
        conflicts: &[String],
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self
            .live
            .board_adopt
            .filter(|pending| pending.board == board && pending.seq == seq)
        else {
            return;
        };
        self.live.board_adopt = None;
        // The writer drew while the adopt was answered: no step undoes it.
        let drawn_since = self.board().is_some_and(|state| state.status.rev > rev);
        let open = self.open_board_id() == Some(board);
        let Some(last) = self.live.last.as_mut() else {
            return;
        };
        last.conflicts.extend(conflicts.iter().cloned());
        last.message = with_conflicts(&last.said, &last.conflicts);
        last.undo = (!drawn_since && open).then_some(Undo::Board {
            board,
            steps: pending.steps_before + 1,
            rev,
        });
        if self.agent_note_is_up(cx) {
            self.show_agent_note(cx);
        }
    }

    /// The adopt an agent's change started failed: the view was loaded again
    /// (its undo history is gone), so the note offers no Undo.
    pub(crate) fn agent_adopt_failed(&mut self, board: DocumentId, cx: &mut Context<Self>) {
        if self
            .live
            .board_adopt
            .take_if(|pending| pending.board == board)
            .is_none()
        {
            return;
        }
        if let Some(last) = self.live.last.as_mut() {
            last.undo = None;
        }
        if self.agent_note_is_up(cx) {
            self.show_agent_note(cx);
        }
    }

    /// The open whiteboard's canvas moved on (a post, a status): once the
    /// writer has drawn after the last adopt of an agent's change, the note
    /// no longer offers to undo it (the undo would take out the writer's
    /// gesture first).
    pub(crate) fn board_moved_on(&mut self, board: DocumentId, cx: &mut Context<Self>) {
        let Some(Undo::Board { board: of, rev, .. }) =
            self.live.last.as_ref().and_then(|last| last.undo)
        else {
            return;
        };
        if of != board {
            return;
        }
        let moved = self
            .board()
            .is_none_or(|state| self.open_board_id() != Some(board) || state.status.rev > rev);
        if !moved {
            return;
        }
        if let Some(last) = self.live.last.as_mut() {
            last.undo = None;
        }
        if self.agent_note_is_up(cx) {
            self.show_agent_note(cx);
        }
    }

    /// An event of the editor says that the text or the settings of the
    /// document changed. Returns whether that is only the change of an
    /// agent arriving: the editor holds what the library holds, and there
    /// is nothing to save. Otherwise the writer has edited, and what an
    /// agent changed before is no longer the newest step of the undo
    /// history: the note stops offering to undo it.
    pub(crate) fn text_is_what_an_agent_stored(&mut self, cx: &mut Context<Self>) -> bool {
        let revision = self.editor.read(cx).state().revision();
        if self.live.as_stored == Some(revision) {
            return true;
        }
        self.live.as_stored = None;
        let buried = self.live.last.as_ref().is_some_and(
            |last| matches!(last.undo, Some(Undo::Text { revision: at, .. }) if at != revision),
        );
        if buried {
            self.withdraw_text_undo(cx);
        }
        false
    }

    /// The editor holds another text (another document was opened, the
    /// open one was closed, a version was restored): its undo history is a
    /// new one, and its revisions start again. Nothing that was noted
    /// about the text that was there holds for this one.
    pub(crate) fn forget_agent_steps(&mut self, cx: &mut Context<Self>) {
        self.live.as_stored = None;
        self.withdraw_text_undo(cx);
    }

    /// The last changes of an agent are no longer the newest steps of the
    /// open document's undo history: the note stops offering to undo them.
    fn withdraw_text_undo(&mut self, cx: &mut Context<Self>) {
        let Some(last) = self
            .live
            .last
            .as_mut()
            .filter(|last| matches!(last.undo, Some(Undo::Text { .. })))
        else {
            return;
        };
        last.undo = None;
        if self.agent_note_is_up(cx) {
            self.show_agent_note(cx);
        }
    }

    // ----- the note -------------------------------------------------------------------

    fn agent_note_is_up(&self, cx: &App) -> bool {
        self.live
            .toast
            .is_some_and(|toast| self.toast_layer.read(cx).ids().contains(&toast))
    }

    /// What the names of the things `request` changed are.
    fn names(&self, changes: &[Change], before: &Before, cx: &App) -> Names {
        let library = self.library.read(cx);
        let mut names = Names::default();
        if let Subject::Document(id) = subject_of(changes) {
            names.document = library.document_meta(id).ok().map(|meta| meta.title);
            names.document_before = before.document(id);
        }
        let folder = changes.iter().rev().find_map(|change| match change {
            Change::FolderCreated { id }
            | Change::FolderRenamed { id }
            | Change::FolderMoved { id }
            | Change::FolderTrashed { id }
            | Change::FolderRestored { id }
            | Change::FolderDeleted { id } => Some(*id),
            _ => None,
        });
        if let Some(id) = folder {
            names.folder = library.folder(id).ok().map(|folder| folder.name);
            names.folder_before = before.folder(id);
        }
        names
    }

    /// How `subject` is named in the note about a run.
    fn subject_name(&self, subject: Subject, cx: &App) -> Option<String> {
        let library = self.library.read(cx);
        match subject {
            Subject::Document(id) => library
                .document_meta(id)
                .ok()
                .map(|meta| quoted(&meta.title)),
            Subject::Folder(id) => library
                .folder(id)
                .ok()
                .map(|folder| format!("the folder {}", quoted(&folder.name))),
            Subject::Library => None,
        }
    }

    /// Notes what `request` changed, as a new note or as one more change
    /// of the run the note that is up is about, and shows the note.
    fn note_agent_change(
        &mut self,
        request: &Request,
        changes: &[Change],
        before: &Before,
        adopted: Option<TakenOver>,
        board: Option<(DocumentId, u64)>,
        cx: &mut Context<Self>,
    ) {
        let who = request.author().unwrap_or(ANONYMOUS).to_string();
        let subject = subject_of(changes);
        // How this one change is taken back in one step. A change to the
        // open whiteboard is known once its view answered the adopt
        // (`agent_adopt_answered`).
        let undo = match (adopted, changes) {
            (Some(taken), _) if taken.undo_step => Some(Undo::Text {
                document: taken.document,
                steps: 1,
                revision: taken.revision,
            }),
            (None, [Change::DocumentTrashed { id }]) => {
                Some(Undo::Trash(LibraryItem::Document(*id)))
            }
            (None, [Change::FolderTrashed { id }]) => Some(Undo::Trash(LibraryItem::Folder(*id))),
            _ => None,
        };
        let place = adopted.and_then(|taken| taken.place.map(|block| (taken.document, block)));
        self.live.total += 1;

        let running = self.agent_note_is_up(cx);
        let joined = self
            .live
            .last
            .take()
            .filter(|last| running && last.who == who);
        // The steps of the open whiteboard's undo this change goes on top
        // of: those of the run, while the writer has not drawn since.
        let board_steps_before = match (&joined, board) {
            (
                Some(AgentChanges {
                    undo:
                        Some(Undo::Board {
                            board: of,
                            steps,
                            rev,
                        }),
                    ..
                }),
                Some((id, _)),
            ) if *of == id && self.board().is_some_and(|state| state.status.rev <= *rev) => *steps,
            _ => 0,
        };
        self.live.board_adopt = board.map(|(board, seq)| BoardAdopt {
            board,
            seq,
            steps_before: board_steps_before,
        });
        self.live.last = Some(match joined {
            Some(mut run) => {
                run.count += 1;
                if run.subject != subject {
                    run.subject = Subject::Library;
                }
                // One step undoes the run only when it is nothing but
                // steps of the open document's undo history, one on top of
                // the other (or of the open whiteboard's, once its view has
                // answered).
                run.undo = match (run.undo, undo, adopted) {
                    (
                        Some(Undo::Text {
                            document,
                            steps,
                            revision,
                        }),
                        Some(Undo::Text { .. }),
                        Some(taken),
                    ) if document == taken.document && revision == taken.revision_before => {
                        Some(Undo::Text {
                            document,
                            steps: steps + 1,
                            revision: taken.revision,
                        })
                    }
                    _ => None,
                };
                run.place = run.place.or(place);
                run.said = run_sentence(
                    &who,
                    run.count,
                    self.subject_name(run.subject, cx).as_deref(),
                );
                run.message = with_conflicts(&run.said, &run.conflicts);
                run
            }
            None => {
                let said = sentence(
                    &who,
                    operation_words(request, changes),
                    &self.names(changes, before, cx),
                );
                AgentChanges {
                    message: said.clone(),
                    said,
                    conflicts: Vec::new(),
                    who,
                    count: 1,
                    subject,
                    undo,
                    place,
                }
            }
        });
        self.show_agent_note(cx);
    }

    /// What the note offers for `changes` as things are now. A page that
    /// is open (one an agent updated reloads by itself) offers nothing.
    fn offer_for(&self, changes: &AgentChanges, cx: &App) -> Option<Offer> {
        let undo_holds = match changes.undo {
            Some(Undo::Board { board, rev, .. }) => self.board_undo_holds(board, rev),
            Some(_) => true,
            None => false,
        };
        if undo_holds {
            return Some(Offer::Undo);
        }
        match changes.subject {
            Subject::Document(id)
                if self.open_item_id() != Some(id)
                    && self.library.read(cx).tree().find_document(id).is_some() =>
            {
                Some(Offer::Open)
            }
            _ => None,
        }
    }

    /// Whether the undo of whiteboard `board` still takes out what agents
    /// changed: it is the open whiteboard and the writer has not drawn
    /// since the canvas was at `rev`.
    fn board_undo_holds(&self, board: DocumentId, rev: u64) -> bool {
        self.open_board_id() == Some(board)
            && self.board().is_some_and(|state| state.status.rev <= rev)
    }

    /// What the note about the last changes of agents offers.
    pub fn agent_change_offer(&self, cx: &App) -> Option<Offer> {
        self.live
            .last
            .as_ref()
            .and_then(|last| self.offer_for(last, cx))
    }

    /// Shows the note about the last changes, in place of the one that is
    /// up: there is never more than one.
    fn show_agent_note(&mut self, cx: &mut Context<Self>) {
        let Some(last) = &self.live.last else {
            return;
        };
        let mut toast = Toast::new(last.message.clone()).duration(TOAST_DURATION);
        toast = match self.offer_for(last, cx) {
            Some(Offer::Undo) => toast.action(Offer::Undo.label(), UndoAgentChange),
            Some(Offer::Open) => toast.action(Offer::Open.label(), ShowAgentChange),
            None => toast,
        };
        let up = self.live.toast.take();
        self.live.toast = Some(self.toast_layer.update(cx, |layer, cx| {
            if let Some(up) = up {
                layer.dismiss(up, cx);
            }
            layer.push(toast, cx)
        }));
    }

    /// The last changes agents made: one, or the run the note counted.
    pub fn last_agent_changes(&self) -> Option<&AgentChanges> {
        self.live.last.as_ref()
    }

    /// How many requests of agents changed something in this window.
    pub fn agent_change_count(&self) -> usize {
        self.live.total
    }

    /// What the last request of an agent that changed something cost on
    /// the main thread.
    pub fn last_agent_change_cost(&self) -> Option<ChangeCost> {
        self.live.cost
    }

    // ----- undo and "go to" -----------------------------------------------------------

    /// Takes the last changes of an agent back, where one step does it:
    /// the steps they added to the undo history of the open document (what
    /// the undo key takes out, all of a run at once), or what they moved
    /// to the trash. Says so when it cannot. Returns whether anything was
    /// taken back.
    pub fn undo_agent_change(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(last) = self.live.last.clone() else {
            self.show_toast(
                Toast::new("No agent has changed anything since Inner Word was opened"),
                cx,
            );
            return false;
        };
        let whose = if last.who == ANONYMOUS {
            "the agent\u{2019}s".to_string()
        } else {
            format!("{}\u{2019}s", last.who)
        };
        let what = if last.count == 1 {
            "change".to_string()
        } else {
            format!("{} changes", last.count)
        };
        let dismiss_note = |this: &mut Self, cx: &mut Context<Self>| {
            if let Some(up) = this.live.toast.take() {
                this.toast_layer
                    .update(cx, |layer, cx| layer.dismiss(up, cx));
            }
        };
        match last.undo {
            Some(Undo::Text {
                document,
                steps,
                revision,
            }) if self.open_document_id() == Some(document)
                && self.editor.read(cx).state().revision() == revision =>
            {
                let undone = self
                    .editor
                    .update(cx, |editor, cx| editor.undo_steps(steps, cx));
                if let Some(last) = self.live.last.as_mut() {
                    last.undo = None;
                }
                dismiss_note(self, cx);
                self.show_toast(Toast::new(format!("Undid {whose} {what}")).success(), cx);
                undone > 0
            }
            Some(Undo::Board { board, steps, rev }) if self.board_undo_holds(board, rev) => {
                drop(self.board_call(
                    board,
                    innerword_board_web::calls::UNDO,
                    innerword_board_web::calls::steps_argument(steps as u32),
                ));
                if let Some(last) = self.live.last.as_mut() {
                    last.undo = None;
                }
                dismiss_note(self, cx);
                self.show_toast(Toast::new(format!("Undid {whose} {what}")).success(), cx);
                true
            }
            Some(Undo::Trash(item)) => {
                let restored = self.library.update(cx, |library, cx| match item {
                    LibraryItem::Document(id) => library.restore_document(id, cx).map(|_| ()),
                    LibraryItem::Folder(id) => library.restore_folder(id, cx).map(|_| ()),
                });
                if let Some(last) = self.live.last.as_mut() {
                    last.undo = None;
                }
                dismiss_note(self, cx);
                match restored {
                    Ok(()) => {
                        self.show_toast(Toast::new("Brought back from the trash").success(), cx);
                        true
                    }
                    Err(error) => {
                        self.toast_error(format!("Could not bring it back: {error}"), cx);
                        false
                    }
                }
            }
            // The text was edited since, another document was opened, or
            // the change was none of the text.
            _ => {
                let text_of_open = matches!(last.subject, Subject::Document(id) if self.open_document_id() == Some(id));
                let board_of_open = matches!(last.subject, Subject::Document(id) if self.open_board_id() == Some(id));
                let mut toast = Toast::new(if text_of_open {
                    "This can no longer be undone in one step. The text from before is in the \
                     version history."
                } else if board_of_open {
                    "This can no longer be undone in one step. The drawing from before is in the \
                     version history."
                } else {
                    "This change cannot be undone in one step."
                })
                .duration(TOAST_DURATION);
                if text_of_open || board_of_open {
                    toast = toast.action("Version History", ShowVersionHistory);
                }
                dismiss_note(self, cx);
                self.show_toast(toast, cx);
                false
            }
        }
    }

    /// Goes to the last change of an agent: opens its document when it is
    /// not the open one, and puts the caret at the first paragraph that
    /// changed when that is known. The keyboard goes to the text. Returns
    /// whether there was somewhere to go.
    pub fn show_agent_change(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(last) = self.live.last.clone() else {
            self.show_toast(
                Toast::new("No agent has changed anything since Inner Word was opened"),
                cx,
            );
            return false;
        };
        let Subject::Document(id) = last.subject else {
            self.show_toast(Toast::new("The last change was not to one document"), cx);
            return false;
        };
        if self.library.read(cx).tree().find_document(id).is_none() {
            self.show_toast(
                Toast::new("That document is not in the library any more"),
                cx,
            );
            return false;
        }
        if !self.open_document(id, true, window, cx) {
            return false;
        }
        if let Some(up) = self.live.toast.take() {
            self.toast_layer
                .update(cx, |layer, cx| layer.dismiss(up, cx));
        }
        let block = last
            .place
            .filter(|(document, _)| *document == id)
            .and_then(|(_, block)| self.editor.read(cx).document().block_index(block));
        if let Some(block) = block {
            self.editor.update(cx, |editor, cx| {
                editor.edit_with(
                    Autoscroll::Center,
                    |state| state.set_caret(Position::new(block, 0)),
                    cx,
                );
            });
        }
        true
    }

    pub(crate) fn on_undo_agent_change(
        &mut self,
        _: &UndoAgentChange,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.undo_agent_change(cx);
    }

    pub(crate) fn on_show_agent_change(
        &mut self,
        _: &ShowAgentChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_agent_change(window, cx);
    }

    // ----- the state line -------------------------------------------------------------

    /// One line about the channel and what came in on it, for the `state`
    /// step of a scripted run: whether the channel is open, the switch,
    /// how many requests wait, how many changed something, and the last
    /// of them (who and what, how many the note counts, what it offers,
    /// what it cost on the main thread).
    pub fn describe_live(&self, cx: &App) -> String {
        let millis = |duration: Duration| format!("{:.1} ms", duration.as_secs_f64() * 1000.);
        let cost = match self.live.cost {
            Some(cost) => format!(
                " cost=[save {}, change {}, window {}]",
                millis(cost.save),
                millis(cost.change),
                millis(cost.window)
            ),
            None => String::new(),
        };
        let last = match &self.live.last {
            Some(last) => format!(
                " last={:?} run={} offers={}{cost}",
                last.message,
                last.count,
                self.offer_for(last, cx)
                    .map_or("nothing", |offer| offer.label()),
            ),
            None => String::new(),
        };
        format!(
            "channel={} switch={} waiting={} changes={}{last}",
            if crate::live::is_open(cx) {
                "open"
            } else {
                "closed"
            },
            if self.agents_may_change(cx) {
                "on"
            } else {
                "off"
            },
            crate::live::waiting(cx),
            self.live.total,
        )
    }
}

/// What the editor took over from an agent's change.
#[derive(Clone, Copy, Debug)]
struct TakenOver {
    document: DocumentId,
    /// The change is a step of the undo history.
    undo_step: bool,
    /// The revision of the text before and after.
    revision_before: u64,
    revision: u64,
    /// The first block that changed.
    place: Option<BlockId>,
}
