//! What Vim mode shows and reports: the status value and the events.

use writer_vim::{CaretShape, CommandLine, HostRequest, Mode};

/// One line for the status area: a message of a command (`"3 lines
/// yanked"`) or an error (`"E486: Pattern not found: xyz"`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VimMessage {
    /// The text.
    pub text: String,
    /// Whether it reports a failure.
    pub is_error: bool,
}

impl VimMessage {
    /// A message.
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
        }
    }

    /// An error.
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
        }
    }
}

/// The state of Vim mode as a status bar shows it. Read it with
/// [`crate::VimMode::status`]; the [`crate::VimMode`] entity notifies its
/// observers and emits [`VimEvent::StatusChanged`] whenever it changes.
///
/// ```ignore
/// cx.observe(&vim, |this, vim, cx| {
///     let status = vim.read(cx).status();
///     this.vim_label = status.attached.then(|| status.mode_label().to_string());
///     cx.notify();
/// })
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VimStatus {
    /// Whether Vim mode is on for the editor. When it is off the other
    /// fields hold their idle values.
    pub attached: bool,
    /// The mode.
    pub mode: Mode,
    /// The one command of `ctrl-o` is awaited: [`VimStatus::mode`] is
    /// Normal, and Insert mode comes back after the command.
    pub insert_pending: bool,
    /// The keys of the unfinished command (`"2d"`, `"\"a"`); empty when
    /// idle.
    pub pending_keys: String,
    /// The command line while `:`, `/` or `?` is open.
    pub command_line: Option<CommandLine>,
    /// The last message or error, until the next command.
    pub message: Option<VimMessage>,
    /// How the engine wants the caret drawn.
    pub caret: CaretShape,
    /// Whether search matches are highlighted.
    pub search_highlighted: bool,
}

impl Default for VimStatus {
    fn default() -> Self {
        Self {
            attached: false,
            mode: Mode::Normal,
            insert_pending: false,
            pending_keys: String::new(),
            command_line: None,
            message: None,
            caret: CaretShape::Block,
            search_highlighted: false,
        }
    }
}

impl VimStatus {
    /// The mode as a status bar shows it: `"NORMAL"`, `"INSERT"`,
    /// `"REPLACE"`, `"VISUAL"`, `"VISUAL LINE"`, `"PENDING"`, `"COMMAND"`,
    /// and `"(INSERT)"` while the one command of `ctrl-o` is awaited.
    pub fn mode_label(&self) -> &'static str {
        if self.insert_pending && self.mode == Mode::Normal {
            "(INSERT)"
        } else {
            self.mode.label()
        }
    }
}

/// What a [`crate::VimMode`] tells its subscribers
/// (`cx.subscribe(&vim, |this, vim, event: &VimEvent, cx| ..)`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VimEvent {
    /// Vim mode was switched on for the editor.
    Attached,
    /// Vim mode was switched off for the editor.
    Detached,
    /// The mode changed.
    ModeChanged(Mode),
    /// [`crate::VimMode::status`] changed (mode, pending keys, command
    /// line, message). Observers of the entity are notified as well.
    StatusChanged,
    /// The engine asks the application for something: `:w`
    /// ([`HostRequest::Save`]), `:q` ([`HostRequest::Close`]), `:wq` / `:x`
    /// / `ZZ` ([`HostRequest::SaveAndClose`]), or an ex command the engine
    /// does not know ([`HostRequest::Command`] with name, arguments, bang
    /// and block range). Report an unknown command name with
    /// [`crate::VimMode::show_error`].
    Request(HostRequest),
    /// A command failed or meant nothing. A host may flash; the status
    /// carries the error text when there is one.
    Bell,
}
