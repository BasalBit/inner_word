//! The backend for Claude Code.
//!
//! [`ClaudeCode`] finds the `claude` program ([`locate`]), asks it for its version and
//! the sign-in state ([`status`]) and for the models it offers ([`catalog`]), and runs
//! one long-lived process per conversation ([`process`]) in the program's streaming
//! mode (`-p --input-format stream-json --output-format stream-json`), started with the
//! flags and the environment of [`args`] so that it is a writing assistant and nothing
//! of the user's coding setup: no settings files, no hooks, no skills or commands, no
//! `CLAUDE.md`, no MCP server but the one given, no built-in tool, no session left in
//! the user's history. The user's sign-in is the program's own: nothing here reads or
//! stores a credential.
//!
//! The program's output is read by [`stream`], whose shapes are those recorded from
//! Claude Code 2.1.322 in `tests/fixtures`.

pub mod args;
pub mod catalog;
pub mod locate;
pub mod process;
pub mod run;
pub mod status;
pub mod stream;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

use crate::backend::{
    AssistantError, AssistantEvent, Availability, Backend, BackendInfo, Catalog, Command,
    Conversation, ConversationOptions, Installation, SignIn, TurnEnd,
};
use crate::events::EventSink;

pub use catalog::{CATALOG_AS_OF, builtin_catalog};
pub use locate::Search;
pub use status::MINIMUM_VERSION;

use process::{Input, Launcher, Worker};

/// Claude Code as the agent behind the assistant.
///
/// ```no_run
/// use std::sync::Arc;
/// use writer_assistant::{Backend, ChatSession, ClaudeCode};
///
/// let claude = ClaudeCode::new();
/// // On a background thread: is it installed, is the user signed in?
/// let availability = claude.availability();
/// if availability.is_ready() {
///     let mut chat = ChatSession::quick(Arc::new(claude));
///     chat.ask_quick("A word for very tired?", None);
/// }
/// ```
#[derive(Clone)]
pub struct ClaudeCode {
    shared: Arc<Shared>,
}

struct Shared {
    /// A program given by the caller instead of the one that is looked for.
    program: Option<PathBuf>,
    /// Where to look; `None`: on this Mac.
    search: Option<Search>,
    availability: Mutex<Option<Availability>>,
    catalog: Mutex<Option<Catalog>>,
    /// One look at a time: a second caller waits for the first and takes its answer.
    looking: Mutex<()>,
}

impl Default for ClaudeCode {
    fn default() -> Self {
        ClaudeCode::new()
    }
}

impl std::fmt::Debug for ClaudeCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeCode")
            .field("program", &self.shared.program)
            .finish()
    }
}

impl ClaudeCode {
    fn with(program: Option<PathBuf>, search: Option<Search>) -> Self {
        ClaudeCode {
            shared: Arc::new(Shared {
                program,
                search,
                availability: Mutex::new(None),
                catalog: Mutex::new(None),
                looking: Mutex::new(()),
            }),
        }
    }

    /// Claude Code as it is installed on this Mac: the program is looked for in the
    /// folders of the `PATH` and of the usual installers and, failing that, through the
    /// user's login shell ([`Search::this_mac`]). Nothing is looked for until
    /// [`Backend::availability`] is asked.
    pub fn new() -> Self {
        ClaudeCode::with(None, None)
    }

    /// Claude Code as this program, without looking for it. Tests run a stand-in.
    pub fn with_program(program: impl Into<PathBuf>) -> Self {
        ClaudeCode::with(Some(program.into()), None)
    }

    /// Claude Code looked for where `search` says. Tests search a temporary home.
    pub fn with_search(search: Search) -> Self {
        ClaudeCode::with(None, Some(search))
    }

    /// Gives the backend a catalog it learned in an earlier run of the app (a
    /// [`Catalog`] serialises; the app may keep the last one it was given), so that the
    /// pickers show the program's own names from the first moment.
    /// [`Backend::refresh_catalog`] replaces it with the program's current answer.
    pub fn seed_catalog(&self, catalog: Catalog) {
        if let Ok(mut cache) = self.shared.catalog.lock()
            && cache.is_none()
        {
            *cache = Some(catalog);
        }
    }
}

impl Shared {
    fn cached(&self) -> Option<Availability> {
        self.availability.lock().ok()?.clone()
    }

    fn look(&self) -> Availability {
        // Whoever comes while a look is under way waits for it.
        let _looking = self.looking.lock();
        let program = match &self.program {
            Some(program) => locate::is_executable(program).then(|| program.clone()),
            None => match &self.search {
                Some(search) => search.find(),
                None => Search::this_mac().find(),
            },
        };
        let found = match program {
            None => Availability::NotInstalled,
            Some(program) => {
                let version = status::version(&program);
                let sign_in = status::sign_in(&program);
                Availability::Installed(Installation {
                    program,
                    version,
                    sign_in,
                })
            }
        };
        if let Ok(mut cache) = self.availability.lock() {
            *cache = Some(found.clone());
        }
        found
    }

    fn availability(&self) -> Availability {
        if let Some(known) = self.cached() {
            return known;
        }
        // Two callers at once: the second finds the first one's answer.
        let waited = self.looking.lock();
        drop(waited);
        self.cached().unwrap_or_else(|| self.look())
    }
}

impl Launcher for Arc<Shared> {
    fn program(&self, fresh: bool) -> Result<PathBuf, AssistantError> {
        let availability = if fresh {
            self.look()
        } else {
            self.availability()
        };
        match availability {
            Availability::NotInstalled => Err(AssistantError::NotInstalled),
            Availability::Installed(installation) => {
                if installation.sign_in == SignIn::SignedOut {
                    return Err(AssistantError::NotSignedIn);
                }
                if let Some(version) = &installation.version
                    && !status::is_at_least(version, MINIMUM_VERSION)
                {
                    return Err(AssistantError::Outdated {
                        found: version.clone(),
                        needed: MINIMUM_VERSION.to_string(),
                    });
                }
                Ok(installation.program)
            }
        }
    }
}

impl Backend for ClaudeCode {
    fn info(&self) -> BackendInfo {
        info()
    }

    fn availability(&self) -> Availability {
        self.shared.availability()
    }

    fn recheck(&self) -> Availability {
        self.shared.look()
    }

    fn cached_availability(&self) -> Option<Availability> {
        self.shared.cached()
    }

    fn catalog(&self) -> Catalog {
        self.shared
            .catalog
            .lock()
            .ok()
            .and_then(|catalog| catalog.clone())
            .unwrap_or_else(builtin_catalog)
    }

    fn refresh_catalog(&self) -> Catalog {
        if let Availability::Installed(installation) = self.shared.availability()
            && installation.sign_in != SignIn::SignedOut
            && let Some(found) = catalog::probe(&installation.program, None)
            && let Ok(mut cache) = self.shared.catalog.lock()
        {
            *cache = Some(found);
        }
        self.catalog()
    }

    fn start(&self, options: ConversationOptions, events: EventSink) -> Conversation {
        let alive = Arc::new(AtomicBool::new(true));
        let (inbox_tx, inbox) = channel::<Input>();
        let worker = Worker::new(
            self.shared.clone(),
            options,
            events.clone(),
            alive.clone(),
            inbox,
            inbox_tx.clone(),
        );
        let started = std::thread::Builder::new()
            .name("writer-assistant-claude".into())
            .spawn(move || worker.run());
        if started.is_err() {
            alive.store(false, Ordering::SeqCst);
        }
        let closed = Arc::new(AtomicBool::new(false));
        let closing = alive.clone();
        Conversation::new(
            move |command| {
                // The thread is gone or going (the conversation was closed, or the
                // thread could not be started): a message is still answered.
                let refuse = |turn| {
                    events.send(AssistantEvent::Finished {
                        turn,
                        text: None,
                        end: TurnEnd::Failed(AssistantError::Exited {
                            code: None,
                            detail: "the conversation has ended".to_string(),
                        }),
                        usage: None,
                    });
                };
                if closed.load(Ordering::SeqCst) {
                    if let Command::Send { turn, .. } = command {
                        refuse(turn);
                    }
                    return;
                }
                if command == Command::Close {
                    closed.store(true, Ordering::SeqCst);
                    closing.store(false, Ordering::SeqCst);
                }
                if let Err(unsent) = inbox_tx.send(Input::Command(command))
                    && let Input::Command(Command::Send { turn, .. }) = unsent.0
                {
                    refuse(turn);
                }
            },
            alive,
        )
        .with_live_selection()
    }
}

/// What Claude Code is called and how it is installed and signed in to.
pub fn info() -> BackendInfo {
    BackendInfo {
        id: "claude-code".to_string(),
        name: "Claude Code".to_string(),
        vendor: "Anthropic".to_string(),
        install_help: "Install Claude Code (claude.com/claude-code), then open the settings again."
            .to_string(),
        install_url: "https://claude.com/claude-code".to_string(),
        sign_in_help: "Open Terminal, run claude and sign in.".to_string(),
    }
}
