//! [`VimMode`]: one Vim engine attached to one [`Editor`].

use std::time::Duration;

use gpui::{App, Context, Entity, EventEmitter, Global, Keystroke, Subscription, Task, WeakEntity};
use writer_core::{EditorState, Position, Selection, TextRange};
use writer_editor::{
    Autoscroll, CaretStyle, Editor, EditorEvent, HighlightStyle, InterceptPriority, KeyDisposition,
};
use writer_ui::theme::{ActiveTheme, GlobalTheme};
use writer_vim::{
    CaretShape, Effect, Key, Mode, Outcome, RegisterContent, Scroll, ScrollAlign, Vim, VimHost,
    VimOptions,
};

use crate::{
    host::{EditorHost, read_clipboard, write_clipboard},
    search::SearchMatches,
    status::{VimEvent, VimMessage, VimStatus},
};

/// The editor's key context entry that carries the mode: `vim_mode=normal`,
/// `insert`, `replace`, `visual`, `visual_line`, `operator` or `command`.
/// Bind keys for one mode with a predicate such as
/// `"Editor && vim_mode == normal"` (such a binding only sees keys the
/// engine leaves alone: `cmd` shortcuts, option chords, function keys).
pub const KEY_CONTEXT_MODE: &str = "vim_mode";

/// The editor highlight layer that marks the search matches.
pub const SEARCH_LAYER: &str = "vim-search";

/// The rows the editor keeps around the caret when it scrolls it into view
/// (`Autoscroll::Fit`): the engine's `scroll_off`, so `H`, `L` and
/// `ctrl-e` leave the caret where the next movement does not shift the
/// view.
const SCROLL_OFF: usize = 1;

/// How long [`VimStatus::bell`] stays on after a command failed.
const BELL_FLASH: Duration = Duration::from_millis(350);

/// What the editor looked like before Vim mode took it over.
#[derive(Clone, Copy)]
struct SavedEditor {
    caret_style: CaretStyle,
    text_input: bool,
}

/// One call into the engine.
enum Call<'a> {
    Key(Key),
    TextInserted(&'a str),
    TextReplaced(TextRange, &'a str),
    CommandLineInput(&'a str),
    Execute(&'a str),
    Sync,
    Attach,
    Detach,
}

impl Call<'_> {
    fn run(&self, vim: &mut Vim, state: &mut EditorState, host: &mut dyn VimHost) -> Outcome {
        match self {
            Call::Key(key) => vim.handle_key(*key, state, host),
            Call::TextInserted(text) => vim.text_inserted(text, state, host),
            Call::TextReplaced(range, text) => vim.text_replaced(*range, text, state, host),
            Call::CommandLineInput(text) => vim.command_line_input(text, state, host),
            Call::Execute(command) => vim.execute(command, state, host),
            Call::Sync => vim.sync(state, host),
            Call::Attach => vim.attach(state, host),
            Call::Detach => vim.detach(state, host),
        }
    }

    /// Whether the Vim cursor should be scrolled into view after the call.
    /// Not for the calls that only bring the engine in line with the
    /// editor: they may nudge the caret onto a character, which must not
    /// undo a scroll position somebody just restored.
    fn follows_caret(&self) -> bool {
        !matches!(self, Call::Sync | Call::Attach | Call::Detach)
    }
}

/// The Vim modes that exist, for finding the one a global action is for.
#[derive(Default)]
struct VimRegistry {
    modes: Vec<WeakEntity<VimMode>>,
    /// The one whose editor had the keyboard focus last.
    last_focused: Option<WeakEntity<VimMode>>,
}

impl Global for VimRegistry {}

/// The [`VimMode`] a window-wide command (the palette's "Toggle Vim mode")
/// is meant for: the one whose editor was focused last, or the only one.
pub(crate) fn target_for_global_action(cx: &mut App) -> Option<Entity<VimMode>> {
    let registry = cx.try_global::<VimRegistry>()?;
    if let Some(mode) = registry.last_focused.as_ref().and_then(WeakEntity::upgrade) {
        return Some(mode);
    }
    let mut live = registry.modes.iter().filter_map(WeakEntity::upgrade);
    let first = live.next()?;
    live.next().is_none().then_some(first)
}

/// Vim mode for one [`Editor`]: owns the [`writer_vim::Vim`] engine, routes
/// the editor's keys to it, keeps the editor's caret, text input and
/// highlights in step with the mode, and reports status and requests.
///
/// Create one per editor and keep it for as long as the editor lives; it
/// starts detached. [`VimMode::attach`] and [`VimMode::detach`] switch Vim
/// mode on and off at any time. While detached it does nothing at all: the
/// editor behaves as if this crate did not exist. Registers and the search
/// and command-line history survive a detach.
///
/// ```ignore
/// let vim = cx.new(|cx| VimMode::new(editor.clone(), cx));
/// vim.update(cx, |vim, cx| vim.set_attached(settings.vim_mode, cx));
/// cx.subscribe(&vim, |this: &mut Shell, _, event: &VimEvent, cx| match event {
///     VimEvent::Request(HostRequest::Save) => this.save(cx),
///     _ => {}
/// }).detach();
/// ```
pub struct VimMode {
    editor: Entity<Editor>,
    vim: Vim,
    /// The options as the host set them; the engine gets them with
    /// `read_only` also following the editor.
    options: VimOptions,
    attached: bool,
    status: VimStatus,
    /// The key interceptor on the editor, while attached.
    keys: Option<Subscription>,
    saved: Option<SavedEditor>,
    search: SearchMatches,
    /// Takes [`VimStatus::bell`] back after a moment.
    bell: Option<Task<()>>,
    /// The editor's document revision and selection after the engine last
    /// looked: a change report that finds them unchanged needs no sync.
    seen: Option<(u64, Selection)>,
    layout_fallbacks: usize,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<VimEvent> for VimMode {}

impl VimMode {
    /// Vim mode for `editor`, detached.
    pub fn new(editor: Entity<Editor>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.subscribe(&editor, Self::on_editor_event),
            // The highlight colours come from the theme.
            cx.observe_global::<GlobalTheme>(|this, cx| this.restyle(cx)),
            // Dropped while attached: give the editor back as it was.
            cx.on_release(|this, cx| this.release(cx)),
        ];
        let handle = cx.weak_entity();
        let registry = cx.default_global::<VimRegistry>();
        registry.modes.retain(|mode| mode.upgrade().is_some());
        registry.modes.push(handle);
        Self {
            editor,
            vim: Vim::new(),
            options: VimOptions::default(),
            attached: false,
            status: VimStatus::default(),
            keys: None,
            saved: None,
            search: SearchMatches::default(),
            bell: None,
            seen: None,
            layout_fallbacks: 0,
            _subscriptions: subscriptions,
        }
    }

    /// The editor this belongs to.
    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    /// Whether Vim mode is on.
    pub fn is_attached(&self) -> bool {
        self.attached
    }

    /// What a status bar shows: mode, pending keys, command line, message.
    /// Idle values while detached.
    pub fn status(&self) -> &VimStatus {
        &self.status
    }

    /// The mode (Normal while detached).
    pub fn mode(&self) -> Mode {
        self.status.mode
    }

    /// The engine, read-only: registers, the search pattern, the last
    /// inserted text.
    pub fn engine(&self) -> &Vim {
        &self.vim
    }

    /// The engine options as set by the host.
    pub fn options(&self) -> &VimOptions {
        &self.options
    }

    /// Changes the engine options. `read_only` is also on whenever the
    /// editor is read-only, whatever is set here.
    pub fn set_options(&mut self, options: VimOptions, cx: &mut Context<Self>) {
        self.options = options;
        self.sync_options(cx);
    }

    /// Layout questions of the engine that were answered without the
    /// editor's layout (see the `host` module). Stays 0 once the editor has
    /// been drawn; tests watch it.
    #[doc(hidden)]
    pub fn layout_fallbacks(&self) -> usize {
        self.layout_fallbacks
    }

    // ----- on and off --------------------------------------------------------------

    /// Switches Vim mode on: Normal mode, block caret on a character, typed
    /// text refused until Insert mode. Does nothing when already on.
    pub fn attach(&mut self, cx: &mut Context<Self>) {
        if self.attached {
            return;
        }
        let editor = self.editor.read(cx);
        self.saved = Some(SavedEditor {
            caret_style: editor.config().caret_style,
            text_input: editor.text_input_enabled(),
        });
        let handle = cx.weak_entity();
        self.keys = Some(editor.intercept_keys(
            InterceptPriority::MODAL,
            move |keystroke, _, cx| {
                handle
                    .update(cx, |this, cx| this.on_key(keystroke, cx))
                    .unwrap_or(KeyDisposition::Pass)
            },
        ));
        self.attached = true;
        self.status = VimStatus {
            attached: true,
            ..VimStatus::default()
        };
        let outcome = self.engine_call(Call::Attach, cx);
        // The attach reports the whole display state as effects.
        self.status.mode = self.vim.mode();
        self.apply_mode(cx);
        self.apply(outcome, false, cx);
        cx.emit(VimEvent::Attached);
        cx.emit(VimEvent::StatusChanged);
        cx.notify();
    }

    /// Switches Vim mode off: an Insert session ends (as one undo step), a
    /// Visual selection collapses, the highlights go, and the editor gets
    /// its caret and its text input back. Does nothing when already off.
    pub fn detach(&mut self, cx: &mut Context<Self>) {
        if !self.attached {
            return;
        }
        self.release(cx);
        cx.emit(VimEvent::Detached);
        cx.emit(VimEvent::StatusChanged);
        cx.notify();
    }

    /// [`VimMode::attach`] or [`VimMode::detach`].
    pub fn set_attached(&mut self, attached: bool, cx: &mut Context<Self>) {
        if attached {
            self.attach(cx);
        } else {
            self.detach(cx);
        }
    }

    /// Switches Vim mode on or off.
    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        self.set_attached(!self.attached, cx);
    }

    /// The detach itself, also run when the entity is dropped.
    fn release(&mut self, cx: &mut App) {
        if !self.attached {
            return;
        }
        self.engine_call(Call::Detach, cx);
        self.attached = false;
        self.keys = None;
        self.seen = None;
        self.bell = None;
        self.search = SearchMatches::default();
        self.status = VimStatus::default();
        let saved = self.saved.take();
        self.editor.update(cx, |editor, cx| {
            editor.clear_highlights(SEARCH_LAYER, cx);
            editor.set_caret_override(None, cx);
            editor.remove_key_context_entry(KEY_CONTEXT_MODE, cx);
            if let Some(saved) = saved {
                editor.set_caret_style(saved.caret_style, cx);
                editor.set_text_input_enabled(saved.text_input, cx);
            }
        });
    }

    // ----- for the host --------------------------------------------------------------

    /// Shows a message in the status (until the next command).
    pub fn show_message(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.set_message(Some(VimMessage::info(text)), cx);
    }

    /// Shows an error in the status: for example
    /// `"E492: Not an editor command: frobnicate"` for a
    /// [`writer_vim::HostRequest::Command`] the application does not know.
    pub fn show_error(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.set_message(Some(VimMessage::error(text)), cx);
    }

    fn set_message(&mut self, message: Option<VimMessage>, cx: &mut Context<Self>) {
        if self.status.message != message {
            self.status.message = message;
            cx.emit(VimEvent::StatusChanged);
            cx.notify();
        }
    }

    /// Runs an ex command line (without the leading `:`), as if it had been
    /// typed and confirmed: `vim.execute("noh", cx)`, `vim.execute("%s/a/b/g", cx)`.
    /// Does nothing while detached.
    pub fn execute(&mut self, command: &str, cx: &mut Context<Self>) {
        if !self.attached {
            return;
        }
        let outcome = self.engine_call(Call::Execute(command), cx);
        self.apply(outcome, true, cx);
    }

    /// Feeds keys in Vim notation (`"ciwnew<Esc>"`, `"<C-r>"`) to the engine
    /// as if they were pressed, for key mappings and scripted commands.
    /// Keys the engine leaves to the editor are dropped, so text "typed" in
    /// Insert mode this way is not inserted. Does nothing while detached.
    pub fn feed_keys(&mut self, keys: &str, cx: &mut Context<Self>) {
        if !self.attached {
            return;
        }
        for key in Key::parse_sequence(keys) {
            let outcome = self.engine_call(Call::Key(key), cx);
            let consumed = outcome.consumed;
            self.apply(outcome, consumed, cx);
        }
    }

    // ----- keys ------------------------------------------------------------------------

    /// One key of the focused editor, before its bindings and text input.
    fn on_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) -> KeyDisposition {
        if !self.attached {
            return KeyDisposition::Pass;
        }
        // The keys of an IME composition belong to the input method.
        if self.editor.read(cx).marked_range().is_some() {
            return KeyDisposition::Pass;
        }
        let modifiers = keystroke.modifiers;
        if modifiers.platform {
            // Application and editor shortcuts, in every mode. One
            // exception: paste goes into an open command line.
            let plain_paste =
                keystroke.key == "v" && !modifiers.control && !modifiers.alt && !modifiers.shift;
            if plain_paste && self.vim.mode() == Mode::CommandLine {
                let text = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .unwrap_or_default();
                let line = text.lines().next().unwrap_or_default().to_string();
                let outcome = self.engine_call(Call::CommandLineInput(&line), cx);
                self.apply(outcome, false, cx);
                return KeyDisposition::Consumed;
            }
            return KeyDisposition::Pass;
        }
        if modifiers.function {
            return KeyDisposition::Pass;
        }
        if let Some(typed) = self.typed_into_popup(keystroke, cx) {
            // The first character typed for a popup that filters by the
            // text in the document starts Insert mode, as `i` does. The
            // editor still refuses typed text for this key (its input
            // handler was registered while Normal mode had it switched
            // off), so the character is inserted here, as typing would.
            if let Some(insert) = Key::parse_sequence("i").into_iter().next() {
                let outcome = self.engine_call(Call::Key(insert), cx);
                self.apply(outcome, true, cx);
            }
            if self.vim.mode() == Mode::Insert {
                self.editor.update(cx, |editor, cx| {
                    editor.edit(|state| state.insert_text(&typed), cx);
                });
                let outcome = self.engine_call(Call::TextInserted(&typed), cx);
                self.apply(outcome, false, cx);
                return KeyDisposition::Consumed;
            }
        }
        let Some(key) = key_from_keystroke(keystroke, wants_literal_key(&self.vim)) else {
            return KeyDisposition::Pass;
        };
        let outcome = self.engine_call(Call::Key(key), cx);
        let consumed = outcome.consumed;
        self.apply(outcome, consumed, cx);
        if consumed {
            KeyDisposition::Consumed
        } else {
            // The editor handles the key as without Vim. If it types text,
            // the editor reports it (`EditorEvent::TextInput`).
            KeyDisposition::Pass
        }
    }

    /// The character `keystroke` types, when it is meant for a popup of the
    /// editor that left the keyboard in the document: the insert menu,
    /// which is filtered by what is typed after it opened. (A popup with a
    /// field of its own has the focus, and its keys never come here.) In
    /// Normal mode such a character would run as a command, on the text
    /// under a menu that is waiting for a name.
    fn typed_into_popup(&self, keystroke: &Keystroke, cx: &App) -> Option<String> {
        let idle_normal = self.vim.mode() == Mode::Normal
            && !self.vim.insert_pending()
            && self.vim.pending_keys().is_empty();
        let editor = self.editor.read(cx);
        if !idle_normal || !editor.has_popup() || editor.is_read_only() {
            return None;
        }
        key_from_keystroke(keystroke, true)?
            .plain_char()
            .map(String::from)
    }

    // ----- editor events ---------------------------------------------------------------

    fn on_editor_event(&mut self, _: Entity<Editor>, event: &EditorEvent, cx: &mut Context<Self>) {
        if matches!(event, EditorEvent::Focused) {
            let handle = cx.weak_entity();
            cx.default_global::<VimRegistry>().last_focused = Some(handle);
        }
        if !self.attached {
            return;
        }
        match event {
            // Text arrived through the platform input handler: a typed
            // character, the commit of an IME composition or a dead key,
            // the emoji palette. It is in the document already, and the
            // `Edited` and `SelectionChanged` of the same insertion follow.
            EditorEvent::TextInput { text, replaced } => {
                let call = match replaced {
                    None if self.vim.mode().is_insert() => Call::TextInserted(text),
                    // The platform replaced text that was already there
                    // (the accent for a held key, a system text
                    // replacement): the engine corrects what it recorded
                    // as typed, where that is what was replaced.
                    Some(range) if self.vim.mode().is_insert() => Call::TextReplaced(*range, text),
                    _ => Call::Sync,
                };
                let outcome = self.engine_call(call, cx);
                self.apply(outcome, false, cx);
            }
            EditorEvent::Edited { change } => {
                self.refresh_search(change, cx);
                self.sync(cx);
            }
            EditorEvent::SelectionChanged => self.sync(cx),
            EditorEvent::DocumentReplaced => self.document_replaced(cx),
            // The keyboard went elsewhere: a half-typed command must not
            // wait there for the first key after coming back.
            EditorEvent::Blurred => {
                let outcome = self.vim.cancel_pending_command();
                self.apply(outcome, false, cx);
                self.present_caret(cx);
            }
            _ => {}
        }
    }

    /// Something changed the editor: let the engine catch up, unless the
    /// editor is exactly as the engine left it (which it is for the
    /// `Edited` and `SelectionChanged` that follow a reported `TextInput`,
    /// and for the engine's own changes). The updates of an IME
    /// composition in progress are neither keys nor typed text: the engine
    /// hears of the composition when it is committed.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let editor = self.editor.read(cx);
        if editor.marked_range().is_some() {
            return;
        }
        let now = (editor.state().revision(), editor.selection());
        if self.seen == Some(now) {
            return;
        }
        let outcome = self.engine_call(Call::Sync, cx);
        self.apply(outcome, false, cx);
    }

    /// Another document was loaded into the editor: start over in Normal
    /// mode. Registers and history stay; marks and the jump list go.
    fn document_replaced(&mut self, cx: &mut Context<Self>) {
        self.status.message = None;
        // The editor dropped its highlight layers and the caret override
        // with the old document.
        self.search = SearchMatches::default();
        let outcome = self.engine_call(Call::Attach, cx);
        self.apply(outcome, false, cx);
    }

    // ----- the engine ------------------------------------------------------------------

    /// The engine follows the editor's read-only state, and keeps the row
    /// of context around the caret that the editor keeps.
    fn sync_options(&mut self, cx: &App) {
        let mut options = self.options.clone();
        options.read_only |= self.editor.read(cx).is_read_only();
        options.scroll_off = SCROLL_OFF;
        if *self.vim.options() != options {
            self.vim.set_options(options);
        }
    }

    /// Whether the call may read the `+` / `*` register, so the clipboard
    /// has to be fetched before it.
    fn wants_clipboard(&self, call: &Call) -> bool {
        match call {
            Call::Key(key) => {
                self.vim.options().unnamed_clipboard
                    || matches!(self.vim.pending_register(), Some('+' | '*'))
                    // The register name itself (after `"` or `ctrl-r`), and
                    // a repeat of a command that named it.
                    || matches!(key.plain_char(), Some('+' | '*' | '.'))
            }
            Call::Execute(_) => true,
            _ => false,
        }
    }

    /// Runs one call into the engine on the editor's own state, with the
    /// editor's row oracle answering the layout questions (see the `host`
    /// module), then carries out what the call left to do: the caret goes
    /// to the Vim cursor, the view follows it or the explicit scroll
    /// requests, and the clipboard is written.
    fn engine_call(&mut self, call: Call, cx: &mut App) -> Outcome {
        self.sync_options(cx);
        let clipboard = self
            .wants_clipboard(&call)
            .then(|| read_clipboard(cx))
            .flatten();
        let detaching = matches!(call, Call::Detach);
        let follows_caret = call.follows_caret();
        let vim = &mut self.vim;
        let (outcome, clipboard_write, fallbacks) = self.editor.update(cx, |editor, cx| {
            let before = (
                editor.state().revision(),
                editor.selection(),
                editor.caret_position(),
            );
            // No autoscroll by the edit itself: it would aim at the
            // selection head, and the Vim cursor is known only afterwards.
            let (outcome, scrolls, clipboard_write, fallbacks) = editor.edit_with_rows(
                Autoscroll::None,
                |state, rows| {
                    let mut host = EditorHost::new(clipboard).with_rows(rows);
                    let outcome = call.run(vim, state, &mut host);
                    (outcome, host.scrolls, host.clipboard_write, host.fallbacks)
                },
                cx,
            );
            if detaching {
                editor.set_caret_override(None, cx);
            } else {
                present_caret(vim, editor, cx);
            }
            let after = (
                editor.state().revision(),
                editor.selection(),
                editor.caret_position(),
            );
            if !scrolls.is_empty() {
                // An explicit scroll decides where the view goes.
                apply_scrolls(editor, &scrolls, cx);
            } else if follows_caret && (outcome.consumed || before != after) {
                editor.scroll_to_caret(Autoscroll::Fit, cx);
            }
            (outcome, clipboard_write, fallbacks)
        });
        self.layout_fallbacks += fallbacks;
        if let Some(content) = clipboard_write {
            write_clipboard(&content, cx);
        }
        let editor = self.editor.read(cx);
        self.seen = Some((editor.state().revision(), editor.selection()));
        outcome
    }

    /// Puts the editor's caret where the Vim cursor is, in the engine's
    /// shape, after something other than an engine call changed either.
    fn present_caret(&mut self, cx: &mut App) {
        let vim = &self.vim;
        self.editor
            .update(cx, |editor, cx| present_caret(vim, editor, cx));
    }

    /// Takes over what a call reported.
    fn apply(&mut self, outcome: Outcome, clear_message: bool, cx: &mut Context<Self>) {
        let before = self.status.clone();
        if clear_message {
            self.status.message = None;
            // The next command ends the flash of the one that failed.
            self.status.bell = false;
            self.bell = None;
        }
        for effect in outcome.effects {
            match effect {
                Effect::ModeChanged(mode) => {
                    self.status.mode = mode;
                    self.apply_mode(cx);
                    cx.emit(VimEvent::ModeChanged(mode));
                }
                Effect::CommandLine(line) => self.status.command_line = line,
                Effect::PendingKeys(keys) => self.status.pending_keys = keys,
                Effect::Message(text) => self.status.message = Some(VimMessage::info(text)),
                Effect::Error(text) => self.status.message = Some(VimMessage::error(text)),
                Effect::SearchHighlight(pattern) => {
                    let editor = self.editor.read(cx);
                    self.search
                        .set(pattern, editor.document(), editor.state().revision());
                    self.paint_search(cx);
                }
                Effect::Request(request) => cx.emit(VimEvent::Request(request)),
                Effect::Bell => {
                    cx.emit(VimEvent::Bell);
                    self.ring(cx);
                }
            }
        }
        self.status.insert_pending = self.vim.insert_pending();
        self.status.caret = self.vim.caret_shape();
        self.status.search_highlighted = self.search.pattern().is_some();
        if self.status != before {
            cx.emit(VimEvent::StatusChanged);
            cx.notify();
        }
    }

    /// A command failed: [`VimStatus::bell`] is on for a moment, which the
    /// status item shows as a flash of the mode chip.
    fn ring(&mut self, cx: &mut Context<Self>) {
        self.status.bell = true;
        self.bell = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(BELL_FLASH).await;
            this.update(cx, |this, cx| {
                this.bell = None;
                if this.status.bell {
                    this.status.bell = false;
                    cx.emit(VimEvent::StatusChanged);
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// The parts of the editor that follow the mode: text input and the
    /// key context. (The caret follows every call, see `present_caret`.)
    fn apply_mode(&mut self, cx: &mut Context<Self>) {
        let mode = self.status.mode;
        self.editor.update(cx, |editor, cx| {
            // Outside Insert and Replace mode nothing typed, composed,
            // dictated or picked from the emoji palette reaches the text.
            editor.set_text_input_enabled(mode.is_insert(), cx);
            editor.set_key_context_value(KEY_CONTEXT_MODE, mode_context_name(mode), cx);
        });
    }

    // ----- caret and highlights --------------------------------------------------------

    /// Hands the search matches to the editor's highlight layer.
    fn paint_search(&mut self, cx: &mut Context<Self>) {
        let style = HighlightStyle::background(cx.theme().colors.find_match);
        let ranges = self.search.ranges().to_vec();
        let highlighted = self.search.pattern().is_some();
        self.editor.update(cx, |editor, cx| {
            if highlighted {
                editor.set_highlights(SEARCH_LAYER, ranges, style, cx);
            } else {
                editor.clear_highlights(SEARCH_LAYER, cx);
            }
        });
    }

    /// The document changed: the highlight ranges are positions in the old
    /// text.
    fn refresh_search(&mut self, change: &writer_core::Change, cx: &mut Context<Self>) {
        let editor = self.editor.read(cx);
        if self
            .search
            .document_changed(change, editor.document(), editor.state().revision())
        {
            self.paint_search(cx);
        }
    }

    /// The theme changed: the layers keep the colours they were set with.
    fn restyle(&mut self, cx: &mut Context<Self>) {
        if !self.attached {
            return;
        }
        if self.search.pattern().is_some() {
            self.paint_search(cx);
        }
        cx.notify();
    }
}

/// Draws the Vim cursor. The editor draws its caret at the head of its
/// selection, which is where the Vim cursor is except in Visual mode: there
/// the selection covers the selected characters, and its head is the far
/// end of the last one (or of the last line, in Visual Line mode). Then
/// the caret is drawn at the cursor through the editor's caret override:
/// the same blinking block, on top of the selection tint.
fn present_caret(vim: &Vim, editor: &mut Editor, cx: &mut Context<Editor>) {
    let style = caret_style(vim.caret_shape());
    if editor.config().caret_style != style {
        editor.set_caret_style(style, cx);
    }
    let state = editor.state();
    let cursor = vim.caret(state);
    let head = state.selection().head;
    editor.set_caret_override((cursor != head).then_some(cursor), cx);
}

fn caret_style(shape: CaretShape) -> CaretStyle {
    match shape {
        CaretShape::Block => CaretStyle::Block,
        CaretShape::Bar => CaretStyle::Bar,
        CaretShape::Underline => CaretStyle::Underline,
    }
}

/// The value of [`KEY_CONTEXT_MODE`] for a mode.
fn mode_context_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => "normal",
        Mode::Insert => "insert",
        Mode::Replace => "replace",
        Mode::Visual => "visual",
        Mode::VisualLine => "visual_line",
        Mode::OperatorPending => "operator",
        Mode::CommandLine => "command",
    }
}

/// Carries out the scroll requests of one engine call.
fn apply_scrolls(editor: &mut Editor, scrolls: &[Scroll], cx: &mut Context<Editor>) {
    for scroll in scrolls {
        match *scroll {
            Scroll::Rows(rows) => {
                scroll_by_visual_rows(editor, rows, cx);
                // The engine moved the caret by the same rows, so it is
                // where it was on screen; at the ends of the document,
                // where the view cannot follow, it is brought back in.
                editor.scroll_to_caret(Autoscroll::Fit, cx);
            }
            Scroll::Align { caret, align } => {
                // The editor keeps a row of context around the caret when
                // it scrolls for a movement. Leave the same row here, or
                // the next movement after `zt` would shift the view.
                let (target, autoscroll) = match align {
                    ScrollAlign::Top => (neighbour_row(editor, caret, -1), Autoscroll::Top),
                    ScrollAlign::Center => (caret, Autoscroll::Center),
                    ScrollAlign::Bottom => (neighbour_row(editor, caret, 1), Autoscroll::Bottom),
                };
                editor.scroll_to_position(target, autoscroll, cx);
            }
        }
    }
}

/// Scrolls the view by `rows` rows of the text that is actually there: the
/// row that many rows below (negative: above) the first visible one goes
/// to the top of the viewport. Counting real rows rather than pixels of
/// body text keeps the view in step with the caret, which the engine moves
/// by rows, also where headings and the gaps between paragraphs make the
/// rows unequal.
fn scroll_by_visual_rows(editor: &mut Editor, rows: isize, cx: &mut Context<Editor>) {
    let Some(top) = editor.first_visible_row() else {
        return;
    };
    let Some(step) = editor.move_by_visual_rows(top.start, rows, None) else {
        return;
    };
    let target = editor
        .visual_row(step.position)
        .map_or(step.position, |row| row.start);
    if target == Position::ZERO {
        // The first row of the document: show what is above it too.
        let offset = editor.scroll_offset();
        editor.scroll_by_pixels(-offset, cx);
    } else {
        editor.scroll_to_position(target, Autoscroll::Top, cx);
    }
}

/// A position on the row above (`-1`) or below (`1`) the row of `position`,
/// or `position` itself at the ends of the document.
fn neighbour_row(editor: &Editor, position: Position, rows: isize) -> Position {
    editor
        .move_by_visual_rows(position, rows, None)
        .map_or(position, |step| step.position)
}

/// The engine's key for a GPUI keystroke; `None` for keys the engine has
/// no use for. `literal` says that the key stands for the character it
/// types (text, a search pattern, the target of `f` or `r`) rather than for
/// a command.
///
/// Two things about keyboard layouts:
///
/// - Where `[`, `{`, `|`, `@` or `~` are typed with the option key, they
///   are characters, not option chords: when option turns the key into
///   another ASCII character, the engine gets that character without the
///   option flag, so `{` and `}` work as motions on a German keyboard.
/// - On a layout without Latin letters (Russian, Greek) GPUI still names
///   the key by its Latin position. Commands are Latin, so a command key
///   that types a non-ASCII character is taken by its position: `j` moves
///   down whatever the layout types for it (Vim's `langmap`).
fn key_from_keystroke(keystroke: &Keystroke, literal: bool) -> Option<Key> {
    let modifiers = keystroke.modifiers;
    let mut key_char = keystroke.key_char.as_deref();
    let typed_with_option = modifiers.alt
        && !modifiers.control
        && key_char.is_some_and(|typed| {
            typed.len() == 1 && typed.as_bytes()[0].is_ascii_graphic() && typed != keystroke.key
        });
    let latin_position = keystroke.key.len() == 1 && keystroke.key.is_ascii();
    if !literal
        && !modifiers.alt
        && latin_position
        && key_char.is_some_and(|typed| !typed.is_ascii())
    {
        key_char = None;
    }
    Key::from_keystroke(
        &keystroke.key,
        key_char,
        modifiers.control,
        modifiers.alt && !typed_with_option,
        modifiers.shift,
    )
}

/// Whether the engine takes the next key as a character rather than as a
/// command: in Insert and Replace mode, in the command line, and for the
/// argument of `f`, `F`, `t`, `T` and `r`.
fn wants_literal_key(vim: &Vim) -> bool {
    matches!(vim.mode(), Mode::Insert | Mode::Replace | Mode::CommandLine)
        || awaits_character(vim.pending_keys())
}

/// Whether the keys of an unfinished command end with `f`, `F`, `t`, `T`
/// or `r` as a command (whose argument is any character), rather than as
/// the name of a register (`"f`, `"r`).
fn awaits_character(pending: &str) -> bool {
    let mut last = None;
    let mut keys = pending.chars();
    while let Some(key) = keys.next() {
        if key == '"' {
            // The key after it names a register.
            keys.next();
            last = None;
        } else {
            last = Some(key);
        }
    }
    matches!(last, Some('f' | 'F' | 't' | 'T' | 'r'))
}

impl VimMode {
    /// Sets register `name` (`'a'`..`'z'`, `'"'`, `'+'` for the system
    /// clipboard, …).
    pub fn set_register(&mut self, name: char, content: RegisterContent, cx: &mut Context<Self>) {
        let mut host = EditorHost::new(None);
        self.vim.set_register(name, content, &mut host);
        if let Some(content) = host.clipboard_write {
            write_clipboard(&content, cx);
        }
    }
}
