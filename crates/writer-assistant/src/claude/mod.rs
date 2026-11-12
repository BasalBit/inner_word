//! The backend for Claude Code.
//!
//! This first commit is the interface only: the program is not looked for yet and a
//! conversation answers every message with [`AssistantError::NotInstalled`]. The
//! implementation follows in this module without changing what is public here.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::backend::{
    AssistantError, AssistantEvent, Availability, Backend, BackendInfo, Catalog, CatalogSource,
    Command, Conversation, ConversationOptions, EffortOption, ModelOption, Selection, TurnEnd,
};
use crate::events::EventSink;

/// The day the built-in table of models and effort levels was read from Claude Code's
/// documentation and program.
pub const CATALOG_AS_OF: &str = "2026-11-12";

/// Claude Code as the agent behind the assistant.
#[derive(Clone)]
pub struct ClaudeCode {
    shared: Arc<Shared>,
}

struct Shared {
    /// A program given by the caller instead of the one that is looked for.
    program: Option<PathBuf>,
    availability: Mutex<Option<Availability>>,
    catalog: Mutex<Option<Catalog>>,
}

impl Default for ClaudeCode {
    fn default() -> Self {
        ClaudeCode::new()
    }
}

impl ClaudeCode {
    /// Claude Code as it is installed on this Mac: the program is looked for in the
    /// usual places and, failing that, through the user's login shell.
    pub fn new() -> Self {
        ClaudeCode {
            shared: Arc::new(Shared {
                program: None,
                availability: Mutex::new(None),
                catalog: Mutex::new(None),
            }),
        }
    }

    /// Claude Code as this program, without looking for it (tests run a stand-in).
    pub fn with_program(program: impl Into<PathBuf>) -> Self {
        ClaudeCode {
            shared: Arc::new(Shared {
                program: Some(program.into()),
                availability: Mutex::new(None),
                catalog: Mutex::new(None),
            }),
        }
    }
}

impl Backend for ClaudeCode {
    fn info(&self) -> BackendInfo {
        info()
    }

    fn availability(&self) -> Availability {
        if let Some(known) = self.cached_availability() {
            return known;
        }
        self.recheck()
    }

    fn recheck(&self) -> Availability {
        let found = Availability::NotInstalled;
        if let Ok(mut cache) = self.shared.availability.lock() {
            *cache = Some(found.clone());
        }
        found
    }

    fn cached_availability(&self) -> Option<Availability> {
        self.shared.availability.lock().ok()?.clone()
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
        self.catalog()
    }

    fn start(&self, _options: ConversationOptions, events: EventSink) -> Conversation {
        let _ = &self.shared.program;
        let alive = Arc::new(AtomicBool::new(true));
        Conversation::new(
            move |command| {
                if let Command::Send { turn, .. } = command {
                    events.send(AssistantEvent::Finished {
                        turn,
                        text: None,
                        end: TurnEnd::Failed(AssistantError::NotInstalled),
                        usage: None,
                    });
                }
            },
            alive,
        )
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

/// The models and effort levels of Claude Code as this version of the app knows them:
/// the aliases the program accepts for `--model` and the levels it accepts for
/// `--effort`. [`Backend::refresh_catalog`] replaces it with what the installed program
/// says of itself.
pub fn builtin_catalog() -> Catalog {
    let model = |id: &str, name: &str, description: &str| ModelOption {
        id: id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        efforts: None,
    };
    let effort = |id: &str, name: &str| EffortOption {
        id: id.to_string(),
        name: name.to_string(),
    };
    Catalog {
        models: vec![
            model("haiku", "Haiku", "Fastest, for short answers"),
            model("sonnet", "Sonnet", "Balanced"),
            model("opus", "Opus", "Most capable"),
            model("fable", "Fable", "Most capable, newest"),
        ],
        efforts: vec![
            effort("low", "Low"),
            effort("medium", "Medium"),
            effort("high", "High"),
            effort("xhigh", "Extra high"),
            effort("max", "Maximum"),
        ],
        quick_default: Selection::named("haiku", "low"),
        side_default: Selection::named("sonnet", "medium"),
        agent_default_label: "Claude Code's default".to_string(),
        source: CatalogSource::BuiltIn {
            as_of: CATALOG_AS_OF.to_string(),
        },
    }
}
