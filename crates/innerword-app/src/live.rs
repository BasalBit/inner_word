//! Agents that write while the app is open: the app's end of the channel.
//!
//! Until 0.6.0 an agent could change the library only while the app was
//! closed: the app holds text that is not saved yet and its own picture of
//! the library, and a second writer underneath it would lose one side's
//! work. Now a change by an agent **is carried out by the app**: the
//! `innerword` tool (and its MCP server) hands the running app the operation,
//! the app does it between two key presses, shows it, and answers. The
//! protocol, the client and the code that carries an operation out are in
//! `innerword_agent::live`, without GPUI; this module and
//! `workspace/live.rs` are the app around them.
//!
//! # What happens to a request
//!
//! 1. [`open`] binds the socket when the app has the lock of its data
//!    directory (`main`), and starts a thread that does nothing but accept
//!    connections. Each request it reads goes into a queue for the main
//!    thread.
//! 2. A task on the main thread takes the requests out of the queue, one
//!    after the other, in the order they came. For each, in one go, so
//!    that no key press comes in between
//!    ([`Workspace::carry_out`](crate::Workspace::carry_out)): what is
//!    unsaved in the open document is saved; the request is carried out on
//!    the library's own store; the library, the sidebar and the open
//!    document are brought up to date from the list of what changed (the
//!    text comes into the editor with the caret, the selection and the
//!    view kept, as one step of undo); the writer is told in one quiet
//!    toast; the answer goes back.
//! 3. A change waits while text is being composed with an input method
//!    (up to [`COMPOSITION_PATIENCE`]; a composition is never broken),
//!    and the requests behind it wait with it. Every request waits, at
//!    most a second each, for a whiteboard that is closing to be saved and
//!    for what the open whiteboard's page holds and has not posted
//!    (`Workspace::must_wait`), so that an agent's change is merged against
//!    the writer's last gesture and nothing late is saved over it.
//!
//! # The files of a page, the pictures of a whiteboard
//!
//! A page an agent adds travels with its files (`add_page`, up to 64 MB
//! before base64), and a drawing an agent makes on a whiteboard with the
//! pictures its `image` elements name (`create_board`, `add_to_board`, up
//! to 32 MB). They are not written on the main thread: the thread that
//! accepts the request stores them through a connection of its own to
//! `images.db` before it queues the request
//! (`innerword_agent::live::store_page_files`), so the main thread only
//! writes the row. While the owner's switch is off that thread
//! refuses such a request at once and stores nothing: it knows the switch
//! ([`set_switch`], which the workspace calls whenever the setting is read
//! or changed).
//!
//! # Quitting
//!
//! [`close`] runs in the quit hook (`cmd-q`, the last window closed,
//! `SIGTERM`): the socket goes away, and every request that is still in
//! the queue is answered "Inner Word is quitting", on which the tool waits
//! for the app to be gone and writes directly. A request is carried out
//! whole or not at all: carrying out and answering are one turn of the
//! main thread, and quitting is another.
//!
//! # The switch
//!
//! "Agents can make changes while Inner Word is open" ([`SWITCH`]) is a
//! setting of the library, on unless it was switched off
//! (`innerword_agent::live::CHANGES_SETTING`); the Agents panel has it. Off
//! means the rule of before: agents read at any time, and when they want to
//! change something they are told to ask for the switch, or for the app to
//! be quit ([`SWITCH_OFF`] says so in the panel). Saving on request
//! (so that an agent reads the text as it is on screen) is not a change
//! and goes on.

use std::{
    collections::VecDeque,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context as TaskContext, Poll, Waker},
    time::Duration,
};

use gpui::{App, AsyncApp, Global, Subscription, Task, WindowHandle, actions};
use innerword_agent::live::{Answer, Closer, Incoming, Listener, Refusal};
use innerword_library::store::{DATABASE_FILE_NAME, ImageFile};
use innerword_ui::CommandDescriptor;

use crate::workspace::Workspace;

pub use innerword_agent::live::CHANGES_SETTING;

actions!(
    live,
    [
        /// Take out again what an agent changed last: the steps it added to
        /// the text, or what it moved to the trash.
        UndoAgentChange,
        /// Open the document an agent changed last, at the place of the
        /// change.
        ShowAgentChange,
    ]
);

/// What the owner's switch is called, in the Agents panel: the name the tool
/// gives it when it tells an agent to ask for it.
pub const SWITCH: &str = innerword_agent::live::CHANGES_SWITCH;

/// What the switch means while it is on.
pub const SWITCH_ON: &str = "Inner Word carries out what an agent asks for, shows it in the \
    window as it arrives and tells you in a note at the bottom. A version of the document from \
    before each change of its text is kept in its version history.";

/// What the switch means while it is off. It says what the tool tells an
/// agent then, in the tool's order: to ask for the switch, or for the app to
/// be quit (`innerword_agent::Refusal::SwitchedOff`; a test holds the two
/// together).
pub const SWITCH_OFF: &str = "While Inner Word is open agents can only read. To change \
    something they are told to ask you to switch this on, or to quit Inner Word.";

/// How long a change waits for a text that is being composed with an input
/// method before it is refused. The tool waits ten seconds for the app, so
/// the refusal reaches it.
pub const COMPOSITION_PATIENCE: Duration = Duration::from_secs(3);

/// How often a waiting change looks whether the composition has ended.
pub const COMPOSITION_POLL: Duration = Duration::from_millis(50);

/// How long the note about a change stays.
pub const TOAST_DURATION: Duration = Duration::from_secs(8);

/// Who made a change, when the agent gave no name.
pub const ANONYMOUS: &str = "An agent";

/// Environment variable: `off` keeps this copy of the app from opening the
/// channel, so it behaves towards agents like a version from before the
/// channel existed (they are told to quit Inner Word first). For trying
/// that out; the owner's own way to say no is the switch.
pub const CHANNEL_ENV: &str = "INNER_WORD_AGENT_CHANNEL";

/// The commands of this module, for the palette and the list of keyboard
/// shortcuts, beside "Agents" in the app's own category. Neither has a key
/// of its own.
pub fn commands() -> Vec<CommandDescriptor> {
    let category = crate::actions::CATEGORY_APP;
    vec![
        CommandDescriptor::new(
            "Undo the last change by an agent",
            category,
            UndoAgentChange,
        )
        .keywords(["claude", "codex", "revert", "take back", "restore", "ai"]),
        CommandDescriptor::new(
            "Go to the last change by an agent",
            category,
            ShowAgentChange,
        )
        .keywords(["claude", "codex", "show", "open", "where", "ai"]),
    ]
}

// ----- the queue between the accepting thread and the main thread ---------------------------

struct Queue<T> {
    items: VecDeque<T>,
    waker: Option<Waker>,
    closed: bool,
}

/// A queue that a thread fills and a task on the main thread empties. No
/// more than that: a mutex, and the waker of the task that waits.
pub(crate) struct Handoff<T>(Arc<Mutex<Queue<T>>>);

impl<T> Clone for Handoff<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Handoff<T> {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Mutex::new(Queue {
            items: VecDeque::new(),
            waker: None,
            closed: false,
        })))
    }

    fn lock(&self) -> MutexGuard<'_, Queue<T>> {
        // A poisoned lock only means that a thread panicked while it held
        // it; the queue is a queue all the same.
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Puts `item` at the end and wakes the task that waits. Gives the
    /// item back when the queue is closed.
    pub(crate) fn push(&self, item: T) -> Result<(), T> {
        let waker = {
            let mut queue = self.lock();
            if queue.closed {
                return Err(item);
            }
            queue.items.push_back(item);
            queue.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        Ok(())
    }

    /// Puts `item` back at the front: it was taken and could not be dealt
    /// with yet. Gives it back when the queue is closed.
    pub(crate) fn put_back(&self, item: T) -> Result<(), T> {
        let mut queue = self.lock();
        if queue.closed {
            return Err(item);
        }
        queue.items.push_front(item);
        Ok(())
    }

    /// The next item, when one is there; `None` once the queue is closed.
    pub(crate) fn next(&self) -> Next<'_, T> {
        Next(self)
    }

    /// Closes the queue and hands out what was still in it. Whoever waits
    /// in [`Handoff::next`] gets `None`.
    pub(crate) fn close(&self) -> Vec<T> {
        let (items, waker) = {
            let mut queue = self.lock();
            queue.closed = true;
            (queue.items.drain(..).collect(), queue.waker.take())
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        items
    }

    /// How many items wait.
    pub(crate) fn len(&self) -> usize {
        self.lock().items.len()
    }
}

/// The future of [`Handoff::next`].
pub(crate) struct Next<'a, T>(&'a Handoff<T>);

impl<T> Future for Next<'_, T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        let mut queue = self.0.lock();
        if let Some(item) = queue.items.pop_front() {
            return Poll::Ready(Some(item));
        }
        if queue.closed {
            return Poll::Ready(None);
        }
        queue.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

// ----- the channel ------------------------------------------------------------------------

/// The open channel: an app global for as long as agents can reach the app.
pub struct Channel {
    socket: PathBuf,
    closer: Closer,
    queue: Handoff<Incoming>,
    /// The owner's switch as the accepting thread knows it.
    switch: Arc<AtomicBool>,
    _task: Task<()>,
    _quit: Subscription,
}

impl Global for Channel {}

/// Opens the channel for the library in `data_dir`, whose lock this
/// process holds, and has the workspace of `window` carry out what comes
/// in. Returns where agents find it (the path of the socket).
///
/// Call it once the main window is open (`main` does, for the real app and
/// for a scripted run; a test that is about the channel calls it with a
/// temporary directory). A second call closes the first channel.
pub fn open(
    data_dir: &Path,
    window: WindowHandle<Workspace>,
    cx: &mut App,
) -> std::io::Result<PathBuf> {
    close(cx);
    let listener = Listener::bind(data_dir)?;
    let closer = listener.closer();
    let socket = listener.path().to_path_buf();
    let queue = Handoff::new();
    let accepted = queue.clone();
    // The switch as it is now; the workspace keeps it in step from here on.
    let on = window
        .read(cx)
        .map(|workspace| workspace.agents_may_change(cx))
        .unwrap_or(true);
    let switch = Arc::new(AtomicBool::new(on));
    let known = switch.clone();
    let database = data_dir.join(DATABASE_FILE_NAME);
    let spawned = std::thread::Builder::new()
        .name("inner-word-agents".to_string())
        .spawn(move || {
            // The page files of a request are written through this
            // connection, never on the main thread.
            let mut files: Option<ImageFile> = None;
            while let Some(mut incoming) = listener.accept() {
                if innerword_agent::live::carries_page_files(incoming.request()) {
                    if !known.load(Ordering::SeqCst) {
                        // Refused at once: nothing is stored.
                        refuse(incoming, Refusal::SwitchedOff);
                        continue;
                    }
                    let files = files.get_or_insert_with(|| ImageFile::beside(&database));
                    if let Err(error) =
                        innerword_agent::live::store_page_files(incoming.request_mut(), files)
                    {
                        incoming.answer(Answer::failed(&error)).ok();
                        continue;
                    }
                }
                if let Err(incoming) = accepted.push(incoming) {
                    refuse(incoming, Refusal::Quitting);
                    break;
                }
            }
        });
    if let Err(error) = spawned {
        closer.close();
        return Err(error);
    }
    let task = cx.spawn({
        let queue = queue.clone();
        async move |cx| serve(queue, window, cx).await
    });
    // Quitting, however it comes about: nobody may be left waiting for an
    // app that is on its way out.
    let quit = cx.on_app_quit(|cx| {
        close(cx);
        async {}
    });
    cx.set_global(Channel {
        socket: socket.clone(),
        closer,
        queue,
        switch,
        _task: task,
        _quit: quit,
    });
    Ok(socket)
}

/// Tells the thread that accepts requests where the owner's switch stands
/// ("Agents can make changes while Inner Word is open"): while it is off a
/// request that brings the files of a page is refused there, before any of
/// them is stored. The workspace calls it whenever the setting is read or
/// changed. Does nothing without a channel.
pub fn set_switch(on: bool, cx: &App) {
    if let Some(channel) = cx.try_global::<Channel>() {
        channel.switch.store(on, Ordering::SeqCst);
    }
}

/// Where the thread that accepts requests has the owner's switch; `None`
/// without a channel. For tests.
pub fn switch_known_to_the_listener(cx: &App) -> Option<bool> {
    cx.try_global::<Channel>()
        .map(|channel| channel.switch.load(Ordering::SeqCst))
}

/// What `main` does once the main window is open: opens the channel for
/// the library in `data_dir` unless [`CHANNEL_ENV`] says not to, and says
/// on stderr when it could not be opened (agents are then told to quit Inner
/// Word before they change anything, as before 0.6.0).
pub fn open_at_launch(data_dir: &Path, window: WindowHandle<Workspace>, cx: &mut App) {
    if std::env::var_os(CHANNEL_ENV).is_some_and(|value| value == "off") {
        eprintln!(
            "Inner Word: {CHANNEL_ENV}=off, agents can change the library only while Inner \
             Word is closed"
        );
        return;
    }
    if let Err(error) = open(data_dir, window, cx) {
        eprintln!(
            "Inner Word: agents can change the library only while Inner Word is closed, the \
             channel for their changes could not be opened: {error}"
        );
    }
}

/// The channel of an app that holds the lock of a data directory and has no
/// window with its library (it shows the window that says the library
/// cannot be opened): every request is refused with the reason.
struct NoLibrary {
    closer: Closer,
    _quit: Subscription,
}

impl Global for NoLibrary {}

/// Opens the channel for an app that has the lock of `data_dir` and could
/// not open the library in it. A tool that asks for a change is told that
/// Inner Word shows no window with this library and that the app has to
/// be quit, instead of taking it for a version from before the channel.
/// A read is not held up: its request to save first is refused at once.
pub fn open_without_library(data_dir: &Path, cx: &mut App) -> std::io::Result<PathBuf> {
    close(cx);
    let listener = Listener::bind(data_dir)?;
    let closer = listener.closer();
    let socket = listener.path().to_path_buf();
    let spawned = std::thread::Builder::new()
        .name("inner-word-agents".to_string())
        .spawn(move || {
            while let Some(incoming) = listener.accept() {
                refuse(incoming, Refusal::NoLibrary);
            }
        });
    if let Err(error) = spawned {
        closer.close();
        return Err(error);
    }
    let quit = cx.on_app_quit(|cx| {
        close(cx);
        async {}
    });
    cx.set_global(NoLibrary {
        closer,
        _quit: quit,
    });
    Ok(socket)
}

/// Closes the channel: the socket goes away, and every request that waits
/// is answered "Inner Word is quitting". Does nothing without a channel.
pub fn close(cx: &mut App) {
    if cx.has_global::<NoLibrary>() {
        cx.remove_global::<NoLibrary>().closer.close();
    }
    if !cx.has_global::<Channel>() {
        return;
    }
    let channel = cx.remove_global::<Channel>();
    channel.closer.close();
    for incoming in channel.queue.close() {
        refuse(incoming, Refusal::Quitting);
    }
}

/// Whether agents can reach this app.
pub fn is_open(cx: &App) -> bool {
    cx.has_global::<Channel>()
}

/// Where agents reach this app: the path of the socket.
pub fn socket(cx: &App) -> Option<PathBuf> {
    cx.try_global::<Channel>()
        .map(|channel| channel.socket.clone())
}

/// How many requests wait for the main thread.
pub fn waiting(cx: &App) -> usize {
    cx.try_global::<Channel>()
        .map_or(0, |channel| channel.queue.len())
}

/// Answers `incoming` with a refusal (which is not sent to a tool that has
/// stopped waiting and was told so already).
fn refuse(incoming: Incoming, refusal: Refusal) {
    incoming.refuse(refusal).ok();
}

/// What one look at the request at the head of the queue came to.
enum Turn {
    /// It waits: for a composition to end, for a whiteboard's page.
    Wait,
    /// The tool stopped waiting and was told that nothing was changed.
    TooLate,
    /// It was carried out; this goes back.
    Done(Answer),
}

/// The task on the main thread: one request after the other.
async fn serve(queue: Handoff<Incoming>, window: WindowHandle<Workspace>, cx: &mut AsyncApp) {
    // Since when the request at the head of the queue has been waiting (for
    // a composition to end, for a whiteboard: `Workspace::must_wait`, which
    // keeps the patience of each wait).
    let mut waiting_since = None;
    while let Some(incoming) = queue.next().await {
        let now = cx.background_executor().now();
        let first = waiting_since.is_none();
        let turn = window.update(cx, |workspace, window, cx| {
            // A change must not break a composition, nor be merged into a
            // whiteboard before what its page holds is saved: let it wait,
            // and the requests behind it.
            if workspace.must_wait(incoming.request(), first, window, cx) {
                return Turn::Wait;
            }
            // From here on in one go. A tool that has stopped waiting was
            // told that nothing was changed, and nothing is.
            if !incoming.begin() {
                return Turn::TooLate;
            }
            Turn::Done(workspace.carry_out(incoming.request(), window, cx))
        });
        // The update has returned: every view has heard of the change (the
        // sidebar, the text, the note at the bottom). Now the agent does.
        match turn {
            Ok(Turn::Done(answer)) => {
                waiting_since = None;
                if let Err(error) = incoming.answer(answer) {
                    eprintln!(
                        "Inner Word: an agent did not take the answer to its request: {error}"
                    );
                }
            }
            Ok(Turn::TooLate) => waiting_since = None,
            Ok(Turn::Wait) => {
                waiting_since.get_or_insert(now);
                if incoming.expired() {
                    // Its tool has given up meanwhile: the next one's turn.
                    waiting_since = None;
                    continue;
                }
                if let Err(incoming) = queue.put_back(incoming) {
                    refuse(incoming, Refusal::Quitting);
                    return;
                }
                cx.background_executor().timer(COMPOSITION_POLL).await;
            }
            // The window is gone: the app is on its way out.
            Err(_) => {
                refuse(incoming, Refusal::Quitting);
                waiting_since = None;
            }
        }
    }
}
