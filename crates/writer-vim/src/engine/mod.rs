//! The Vim state machine.

mod cmdline;
mod insert;
mod marks;
mod motions;
mod normal;
mod operators;

use std::collections::HashMap;

use writer_core::{EditorState, Position, Selection, TextRange};

use crate::buf::{self, ClassOpts};
use crate::effect::{CaretShape, CommandLine, Effect, Mode, Outcome, SearchPattern};
use crate::ex::SubstArgs;
use crate::host::{GoalColumn, VimHost};
use crate::key::Key;
use crate::register::{RegisterContent, Registers};

use cmdline::CmdLineState;
use insert::{InsertAction, InsertSession};
use marks::{JumpList, Mark, PlaceMark};

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
    /// Refuse every command that would change the document (with an error
    /// message): moving, searching, selecting and yanking still work. For
    /// documents the editor shows read-only. Default: off.
    pub read_only: bool,
    /// Rows of context kept above and below the caret where the engine
    /// decides (Vim's `scrolloff`): `H` and `L` land this many rows inside
    /// the viewport, and `ctrl-e` / `ctrl-y` drag the caret along when it
    /// comes closer than this to the edge. Not applied at the first and
    /// last row of the document. Set it to the margin the host itself keeps
    /// when it scrolls the caret into view, so the next movement after one
    /// of these commands does not shift the view. Default: 0.
    pub scroll_off: usize,
}

impl Default for VimOptions {
    fn default() -> VimOptions {
        VimOptions {
            apostrophe_in_words: true,
            unnamed_clipboard: false,
            incremental_search: true,
            highlight_search: true,
            shift_width: 4,
            read_only: false,
            scroll_off: 0,
        }
    }
}

/// What a spelling command says when the host has no spell checker.
pub(crate) const SPELL_OFF: &str = "E756: Spell checking is off";

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
    /// After `]` or `[`.
    Bracket { forward: bool },
    /// After `zu`.
    ZU,
}

/// What the next key will ask the host's spell checker, as
/// [`Vim::spell_lookahead`] foresees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpellLookahead {
    /// The key completes `]s` or `[s` (or repeats a change made with one):
    /// the engine will walk `count` misspelled words from `from` through
    /// [`VimHost::misspelling_from`], exactly as
    /// [`walk_misspellings`](crate::walk_misspellings) does.
    Misspellings {
        /// Where the walk starts: the Vim cursor.
        from: Position,
        /// `]s` rather than `[s`.
        forward: bool,
        /// The count of the command, at least 1.
        count: usize,
    },
    /// The key completes `z=`, `zg` or `zug`: the engine will hand the host
    /// a [`SpellCommand`](crate::SpellCommand).
    Command,
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
    /// The selection ran to the end of its last line, line break included
    /// (`v$`).
    pub to_eol: bool,
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
    pub anchor: PlaceMark,
    pub cursor: PlaceMark,
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
    /// The start of the line the caret was at the end of when `ctrl-o` was
    /// pressed: if the command leaves the caret on the last character of
    /// that line, typing goes on after it.
    pub eol_line: Option<Mark>,
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
    /// The `''` mark before the last command that set it.
    pub(crate) prev_context: Option<Mark>,
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
    pub(crate) undo_hint: Option<Position>,
    /// Remembered carets for `u`, by the undo depth their change produced.
    pub(crate) undo_hints: Vec<(usize, Position)>,
    /// Carets to return to when the steps undone by `u` are redone.
    /// The flag says whether the caret was remembered by the command (and
    /// so lies in the text it touched) rather than taken from the editor.
    pub(crate) redo_carets: Vec<(Position, bool)>,
    /// The command being executed is an undo or redo.
    pub(crate) keep_redo_carets: bool,
    /// The Visual-mode delete being executed fills register 1 whatever its
    /// size (backspace).
    pub(crate) visual_big: bool,
    /// The operator being executed got its range from an inclusive motion
    /// or a text object (`c$`, `ciw`): changing nothing on an empty line
    /// then still empties the registers, as in Vim.
    pub(crate) op_inclusive: bool,
    /// The command being executed left the caret after the last character
    /// of a line, from where it was put back onto that character (this
    /// position). After `ctrl-o` typing resumes after the character.
    pub(crate) clamped_from_eol: Option<Position>,
    /// The document revision when the command being typed started.
    pub(crate) command_revision: u64,
    /// The document revision and selection the engine last left or saw:
    /// when they differ, something outside the engine changed the editor.
    seen: Option<(u64, Selection)>,
    /// A key was left to the host (or typed text announced): the next
    /// change of the editor is the host carrying it out.
    expect_host: bool,
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
            prev_context: None,
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
            visual_big: false,
            clamped_from_eol: None,
            op_inclusive: false,
            command_revision: 0,
            seen: None,
            expect_host: false,
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
                if self.pending.operator.is_some() || self.pending.awaiting == Some(Await::Replace)
                {
                    CaretShape::Underline
                } else {
                    CaretShape::Block
                }
            }
        }
    }

    /// What handling `key` next would ask the host's spell checker, if
    /// anything. For a host that has to prepare its answers before the call
    /// (see [`walk_misspellings`](crate::walk_misspellings)); a host that
    /// can answer [`VimHost::misspelling_from`] at any time has no use for
    /// this. It may foresee a question that is then not asked (a repeated
    /// change that fails before its motion), never the other way round, as
    /// long as the editor is as the engine last saw it.
    pub fn spell_lookahead(&self, key: Key, ed: &EditorState) -> Option<SpellLookahead> {
        if self.cmdline.is_some() || matches!(self.base, Base::Insert | Base::Replace) {
            return None;
        }
        let c = key.plain_char()?;
        match self.pending.awaiting {
            Some(Await::Bracket { forward }) if matches!(c, 's' | 'S') => {
                Some(SpellLookahead::Misspellings {
                    from: self.caret(ed),
                    forward,
                    count: self.pending.count1(),
                })
            }
            Some(Await::Z) if matches!(c, '=' | 'g') => Some(SpellLookahead::Command),
            Some(Await::ZU) if c == 'g' => Some(SpellLookahead::Command),
            None if c == '.' && self.pending.operator.is_none() => {
                // A repeated `d]s`: the motion is among the recorded keys.
                let rec = self.last_change.as_ref()?;
                let forward = rec.keys.windows(2).find_map(|pair| {
                    let forward = match pair[0].plain_char()? {
                        ']' => true,
                        '[' => false,
                        _ => return None,
                    };
                    matches!(pair[1].plain_char()?, 's' | 'S').then_some(forward)
                })?;
                Some(SpellLookahead::Misspellings {
                    from: self.caret(ed),
                    forward,
                    count: self.pending.count_opt().or(rec.count).unwrap_or(1).max(1),
                })
            }
            _ => None,
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
    /// reports Normal mode meanwhile.
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
    pub fn handle_key(
        &mut self,
        key: Key,
        ed: &mut EditorState,
        host: &mut dyn VimHost,
    ) -> Outcome {
        let mut cx = Cx { ed, host };
        self.catch_up(&mut cx);
        let one_shot = self.resume.is_some();
        let consumed = self.dispatch(key, &mut cx);
        if one_shot {
            self.maybe_resume_insert(&mut cx);
        }
        if !self.pending.active() && self.cmdline.is_none() {
            self.check_context_mark(&cx);
        }
        // A key left to the host will change the editor behind the engine.
        self.expect_host = !consumed;
        self.observe(&cx);
        self.finish(consumed)
    }

    /// Remembers the state the engine leaves the editor in.
    fn observe(&mut self, cx: &Cx) {
        self.seen = Some((cx.ed.revision(), cx.ed.selection()));
    }

    /// Whether the editor changed since the engine last looked.
    fn changed_outside(&self, cx: &Cx) -> bool {
        self.seen != Some((cx.ed.revision(), cx.ed.selection()))
    }

    /// Before a key: if the editor changed behind the engine and nobody
    /// called [`Vim::sync`], do it now. In Insert mode changes are the
    /// host's typing and are left alone.
    fn catch_up(&mut self, cx: &mut Cx) {
        self.expect_host = false;
        if self.seen.is_none() || !self.changed_outside(cx) {
            return;
        }
        if !matches!(self.base, Base::Insert | Base::Replace) {
            self.sync_inner(cx, false);
        }
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

    /// Tells the engine that the host inserts `text` at the caret through
    /// its normal typing path (a typed character or committed IME text).
    /// Call it once per insertion, with the text as typed, right after the
    /// document changed (right before works too, as long as the insertion
    /// then happens at the same caret).
    ///
    /// In Insert mode this feeds dot-repeat, counts (`3ihello`) and the `.`
    /// register. In Replace mode the engine also removes the characters the
    /// new text overwrites. In other modes it does nothing.
    pub fn text_inserted(
        &mut self,
        text: &str,
        ed: &mut EditorState,
        host: &mut dyn VimHost,
    ) -> Outcome {
        let mut cx = Cx { ed, host };
        // Called after the insertion, the change is accounted for now;
        // called before it, the change is still to come.
        self.expect_host = !self.changed_outside(&cx);
        self.note_inserted(text, &mut cx);
        self.observe(&cx);
        self.finish(true)
    }

    /// Tells the engine that the host's input method replaced `range` of
    /// the document with `text`: a system text replacement, or the accent
    /// picked for a letter that was just typed. `range` is in positions of
    /// the document before the replacement. Call it right after the
    /// document changed, instead of [`Vim::text_inserted`].
    ///
    /// In Insert mode, where the replaced text is the end of what was just
    /// typed (it lies right before the caret, in the text typed last), the
    /// record of the session is corrected: dot-repeat and counts type the
    /// replacement. Anything else is taken as a change from outside, as
    /// [`Vim::sync`] takes it: what was typed before no longer repeats.
    pub fn text_replaced(
        &mut self,
        range: TextRange,
        text: &str,
        ed: &mut EditorState,
        host: &mut dyn VimHost,
    ) -> Outcome {
        let mut cx = Cx { ed, host };
        // The caret before the replacement: where the engine last saw it.
        let caret = self
            .seen
            .map(|(_, selection)| selection)
            .filter(Selection::is_collapsed)
            .map(|selection| selection.head);
        let amended = self.base == Base::Insert
            && caret.is_some_and(|caret| self.amend_typed(range, caret, text));
        self.expect_host = false;
        if !amended && self.changed_outside(&cx) {
            self.sync_inner(&mut cx, false);
        }
        self.observe(&cx);
        self.finish(true)
    }

    /// Types `text` into the command line (for text that does not arrive as
    /// keys, such as committed IME input while `:` or `/` is open). Does
    /// nothing when the command line is closed.
    pub fn command_line_input(
        &mut self,
        text: &str,
        ed: &mut EditorState,
        host: &mut dyn VimHost,
    ) -> Outcome {
        let mut cx = Cx { ed, host };
        let used = self.cmdline.is_some();
        if used {
            self.cmdline_insert(text, &mut cx);
        }
        self.observe(&cx);
        self.finish(used)
    }

    /// Runs an ex command line (without the leading `:`), as if it had been
    /// typed and confirmed. For command palette entries and tests.
    pub fn execute(
        &mut self,
        command: &str,
        ed: &mut EditorState,
        host: &mut dyn VimHost,
    ) -> Outcome {
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
        self.observe(&cx);
        self.finish(true)
    }

    /// Brings the engine in line with the editor after something changed
    /// the selection or the document without going through the engine: a
    /// mouse click or drag, a host command, find and replace, undo from a
    /// menu.
    ///
    /// - Normal mode: a collapsed caret is moved onto a character; a
    ///   selection starts Visual mode (as a mouse drag does in Vim).
    /// - Visual mode: the Visual selection follows the editor's selection;
    ///   a collapsed selection ends Visual mode.
    /// - Insert and Replace mode: what was typed so far no longer counts
    ///   for dot-repeat (as after a mouse click in Vim).
    /// - An unfinished command and an open command line are cancelled.
    ///
    /// The call does nothing when the editor is as the engine left it, and
    /// in Insert mode it also accepts the one change that follows a key the
    /// engine left to the host (or a [`Vim::text_inserted`]). So a host may
    /// simply call `sync` whenever the editor reports a changed selection
    /// or document, with one exception: not while the platform input
    /// method has marked text, whose updates are neither keys nor typed
    /// text yet.
    ///
    /// The engine also catches up by itself at the next key outside Insert
    /// mode; calling `sync` right away is what keeps the caret and the
    /// mode indicator correct in between.
    pub fn sync(&mut self, ed: &mut EditorState, host: &mut dyn VimHost) -> Outcome {
        let mut cx = Cx { ed, host };
        if self.changed_outside(&cx) {
            let typing = matches!(self.base, Base::Insert | Base::Replace);
            if typing && std::mem::take(&mut self.expect_host) {
                // The host carrying out the key the engine let through; a
                // cursor key that moved the caret ends what `.` repeats.
                self.settle_cursor_key(&mut cx, false);
            } else {
                self.sync_inner(&mut cx, false);
            }
        }
        self.observe(&cx);
        self.finish(true)
    }

    /// The document was changed by somebody else than the writer, and the
    /// editor kept the selection on its text: an agent's change that the
    /// app took over while the document is open
    /// (`writer_core::EditorState::adopt`). The engine catches up as
    /// [`Vim::sync`] does, with one difference: **nothing the writer is in
    /// the middle of is dropped.** An operator waiting for its motion, a
    /// count, a register name, a key waiting for its argument and an open
    /// command line stay as they are. The writer did nothing, so the next
    /// key they press means what it would have meant: after `d`, a `w`
    /// still deletes a word, and the rest of a search typed after `/` is
    /// still the search (after a [`Vim::sync`] those letters would run as
    /// commands of Normal mode).
    ///
    /// What follows the place is brought up to date as in `sync`: in
    /// Normal mode the caret is put on a character, a Visual selection
    /// follows the editor's selection, and in Insert mode what was typed
    /// before the change is an undo step of its own (the change is one
    /// too) and no longer repeats.
    ///
    /// A host calls this right after the editor took the change over,
    /// before it calls `sync` for the same change; the `sync` that follows
    /// then finds the editor as the engine left it.
    pub fn changed_elsewhere(&mut self, ed: &mut EditorState, host: &mut dyn VimHost) -> Outcome {
        let mut cx = Cx { ed, host };
        if self.changed_outside(&cx) {
            self.sync_inner(&mut cx, true);
        }
        self.observe(&cx);
        self.finish(true)
    }

    /// Drops a half-typed command: a count, a register name, an operator
    /// waiting for its motion, a key waiting for its argument (`f`, `r`,
    /// `m`, `g`, `z`, …), and the register name `ctrl-r` waits for in
    /// Insert mode. Modes, an open command line and the one command of
    /// `ctrl-o` are left as they are.
    ///
    /// For a host to call when the editor loses the keyboard focus: a `d`
    /// typed before looking something up in another panel must not delete
    /// with the first key pressed after coming back.
    pub fn cancel_pending_command(&mut self) -> Outcome {
        if self.cmdline.is_none() {
            self.cancel_pending();
        }
        if let Some(session) = self.insert.as_mut() {
            session.await_register = false;
        }
        // The editor was not touched: what the engine last saw of it stays
        // what it compares with.
        self.finish(true)
    }

    /// Attaches the engine to an editor state: call once when Vim mode is
    /// switched on, and after loading a different document. Resets to
    /// Normal mode (keeping registers, marks are dropped) and places the
    /// caret on a character. The returned effects describe the full display
    /// state.
    pub fn attach(&mut self, ed: &mut EditorState, host: &mut dyn VimHost) -> Outcome {
        let mut cx = Cx { ed, host };
        if let Some(s) = self.insert.take()
            && s.group_open
        {
            // Attached again without a detach: do not leave the undo group
            // of the old session open.
            cx.ed.end_undo_group();
        }
        self.reset_transient();
        self.marks.clear();
        self.jumps = JumpList::default();
        self.last_visual = None;
        let sel = cx.ed.selection();
        if sel.is_collapsed() {
            let p = buf::clamp_normal(cx.ed.document(), sel.head);
            cx.ed.set_caret(p);
        } else {
            self.sync_inner(&mut cx, false);
        }
        // The context mark starts at the top of the document, as in a file
        // Vim has just opened, so `''` works before the first jump.
        self.set_mark(&cx, '\'', Position::ZERO);
        self.prev_context = self.marks.get(&'\'').copied();
        // Report everything, not only what changed.
        self.shown = None;
        self.expect_host = false;
        self.observe(&cx);
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
        self.seen = None;
        self.expect_host = false;
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

    /// Brings the engine in line with an editor that was changed from
    /// outside. `keep_command`: the change was not the writer's doing
    /// ([`Vim::changed_elsewhere`]), so what they are in the middle of
    /// typing stays: a half-typed command and an open command line.
    fn sync_inner(&mut self, cx: &mut Cx, keep_command: bool) {
        if !keep_command {
            self.cmdline = None;
            self.pending = Pending::default();
        }
        self.goal = None;
        let sel = cx.ed.selection();
        // The document itself was changed from outside (an undo or redo of
        // the editor, a command of the application), not just the
        // selection.
        let edited = self
            .seen
            .is_some_and(|(revision, _)| revision != cx.ed.revision());
        match self.base {
            Base::Normal if edited && !sel.is_collapsed() => {
                // An edit made from Normal mode does not start Visual mode,
                // whatever selection it leaves behind: the editor's undo
                // brings back the selection a step was made on, and the
                // next operator key would act on it. The cursor goes to
                // the start of what was selected, as after `u`.
                let p = buf::clamp_normal(cx.ed.document(), sel.start());
                cx.ed.set_caret(p);
            }
            Base::Insert | Base::Replace => {
                // The caret was moved (a click): what was typed before is
                // an undo step of its own and no longer repeats.
                let head = cx.ed.selection().head;
                if let Some(s) = self.insert.as_mut() {
                    s.interrupted();
                    s.moved_check = None;
                    s.start = head;
                    if s.group_open {
                        cx.ed.end_undo_group();
                        cx.ed.begin_undo_group();
                    }
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
                        (
                            sel.anchor,
                            buf::prev_cell(doc, sel.head).unwrap_or(sel.head),
                        )
                    } else {
                        (
                            buf::prev_cell(doc, sel.anchor).unwrap_or(sel.anchor),
                            sel.head,
                        )
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

    /// Whether the document may be changed; reports the refusal if not.
    pub(crate) fn writable(&mut self) -> bool {
        if self.options.read_only {
            self.error("E21: Cannot make changes, the document is read-only");
            false
        } else {
            true
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
        let on = buf::clamp_normal(cx.ed.document(), p);
        self.clamped_from_eol = (on != cx.ed.document().clamp_position(p)).then_some(on);
        cx.ed.set_selection(Selection::caret(on));
    }
}
