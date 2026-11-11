//! The Vim state machine.

mod cmdline;
mod insert;
mod marks;
mod motions;
mod normal;
mod operators;

use std::collections::HashMap;

use writer_core::{EditorState, Position, Selection};

use crate::buf::{self, ClassOpts};
use crate::effect::{CaretShape, CommandLine, Effect, Mode, Outcome, SearchPattern};
use crate::ex::SubstArgs;
use crate::host::{GoalColumn, VimHost};
use crate::key::Key;
use crate::register::{RegisterContent, Registers};

use cmdline::CmdLineState;
use insert::{InsertAction, InsertSession};
use marks::{JumpList, Mark};

/// Switches for the few places where the engine can deviate from Vim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VimOptions {
    /// Treat an apostrophe between two letters (`don't`, `it’s`) as part of
    /// the word for `w`, `b`, `e`, `iw` and friends. Vim stops at it; prose
    /// reads better when it does not. Default: on.
    pub apostrophe_in_words: bool,
    /// Mirror the unnamed register to the system clipboard (Vim's
    /// `clipboard=unnamed`): plain `y`, `d` and `p` then use the clipboard.
    /// Default: off.
    pub unnamed_clipboard: bool,
    /// Move the caret to the first match while a search is being typed.
    /// Default: on.
    pub incremental_search: bool,
    /// Report search highlights (`hlsearch`). Default: on.
    pub highlight_search: bool,
    /// Spaces that `>` and `<` add to or remove from the lines of a code
    /// block. Default: 4.
    pub shift_width: usize,
}

impl Default for VimOptions {
    fn default() -> VimOptions {
        VimOptions {
            apostrophe_in_words: true,
            unnamed_clipboard: false,
            incremental_search: true,
            highlight_search: true,
            shift_width: 4,
        }
    }
}

/// Mutable access to the editor and the host for the length of one call.
pub(crate) struct Cx<'a> {
    pub ed: &'a mut EditorState,
    pub host: &'a mut dyn VimHost,
}

/// The base mode. Operator-pending and command-line entry are layered on
/// top (see [`Pending`] and [`CmdLineState`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Base {
    Normal,
    Insert,
    Replace,
    Visual { line: bool },
}

/// An operator waiting for a motion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operator {
    Delete,
    Change,
    Yank,
    ShiftRight,
    ShiftLeft,
    Lower,
    Upper,
    ToggleCase,
}

/// What the next key is an argument of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Await {
    /// After `g`.
    G,
    /// After `z`.
    Z,
    /// After `Z`.
    BigZ,
    /// After `"`.
    Register,
    /// After `f`, `F`, `t`, `T`.
    Find { forward: bool, till: bool },
    /// After `r`.
    Replace,
    /// After `m`.
    SetMark,
    /// After `'` or `` ` ``.
    Jump { exact: bool },
    /// After `i` or `a` with an operator pending or in Visual mode.
    Object { around: bool },
}

/// The command being typed.
#[derive(Clone, Debug, Default)]
pub(crate) struct Pending {
    /// The count being typed (before or after the operator).
    pub count: Option<usize>,
    /// The count typed before the operator.
    pub op_count: Option<usize>,
    pub register: Option<char>,
    pub operator: Option<Operator>,
    pub awaiting: Option<Await>,
    /// The keys of the command without counts and register, for `.`.
    pub keys: Vec<Key>,
    /// What to show while the command is unfinished.
    pub display: String,
}

impl Pending {
    pub fn active(&self) -> bool {
        self.count.is_some()
            || self.op_count.is_some()
            || self.register.is_some()
            || self.operator.is_some()
            || self.awaiting.is_some()
    }

    /// The count of the command: the product of the counts typed before
    /// and after the operator, if any was typed.
    pub fn count_opt(&self) -> Option<usize> {
        match (self.op_count, self.count) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(1).saturating_mul(b.unwrap_or(1)).max(1)),
        }
    }

    pub fn count1(&self) -> usize {
        self.count_opt().unwrap_or(1)
    }
}

/// The Visual selection, in cells: both ends are included.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct VisualState {
    pub anchor: Position,
    pub cursor: Position,
}

/// The size of a Visual selection an operator was applied to, so `.` can
/// apply it again from the caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VisualRepeat {
    pub line: bool,
    /// Blocks spanned beyond the first.
    pub blocks: usize,
    /// Graphemes selected in a one-block characterwise selection; for more
    /// blocks, the grapheme column of the last selected character.
    pub cols: usize,
}

/// The last repeatable change.
#[derive(Clone, Debug)]
pub(crate) struct ChangeRecord {
    pub register: Option<char>,
    pub count: Option<usize>,
    pub keys: Vec<Key>,
    pub visual: Option<VisualRepeat>,
    /// What was typed in the Insert session the command started.
    pub insert: Vec<InsertAction>,
}

/// A remembered Visual selection (`gv`, `'<`, `'>`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct SavedVisual {
    pub anchor: Mark,
    pub cursor: Mark,
    pub line: bool,
}

/// The last `f` / `F` / `t` / `T`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FindSpec {
    pub ch: char,
    pub forward: bool,
    pub till: bool,
}

/// Search state.
#[derive(Clone, Debug, Default)]
pub(crate) struct SearchState {
    /// The last pattern as typed (register `/`).
    pub raw: String,
    pub pattern: Option<SearchPattern>,
    /// Direction of the last `/` or `?`.
    pub forward: bool,
    /// Whether highlights are on (`:noh` turns them off until the next
    /// search command).
    pub highlight: bool,
    pub history: Vec<String>,
}

/// Insert mode to return to after the one command of `ctrl-o`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Resume {
    pub replace: bool,
}

/// What the host was last told, to report only changes.
#[derive(Clone, Debug, PartialEq)]
struct Shown {
    mode: Mode,
    pending: String,
    cmdline: Option<CommandLine>,
    highlight: Option<SearchPattern>,
}

/// The Vim emulation engine: one per editor.
///
/// See the [crate documentation](crate) for how a host drives it.
#[derive(Debug)]
pub struct Vim {
    pub(crate) options: VimOptions,
    pub(crate) base: Base,
    pub(crate) pending: Pending,
    pub(crate) visual: VisualState,
    pub(crate) insert: Option<InsertSession>,
    pub(crate) cmdline: Option<CmdLineState>,
    pub(crate) registers: Registers,
    pub(crate) marks: HashMap<char, Mark>,
    pub(crate) jumps: JumpList,
    pub(crate) goal: Option<GoalColumn>,
    pub(crate) last_find: Option<FindSpec>,
    pub(crate) search: SearchState,
    pub(crate) last_subst: Option<SubstArgs>,
    pub(crate) last_change: Option<ChangeRecord>,
    pub(crate) last_visual: Option<SavedVisual>,
    pub(crate) last_inserted: String,
    pub(crate) last_ex: String,
    pub(crate) ex_history: Vec<String>,
    pub(crate) resume: Option<Resume>,
    pub(crate) replaying: bool,
    pub(crate) effects: Vec<Effect>,
    /// The Visual selection the command being executed was applied to.
    pub(crate) visual_repeat: Option<VisualRepeat>,
    /// Where `u` should put the caret after undoing the command being
    /// executed.
    pub(crate) undo_hint: Option<Mark>,
    /// Remembered carets for `u`, by the undo depth their change produced.
    pub(crate) undo_hints: Vec<(usize, Mark)>,
    /// Carets to return to when the steps undone by `u` are redone.
    pub(crate) redo_carets: Vec<Mark>,
    /// The command being executed is an undo or redo.
    pub(crate) keep_redo_carets: bool,
    /// The document revision when the command being typed started.
    pub(crate) command_revision: u64,
    /// What the host was last told; `None` forces a full report.
    shown: Option<Shown>,
}

impl Default for Vim {
    fn default() -> Vim {
        Vim::new()
    }
}

impl Vim {
    /// An engine in Normal mode with default options.
    pub fn new() -> Vim {
        Vim::with_options(VimOptions::default())
    }

    /// An engine in Normal mode.
    pub fn with_options(options: VimOptions) -> Vim {
        let mut registers = Registers::default();
        registers.unnamed_clipboard = options.unnamed_clipboard;
        Vim {
            options,
            base: Base::Normal,
            pending: Pending::default(),
            visual: VisualState::default(),
            insert: None,
            cmdline: None,
            registers,
            marks: HashMap::new(),
            jumps: JumpList::default(),
            goal: None,
            last_find: None,
            search: SearchState {
                forward: true,
                ..SearchState::default()
            },
            last_subst: None,
            last_change: None,
            last_visual: None,
            last_inserted: String::new(),
            last_ex: String::new(),
            ex_history: Vec::new(),
            resume: None,
            replaying: false,
            effects: Vec::new(),
            visual_repeat: None,
            undo_hint: None,
            undo_hints: Vec::new(),
            redo_carets: Vec::new(),
            keep_redo_carets: false,
            command_revision: 0,
            shown: Some(Shown {
                mode: Mode::Normal,
                pending: String::new(),
                cmdline: None,
                highlight: None,
            }),
        }
    }

    /// The options.
    pub fn options(&self) -> &VimOptions {
        &self.options
    }

    /// Changes the options.
    pub fn set_options(&mut self, options: VimOptions) {
        self.registers.unnamed_clipboard = options.unnamed_clipboard;
        self.options = options;
    }

    // ----- what to show ----------------------------------------------------------------

    /// The current mode.
    pub fn mode(&self) -> Mode {
        if self.cmdline.is_some() {
            return Mode::CommandLine;
        }
        match self.base {
            Base::Insert => Mode::Insert,
            Base::Replace => Mode::Replace,
            Base::Visual { line: false } => Mode::Visual,
            Base::Visual { line: true } => Mode::VisualLine,
            Base::Normal => {
                if self.pending.operator.is_some() {
                    Mode::OperatorPending
                } else {
                    Mode::Normal
                }
            }
        }
    }

    /// The keys of the unfinished command (`"2d"`), empty when idle.
    pub fn pending_keys(&self) -> &str {
        &self.pending.display
    }

    /// The command line, while it is open.
    pub fn command_line(&self) -> Option<CommandLine> {
        self.cmdline.as_ref().map(CmdLineState::view)
    }

    /// The pattern whose matches should be highlighted, if any.
    pub fn search_highlight(&self) -> Option<&SearchPattern> {
        if self.options.highlight_search && self.search.highlight {
            self.search.pattern.as_ref()
        } else {
            None
        }
    }

    /// The last search pattern, highlighted or not.
    pub fn search_pattern(&self) -> Option<&SearchPattern> {
        self.search.pattern.as_ref()
    }

    /// How the caret should be drawn.
    pub fn caret_shape(&self) -> CaretShape {
        if self.cmdline.is_some() {
            return CaretShape::Bar;
        }
        match self.base {
            Base::Insert => CaretShape::Bar,
            Base::Replace => CaretShape::Underline,
            Base::Visual { .. } => CaretShape::Block,
            Base::Normal => {
                if self.pending.operator.is_some()
                    || self.pending.awaiting == Some(Await::Replace)
                {
                    CaretShape::Underline
                } else {
                    CaretShape::Block
                }
            }
        }
    }

    /// Where the caret should be drawn: the character the Vim cursor is on.
    ///
    /// In Normal, Insert and Replace mode this is the head of the editor's
    /// selection. In Visual mode the editor's selection covers the selected
    /// characters, so its head lies *after* the cursor when the selection
    /// runs forward; this returns the cursor itself.
    pub fn caret(&self, ed: &EditorState) -> Position {
        match self.base {
            Base::Visual { .. } => ed.document().clamp_position(self.visual.cursor),
            _ => ed.selection().head,
        }
    }

    /// The blocks a Visual Line selection covers, so a host can tint whole
    /// lines (an empty block has no selected text to draw). `None` outside
    /// Visual Line mode.
    pub fn visual_line_blocks(&self, ed: &EditorState) -> Option<std::ops::Range<usize>> {
        match self.base {
            Base::Visual { line: true } => {
                let n = ed.document().block_count();
                let a = self.visual.anchor.block.min(n - 1);
                let c = self.visual.cursor.block.min(n - 1);
                Some(a.min(c)..a.max(c) + 1)
            }
            _ => None,
        }
    }

    /// The content of register `name` (`'"'`, `'a'`..`'z'`, `'0'`..`'9'`,
    /// `'-'`). The clipboard registers are not readable here; ask the host.
    pub fn register(&self, name: char) -> Option<&RegisterContent> {
        self.registers.peek(name)
    }

    /// Sets register `name`. Writing `'+'` or `'*'` goes to the host's
    /// clipboard; an upper-case letter appends.
    pub fn set_register(&mut self, name: char, content: RegisterContent, host: &mut dyn VimHost) {
        self.registers.set(name, content, host);
    }

    /// The register named with `"x` for the command being typed, if any.
    /// A host whose clipboard is slow to read can fetch it only when this
    /// is `'+'` or `'*'` (or [`VimOptions::unnamed_clipboard`] is on).
    pub fn pending_register(&self) -> Option<char> {
        self.pending.register
    }

    /// Whether the engine is waiting for the one Normal-mode command of
    /// `ctrl-o` and will return to Insert mode after it. [`Vim::mode`]
    /// reports Normal mode meanwhile, but the caret may still sit after the
    /// last character of its line.
    pub fn insert_pending(&self) -> bool {
        self.resume.is_some()
    }

    /// The text typed in the last Insert session (register `.`).
    pub fn last_inserted_text(&self) -> &str {
        &self.last_inserted
    }

    // ----- entry points ------------------------------------------------------------------

    /// Handles one key.
    ///
    /// Returns whether the key was consumed and what happened. In Normal,
    /// Visual, operator-pending and command-line mode every character key
    /// is consumed. In Insert and Replace mode only the keys Vim owns are
    /// consumed (escape, `ctrl-[`, `ctrl-c`, `ctrl-w`, `ctrl-u`, `ctrl-h`,
    /// `ctrl-o`, `ctrl-r`, and backspace in Replace mode); the host handles
    /// every other key as it would without Vim and reports typed text
    /// through [`Vim::text_inserted`].
    pub fn handle_key(&mut self, key: Key, ed: &mut EditorState, host: &mut dyn VimHost) -> Outcome {
        let mut cx = Cx { ed, host };
        let one_shot = self.resume.is_some();
        let consumed = self.dispatch(key, &mut cx);
        if one_shot {
            self.maybe_resume_insert(&mut cx);
        }
        self.finish(consumed)
    }

    /// Feeds a sequence of keys (see [`Key::parse_sequence`]) and merges the
    /// outcomes; `consumed` is true when every key was consumed. Meant for
    /// tests, scripted commands and key mappings. Keys the engine does not
    /// consume are dropped, so text typed in Insert mode is not inserted.
    pub fn handle_keys(
        &mut self,
        keys: &[Key],
        ed: &mut EditorState,
        host: &mut dyn VimHost,
    ) -> Outcome {
        let mut all = Outcome {
            consumed: true,
            effects: Vec::new(),
        };
        for k in keys {
            let o = self.handle_key(*k, ed, host);
            all.consumed &= o.consumed;
            all.effects.extend(o.effects);
        }
        all
    }

    pub(crate) fn dispatch(&mut self, key: Key, cx: &mut Cx) -> bool {
        if self.cmdline.is_some() {
            // What an ex command changes is one undo step.
            cx.ed.begin_undo_group();
            let used = self.cmdline_key(key, cx);
            cx.ed.end_undo_group();
            return used;
        }
        match self.base {
            Base::Insert | Base::Replace => self.insert_key(key, cx),
            Base::Normal | Base::Visual { .. } => self.normal_key(key, cx),
        }
    }

    /// Tells the engine that the host inserted `text` at the caret through
    /// its normal typing path (a typed character or committed IME text).
    /// Call it after the document changed, with the text as typed.
    ///
    /// In Insert mode this feeds dot-repeat, counts (`3ihello`) and the `.`
    /// register. In Replace mode the engine also removes the characters the
    /// new text overwrites. In other modes it does nothing.
    pub fn text_inserted(&mut self, text: &str, ed: &mut EditorState, host: &mut dyn VimHost) -> Outcome {
        let mut cx = Cx { ed, host };
        self.note_inserted(text, &mut cx);
        self.finish(true)
    }

    /// Types `text` into the command line (for text that does not arrive as
    /// keys, such as committed IME input while `:` or `/` is open). Does
    /// nothing when the command line is closed.
    pub fn command_line_input(&mut self, text: &str, ed: &mut EditorState, host: &mut dyn VimHost) -> Outcome {
        let mut cx = Cx { ed, host };
        let used = self.cmdline.is_some();
        if used {
            self.cmdline_insert(text, &mut cx);
        }
        self.finish(used)
    }

    /// Runs an ex command line (without the leading `:`), as if it had been
    /// typed and confirmed. For command palette entries and tests.
    pub fn execute(&mut self, command: &str, ed: &mut EditorState, host: &mut dyn VimHost) -> Outcome {
        let mut cx = Cx { ed, host };
        if matches!(self.base, Base::Insert | Base::Replace) {
            self.leave_insert(&mut cx);
        }
        self.cmdline = None;
        self.pending = Pending::default();
        self.command_revision = cx.ed.revision();
        cx.ed.begin_undo_group();
        self.run_ex(command, &mut cx);
        if matches!(self.base, Base::Visual { .. }) {
            self.exit_visual(&mut cx);
        }
        self.finish_command(&mut cx, false);
        cx.ed.end_undo_group();
        self.finish(true)
    }

    /// Brings the engine in line with the editor after something changed
    /// the selection or the document without going through the engine: a
    /// mouse click or drag, a host command, find and replace, undo from a
    /// menu, another document being loaded into the same editor state.
    ///
    /// - Normal mode: a collapsed caret is moved onto a character; a
    ///   selection starts Visual mode (as a mouse drag does in Vim).
    /// - Visual mode: the Visual selection follows the editor's selection;
    ///   a collapsed selection ends Visual mode.
    /// - Insert and Replace mode: what was typed so far no longer counts
    ///   for dot-repeat (as after a mouse click in Vim).
    /// - An unfinished command and an open command line are cancelled.
    pub fn sync(&mut self, ed: &mut EditorState, host: &mut dyn VimHost) -> Outcome {
        let mut cx = Cx { ed, host };
        self.sync_inner(&mut cx);
        self.finish(true)
    }

    /// Attaches the engine to an editor state: call once when Vim mode is
    /// switched on, and after loading a different document. Resets to
    /// Normal mode (keeping registers, marks are dropped) and places the
    /// caret on a character. The returned effects describe the full display
    /// state.
    pub fn attach(&mut self, ed: &mut EditorState, host: &mut dyn VimHost) -> Outcome {
        let mut cx = Cx { ed, host };
        self.reset_transient();
        self.marks.clear();
        self.jumps = JumpList::default();
        self.last_visual = None;
        let sel = cx.ed.selection();
        if sel.is_collapsed() {
            let p = buf::clamp_normal(cx.ed.document(), sel.head);
            cx.ed.set_caret(p);
        } else {
            self.sync_inner(&mut cx);
        }
        // Report everything, not only what changed.
        self.shown = None;
        self.finish(true)
    }

    /// Detaches the engine: call before Vim mode is switched off or the
    /// editor state goes away. Ends an Insert session (closing its undo
    /// group), leaves Visual mode with the selection collapsed onto the
    /// cursor, cancels any unfinished command and clears the highlights.
    pub fn detach(&mut self, ed: &mut EditorState, host: &mut dyn VimHost) -> Outcome {
        let cx = Cx { ed, host };
        if let Some(s) = self.insert.take()
            && s.group_open
        {
            cx.ed.end_undo_group();
        }
        if matches!(self.base, Base::Visual { .. }) {
            let p = cx.ed.document().clamp_position(self.visual.cursor);
            cx.ed.set_caret(p);
        }
        self.reset_transient();
        self.search.highlight = false;
        self.finish(true)
    }

    fn reset_transient(&mut self) {
        self.base = Base::Normal;
        self.pending = Pending::default();
        self.insert = None;
        self.cmdline = None;
        self.resume = None;
        self.goal = None;
        self.replaying = false;
        self.visual_repeat = None;
        self.undo_hint = None;
        self.undo_hints.clear();
        self.redo_carets.clear();
    }

    fn sync_inner(&mut self, cx: &mut Cx) {
        self.cmdline = None;
        self.pending = Pending::default();
        self.goal = None;
        let sel = cx.ed.selection();
        match self.base {
            Base::Insert | Base::Replace => {
                if let Some(s) = self.insert.as_mut() {
                    s.interrupted();
                }
            }
            Base::Normal | Base::Visual { .. } => {
                if sel.is_collapsed() {
                    self.base = Base::Normal;
                    let p = buf::clamp_normal(cx.ed.document(), sel.head);
                    cx.ed.set_caret(p);
                } else {
                    let doc = cx.ed.document();
                    let line = matches!(self.base, Base::Visual { line: true });
                    let (anchor, cursor) = if sel.head >= sel.anchor {
                        (sel.anchor, buf::prev_cell(doc, sel.head).unwrap_or(sel.head))
                    } else {
                        (buf::prev_cell(doc, sel.anchor).unwrap_or(sel.anchor), sel.head)
                    };
                    self.visual = VisualState { anchor, cursor };
                    self.base = Base::Visual { line };
                    if line {
                        self.sync_visual_selection(cx);
                    }
                }
            }
        }
    }

    /// Builds the outcome: queued effects, preceded by the display changes
    /// since the last call.
    fn finish(&mut self, consumed: bool) -> Outcome {
        let now = Shown {
            mode: self.mode(),
            pending: self.pending.display.clone(),
            cmdline: self.command_line(),
            highlight: self.search_highlight().cloned(),
        };
        let mut effects = Vec::new();
        let old = self.shown.as_ref();
        if old.is_none_or(|o| o.mode != now.mode) {
            effects.push(Effect::ModeChanged(now.mode));
        }
        if old.is_none_or(|o| o.cmdline != now.cmdline) {
            effects.push(Effect::CommandLine(now.cmdline.clone()));
        }
        if old.is_none_or(|o| o.pending != now.pending) {
            effects.push(Effect::PendingKeys(now.pending.clone()));
        }
        if old.is_none_or(|o| o.highlight != now.highlight) {
            effects.push(Effect::SearchHighlight(now.highlight.clone()));
        }
        effects.append(&mut self.effects);
        self.shown = Some(now);
        Outcome { consumed, effects }
    }

    // ----- small helpers shared by the submodules -------------------------------------------

    pub(crate) fn bell(&mut self) {
        self.effects.push(Effect::Bell);
    }

    pub(crate) fn message(&mut self, text: impl Into<String>) {
        self.effects.push(Effect::Message(text.into()));
    }

    pub(crate) fn error(&mut self, text: impl Into<String>) {
        self.effects.push(Effect::Error(text.into()));
    }

    pub(crate) fn class_opts(&self, big: bool) -> ClassOpts {
        ClassOpts {
            big,
            apostrophe: self.options.apostrophe_in_words,
        }
    }

    pub(crate) fn is_visual(&self) -> bool {
        matches!(self.base, Base::Visual { .. })
    }

    /// The Vim cursor: the cell the caret is on.
    pub(crate) fn cursor(&self, cx: &Cx) -> Position {
        match self.base {
            Base::Visual { .. } => cx.ed.document().clamp_position(self.visual.cursor),
            _ => cx.ed.selection().head,
        }
    }

    /// Places a collapsed caret for Normal mode.
    pub(crate) fn set_normal_caret(&mut self, cx: &mut Cx, p: Position) {
        let p = buf::clamp_normal(cx.ed.document(), p);
        cx.ed.set_selection(Selection::caret(p));
    }
}
