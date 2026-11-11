//! What the engine reports back to the host: plain data.

use std::ops::Range;

use writer_core::{Document, SearchOptions};

/// The mode the engine is in, as the host needs to know it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Normal mode: keys are commands.
    Normal,
    /// Insert mode: typing goes through the editor's own input path.
    Insert,
    /// Replace mode (`R`): like Insert, but typed text overwrites.
    Replace,
    /// Characterwise Visual mode (`v`).
    Visual,
    /// Linewise Visual mode (`V`).
    VisualLine,
    /// An operator (`d`, `c`, `y`, …) is waiting for its motion.
    OperatorPending,
    /// The command line is open (`:`, `/` or `?`).
    CommandLine,
}

impl Mode {
    /// Whether typed text belongs to the document: Insert and Replace mode.
    /// In every other mode the host must not insert text for a key the
    /// engine consumed, and the engine consumes every character key.
    pub fn is_insert(self) -> bool {
        matches!(self, Mode::Insert | Mode::Replace)
    }

    /// Whether a selection is being made: Visual and Visual Line mode.
    pub fn is_visual(self) -> bool {
        matches!(self, Mode::Visual | Mode::VisualLine)
    }

    /// The name Vim shows in its status line (`"NORMAL"`, `"INSERT"`,
    /// `"REPLACE"`, `"VISUAL"`, `"VISUAL LINE"`, `"PENDING"`, `"COMMAND"`).
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Replace => "REPLACE",
            Mode::Visual => "VISUAL",
            Mode::VisualLine => "VISUAL LINE",
            Mode::OperatorPending => "PENDING",
            Mode::CommandLine => "COMMAND",
        }
    }
}

/// How the caret should be drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CaretShape {
    /// A block over the character (Normal and Visual mode).
    Block,
    /// A thin bar before the character (Insert mode, command line).
    Bar,
    /// An underline below the character (Replace mode, operator pending,
    /// waiting for the character of `r`).
    Underline,
}

/// The command line as it should be displayed.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CommandLine {
    /// `':'`, `'/'` or `'?'`.
    pub prefix: char,
    /// The text typed so far.
    pub text: String,
    /// Byte offset of the cursor in `text`.
    pub cursor: usize,
}

/// A search pattern in the form the document search understands. The host
/// draws the highlights; [`SearchPattern::matches_in_block`] gives the
/// ranges for one block.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SearchPattern {
    /// The literal text to find.
    pub text: String,
    /// Case sensitivity and whole-word matching.
    pub options: SearchOptions,
}

impl SearchPattern {
    /// The matches in block `index` as byte ranges of its text.
    pub fn matches_in_block(&self, doc: &Document, index: usize) -> Vec<Range<usize>> {
        doc.find_in_block(index, &self.text, self.options)
    }
}

/// Something the engine cannot do itself and asks the host for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum HostRequest {
    /// `:w`: save the document now.
    Save,
    /// `:q`: close the document. `force` is set by `:q!`.
    Close {
        /// Close without asking (`!`).
        force: bool,
    },
    /// `:wq`, `:x`, `ZZ`: save, then close.
    SaveAndClose,
    /// An ex command the engine does not know, for the application to
    /// interpret (for example `:export md`). The host reports unknown names
    /// itself.
    Command {
        /// The command name as typed (`"export"`).
        name: String,
        /// Everything after the name, trimmed.
        args: String,
        /// A `!` directly after the name.
        bang: bool,
        /// The block range given before the name (`:2,5name`, `:%name`,
        /// `:'<,'>name`), as block indices.
        range: Option<Range<usize>>,
    },
}

/// One thing that happened during a call into the engine.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// The mode changed.
    ModeChanged(Mode),
    /// The command line opened or changed (`Some`), or closed (`None`).
    CommandLine(Option<CommandLine>),
    /// The keys of the unfinished command to display (`"2d"`, `"\"a"`);
    /// empty when the command finished or was cancelled.
    PendingKeys(String),
    /// A message for the status area (`"3 lines yanked"`).
    Message(String),
    /// An error message (`"Pattern not found: xyz"`).
    Error(String),
    /// Show highlights for this pattern (`Some`) or clear them (`None`).
    SearchHighlight(Option<SearchPattern>),
    /// Ask the host to do something.
    Request(HostRequest),
    /// The command failed or made no sense; a host may flash or stay quiet.
    Bell,
}

/// The result of handing the engine a key or a notification.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Outcome {
    /// Whether the engine used the key. A key that was not consumed must be
    /// handled by the host as if the engine were not there.
    pub consumed: bool,
    /// What happened, in order.
    pub effects: Vec<Effect>,
}

impl Outcome {
    /// The first error message among the effects, if any.
    pub fn error(&self) -> Option<&str> {
        self.effects.iter().find_map(|e| match e {
            Effect::Error(m) => Some(m.as_str()),
            _ => None,
        })
    }

    /// Whether the effects contain a bell.
    pub fn rang_bell(&self) -> bool {
        self.effects.iter().any(|e| matches!(e, Effect::Bell))
    }
}
