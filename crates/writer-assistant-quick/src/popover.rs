//! [`QuickChat`]: the popover.

use std::{cell::Cell, rc::Rc, sync::Arc};

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, ClickEvent, ClipboardItem, Context, Entity,
    EventEmitter, FocusHandle, Focusable, IntoElement, Pixels, Render, SharedString, Subscription,
    Task, Window, div, prelude::*, px,
};
use writer_assistant::{
    AssistantEvent, Availability, Backend, BackendInfo, Catalog, ChatSession, MessageId, Mode,
    Role, Selection, Transcript,
};
use writer_assistant_ui::{
    AnswerAction, AnswerStyle, ChatKeys, ChooseEffort, ChooseModel, Composer, ComposerEvent,
    ComposerOptions, ControlBounds, CopyAnswer, DismissChat, FocusNextArea, FocusPreviousArea,
    MessageList, MessageListEvent, MessageListOptions, Notice, PickerKind, ReplaceSelection,
    StopAnswer, controls::icon_button, notice_row, open_picker_menu, picker_button,
};
use writer_core::TextRange;
use writer_editor::{Editor, EditorEvent};
use writer_ui::{
    ContextMenu, Icon, IconName, KeyHint, Toast,
    theme::{ActiveTheme, RADIUS_LG, SPACE_1, SPACE_2, UI_LINE_HEIGHT, UI_TEXT_BASE, UI_TEXT_SM},
    toast::show_toast,
};

use crate::{
    actions::{EDITOR_FLAG, InsertBelow, KEY_CONTEXT},
    apply,
    capture::Capture,
    layer::{DRAW_PRIORITY, Layer},
    place::{self, Anchor, FULL_HEIGHT, Plan, Side},
};

/// The height the conversation starts with when the first question is
/// sent; it grows from there with what is said.
const FIRST_LIST_HEIGHT: Pixels = px(56.);

/// How often the conversation is measured in one frame before the popover
/// is drawn (each time it turns out to want more room than it was given).
const MEASURING_PASSES: usize = 3;

/// What the composer says while it is empty, once a question was asked.
const FOLLOW_UP_PLACEHOLDER: &str = "Ask a follow-up";

/// How tall the composer's field grows in the popover before it scrolls.
const COMPOSER_HEIGHT: Pixels = px(96.);

/// One of the three things that can be done with an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QuickAction {
    /// Replace Selection: the answer goes where the selected text was.
    Replace,
    /// Insert Below: the answer becomes new blocks under the block the
    /// chat was opened in.
    InsertBelow,
    /// Copy: the answer goes to the clipboard.
    Copy,
}

impl QuickAction {
    /// Every action, in the order of the buttons.
    pub const ALL: [QuickAction; 3] = [
        QuickAction::Replace,
        QuickAction::InsertBelow,
        QuickAction::Copy,
    ];

    /// The name of the action in the shared message list
    /// (`AnswerAction::id`); the button of an answer has the debug
    /// selector `assistant-action-<id>-<message id>`.
    pub fn id(self) -> &'static str {
        match self {
            QuickAction::Replace => "replace",
            QuickAction::InsertBelow => "insert",
            QuickAction::Copy => "copy",
        }
    }

    /// The words on the button.
    pub fn label(self) -> &'static str {
        match self {
            QuickAction::Replace => "Replace Selection",
            QuickAction::InsertBelow => "Insert Below",
            QuickAction::Copy => "Copy",
        }
    }

    fn from_id(id: &str) -> Option<QuickAction> {
        QuickAction::ALL
            .into_iter()
            .find(|action| action.id() == id)
    }
}

/// Why "Replace Selection" can or cannot be used right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReplaceState {
    /// The selected text is where it was, unchanged.
    Ready,
    /// The chat was opened without a selection.
    NothingSelected,
    /// The selected text has changed (or is gone) since the chat opened.
    TextChanged,
    /// The document cannot be edited.
    ReadOnly,
}

/// The notices of the popover, above the composer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QuickNotice {
    /// The agent is missing or nobody is signed in. Its button looks again.
    Availability,
    /// Where the text goes, shown until it is acknowledged.
    Privacy,
    /// Something the agent itself wants the user to know.
    Agent,
}

impl QuickNotice {
    /// The debug selector of the notice's row (its button is
    /// `<selector>-action`).
    pub fn selector(self) -> &'static str {
        match self {
            QuickNotice::Availability => "quick-chat-availability",
            QuickNotice::Privacy => "quick-chat-privacy",
            QuickNotice::Agent => "quick-chat-agent-notice",
        }
    }
}

/// What a [`QuickChat`] tells the app.
#[derive(Clone, Debug, PartialEq)]
pub enum QuickChatEvent {
    /// The popover opened.
    Opened,
    /// The popover closed; its conversation is over.
    Closed,
    /// The user chose a model or an effort level in the popover: what the
    /// quick chat asks with from now on, to be remembered.
    SelectionChanged(Selection),
    /// The note about where the text goes was read ("OK", or the first
    /// message was sent): do not show it again.
    PrivacyNoticeAcknowledged,
    /// The popover looked again for the agent ("Check Again", or a message
    /// sent while the agent was known to be signed out) and found this.
    AvailabilityChanged(Availability),
    /// An answer was put into the document; `range` is selected.
    Applied {
        /// Replace Selection or Insert Below.
        action: QuickAction,
        /// The text that was put in, as it is in the document now.
        range: TextRange,
    },
    /// An answer (or a message) was copied to the clipboard.
    Copied,
}

/// Where the popover was drawn in the last frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drawn {
    /// Its rectangle in the window.
    pub bounds: Bounds<Pixels>,
    /// The side of its text it is on.
    pub side: Side,
    /// The height it may grow to where it is.
    pub max_height: Pixels,
    /// Whether it stays clear of the text it was opened on.
    pub clear: bool,
    /// The text it is attached to: the first and the last row of the
    /// selection, or the caret.
    pub anchor: Anchor,
}

/// The open popover: one conversation about one place in the text.
struct Open {
    capture: Capture,
    session: ChatSession,
    catalog: Catalog,
    messages: Entity<MessageList>,
    composer: Entity<Composer>,
    model_bounds: ControlBounds,
    effort_bounds: ControlBounds,
    /// The height the conversation is given (before the popover's own
    /// limit).
    list_height: Pixels,
    /// The most that height can usefully be where the popover is now.
    list_limit: Pixels,
    /// The conversation may have grown since it was last measured (a
    /// message, a piece of an answer, other room): it is measured before
    /// the next frame draws it.
    measure: bool,
    /// The room (width and greatest height) the conversation was last
    /// measured in.
    measured_in: Option<(Pixels, Pixels)>,
    /// What the answer buttons were last set for.
    replace: Option<(ReplaceState, bool)>,
    /// Whether the agent's program was started ahead of the first message.
    prepared: bool,
    /// A message that waits for the answer of "is the user signed in?".
    waiting_to_send: Option<String>,
    /// Reads the events of the conversation. Dropped with the popover.
    _pump: Task<()>,
    _subscriptions: Vec<Subscription>,
}

/// The quick chat: a small popover at the selection or the caret of an
/// editor, for a word, a sentence or a paragraph. See the crate
/// documentation for how the app mounts it.
pub struct QuickChat {
    editor: Entity<Editor>,
    backend: Arc<dyn Backend>,
    info: BackendInfo,
    selection: Selection,
    availability: Option<Availability>,
    checking: bool,
    privacy_notice: bool,
    open: Option<Open>,
    /// The scope of the popover: the keyboard is in the quick chat while
    /// it is in here.
    focus_handle: FocusHandle,
    drawn: Rc<Cell<Option<Drawn>>>,
    /// How often a frame was drawn with a conversation that wanted more
    /// room than it had (it is given the room in the frame after).
    late_growths: usize,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<QuickChatEvent> for QuickChat {}

impl Focusable for QuickChat {
    /// The scope of the popover. Focusing it puts the keyboard in the
    /// composer while the popover is open.
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl QuickChat {
    /// The quick chat of `editor`, on `backend`. Nothing is shown, started
    /// or sent until it is opened and a message is sent.
    ///
    /// The model and effort are the backend's own suggestion for a quick
    /// answer until the app says otherwise ([`QuickChat::set_selection`]).
    pub fn new(
        editor: Entity<Editor>,
        backend: Arc<dyn Backend>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let subscriptions = vec![
            cx.subscribe_in(&editor, window, Self::on_editor_event),
            // A press on the popover beside its controls lands on the
            // popover itself: the keyboard belongs in the composer.
            cx.on_focus(&focus_handle, window, |this, window, cx| {
                if let Some(open) = &this.open {
                    open.composer.read(cx).focus(window, cx);
                }
            }),
            // The keyboard left the popover for somewhere else in the
            // window (a panel, a modal, the text through a command): the
            // popover goes, the keyboard stays where it went. Another app
            // coming to the front is no reason to close.
            cx.on_focus_out(&focus_handle, window, |this, _, window, cx| {
                if window.is_window_active()
                    && this.open.is_some()
                    && !this.has_keyboard(window, cx)
                {
                    this.close_and(false, window, cx);
                }
            }),
        ];
        Self {
            info: backend.info(),
            selection: backend.catalog().default_for(Mode::Quick),
            availability: backend.cached_availability(),
            editor,
            backend,
            checking: false,
            privacy_notice: false,
            open: None,
            focus_handle,
            drawn: Rc::new(Cell::new(None)),
            late_growths: 0,
            _subscriptions: subscriptions,
        }
    }

    // ----- reading -----------------------------------------------------------------

    /// The editor the chat is about.
    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    /// The agent the chat runs on.
    pub fn backend(&self) -> &Arc<dyn Backend> {
        &self.backend
    }

    /// Whether the popover is open.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The model and effort the quick chat asks with.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// What is known about the agent on this Mac.
    pub fn availability(&self) -> Option<&Availability> {
        self.availability.as_ref()
    }

    /// Whether the popover is looking for the agent right now.
    pub fn is_checking(&self) -> bool {
        self.checking
    }

    /// Whether the note about where the text goes is still to be shown.
    pub fn shows_privacy_notice(&self) -> bool {
        self.privacy_notice
    }

    /// What the open popover was opened on.
    pub fn capture(&self) -> Option<&Capture> {
        self.open.as_ref().map(|open| &open.capture)
    }

    /// The conversation of the open popover.
    pub fn transcript(&self) -> Option<&Transcript> {
        self.open.as_ref().map(|open| open.session.transcript())
    }

    /// Whether an answer is being written.
    pub fn is_answering(&self) -> bool {
        self.open
            .as_ref()
            .is_some_and(|open| open.session.is_answering())
    }

    /// The composer of the open popover.
    pub fn composer(&self) -> Option<&Entity<Composer>> {
        self.open.as_ref().map(|open| &open.composer)
    }

    /// The message list of the open popover.
    pub fn messages(&self) -> Option<&Entity<MessageList>> {
        self.open.as_ref().map(|open| &open.messages)
    }

    /// The quote of the selection the popover shows above an empty
    /// conversation (after the first question the question itself carries
    /// it).
    pub fn quote(&self) -> Option<String> {
        let open = self.open.as_ref()?;
        if !open.session.transcript().is_empty() {
            return None;
        }
        open.capture.quote()
    }

    /// Where the popover was drawn in the last frame. `None` while it is
    /// closed or was not drawn (the editor has no layout yet).
    pub fn drawn(&self) -> Option<Drawn> {
        self.drawn.get()
    }

    /// The rectangle of the popover in the last frame.
    pub fn bounds(&self) -> Option<Bounds<Pixels>> {
        self.drawn.get().map(|drawn| drawn.bounds)
    }

    /// Whether the keyboard is in the popover.
    pub fn has_keyboard(&self, window: &Window, cx: &App) -> bool {
        let Some(open) = &self.open else {
            return false;
        };
        // The second and third answer for a popover that was given the
        // keyboard and has not been drawn yet.
        self.focus_handle.contains_focused(window, cx)
            || open.composer.read(cx).is_focused(window, cx)
            || open.messages.read(cx).contains_focus(window, cx)
    }

    /// Whether "Replace Selection" can be used, and if not, why.
    pub fn replace_state(&self, cx: &App) -> Option<ReplaceState> {
        let open = self.open.as_ref()?;
        let editor = self.editor.read(cx);
        Some(if !open.capture.has_selection() {
            ReplaceState::NothingSelected
        } else if editor.is_read_only() {
            ReplaceState::ReadOnly
        } else if open.capture.is_intact(editor.document()) {
            ReplaceState::Ready
        } else {
            ReplaceState::TextChanged
        })
    }

    /// The notices above the composer, most pressing first: the agent is
    /// missing or nobody is signed in, the note about where the text goes,
    /// what the agent itself wants the user to know.
    pub fn notices(&self) -> Vec<(QuickNotice, Notice)> {
        let mut notices = Vec::new();
        match &self.availability {
            Some(Availability::NotInstalled) => {
                notices.push((QuickNotice::Availability, Notice::not_installed(&self.info)))
            }
            Some(availability) if availability.is_signed_out() => {
                notices.push((QuickNotice::Availability, Notice::signed_out(&self.info)))
            }
            _ => {}
        }
        if self.privacy_notice && notices.is_empty() {
            notices.push((
                QuickNotice::Privacy,
                Notice::privacy(&self.info, Mode::Quick),
            ));
        }
        if let Some(text) = self.open.as_ref().and_then(|open| open.session.notice()) {
            notices.push((
                QuickNotice::Agent,
                Notice::info(text.to_string(), "").action("Dismiss"),
            ));
        }
        notices
    }

    /// One line about the popover for scripted runs: `None` while it is
    /// closed.
    pub fn describe(&self, cx: &App) -> Option<String> {
        let open = self.open.as_ref()?;
        let transcript = open.session.transcript();
        let side = match self.drawn.get().map(|drawn| drawn.side) {
            Some(Side::Below) => "below",
            Some(Side::Above) => "above",
            None => "not drawn",
        };
        let last = transcript
            .last_answer()
            .map(|answer| format!(" last={:?}", crate::capture::quote(&answer.text, 60)))
            .unwrap_or_default();
        let notices: Vec<&str> = self
            .notices()
            .iter()
            .map(|(notice, _)| match notice {
                QuickNotice::Availability => "availability",
                QuickNotice::Privacy => "privacy",
                QuickNotice::Agent => "agent",
            })
            .collect();
        Some(format!(
            "open side={side} selection={:?} messages={} answering={} model={:?} effort={:?} replace={:?} notices={notices:?} draft={:?}{last}",
            open.capture.quote().unwrap_or_default(),
            transcript.len(),
            open.session.is_answering(),
            open.catalog.model_label(&self.selection.model),
            open.catalog.effort_label(&self.selection.effort),
            self.replace_state(cx)
                .unwrap_or(ReplaceState::NothingSelected),
            open.composer.read(cx).text(cx),
        ))
    }

    // ----- what the app tells it ---------------------------------------------------

    /// Sets the model and effort the quick chat asks with (what the app
    /// remembers for it, or a change made in the settings). An open
    /// conversation continues with them from its next message. No event is
    /// emitted: the app already knows.
    pub fn set_selection(&mut self, selection: Selection, cx: &mut Context<Self>) {
        if self.selection != selection {
            if let Some(open) = &mut self.open {
                open.session.set_selection(selection.clone());
            }
            self.selection = selection;
            cx.notify();
        }
    }

    /// Says what is known about the agent (the app looks at launch and
    /// when the settings open).
    pub fn set_availability(&mut self, availability: Availability, cx: &mut Context<Self>) {
        if self.availability.as_ref() != Some(&availability) {
            self.availability = Some(availability);
            cx.notify();
        }
    }

    /// Shows or hides the note about where the text goes. The app shows it
    /// until [`QuickChatEvent::PrivacyNoticeAcknowledged`] (of either
    /// chat).
    pub fn set_privacy_notice(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.privacy_notice != show {
            self.privacy_notice = show;
            cx.notify();
        }
    }

    // ----- opening and closing -------------------------------------------------------

    /// What the app's key, menu item and format bar button do: opens the
    /// popover at the selection (or the caret), or closes it when it is
    /// open.
    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open.is_some() {
            self.close(window, cx);
        } else {
            self.open(window, cx);
        }
    }

    /// Opens the popover on the editor's selection as it is now (or at the
    /// caret, without context, when nothing is selected) and puts the
    /// keyboard in its composer. An open popover only gets the keyboard
    /// back. Returns whether the popover is open afterwards: a compact
    /// editor has no quick chat.
    ///
    /// Nothing is sent and no program is started by opening.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(open) = &self.open {
            open.composer.read(cx).focus(window, cx);
            return true;
        }
        let capture = {
            let editor = self.editor.read(cx);
            if editor.config().mode.is_compact() {
                return false;
            }
            Capture::new(editor.document(), editor.selection().range())
        };
        // What the backend has learned since (the app's check at launch,
        // the side chat's "Check Again").
        if let Some(cached) = self.backend.cached_availability() {
            self.availability = Some(cached);
        }
        let catalog = self.backend.catalog();
        let session = ChatSession::quick(self.backend.clone())
            .with_selection(catalog.normalized(&self.selection));
        let keys = ChatKeys::new(EDITOR_FLAG);
        let messages = cx.new(|cx| {
            MessageList::new(
                MessageListOptions::new(self.info.clone())
                    .style(AnswerStyle::small())
                    .keys(keys.clone()),
                window,
                cx,
            )
        });
        let placeholder = if capture.has_selection() {
            "Ask about the selection"
        } else {
            "Ask anything"
        };
        let composer = cx.new(|cx| {
            Composer::new(
                ComposerOptions::new(placeholder)
                    .max_height(COMPOSER_HEIGHT)
                    .keys(keys),
                window,
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe_in(&composer, window, Self::on_composer_event),
            cx.subscribe_in(&messages, window, Self::on_list_event),
        ];
        let events = session.events();
        let pump = cx.spawn_in(window, async move |this, cx| {
            while let Some(batch) = events.next_batch().await {
                let alive = this.update_in(cx, |this, window, cx| {
                    this.apply_events(batch, window, cx);
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        composer.read(cx).focus(window, cx);
        self.open = Some(Open {
            capture,
            session,
            catalog,
            messages,
            composer,
            model_bounds: ControlBounds::new(),
            effort_bounds: ControlBounds::new(),
            list_height: px(0.),
            list_limit: FULL_HEIGHT,
            measure: true,
            measured_in: None,
            replace: None,
            prepared: false,
            waiting_to_send: None,
            _pump: pump,
            _subscriptions: subscriptions,
        });
        self.refresh_actions(cx);
        cx.emit(QuickChatEvent::Opened);
        cx.notify();
        true
    }

    /// Closes the popover. The conversation ends: an answer that is being
    /// written is stopped, the agent's program goes, and the next opening
    /// starts afresh. The keyboard goes back to the text if it was in the
    /// popover.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_and(true, window, cx);
    }

    /// Closes the popover; with `refocus` the keyboard goes to the text
    /// when it was in the popover.
    fn close_and(&mut self, refocus: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.open.is_none() {
            return;
        }
        let had_keyboard = self.has_keyboard(window, cx);
        // Dropping the session closes the conversation and ends the
        // agent's process; dropping the pump ends the reading of events.
        self.open = None;
        self.drawn.set(None);
        if refocus && had_keyboard {
            self.editor.read(cx).focus(window);
        }
        cx.emit(QuickChatEvent::Closed);
        cx.notify();
    }

    /// What `escape` does: an answer that is being written is stopped;
    /// with nothing being written the popover closes.
    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &mut self.open {
            Some(open) if open.session.is_answering() => {
                open.session.stop();
                cx.notify();
            }
            Some(_) => self.close(window, cx),
            None => {}
        }
    }

    // ----- the conversation ----------------------------------------------------------

    /// Sends a question, as typing it into the composer and pressing
    /// `enter` does: with the selected text the popover was opened on (it
    /// goes with the first question; the conversation has it from then
    /// on), or alone when nothing was selected. With the agent known to be
    /// signed out (or missing) the popover looks again first, and sends
    /// when the user is signed in by then; otherwise the question is put
    /// back into the composer.
    pub fn ask(&mut self, text: impl Into<String>, window: &mut Window, cx: &mut Context<Self>) {
        let text = text.into();
        if self.open.is_none() || text.trim().is_empty() {
            return;
        }
        let ready = self
            .availability
            .as_ref()
            .is_none_or(Availability::is_ready);
        if !ready {
            if let Some(open) = &mut self.open {
                open.waiting_to_send = Some(text);
            }
            self.recheck(window, cx);
            return;
        }
        self.send_now(text, window, cx);
    }

    fn send_now(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.privacy_notice {
            self.privacy_notice = false;
            cx.emit(QuickChatEvent::PrivacyNoticeAcknowledged);
        }
        let Some(open) = &mut self.open else {
            return;
        };
        open.session.dismiss_notice();
        // The selected text goes with the first question. A question whose
        // answer failed may never have arrived: the next one carries the
        // selection again.
        let selection = (open.capture.has_selection()
            && !selection_delivered(open.session.transcript()))
        .then(|| open.capture.text().to_string());
        open.session.ask_quick(&text, selection.as_deref());
        open.prepared = true;
        if open.list_height < FIRST_LIST_HEIGHT {
            open.list_height = FIRST_LIST_HEIGHT;
        }
        open.messages
            .update(cx, |list, cx| list.scroll_to_bottom(cx));
        open.composer.update(cx, |composer, cx| {
            composer.set_placeholder(FOLLOW_UP_PLACEHOLDER, cx)
        });
        self.sync(window, cx);
    }

    /// Asks the question of a failed answer again.
    pub fn retry(&mut self, answer: MessageId, window: &mut Window, cx: &mut Context<Self>) {
        let question = self.open.as_ref().and_then(|open| {
            open.session
                .transcript()
                .question_of(answer)
                .map(|question| question.text.clone())
        });
        if let Some(question) = question {
            self.ask(question, window, cx);
        }
    }

    /// Stops the answer that is being written; what had arrived stays.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = &mut self.open {
            open.session.stop();
            cx.notify();
        }
    }

    /// Looks again for the agent, in the background ("Check Again").
    pub fn recheck(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.checking {
            return;
        }
        self.checking = true;
        cx.notify();
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let found = cx.background_spawn(async move { backend.recheck() }).await;
            this.update_in(cx, |this, window, cx| {
                this.checking = false;
                let changed = this.availability.as_ref() != Some(&found);
                this.availability = Some(found.clone());
                if changed {
                    cx.emit(QuickChatEvent::AvailabilityChanged(found.clone()));
                }
                let waiting = this
                    .open
                    .as_mut()
                    .and_then(|open| open.waiting_to_send.take());
                if let Some(text) = waiting {
                    if found.is_ready() {
                        this.send_now(text, window, cx);
                    } else if let Some(open) = &this.open
                        && open.composer.read(cx).is_blank(cx)
                    {
                        // Not yet: what was typed is not lost.
                        open.composer
                            .update(cx, |composer, cx| composer.set_text(&text, cx));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Does what the button of a notice does.
    pub fn press_notice(
        &mut self,
        notice: QuickNotice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match notice {
            QuickNotice::Availability => self.recheck(window, cx),
            QuickNotice::Privacy => {
                if self.privacy_notice {
                    self.privacy_notice = false;
                    cx.emit(QuickChatEvent::PrivacyNoticeAcknowledged);
                    cx.notify();
                }
            }
            QuickNotice::Agent => {
                if let Some(open) = &mut self.open {
                    open.session.dismiss_notice();
                    cx.notify();
                }
            }
        }
    }

    /// Chooses the model and effort, as the pickers do: the open
    /// conversation continues with them, and the app is told
    /// ([`QuickChatEvent::SelectionChanged`]).
    pub fn choose(&mut self, selection: Selection, cx: &mut Context<Self>) {
        if self.selection == selection {
            return;
        }
        if let Some(open) = &mut self.open {
            open.session.set_selection(selection.clone());
        }
        self.selection = selection.clone();
        cx.emit(QuickChatEvent::SelectionChanged(selection));
        cx.notify();
    }

    /// Opens the menu of a picker under its control. From the keyboard the
    /// first item is marked.
    pub fn open_picker(
        &mut self,
        kind: PickerKind,
        from_keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = &self.open else {
            return;
        };
        let bounds = match kind {
            PickerKind::Model => open.model_bounds.clone(),
            PickerKind::Effort => open.effort_bounds.clone(),
        };
        let this = cx.weak_entity();
        open_picker_menu(
            kind,
            &open.catalog,
            &self.selection,
            &bounds,
            from_keyboard,
            move |selection, _, cx| {
                this.update(cx, |this, cx| this.choose(selection, cx)).ok();
            },
            window,
            cx,
        );
    }

    /// Takes a batch of the conversation's events into the popover.
    fn apply_events(
        &mut self,
        batch: Vec<AssistantEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = &mut self.open else {
            return;
        };
        let mut changed = false;
        for event in batch {
            changed |= open.session.apply(event);
        }
        if changed {
            self.sync(window, cx);
        }
    }

    /// Shows the conversation as it is now.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &mut self.open else {
            return;
        };
        open.measure = true;
        let messages = open.session.transcript().messages.clone();
        open.messages
            .update(cx, |list, cx| list.set_messages(&messages, window, cx));
        let busy = open.session.is_answering();
        open.composer
            .update(cx, |composer, cx| composer.set_busy(busy, cx));
        cx.notify();
    }

    // ----- applying an answer --------------------------------------------------------

    /// The buttons under an answer, for the document as it is now.
    fn refresh_actions(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.replace_state(cx) else {
            return;
        };
        let can_insert = apply::can_insert(self.editor.read(cx));
        let Some(open) = &mut self.open else {
            return;
        };
        if open.replace == Some((state, can_insert)) {
            return;
        }
        open.replace = Some((state, can_insert));
        let replace_tooltip = match state {
            ReplaceState::Ready => "Replace the selected text with this answer",
            ReplaceState::NothingSelected => "Nothing was selected when the chat was opened",
            ReplaceState::TextChanged => "The selected text has changed since the chat was opened",
            ReplaceState::ReadOnly => "This document cannot be changed",
        };
        let insert_tooltip = if can_insert {
            "Insert this answer as a new paragraph below"
        } else {
            "This document cannot be changed"
        };
        let actions = vec![
            AnswerAction::new(QuickAction::Replace.id(), QuickAction::Replace.label())
                .tooltip(replace_tooltip)
                .enabled(state == ReplaceState::Ready)
                .key_of(ReplaceSelection),
            AnswerAction::new(
                QuickAction::InsertBelow.id(),
                QuickAction::InsertBelow.label(),
            )
            .tooltip(insert_tooltip)
            .enabled(can_insert)
            .key_of(InsertBelow),
            AnswerAction::new(QuickAction::Copy.id(), QuickAction::Copy.label())
                .tooltip("Copy this answer")
                .key_of(CopyAnswer),
        ];
        open.messages
            .update(cx, |list, cx| list.set_actions(actions, cx));
    }

    /// Does `action` with the answer a key acts on: the marked answer
    /// while the keyboard is in the conversation, else the latest one.
    /// Returns whether it was done (an answer that is still being written,
    /// or a disabled action, does nothing).
    pub fn apply_answer(
        &mut self,
        action: QuickAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(open) = &self.open else {
            return false;
        };
        let list = open.messages.read(cx);
        let Some(message) = list.target_answer(window, cx).map(|message| message.id) else {
            return false;
        };
        let enabled = list
            .actions()
            .iter()
            .any(|candidate| candidate.id.as_ref() == action.id() && candidate.enabled);
        enabled && self.apply_to(action, message, window, cx)
    }

    /// Does `action` with the answer `message`. Returns whether anything
    /// was done.
    pub fn apply_to(
        &mut self,
        action: QuickAction,
        message: MessageId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(open) = &self.open else {
            return false;
        };
        let Some(text) = open
            .session
            .transcript()
            .message(message)
            .filter(|message| message.is_applicable())
            .map(|message| message.text.clone())
        else {
            return false;
        };
        let capture = open.capture.clone();
        let range = match action {
            QuickAction::Copy => {
                cx.write_to_clipboard(answer_clipboard(&text));
                show_toast(Toast::new("Copied the answer"), cx);
                cx.emit(QuickChatEvent::Copied);
                return true;
            }
            QuickAction::Replace => self.editor.update(cx, |editor, cx| {
                apply::replace_selection(editor, &capture, &text, cx)
            }),
            QuickAction::InsertBelow => self.editor.update(cx, |editor, cx| {
                apply::insert_below(editor, &capture, &text, cx)
            }),
        };
        match range {
            Some(range) => {
                // The answer is in the text and selected: the popover has
                // done its work, and the keyboard goes back to the text.
                self.close_and(false, window, cx);
                self.editor.read(cx).focus(window);
                cx.emit(QuickChatEvent::Applied { action, range });
                true
            }
            None => {
                // The document is not what the buttons were set for.
                self.refresh_actions(cx);
                cx.notify();
                false
            }
        }
    }

    /// Copies a message of the conversation (`cmd-c` in the list, "Copy"
    /// in the menu of a question).
    fn copy_message(&mut self, message: MessageId, cx: &mut Context<Self>) {
        let Some(message) = self
            .open
            .as_ref()
            .and_then(|open| open.session.transcript().message(message))
        else {
            return;
        };
        let item = match message.role {
            Role::Assistant => answer_clipboard(&message.text),
            Role::User => ClipboardItem::new_string(message.text.clone()),
        };
        cx.write_to_clipboard(item);
        show_toast(Toast::new("Copied"), cx);
        cx.emit(QuickChatEvent::Copied);
    }

    // ----- events of the parts ---------------------------------------------------------

    fn on_editor_event(
        &mut self,
        _: &Entity<Editor>,
        event: &EditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open.is_none() {
            return;
        }
        match event {
            // The text changed under the popover: "Replace Selection"
            // follows, and the popover is placed again.
            EditorEvent::Edited { .. } => {
                self.refresh_actions(cx);
                cx.notify();
            }
            // Another document: the place the chat was about is gone.
            EditorEvent::DocumentReplaced => self.close(window, cx),
            _ => {}
        }
    }

    fn on_composer_event(
        &mut self,
        composer: &Entity<Composer>,
        event: &ComposerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ComposerEvent::Submitted(text) => self.ask(text.clone(), window, cx),
            ComposerEvent::StopRequested => self.stop(cx),
            ComposerEvent::Dismissed => self.dismiss(window, cx),
            ComposerEvent::Changed => {
                // The first character of a question: the agent's program
                // is started now, so the answer does not also wait for
                // that. Nothing of the user's text is sent by this, and
                // nothing is started before the note about where the text
                // goes was read.
                let blank = composer.read(cx).is_blank(cx);
                let ready = self
                    .availability
                    .as_ref()
                    .is_none_or(Availability::is_ready);
                if let Some(open) = &mut self.open
                    && !open.prepared
                    && !blank
                    && ready
                    && !self.privacy_notice
                {
                    open.prepared = true;
                    open.session.prepare();
                }
            }
        }
    }

    fn on_list_event(
        &mut self,
        _: &Entity<MessageList>,
        event: &MessageListEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            MessageListEvent::ActionInvoked { action, message } => {
                if let Some(action) = QuickAction::from_id(action) {
                    self.apply_to(action, *message, window, cx);
                }
            }
            MessageListEvent::CopyRequested { message } => self.copy_message(*message, cx),
            MessageListEvent::RetryRequested { message } => self.retry(*message, window, cx),
            MessageListEvent::ExampleChosen(text) => {
                if let Some(open) = &self.open {
                    open.composer.update(cx, |composer, cx| {
                        composer.set_text(text.as_ref(), cx);
                        composer.focus(window, cx);
                    });
                }
            }
            MessageListEvent::Dismissed => self.dismiss(window, cx),
        }
    }

    /// `tab` and `shift-tab`: between the composer and the conversation
    /// (once something was said).
    fn cycle_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &self.open else {
            return;
        };
        let in_composer = open.composer.read(cx).is_focused(window, cx);
        if in_composer && !open.session.transcript().is_empty() {
            open.messages.update(cx, |list, cx| list.focus(window, cx));
        } else {
            open.composer.read(cx).focus(window, cx);
        }
    }

    /// A mouse button went down outside the popover: it closes. The press
    /// goes on to what is under it; when that does not take the keyboard,
    /// the text has it.
    fn pressed_outside(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A press while a context menu is open (a picker's, the
        // composer's) belongs to that menu.
        if !ContextMenu::is_open(window, cx) {
            self.close(window, cx);
        }
    }

    // ----- drawing ---------------------------------------------------------------------

    /// The text the popover is attached to, in the window: the rows of the
    /// captured selection where they are now, or the caret position.
    fn anchor(&self, editor: &Editor) -> Option<Anchor> {
        let open = self.open.as_ref()?;
        match open.capture.resolve(editor.document()) {
            Some(range) if !range.is_empty() => editor
                .range_bounds(range)
                .and_then(|rows| Anchor::of_rows(&rows))
                .or_else(|| editor.position_bounds(range.start).map(Anchor::at)),
            Some(range) => editor.position_bounds(range.start).map(Anchor::at),
            // The blocks of the capture are gone: beside the caret.
            None => editor.caret_bounds().map(Anchor::at),
        }
    }

    /// The popover's part of a frame's prepaint: places it against the
    /// editor's geometry of this frame and hands it to the window to be
    /// drawn on top.
    ///
    /// The conversation is a list that fills the height it is given, so
    /// how tall it wants to be is only known once it has been laid out.
    /// After the conversation changed, the popover is therefore laid out
    /// here first without being drawn, to see how much of the
    /// conversation is hidden, and given that much more room (up to its
    /// limit) before the frame draws it: an answer that grows by a line
    /// makes the popover a line taller in the same frame, instead of
    /// scrolling for one frame and growing in the next.
    pub(crate) fn prepaint_popover(chat: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let this = chat.read(cx);
        let drawn = this.drawn.clone();
        if this.open.is_none() {
            return drawn.set(None);
        }
        let editor = this.editor.read(cx);
        let (Some(viewport), Some(anchor)) = (editor.viewport_bounds(), this.anchor(editor)) else {
            return drawn.set(None);
        };
        let plan = place::plan(anchor, viewport);
        for _ in 0..MEASURING_PASSES {
            if !chat.update(cx, |chat, _| chat.wants_measuring(plan)) {
                break;
            }
            let mut probe = chat.update(cx, |chat, cx| chat.render_popover(plan, window, cx));
            let size = probe.layout_as_root(AvailableSpace::min_size(), window, cx);
            // Where it would be drawn: what a prepaint leaves behind in
            // the frame (hitboxes) then lies under the popover itself.
            probe.prepaint_at(place::origin(anchor, plan.side, size, viewport), window, cx);
            drop(probe);
            // At its greatest height the popover has no more room to give.
            let full = size.height >= plan.max_height - px(0.5);
            if !chat.update(cx, |chat, cx| chat.grow_to_fit(full, cx)) {
                break;
            }
        }
        let mut popover = chat.update(cx, |chat, cx| chat.render_popover(plan, window, cx));
        let size = popover.layout_as_root(AvailableSpace::min_size(), window, cx);
        let origin = place::origin(anchor, plan.side, size, viewport);
        drawn.set(Some(Drawn {
            bounds: Bounds::new(origin, size),
            side: plan.side,
            max_height: plan.max_height,
            clear: plan.clear,
            anchor,
        }));
        window.defer_draw(popover, origin, DRAW_PRIORITY);
    }

    /// Whether the conversation is to be measured before this frame draws
    /// it: it may have changed, and it could be given more room.
    fn wants_measuring(&mut self, plan: Plan) -> bool {
        let Some(open) = &mut self.open else {
            return false;
        };
        let room = (plan.width, plan.max_height);
        if open.measured_in != Some(room) {
            open.measured_in = Some(room);
            open.measure = true;
        }
        open.list_limit = plan.max_height.min(FULL_HEIGHT);
        open.measure && !open.session.transcript().is_empty() && open.list_height < open.list_limit
    }

    /// After a layout of the popover: gives the conversation the room it
    /// was short of, up to its limit. `full`: the popover is as tall as it
    /// may be, so there is no room to give. Returns whether the
    /// conversation was given any.
    fn grow_to_fit(&mut self, full: bool, cx: &App) -> bool {
        let Some(open) = &mut self.open else {
            return false;
        };
        let (_, hidden) = open.messages.read(cx).scroll_position();
        if !full && hidden > px(0.5) && open.list_height < open.list_limit {
            open.list_height = (open.list_height + hidden).min(open.list_limit);
            true
        } else {
            // It has what it needs.
            open.measure = false;
            false
        }
    }

    /// After the popover's own prepaint: the conversation that is drawn
    /// has been measured. When more of it is hidden than has to be (a
    /// change the measuring before the frame did not see, or more growth
    /// than its passes covered), the popover grows in the next frame.
    pub(crate) fn after_prepaint(chat: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let this = chat.read(cx);
        let Some(open) = &this.open else {
            return;
        };
        let full = this
            .drawn
            .get()
            .is_none_or(|drawn| drawn.bounds.size.height >= drawn.max_height - px(0.5));
        if full || open.session.transcript().is_empty() || open.list_height >= open.list_limit {
            return;
        }
        let (_, hidden) = open.messages.read(cx).scroll_position();
        if hidden > px(0.5) {
            let chat = chat.downgrade();
            // State changes wait for the end of the frame.
            window.defer(cx, move |_, cx| {
                chat.update(cx, |chat, cx| {
                    if chat.grow_to_fit(false, cx) {
                        chat.late_growths += 1;
                        cx.notify();
                    }
                })
                .ok();
            });
        }
    }

    /// How often a frame was drawn with a conversation that wanted more
    /// room than it had and got it one frame late. For tests: the
    /// measuring before a frame is there to keep this at zero.
    #[doc(hidden)]
    pub fn late_growths(&self) -> usize {
        self.late_growths
    }

    /// The popover as an element, for the room `plan` gives it.
    fn render_popover(&mut self, plan: Plan, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(open) = &self.open else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let colors = theme.colors;
        let shadow = theme.shadow_popover();
        let ui_font = theme.fonts.ui.clone();

        // A conversation that is scrolled passes under the header: a
        // hairline says where it goes.
        let scrolled = !open.session.transcript().is_empty()
            && open.messages.read(cx).scroll_position().0 > px(0.5);
        let header = div()
            .debug_selector(|| "quick-chat-header".into())
            .border_b_1()
            .border_color(if scrolled {
                colors.hairline
            } else {
                gpui::transparent_black()
            })
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap(SPACE_1)
            .h(px(32.))
            .pl(px(10.))
            .pr(px(5.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        Icon::new(IconName::Sparkle)
                            .size(px(13.))
                            .color(colors.muted),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(UI_TEXT_SM)
                            .text_color(colors.muted)
                            .child(SharedString::from(self.info.name.clone())),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(2.))
                    .child(
                        picker_button(
                            PickerKind::Model,
                            &open.catalog,
                            &self.selection,
                            &open.model_bounds,
                            false,
                            &colors,
                            cx,
                        )
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| {
                                this.open_picker(PickerKind::Model, false, window, cx)
                            },
                        )),
                    )
                    .child(
                        picker_button(
                            PickerKind::Effort,
                            &open.catalog,
                            &self.selection,
                            &open.effort_bounds,
                            false,
                            &colors,
                            cx,
                        )
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| {
                                this.open_picker(PickerKind::Effort, false, window, cx)
                            },
                        )),
                    )
                    .child(
                        icon_button(
                            "quick-chat-close",
                            "quick-chat-close",
                            IconName::Close,
                            "Close",
                            KeyHint::parse("escape"),
                            false,
                            false,
                            &colors,
                        )
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| this.close(window, cx)),
                        ),
                    ),
            );

        // The text the question is about, until the first question shows
        // it itself.
        let quote = self.quote().map(|quote| {
            div().flex_none().px(px(10.)).pb(SPACE_2).child(
                div()
                    .debug_selector(|| "quick-chat-quote".into())
                    .pl(SPACE_2)
                    .border_l_2()
                    .border_color(colors.border_strong)
                    .text_size(UI_TEXT_SM)
                    .line_height(px(17.))
                    .text_color(colors.muted)
                    .line_clamp(2)
                    .child(SharedString::from(quote)),
            )
        });

        let conversation = (!open.session.transcript().is_empty()).then(|| {
            div()
                .debug_selector(|| "quick-chat-conversation".into())
                .flex_shrink()
                .min_h_0()
                .h(open.list_height)
                .px(px(10.))
                .child(open.messages.clone())
        });

        let notices: Vec<AnyElement> = self
            .notices()
            .into_iter()
            .map(|(kind, notice)| {
                div()
                    .flex_none()
                    .px(SPACE_2)
                    .pb(SPACE_2)
                    .child(
                        notice_row(kind.selector(), &notice, false, &colors).on_action_click(
                            cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.press_notice(kind, window, cx)
                            }),
                        ),
                    )
                    .into_any_element()
            })
            .collect();

        div()
            .id("quick-chat")
            .debug_selector(|| "quick-chat".into())
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            // Over the text: a press must not reach what is under the
            // popover.
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.pressed_outside(window, cx)))
            .on_action(cx.listener(|this, _: &DismissChat, window, cx| this.dismiss(window, cx)))
            .on_action(cx.listener(|this, _: &StopAnswer, _, cx| this.stop(cx)))
            .on_action(
                cx.listener(|this, _: &FocusNextArea, window, cx| this.cycle_focus(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &FocusPreviousArea, window, cx| this.cycle_focus(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ChooseModel, window, cx| {
                this.open_picker(PickerKind::Model, true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ChooseEffort, window, cx| {
                this.open_picker(PickerKind::Effort, true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ReplaceSelection, window, cx| {
                this.apply_answer(QuickAction::Replace, window, cx);
            }))
            .on_action(cx.listener(|this, _: &InsertBelow, window, cx| {
                this.apply_answer(QuickAction::InsertBelow, window, cx);
            }))
            .on_action(cx.listener(|this, _: &CopyAnswer, window, cx| {
                this.apply_answer(QuickAction::Copy, window, cx);
            }))
            .w(plan.width)
            .max_h(plan.max_height)
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(colors.elevated)
            .border_1()
            .border_color(colors.hairline)
            .rounded(RADIUS_LG)
            .shadow(shadow)
            .font_family(ui_font)
            .text_size(UI_TEXT_BASE)
            .line_height(UI_LINE_HEIGHT)
            .text_color(colors.ink)
            .child(header)
            .children(quote)
            .children(conversation)
            .children(notices)
            .child(
                div()
                    .flex_none()
                    .px(SPACE_2)
                    .pb(SPACE_2)
                    .child(open.composer.clone()),
            )
            .into_any_element()
    }
}

impl Render for QuickChat {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        Layer::new(cx.entity())
    }
}

/// Whether the agent has the selected text already: a question that
/// carried it was sent and its answer did not fail.
fn selection_delivered(transcript: &Transcript) -> bool {
    let mut carried = false;
    for message in &transcript.messages {
        match message.role {
            Role::User => carried = message.quote.is_some(),
            Role::Assistant => {
                if carried && message.error().is_none() {
                    return true;
                }
            }
        }
    }
    false
}

/// An answer for the clipboard: its Markdown as the text other apps get,
/// and the formatted text for pasting into a document of this app.
fn answer_clipboard(markdown: &str) -> ClipboardItem {
    let text = markdown.trim().to_string();
    let fragment = apply::answer_fragment(markdown);
    if fragment.is_empty() {
        ClipboardItem::new_string(text)
    } else {
        ClipboardItem::new_string_with_metadata(text, fragment.to_json())
    }
}
