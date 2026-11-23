//! The [`AssistantPanel`] entity: the side chat. State and commands; the
//! drawing is in `render`, the picker of the context row in `context`.
//!
//! # What the panel owns and what the host does
//!
//! The panel owns the conversation with the agent: one
//! `innerword_assistant::ChatSession` for the document that is open (and one
//! for every document whose answer is still being written although another
//! document is open now), the message list, the composer, the pickers. It
//! starts the agent only when a message is sent.
//!
//! The host owns everything about the app:
//!
//! | The host... | With |
//! |---|---|
//! | says which document is open and hands over its stored conversation | [`AssistantPanel::set_document`], [`AssistantPanel::set_document_title`] |
//! | hands over the document's text when a message is sent | [`AssistantPanel::set_document_source`] |
//! | brings the document up to date before it is read for a message, when that takes a moment | [`AssistantPanel::set_before_send`] |
//! | lists the library for the picker of the context row | [`AssistantPanel::set_library_source`] |
//! | says whether there is a caret to insert at and a selection to replace | [`AssistantPanel::set_apply_state`] |
//! | says what it knows about the agent | [`AssistantPanel::set_availability`] |
//! | says whether the panel is on screen | [`AssistantPanel::set_shown`], [`AssistantPanel::end_idle_processes`] |
//! | stores the conversations and the chosen model and effort | [`AssistantPanelEvent::TranscriptChanged`], [`AssistantPanelEvent::SelectionChanged`] |
//! | carries out what the user does with an answer | [`AssistantPanelEvent::Apply`], [`AssistantPanelEvent::CopyText`] |
//! | says what an answer's drawing is for, names the elements it names, and draws it | [`DocumentRef::with_drawing`], [`AssistantPanel::set_drawing_target`], [`AssistantPanel::set_element_names`], [`AssistantPanelEvent::Draw`], [`AssistantPanel::mark_drawn`] |
//! | moves the keyboard back to the text, closes the panel | [`AssistantPanelEvent::FocusTextRequested`], [`AssistantPanelEvent::CloseRequested`] |
//! | remembers that the note about where the text goes was read | [`AssistantPanelEvent::PrivacyNoticeAcknowledged`] |
//!
//! # New chat and clear
//!
//! "New Chat" starts a new conversation: the messages go and so does what
//! was added as context. "Clear" removes the messages and keeps the
//! context. Either way the agent starts afresh with the next message (it
//! must not remember what the user no longer sees). The conversation that
//! went can be brought back until the next message is sent (the toast's
//! "Undo", [`AssistantPanel::restore_cleared`]).
//!
//! # Drawings
//!
//! An answer can hold a drawing (`innerword_assistant::answer_drawing`). The
//! host says what a drawing of the open item is for
//! (`innerword_assistant::DrawingTarget`: on the open whiteboard, as a new
//! whiteboard beside the open document, or nothing), on the
//! [`DocumentRef`] of the item, so each item's conversation starts with its
//! own prompt. The answer shows its drawing as one line
//! (`innerword_assistant::shown_markdown`), and its first button ("Draw" or
//! "New Whiteboard"), the last item of its menu, [`DrawAnswer`]'s key and
//! its command ask the host to draw it ([`AssistantPanelEvent::Draw`]).
//! The panel draws nothing itself. Insert, Replace and Copy take the
//! answer without its drawing.
//!
//! An answer drawn before ([`AssistantPanel::mark_drawn`]) says so on its
//! button ("Draw Again"); its key and its command draw it only at a second
//! press within [`DRAW_AGAIN_WITHIN`], the first press saying so.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    AnyWindowHandle, App, AppContext, Context, DismissEvent, Entity, EventEmitter, FocusHandle,
    Focusable, Global, SharedString, Subscription, Task, WeakEntity, WeakFocusHandle, Window,
};
use innerword_assistant::{
    Attachment, AttachmentKind, Availability, Backend, BackendInfo, Catalog, ChatSession, Drawing,
    DrawingProblem, DrawingTarget, MessageId, Mode, OpenDocument, Role, Selection, ToolServer,
    Transcript, answer_drawing, drawing_blocks, drawing_fits, without_drawings,
};
use innerword_ui::{Toast, toast::show_toast};

use crate::{
    ChatKeys,
    actions::{CopyAnswer, DrawAnswer, InsertAtCaret, ReplaceSelection, SIDE_EDITOR_FLAG},
    composer::{Composer, ComposerEvent, ComposerOptions},
    markdown::AnswerStyle,
    messages::{
        ActionState, ActionWords, AnswerAction, AnswerFacts, EmptyState, MessageList,
        MessageListEvent, MessageListOptions, SharedElementNames,
    },
    notice::Notice,
    pickers::{ControlBounds, PickerKind, open_picker_menu, picker_has_choices},
};

pub mod context;
mod render;

pub(crate) use render::global_actions;

use context::{ContextPicker, ContextPickerDelegate};

/// The name of the "Insert at Caret" action of an answer.
pub const ACTION_INSERT: &str = "insert";
/// The name of the "Replace Selection" action of an answer.
pub const ACTION_REPLACE: &str = "replace";
/// The name of the "Copy" action of an answer.
pub const ACTION_COPY: &str = "copy";
/// The name of the drawing action of an answer ("Draw", "New
/// Whiteboard").
pub const ACTION_DRAW: &str = "draw";

/// How soon a second press of the key or the command draws an answer that
/// was drawn before.
pub const DRAW_AGAIN_WITHIN: Duration = Duration::from_secs(5);

/// What the key and the command of a drawing say where no drawing can be
/// applied, when the host gave no reason of its own.
pub const DRAWING_UNAVAILABLE: &str = "Open a whiteboard or a document to draw the answer.";
/// What the key and the command say for an answer without a drawing.
pub const NO_DRAWING: &str = "The answer has no drawing.";
/// What they say for an answer that is still being written.
pub const STILL_WRITTEN: &str = "The answer is still being written.";
/// What they say at the first press for an answer drawn on a whiteboard
/// before.
pub const DRAWN_ALREADY: &str = "This answer was drawn already. Press again to draw it once more.";
/// What they say at the first press for an answer made into a whiteboard
/// before.
pub const MADE_ALREADY: &str =
    "This answer was made into a whiteboard already. Press again to make another one.";
/// Why Insert, Replace and Copy are dimmed under an answer that is a
/// drawing and nothing else.
pub const ONLY_A_DRAWING: &str = "The answer has nothing but a drawing.";

/// The document that is open beside the side chat: whose conversation is
/// shown, and what its chip says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentRef {
    /// Its id in the library. Each id has a conversation of its own; a
    /// document without one (`None`) shares one.
    pub id: Option<i64>,
    /// Its title, for the chip.
    pub title: String,
    /// What drawings the item offers (`None` unless told).
    pub drawing_target: DrawingTarget,
    /// With `DrawingTarget::None`: what the key and the command say (else
    /// [`DRAWING_UNAVAILABLE`]), e.g. why a whiteboard cannot be drawn on
    /// now.
    pub drawing_unavailable: Option<SharedString>,
    /// The empty state's line about where the item came from ("Drawn from
    /// an answer about “Chapter 2”."), shown until its first message.
    pub origin_note: Option<SharedString>,
}

impl DocumentRef {
    /// A document of the library.
    pub fn new(id: i64, title: impl Into<String>) -> Self {
        Self {
            id: Some(id),
            title: title.into(),
            drawing_target: DrawingTarget::None,
            drawing_unavailable: None,
            origin_note: None,
        }
    }

    /// The item offers the drawings of `target` (`why_not` with
    /// `DrawingTarget::None`: what the key and the command say).
    pub fn with_drawing(mut self, target: DrawingTarget, why_not: Option<SharedString>) -> Self {
        self.drawing_target = target;
        self.drawing_unavailable = why_not;
        self
    }

    /// The empty state names where the item came from.
    pub fn with_origin_note(mut self, note: impl Into<SharedString>) -> Self {
        self.origin_note = Some(note.into());
        self
    }
}

/// What the user wants done with an answer. The host carries it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ApplyKind {
    /// Insert the answer at the caret of the document.
    InsertAtCaret,
    /// Replace the selection of the document with the answer.
    ReplaceSelection,
    /// Put the answer on the clipboard.
    Copy,
}

impl ApplyKind {
    fn from_action(action: &str) -> Option<Self> {
        match action {
            ACTION_INSERT => Some(ApplyKind::InsertAtCaret),
            ACTION_REPLACE => Some(ApplyKind::ReplaceSelection),
            ACTION_COPY => Some(ApplyKind::Copy),
            _ => None,
        }
    }
}

/// What an [`AssistantPanel`] tells its host.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum AssistantPanelEvent {
    /// A button under an answer, its key or its menu item: do this with
    /// the answer. `markdown` is the answer as the assistant wrote it.
    Apply {
        /// What to do.
        kind: ApplyKind,
        /// The answer.
        message: MessageId,
        /// Its text, as Markdown.
        markdown: String,
    },
    /// Copy this text (a message of the user's own).
    CopyText(String),
    /// The conversation about a document changed (a message was sent, an
    /// answer ended, it was cleared, something was added to or removed
    /// from its context): store it. Also sent for a document that is not
    /// the open one any more, when an answer it was waiting for ended.
    TranscriptChanged {
        /// The document the conversation is about.
        document: Option<i64>,
        /// The conversation as it is now.
        transcript: Transcript,
    },
    /// The user chose a model or an effort level: what the side chat asks
    /// with from now on, to be remembered.
    SelectionChanged(Selection),
    /// The keyboard should go back to the text (`escape`).
    FocusTextRequested,
    /// The close button.
    CloseRequested,
    /// The note about where the text goes was read ("OK", or the first
    /// message was sent): do not show it again.
    PrivacyNoticeAcknowledged,
    /// The panel looked again for the agent ("Check Again") and found
    /// this.
    AvailabilityChanged(Availability),
    /// The drawing of an answer is to be applied: on the open whiteboard
    /// (`DrawingTarget::OpenWhiteboard`), or as a new whiteboard
    /// (`NewWhiteboard`). The button, its menu item, [`DrawAnswer`]'s key
    /// or its command ([`AssistantPanel::draw`]).
    Draw {
        /// The answer.
        message: MessageId,
        /// What the drawing is for.
        target: DrawingTarget,
        /// The drawing, read from the answer's blocks.
        drawing: Drawing,
    },
}

/// What an [`AssistantPanel`] is made with.
#[derive(Clone, Debug, Default)]
pub struct AssistantPanelOptions {
    /// The tools of the library (`innerword_assistant::library_tools`), or
    /// `None` when the `innerword` tool is not there: the chat then works on
    /// the open document alone.
    pub tools: Option<ToolServer>,
    /// The model and effort the side chat was last used with. `None`: the
    /// engine's default for the side chat.
    pub selection: Option<Selection>,
    /// Show the note about where the text goes until it is acknowledged.
    pub privacy_notice: bool,
    /// What the host already knows about the agent. `None`: what the
    /// backend has cached, or nothing yet.
    pub availability: Option<Availability>,
}

/// The parts of the panel the keyboard can be in, in the order `tab`
/// walks them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanelArea {
    /// The controls of the header: `left` / `right` move, `enter` presses.
    Header,
    /// The context chips and "Add": `left` / `right` move, `enter`
    /// presses, `delete` removes a chip.
    Context,
    /// The message list.
    Messages,
    /// The composer.
    Composer,
}

/// The controls of the header, in the order the arrows walk them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HeaderControl {
    /// The model picker.
    Model,
    /// The effort picker.
    Effort,
    /// New Chat.
    NewChat,
    /// Clear.
    Clear,
    /// Close.
    Close,
}

impl HeaderControl {
    /// Every control, in order.
    pub const ALL: [HeaderControl; 5] = [
        HeaderControl::Model,
        HeaderControl::Effort,
        HeaderControl::NewChat,
        HeaderControl::Clear,
        HeaderControl::Close,
    ];
}

/// Hands over the open document as it is now.
type DocumentSource = Box<dyn Fn(&App) -> Option<OpenDocument>>;
/// Lists the documents and folders of the library.
type LibrarySource = Box<dyn Fn(&App) -> Vec<Attachment>>;

/// What the host does before the open document is read for a message
/// ([`AssistantPanel::set_before_send`]).
type BeforeSend = Box<dyn Fn(&mut Window, &mut App) -> Option<Task<()>>>;

/// Messages that wait for the host's task of [`AssistantPanel::set_before_send`]
/// before the document is read and they are sent, in the order they were sent.
struct HeldMessages {
    texts: Vec<String>,
    _wait: Task<()>,
}

/// One conversation and the agent behind it.
struct Chat {
    /// Names the chat for the task that reads its events.
    key: u64,
    document: Option<i64>,
    session: ChatSession,
    /// Whether the agent's program was started ahead of the first message.
    prepared: bool,
    _pump: Task<()>,
}

/// A conversation that was cleared, kept until the next change so that
/// "Undo" can bring it back.
struct Cleared {
    document: Option<i64>,
    transcript: Transcript,
}

/// The panels of the open windows, for the actions that reach a panel
/// from wherever the keyboard is.
#[derive(Default)]
struct WindowPanels(Vec<(AnyWindowHandle, WeakEntity<AssistantPanel>)>);

impl Global for WindowPanels {}

/// The side chat.
pub struct AssistantPanel {
    backend: Arc<dyn Backend>,
    info: BackendInfo,
    availability: Option<Availability>,
    checking: bool,
    tools: Option<ToolServer>,
    selection: Selection,
    chat: Chat,
    /// Conversations of documents that are not open any more, whose answer
    /// is still being written.
    parked: Vec<Chat>,
    next_chat_key: u64,
    document: Option<DocumentRef>,
    document_source: Option<DocumentSource>,
    library_source: Option<LibrarySource>,
    before_send: Option<BeforeSend>,
    /// Messages sent while the host's task before sending runs.
    held: Option<HeldMessages>,
    messages: Entity<MessageList>,
    composer: Entity<Composer>,
    privacy_notice: bool,
    can_insert: bool,
    can_replace: bool,
    shown: bool,
    cleared: Option<Cleared>,
    /// A message that waits for the answer of "is the user signed in?".
    waiting_to_send: Option<String>,
    header_focus: FocusHandle,
    header_control: HeaderControl,
    context_focus: FocusHandle,
    context_index: usize,
    picker: Option<Entity<ContextPicker>>,
    picker_return: Option<WeakFocusHandle>,
    pub(crate) model_bounds: ControlBounds,
    pub(crate) effort_bounds: ControlBounds,
    pub(crate) add_bounds: ControlBounds,
    /// The names of the open whiteboard's elements, from the host.
    element_names: Option<SharedElementNames>,
    /// The first press of the key or the command on an answer drawn
    /// before: the chat, the answer, when.
    draw_again: Option<(u64, MessageId, Instant)>,
    _subscriptions: Vec<Subscription>,
    _picker_subscription: Option<Subscription>,
}

impl EventEmitter<AssistantPanelEvent> for AssistantPanel {}

impl Focusable for AssistantPanel {
    /// The composer: the side chat is entered to write a message.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.composer.focus_handle(cx)
    }
}

impl AssistantPanel {
    /// A side chat on `backend`, with an empty conversation and no open
    /// document. Nothing is started and nothing is sent until the first
    /// message.
    pub fn new(
        backend: Arc<dyn Backend>,
        options: AssistantPanelOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let info = backend.info();
        let catalog = backend.catalog();
        let selection = catalog.normalized(
            &options
                .selection
                .unwrap_or_else(|| catalog.default_for(Mode::Side)),
        );
        let availability = options
            .availability
            .or_else(|| backend.cached_availability());
        let keys = ChatKeys::new(SIDE_EDITOR_FLAG);
        let messages = cx.new(|cx| {
            MessageList::new(
                MessageListOptions::new(info.clone())
                    .style(AnswerStyle::default())
                    .keys(keys.clone())
                    .drawings(true),
                window,
                cx,
            )
        });
        let composer = cx.new(|cx| {
            Composer::new(
                ComposerOptions::new("Ask about this document").keys(keys),
                window,
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe_in(&composer, window, Self::composer_event),
            cx.subscribe_in(&messages, window, Self::messages_event),
        ];
        let handle = window.window_handle();
        let this = cx.weak_entity();
        let panels = cx.default_global::<WindowPanels>();
        panels.0.retain(|(_, panel)| panel.upgrade().is_some());
        panels.0.push((handle, this));

        let chat = Self::new_chat_for(
            &backend,
            &options.tools,
            &selection,
            None,
            DrawingTarget::None,
            Transcript::new(),
            0,
            window,
            cx,
        );
        let mut panel = Self {
            backend,
            info,
            availability,
            checking: false,
            tools: options.tools,
            selection,
            chat,
            parked: Vec::new(),
            next_chat_key: 1,
            document: None,
            document_source: None,
            library_source: None,
            before_send: None,
            held: None,
            messages,
            composer,
            privacy_notice: options.privacy_notice,
            can_insert: true,
            can_replace: false,
            shown: true,
            cleared: None,
            waiting_to_send: None,
            header_focus: cx.focus_handle(),
            header_control: HeaderControl::Model,
            context_focus: cx.focus_handle(),
            context_index: 0,
            picker: None,
            picker_return: None,
            model_bounds: ControlBounds::new(),
            effort_bounds: ControlBounds::new(),
            add_bounds: ControlBounds::new(),
            element_names: None,
            draw_again: None,
            _subscriptions: subscriptions,
            _picker_subscription: None,
        };
        panel.sync_actions(cx);
        panel.sync(window, cx);
        panel
    }

    #[allow(clippy::too_many_arguments)]
    fn new_chat_for(
        backend: &Arc<dyn Backend>,
        tools: &Option<ToolServer>,
        selection: &Selection,
        document: Option<i64>,
        target: DrawingTarget,
        transcript: Transcript,
        key: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Chat {
        let session = ChatSession::side(backend.clone(), tools.clone())
            .with_selection(selection.clone())
            .with_drawing_target(target)
            .with_transcript(transcript);
        let events = session.events();
        let pump = cx.spawn_in(window, async move |this, cx| {
            while let Some(batch) = events.next_batch().await {
                let alive = this.update_in(cx, |this, window, cx| {
                    this.apply_events(key, batch, window, cx);
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        Chat {
            key,
            document,
            session,
            prepared: false,
            _pump: pump,
        }
    }

    // ----- reading -------------------------------------------------------------

    /// The agent the chat runs on.
    pub fn backend(&self) -> &Arc<dyn Backend> {
        &self.backend
    }

    /// What the agent is called.
    pub fn backend_info(&self) -> &BackendInfo {
        &self.info
    }

    /// The models and effort levels the pickers offer.
    pub fn catalog(&self) -> Catalog {
        self.backend.catalog()
    }

    /// The document whose conversation is shown.
    pub fn document(&self) -> Option<&DocumentRef> {
        self.document.as_ref()
    }

    /// The conversation that is shown.
    pub fn transcript(&self) -> &Transcript {
        self.chat.session.transcript()
    }

    /// Whether an answer of the shown conversation is being written.
    pub fn is_answering(&self) -> bool {
        self.chat.session.is_answering()
    }

    /// Whether a message was sent and waits for the host's task before
    /// sending ([`AssistantPanel::set_before_send`]): it is not in the
    /// conversation yet.
    pub fn is_holding_a_message(&self) -> bool {
        self.held.is_some()
    }

    /// The documents whose conversation is still being answered although
    /// another document is open.
    pub fn answering_elsewhere(&self) -> Vec<Option<i64>> {
        self.parked.iter().map(|chat| chat.document).collect()
    }

    /// The model and effort the next message is asked with.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// The library items attached to the shown conversation.
    pub fn attachments(&self) -> &[Attachment] {
        self.chat.session.attachments()
    }

    /// What is known about the agent on this Mac.
    pub fn availability(&self) -> Option<&Availability> {
        self.availability.as_ref()
    }

    /// Whether the note about where the text goes is showing.
    pub fn shows_privacy_notice(&self) -> bool {
        self.privacy_notice
    }

    /// The message list.
    pub fn messages(&self) -> &Entity<MessageList> {
        &self.messages
    }

    /// The composer.
    pub fn composer(&self) -> &Entity<Composer> {
        &self.composer
    }

    /// The open picker of the context row.
    pub fn context_picker(&self) -> Option<&Entity<ContextPicker>> {
        self.picker.as_ref()
    }

    /// The notices above the composer, most pressing first: the agent is
    /// missing or nobody is signed in, the note about where the text
    /// goes, what the agent itself wants the user to know.
    pub fn notices(&self) -> Vec<(PanelNotice, Notice)> {
        let mut notices = Vec::new();
        match &self.availability {
            Some(Availability::NotInstalled) => {
                notices.push((PanelNotice::Availability, Notice::not_installed(&self.info)))
            }
            Some(availability) if availability.is_signed_out() => {
                notices.push((PanelNotice::Availability, Notice::signed_out(&self.info)))
            }
            _ => {}
        }
        if self.privacy_notice && notices.is_empty() {
            notices.push((
                PanelNotice::Privacy,
                Notice::privacy(&self.info, Mode::Side),
            ));
        }
        if let Some(text) = self.chat.session.notice() {
            notices.push((
                PanelNotice::Agent,
                Notice::info(text.to_string(), "").action("Dismiss"),
            ));
        }
        notices
    }

    /// One line about the panel for scripted runs: the document, the
    /// messages, whether an answer is being written, the model and effort,
    /// what is attached, the notices, where the keyboard is.
    pub fn debug_state(&self, window: &Window, cx: &App) -> String {
        let transcript = self.transcript();
        let last = transcript
            .messages
            .last()
            .map(|message| format!("{:?}", message.state))
            .unwrap_or_else(|| "none".into());
        let last = last.split(['(', ' ']).next().unwrap_or("").to_lowercase();
        let drawn = transcript
            .messages
            .iter()
            .rev()
            .find(|message| message.role == Role::Assistant)
            .is_some_and(|message| message.drawn_at.is_some());
        format!(
            "document={} messages={} answering={} last={} model={} effort={} attached={} notices=[{}] focus={} draw={} drawn={}",
            self.document
                .as_ref()
                .map(|document| format!("{:?}", document.title))
                .unwrap_or_else(|| "none".into()),
            transcript.len(),
            self.is_answering(),
            last,
            self.selection.model.id().unwrap_or("default"),
            self.selection.effort.id().unwrap_or("default"),
            self.attachments().len(),
            self.notices()
                .iter()
                .map(|(kind, _)| format!("{kind:?}").to_lowercase())
                .collect::<Vec<_>>()
                .join(","),
            match self.focused_area(window, cx) {
                Some(PanelArea::Header) => "header",
                Some(PanelArea::Context) => "context",
                Some(PanelArea::Messages) => "messages",
                Some(PanelArea::Composer) => "composer",
                None if self.picker.is_some() => "picker",
                None => "elsewhere",
            },
            match self.drawing_target() {
                DrawingTarget::OpenWhiteboard => "whiteboard",
                DrawingTarget::NewWhiteboard => "new",
                DrawingTarget::None => "none",
            },
            if drawn { "yes" } else { "no" },
        )
    }

    // ----- what the host tells the panel -----------------------------------------

    /// Says which document is open, and hands over the conversation that
    /// is stored for it.
    ///
    /// Another document than before: its conversation is shown. That is the
    /// one the panel still holds when an answer was being written while
    /// the document was away, else `stored` (or an empty one). The
    /// conversation of the document that was open is not lost: every change
    /// of it was reported ([`AssistantPanelEvent::TranscriptChanged`]), and
    /// an answer that is still being written goes on and is reported when
    /// it ends.
    ///
    /// The same document as before: only its title is taken (`stored` is
    /// ignored: the panel's own copy is newer).
    pub fn set_document(
        &mut self,
        document: Option<DocumentRef>,
        stored: Option<Transcript>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = document.as_ref().and_then(|document| document.id);
        let target = document
            .as_ref()
            .map_or(DrawingTarget::None, |document| document.drawing_target);
        let same = self.chat.document == key && self.document.is_some() == document.is_some();
        self.document = document;
        if same {
            self.drawing_target_changed(target, cx);
            self.sync_empty_state(cx);
            cx.notify();
            return;
        }
        let chat = match self.parked.iter().position(|chat| chat.document == key) {
            Some(index) if key.is_some() => {
                let mut chat = self.parked.remove(index);
                chat.session.set_drawing_target(target);
                chat
            }
            _ => {
                let chat_key = self.next_chat_key;
                self.next_chat_key += 1;
                Self::new_chat_for(
                    &self.backend,
                    &self.tools,
                    &self.selection,
                    key,
                    target,
                    stored.unwrap_or_default(),
                    chat_key,
                    window,
                    cx,
                )
            }
        };
        let previous = std::mem::replace(&mut self.chat, chat);
        if previous.session.is_answering() && previous.document.is_some() {
            // Its answer goes on; it is reported when it ends.
            self.parked.push(previous);
        }
        // (Otherwise the previous chat is dropped here, and its agent's
        // program ends with it.)
        self.cleared = None;
        self.waiting_to_send = None;
        self.draw_again = None;
        // A message that waited for the host was about the other document:
        // it is not sent, and what was typed goes back into the composer.
        self.give_back_held(cx);
        self.close_context_picker(window, cx);
        self.context_index = 0;
        // Another conversation: its messages have the same ids.
        self.messages.update(cx, |list, cx| list.clear(cx));
        self.drawing_target_changed(target, cx);
        self.sync(window, cx);
    }

    /// A change of what the open item offers while the same item stays
    /// open (a page engine found or gone, the whiteboard drawable or not).
    /// Sets the shown chat's session (from its next message on), the
    /// drawing action's label, tooltip and state, and the empty state.
    pub fn set_drawing_target(
        &mut self,
        target: DrawingTarget,
        why_not: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = &mut self.document else {
            return;
        };
        if document.drawing_target == target && document.drawing_unavailable == why_not {
            return;
        }
        document.drawing_target = target;
        document.drawing_unavailable = why_not;
        self.drawing_target_changed(target, cx);
        self.sync_empty_state(cx);
        cx.notify();
    }

    /// What drawings the open item offers, as the host said.
    pub fn drawing_target(&self) -> DrawingTarget {
        self.document
            .as_ref()
            .map_or(DrawingTarget::None, |document| document.drawing_target)
    }

    /// The names of the open whiteboard's elements for the line of a
    /// drawing (`None` over a document, or when the host has none). The
    /// lines already shown are made again; the host sets it when the scene
    /// it holds of the open whiteboard changed.
    pub fn set_element_names(&mut self, names: Option<SharedElementNames>, cx: &mut Context<Self>) {
        self.element_names = names;
        let target = self.drawing_target();
        let names = self.element_names.clone();
        self.messages
            .update(cx, |list, cx| list.set_drawings(target, names, cx));
    }

    /// The shown chat, the list and the actions follow `target`.
    fn drawing_target_changed(&mut self, target: DrawingTarget, cx: &mut Context<Self>) {
        self.chat.session.set_drawing_target(target);
        let names = self.element_names.clone();
        self.messages
            .update(cx, |list, cx| list.set_drawings(target, names, cx));
        self.sync_actions(cx);
    }

    /// Changes the title on the chip of the open document.
    pub fn set_document_title(&mut self, title: impl Into<String>, cx: &mut Context<Self>) {
        let title = title.into();
        if let Some(document) = &mut self.document
            && document.title != title
        {
            document.title = title;
            cx.notify();
        }
    }

    /// What the host does before the open document is read for a message,
    /// when it needs a moment to bring it up to date (what the source
    /// reads is held somewhere that has not handed over everything yet).
    ///
    /// `hook` is called as a message is sent. `None`: nothing to wait for,
    /// the message goes at once. A task: the message waits for it, and is
    /// sent (the document read from the source then) when the task ends.
    /// The panel sets no limit; the task must end, and the host decides how
    /// long it may take. Meanwhile the message is not in the conversation,
    /// the composer offers Stop (which puts the text back), and more
    /// messages wait behind it in order. Another document opened meanwhile
    /// puts the text back into the composer too: it was about the other.
    pub fn set_before_send(
        &mut self,
        hook: impl Fn(&mut Window, &mut App) -> Option<Task<()>> + 'static,
    ) {
        self.before_send = Some(Box::new(hook));
    }

    /// Where the panel gets the open document from when a message is
    /// sent: its text as Markdown as it is at that moment, with its title
    /// and library path. `None`: there is no open document, the question
    /// goes alone.
    pub fn set_document_source(&mut self, source: impl Fn(&App) -> Option<OpenDocument> + 'static) {
        self.document_source = Some(Box::new(source));
    }

    /// Where the picker of the context row gets the documents and folders
    /// of the library from (asked each time the picker opens).
    pub fn set_library_source(&mut self, source: impl Fn(&App) -> Vec<Attachment> + 'static) {
        self.library_source = Some(Box::new(source));
    }

    /// Says what can be done with an answer now: whether there is a caret
    /// to insert at (a document is open and can be changed) and whether
    /// there is a selection to replace.
    pub fn set_apply_state(&mut self, can_insert: bool, can_replace: bool, cx: &mut Context<Self>) {
        if self.can_insert != can_insert || self.can_replace != can_replace {
            self.can_insert = can_insert;
            self.can_replace = can_replace;
            self.sync_actions(cx);
        }
    }

    /// Says what is known about the agent (the host looks at launch and
    /// when the settings open).
    pub fn set_availability(&mut self, availability: Availability, cx: &mut Context<Self>) {
        if self.availability.as_ref() != Some(&availability) {
            self.availability = Some(availability);
            cx.notify();
        }
    }

    /// Gives the chat other tools (or none), from the next message on.
    pub fn set_tools(&mut self, tools: Option<ToolServer>, cx: &mut Context<Self>) {
        self.chat.session.set_tools(tools.clone());
        self.tools = tools;
        cx.notify();
    }

    /// Shows or hides the note about where the text goes.
    pub fn set_privacy_notice(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.privacy_notice != show {
            self.privacy_notice = show;
            cx.notify();
        }
    }

    /// Says whether the panel is on screen. While it is not, the actions
    /// that reach a panel from anywhere in the window ([`crate::init`])
    /// leave it alone.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.shown != shown {
            self.shown = shown;
            cx.notify();
        }
    }

    /// Whether the panel is on screen, as the host said.
    pub fn is_shown(&self) -> bool {
        self.shown
    }

    /// Starts the agent's program now, without sending it anything, so
    /// that the first answer does not also wait for the program to start
    /// (about a second). The panel never does this by itself: a process
    /// is started when a message is sent, not before. A host that wants
    /// the first answer sooner calls this when the user is clearly about
    /// to ask (and never while the note about where the text goes is
    /// still unread: nothing happens then).
    pub fn prepare(&mut self, _: &mut Context<Self>) {
        let ready = self
            .availability
            .as_ref()
            .is_some_and(Availability::is_ready);
        if ready && !self.privacy_notice && !self.chat.prepared {
            self.chat.prepared = true;
            self.chat.session.prepare();
        }
    }

    /// Ends the agent's programs of conversations that are not being
    /// answered (the panel was closed). The conversations stay; the next
    /// message starts the agent again and continues.
    pub fn end_idle_processes(&mut self, cx: &mut Context<Self>) {
        if !self.chat.session.is_answering() {
            self.chat.session.close();
            self.chat.prepared = false;
        }
        cx.notify();
    }

    // ----- the conversation --------------------------------------------------------

    /// Sends a message with the open document as it is now and what is
    /// attached. With the agent known to be signed out (or missing) the
    /// panel looks again first, and sends when the user is signed in by
    /// then; otherwise the message is put back into the composer.
    pub fn send(&mut self, text: impl Into<String>, window: &mut Window, cx: &mut Context<Self>) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }
        let ready = self
            .availability
            .as_ref()
            .is_none_or(Availability::is_ready);
        if !ready {
            self.waiting_to_send = Some(text);
            self.recheck(window, cx);
            return;
        }
        self.send_now(text, window, cx);
    }

    fn send_now(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        // Behind a message that waits for the host, in order.
        if let Some(held) = &mut self.held {
            held.texts.push(text);
            return;
        }
        let task = self.before_send.as_ref().and_then(|hook| hook(window, cx));
        if let Some(task) = task {
            let wait = cx.spawn_in(window, async move |this, cx| {
                task.await;
                this.update_in(cx, |this, window, cx| this.send_held(window, cx))
                    .ok();
            });
            self.held = Some(HeldMessages {
                texts: vec![text],
                _wait: wait,
            });
            self.sync(window, cx);
            return;
        }
        self.ask(text, window, cx);
    }

    /// The host's task before sending ended: what waited is sent now.
    fn send_held(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(held) = self.held.take() else {
            return;
        };
        for text in held.texts {
            self.ask(text, window, cx);
        }
        self.sync(window, cx);
    }

    /// Messages that waited for the host are not sent: their text goes back
    /// into the composer when it is empty.
    fn give_back_held(&mut self, cx: &mut Context<Self>) {
        let Some(held) = self.held.take() else {
            return;
        };
        if self.composer.read(cx).is_blank(cx) {
            let text = held.texts.join("\n\n");
            self.composer
                .update(cx, |composer, cx| composer.set_text(&text, cx));
        }
    }

    /// Asks the agent now, with the open document as the source has it.
    fn ask(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.privacy_notice {
            self.privacy_notice = false;
            cx.emit(AssistantPanelEvent::PrivacyNoticeAcknowledged);
        }
        self.cleared = None;
        self.chat.session.dismiss_notice();
        let document = self.document_source.as_ref().and_then(|source| source(cx));
        self.chat.session.ask_side(&text, document.as_ref());
        self.chat.prepared = true;
        self.messages
            .update(cx, |list, cx| list.scroll_to_bottom(cx));
        self.sync(window, cx);
        self.report_transcript(cx);
    }

    /// Asks the question of a failed answer again.
    pub fn retry(&mut self, answer: MessageId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(question) = self
            .transcript()
            .question_of(answer)
            .map(|question| question.text.clone())
        else {
            return;
        };
        self.send(question, window, cx);
    }

    /// Stops the answer that is being written; what had arrived stays.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if self.held.is_some() {
            // A message that waits for the host is not sent.
            self.give_back_held(cx);
            let busy = self.chat.session.is_answering();
            self.composer
                .update(cx, |composer, cx| composer.set_busy(busy, cx));
        }
        self.chat.session.stop();
        cx.notify();
    }

    /// A new conversation: the messages go, and so does what was added as
    /// context; the open document stays. "Undo" in the toast (or
    /// [`AssistantPanel::restore_cleared`]) brings it back until the next
    /// message.
    pub fn new_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.forget_conversation(true, window, cx);
    }

    /// Removes the messages; what was added as context stays.
    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.forget_conversation(false, window, cx);
    }

    fn forget_conversation(
        &mut self,
        attachments_too: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.transcript().clone();
        let nothing_to_forget =
            before.is_empty() && (!attachments_too || before.attachments.is_empty());
        if attachments_too {
            self.chat.session.start_new();
        } else {
            self.chat.session.clear();
        }
        self.chat.prepared = false;
        self.context_index = 0;
        self.messages.update(cx, |list, cx| list.clear(cx));
        self.sync(window, cx);
        if nothing_to_forget {
            return;
        }
        self.cleared = Some(Cleared {
            document: self.chat.document,
            transcript: before,
        });
        self.report_transcript(cx);
        let panel = cx.weak_entity();
        show_toast(
            Toast::new(if attachments_too {
                "A new chat was started"
            } else {
                "The chat was cleared"
            })
            .on_action("Undo", move |window, cx| {
                panel
                    .update(cx, |panel, cx| {
                        panel.restore_cleared(window, cx);
                    })
                    .ok();
            }),
            cx,
        );
    }

    /// Whether a cleared conversation can still be brought back.
    pub fn can_restore_cleared(&self) -> bool {
        self.cleared
            .as_ref()
            .is_some_and(|cleared| cleared.document == self.chat.document)
            && self.transcript().is_empty()
    }

    /// Brings back the conversation that was cleared last, while nothing
    /// was said since and the same document is open. Returns whether it
    /// did.
    pub fn restore_cleared(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.can_restore_cleared() {
            return false;
        }
        let Some(cleared) = self.cleared.take() else {
            return false;
        };
        self.chat.session.replace_transcript(cleared.transcript);
        self.messages.update(cx, |list, cx| list.clear(cx));
        self.sync(window, cx);
        self.report_transcript(cx);
        true
    }

    /// Chooses the model and effort for the messages from now on.
    pub fn set_selection(&mut self, selection: Selection, cx: &mut Context<Self>) {
        let selection = self.catalog().normalized(&selection);
        if self.selection != selection {
            self.selection = selection.clone();
            self.chat.session.set_selection(selection.clone());
            cx.emit(AssistantPanelEvent::SelectionChanged(selection));
            cx.notify();
        }
    }

    /// Attaches a library item to the conversation. Returns `false` when
    /// it is attached already, is the open document itself, or the
    /// conversation has as many as it takes.
    pub fn attach(&mut self, attachment: Attachment, cx: &mut Context<Self>) -> bool {
        if self.is_open_document(&attachment) {
            return false;
        }
        let attached = self.chat.session.attach(attachment);
        if attached {
            self.report_transcript(cx);
        }
        cx.notify();
        attached
    }

    /// Removes an attached item. Returns whether it was attached.
    pub fn detach(&mut self, kind: AttachmentKind, id: i64, cx: &mut Context<Self>) -> bool {
        let detached = self.chat.session.detach(kind, id);
        if detached {
            self.context_index = self.context_index.min(self.attachments().len());
            self.report_transcript(cx);
        }
        cx.notify();
        detached
    }

    fn is_open_document(&self, attachment: &Attachment) -> bool {
        attachment.kind == AttachmentKind::Document
            && self
                .document
                .as_ref()
                .is_some_and(|document| document.id == Some(attachment.id))
    }

    /// Looks again for the agent, in the background ("Check Again"). The
    /// answer is taken ([`AssistantPanelEvent::AvailabilityChanged`]), and
    /// a message that was waiting for it is sent when the user is signed
    /// in now.
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
                    cx.emit(AssistantPanelEvent::AvailabilityChanged(found.clone()));
                }
                if let Some(text) = this.waiting_to_send.take() {
                    if found.is_ready() {
                        this.send_now(text, window, cx);
                    } else if this.composer.read(cx).is_blank(cx) {
                        // Not yet: what was typed is not lost.
                        this.composer
                            .update(cx, |composer, cx| composer.set_text(&text, cx));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Whether the panel is looking for the agent right now.
    pub fn is_checking(&self) -> bool {
        self.checking
    }

    /// Does what the button of a notice does.
    pub fn press_notice(
        &mut self,
        notice: PanelNotice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match notice {
            PanelNotice::Availability => self.recheck(window, cx),
            PanelNotice::Privacy => {
                if self.privacy_notice {
                    self.privacy_notice = false;
                    cx.emit(AssistantPanelEvent::PrivacyNoticeAcknowledged);
                    cx.notify();
                }
            }
            PanelNotice::Agent => {
                self.chat.session.dismiss_notice();
                cx.notify();
            }
        }
    }

    // ----- applying an answer --------------------------------------------------------

    /// Tells the host to do `kind` with an answer. Nothing happens for an
    /// answer that cannot be applied (it is still being written, it is
    /// empty) or when the host said `kind` cannot be done now. Returns
    /// whether the host was told.
    pub fn apply(&mut self, kind: ApplyKind, message: MessageId, cx: &mut Context<Self>) -> bool {
        let allowed = match kind {
            ApplyKind::InsertAtCaret => self.can_insert,
            ApplyKind::ReplaceSelection => self.can_replace,
            ApplyKind::Copy => true,
        };
        let Some(markdown) = self
            .transcript()
            .message(message)
            .filter(|message| message.is_applicable())
            .map(|message| message.text.clone())
        else {
            return false;
        };
        // What goes into the text or onto the clipboard is the answer
        // without its drawing.
        let markdown = if drawing_blocks(&markdown).is_empty() {
            markdown
        } else {
            without_drawings(&markdown)
        };
        if !allowed || markdown.trim().is_empty() {
            return false;
        }
        cx.emit(AssistantPanelEvent::Apply {
            kind,
            message,
            markdown,
        });
        true
    }

    /// What the keys of the answer actions do: `kind` with the marked
    /// answer while the keyboard is in the message list, else with the
    /// latest answer.
    pub fn apply_to_target(
        &mut self,
        kind: ApplyKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let target = self
            .messages
            .read(cx)
            .target_answer(window, cx)
            .map(|message| message.id);
        match target {
            Some(message) => self.apply(kind, message, cx),
            None => false,
        }
    }

    // ----- drawing an answer ----------------------------------------------------------

    /// Tells the host to apply answer `message`'s drawing
    /// ([`AssistantPanelEvent::Draw`]). Nothing happens for an answer that
    /// is being written or has no usable drawing for the target; then a
    /// toast says why, when `say` (the key, the command). An answer drawn
    /// before (`drawn_at`) is drawn by the button at once; by the key or
    /// the command (`say`) only at a second press of the same answer within
    /// [`DRAW_AGAIN_WITHIN`], the first press saying so. Returns whether
    /// the host was told.
    pub fn draw(&mut self, message: MessageId, say: bool, cx: &mut Context<Self>) -> bool {
        let target = self.drawing_target();
        let drawing = match self.drawing_of(message, target) {
            Ok(drawing) => drawing,
            Err(why) => {
                if say {
                    show_toast(Toast::new(why), cx);
                }
                return false;
            }
        };
        let drawn_before = self
            .transcript()
            .message(message)
            .is_some_and(|found| found.drawn_at.is_some());
        if say && drawn_before {
            let now = cx.background_executor().now();
            let again = self.draw_again.is_some_and(|(chat, id, at)| {
                chat == self.chat.key
                    && id == message
                    && now.saturating_duration_since(at) <= DRAW_AGAIN_WITHIN
            });
            if !again {
                self.draw_again = Some((self.chat.key, message, now));
                show_toast(
                    Toast::new(match target {
                        DrawingTarget::NewWhiteboard => MADE_ALREADY,
                        _ => DRAWN_ALREADY,
                    }),
                    cx,
                );
                return false;
            }
        }
        self.draw_again = None;
        cx.emit(AssistantPanelEvent::Draw {
            message,
            target,
            drawing,
        });
        true
    }

    /// The drawing of answer `message` that can be applied for `target`,
    /// or what the key says when there is none.
    fn drawing_of(&self, message: MessageId, target: DrawingTarget) -> Result<Drawing, String> {
        if target == DrawingTarget::None {
            return Err(self.drawing_unavailable());
        }
        let Some(found) = self
            .transcript()
            .message(message)
            .filter(|found| found.role == Role::Assistant)
        else {
            return Err(NO_DRAWING.into());
        };
        if found.is_streaming() {
            return Err(STILL_WRITTEN.into());
        }
        let nothing_drawn =
            |problem: DrawingProblem| format!("Nothing was drawn: {}.", problem.reason());
        match answer_drawing(&found.text) {
            None => Err(NO_DRAWING.into()),
            Some(Err(problem)) => Err(nothing_drawn(problem)),
            Some(Ok(drawing)) => match drawing_fits(&drawing, target) {
                Ok(()) => Ok(drawing),
                Err(problem) => Err(nothing_drawn(problem)),
            },
        }
    }

    /// What the key and the command say where no drawing can be applied.
    fn drawing_unavailable(&self) -> String {
        self.document
            .as_ref()
            .and_then(|document| document.drawing_unavailable.clone())
            .map(|why| why.to_string())
            .unwrap_or_else(|| DRAWING_UNAVAILABLE.into())
    }

    /// The answer [`DrawAnswer`]'s key would draw now: the marked answer
    /// while the keyboard is in the message list, else the latest answer.
    pub fn draw_target(&self, window: &Window, cx: &App) -> Option<MessageId> {
        let list = self.messages.read(cx);
        if list.contains_focus(window, cx)
            && let Some(marked) = list.selected()
        {
            return list
                .message(marked)
                .filter(|message| message.role == Role::Assistant)
                .map(|message| message.id);
        }
        self.transcript()
            .messages
            .iter()
            .rev()
            .find(|message| message.role == Role::Assistant)
            .map(|message| message.id)
    }

    /// What `cmd-shift-d` and the command do: [`AssistantPanel::draw`]
    /// with the answer of [`AssistantPanel::draw_target`], saying why when
    /// nothing is drawn.
    pub fn draw_target_answer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match self.draw_target(window, cx) {
            Some(message) => self.draw(message, true, cx),
            None => {
                let why = match self.drawing_target() {
                    DrawingTarget::None => self.drawing_unavailable(),
                    _ => NO_DRAWING.into(),
                };
                show_toast(Toast::new(why), cx);
                false
            }
        }
    }

    /// Marks answer `message` in the message list (as `up` / `down` would)
    /// and scrolls it into view, without taking the keyboard.
    pub fn show_answer(&mut self, message: MessageId, cx: &mut Context<Self>) {
        self.messages
            .update(cx, |list, cx| list.select(Some(message), cx));
        cx.notify();
    }

    /// Records that answer `message`'s drawing was applied (its button reads
    /// "Draw Again" from now on) and reports the conversation
    /// ([`AssistantPanelEvent::TranscriptChanged`]), so that it is stored.
    pub fn mark_drawn(&mut self, message: MessageId, cx: &mut Context<Self>) {
        if !self
            .chat
            .session
            .mark_drawn(message, innerword_assistant::time::now_millis())
        {
            return;
        }
        let messages = self.chat.session.transcript().messages.clone();
        self.messages.update(cx, |list, cx| {
            list.update_in_place(&messages, cx);
        });
        self.report_transcript(cx);
        cx.notify();
    }

    /// Puts `text` into the composer when it is empty (the keyboard stays
    /// where it is). Returns whether it did: the host offers the writer a
    /// message to send.
    pub fn offer_message(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        if !self.composer.read(cx).is_blank(cx) {
            return false;
        }
        self.composer
            .update(cx, |composer, cx| composer.set_text(text, cx));
        true
    }

    fn sync_actions(&mut self, cx: &mut Context<Self>) {
        let besides_a_drawing = |facts: &AnswerFacts<'_>| match facts.has_text {
            true => ActionState::Enabled,
            false => ActionState::Disabled(ONLY_A_DRAWING.into()),
        };
        let mut actions = Vec::new();
        let target = self.drawing_target();
        let words = match target {
            DrawingTarget::OpenWhiteboard => Some((
                "Draw",
                "Draw on Whiteboard",
                ActionWords {
                    label: "Draw Again".into(),
                    tooltip: "Already drawn once: drawing it again adds its elements again \
                              and moves again what it moves"
                        .into(),
                    menu: "Draw Again on Whiteboard".into(),
                },
            )),
            DrawingTarget::NewWhiteboard => Some((
                "New Whiteboard",
                "New Whiteboard from Answer",
                ActionWords {
                    label: "New Whiteboard Again".into(),
                    tooltip: "Already made into a whiteboard once: this makes another one".into(),
                    menu: "New Whiteboard from Answer Again".into(),
                },
            )),
            DrawingTarget::None => None,
        };
        if let Some((label, tooltip, again)) = words {
            actions.push(
                AnswerAction::new(ACTION_DRAW, label)
                    .tooltip(tooltip)
                    .key_of(DrawAnswer)
                    .menu_last()
                    .state_for(move |facts| {
                        let cannot = |problem: DrawingProblem| {
                            ActionState::Disabled(
                                format!("Drawing that cannot be used: {}.", problem.reason())
                                    .into(),
                            )
                        };
                        match facts.drawing {
                            None => ActionState::Hidden,
                            Some(Err(problem)) => cannot(problem.clone()),
                            Some(Ok(drawing)) => match drawing_fits(drawing, target) {
                                Ok(()) => ActionState::Enabled,
                                Err(problem) => cannot(problem),
                            },
                        }
                    })
                    .words_for(move |facts| facts.message.drawn_at.map(|_| again.clone())),
            );
        }
        actions.extend([
            AnswerAction::new(ACTION_INSERT, "Insert")
                .tooltip("Insert at Caret")
                .enabled(self.can_insert)
                .key_of(InsertAtCaret)
                .state_for(besides_a_drawing),
            AnswerAction::new(ACTION_REPLACE, "Replace")
                .tooltip("Replace Selection")
                .enabled(self.can_replace)
                .key_of(ReplaceSelection)
                .state_for(besides_a_drawing),
            AnswerAction::new(ACTION_COPY, "Copy")
                .tooltip("Copy Answer")
                .key_of(CopyAnswer)
                .state_for(besides_a_drawing),
        ]);
        self.messages
            .update(cx, |list, cx| list.set_actions(actions, cx));
    }

    // ----- keeping the pieces in step -------------------------------------------------

    /// Shows the conversation as it is now.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let messages = self.chat.session.transcript().messages.clone();
        self.sync_empty_state(cx);
        self.messages.update(cx, |list, cx| {
            list.set_messages(&messages, window, cx);
        });
        let busy = self.chat.session.is_answering() || self.held.is_some();
        self.composer.update(cx, |composer, cx| {
            composer.set_busy(busy, cx);
        });
        cx.notify();
    }

    /// What an empty conversation shows, and the composer's placeholder, for
    /// the open item.
    fn sync_empty_state(&mut self, cx: &mut Context<Self>) {
        let whiteboard = self.drawing_target() == DrawingTarget::OpenWhiteboard;
        let note = self
            .document
            .as_ref()
            .and_then(|document| document.origin_note.clone())
            .unwrap_or_default();
        let empty = match (&self.document, whiteboard) {
            (Some(_), true) => EmptyState {
                title: "Ask about this whiteboard".into(),
                text: "The assistant reads the whiteboard's elements as they are when you send \
                       a message. It can propose a drawing; you decide whether it goes on the \
                       whiteboard."
                    .into(),
                note,
                examples: vec![
                    "Group these into frames by topic.".into(),
                    "Add a legend that explains the colours.".into(),
                    "What is missing from this diagram?".into(),
                ],
            },
            (Some(_), false) => EmptyState {
                title: "Ask about this document".into(),
                text: "The assistant reads the document as it is when you send a message, \
                     and what you add as context. It suggests; you decide what goes into \
                     the text."
                    .into(),
                note,
                examples: vec![
                    "Where does this lose pace, and why?".into(),
                    "Which sentences are harder to read than they need to be?".into(),
                    "Summarise this in three sentences.".into(),
                ],
            },
            (None, _) => EmptyState {
                title: "Ask the assistant".into(),
                text: "Open a document to ask about it.".into(),
                note: SharedString::default(),
                examples: Vec::new(),
            },
        };
        let placeholder: SharedString = match (&self.document, whiteboard) {
            (Some(_), true) => "Ask about this whiteboard".into(),
            (Some(_), false) => "Ask about this document".into(),
            (None, _) => "Ask the assistant".into(),
        };
        self.messages
            .update(cx, |list, cx| list.set_empty_state(Some(empty), cx));
        self.composer.update(cx, |composer, cx| {
            composer.set_placeholder(placeholder, cx);
        });
    }

    fn report_transcript(&mut self, cx: &mut Context<Self>) {
        cx.emit(AssistantPanelEvent::TranscriptChanged {
            document: self.chat.document,
            transcript: self.chat.session.transcript().clone(),
        });
    }

    /// The events of one chat: the open one, or one that goes on while
    /// another document is open.
    fn apply_events(
        &mut self,
        key: u64,
        batch: Vec<innerword_assistant::AssistantEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.chat.key == key {
            let mut changed = false;
            let mut ended = false;
            for event in batch {
                ended |= matches!(event, innerword_assistant::AssistantEvent::Finished { .. });
                changed |= self.chat.session.apply(event);
            }
            if changed {
                self.sync(window, cx);
            }
            if ended {
                self.report_transcript(cx);
            }
            return;
        }
        let Some(index) = self.parked.iter().position(|chat| chat.key == key) else {
            return;
        };
        for event in batch {
            self.parked[index].session.apply(event);
        }
        if !self.parked[index].session.is_answering() {
            // Its answer has ended: hand the conversation to the host and
            // let the agent's program go.
            let chat = self.parked.remove(index);
            cx.emit(AssistantPanelEvent::TranscriptChanged {
                document: chat.document,
                transcript: chat.session.transcript().clone(),
            });
        }
    }

    fn composer_event(
        &mut self,
        _: &Entity<Composer>,
        event: &ComposerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ComposerEvent::Submitted(text) => self.send(text.clone(), window, cx),
            ComposerEvent::StopRequested => self.stop(cx),
            ComposerEvent::Dismissed => cx.emit(AssistantPanelEvent::FocusTextRequested),
            ComposerEvent::Changed => {}
        }
    }

    fn messages_event(
        &mut self,
        _: &Entity<MessageList>,
        event: &MessageListEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            MessageListEvent::ActionInvoked { action, message } => {
                if action.as_ref() == ACTION_DRAW {
                    self.draw(*message, false, cx);
                } else if let Some(kind) = ApplyKind::from_action(action) {
                    self.apply(kind, *message, cx);
                }
            }
            MessageListEvent::CopyRequested { message } => {
                let Some(found) = self.transcript().message(*message) else {
                    return;
                };
                match found.role {
                    Role::Assistant => {
                        self.apply(ApplyKind::Copy, *message, cx);
                    }
                    Role::User => cx.emit(AssistantPanelEvent::CopyText(found.text.clone())),
                }
            }
            MessageListEvent::RetryRequested { message } => self.retry(*message, window, cx),
            MessageListEvent::ExampleChosen(example) => {
                self.composer.update(cx, |composer, cx| {
                    composer.set_text(example, cx);
                    composer.focus(window, cx);
                });
            }
            MessageListEvent::Dismissed => cx.emit(AssistantPanelEvent::FocusTextRequested),
        }
    }

    // ----- the keyboard ---------------------------------------------------------------

    /// Moves the keyboard into the composer (what the app's "go to the
    /// assistant" does).
    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_area(PanelArea::Composer, window, cx);
    }

    /// Moves the keyboard to a part of the panel. An open picker of the
    /// context row closes.
    pub fn focus_area(&mut self, area: PanelArea, window: &mut Window, cx: &mut Context<Self>) {
        self.close_context_picker(window, cx);
        match area {
            PanelArea::Header => {
                if !self.header_control_enabled(self.header_control) {
                    self.header_control = HeaderControl::Model;
                }
                window.focus(&self.header_focus);
            }
            PanelArea::Context => {
                self.context_index = self.context_index.min(self.attachments().len());
                window.focus(&self.context_focus);
            }
            PanelArea::Messages => {
                self.messages.update(cx, |list, cx| list.focus(window, cx));
            }
            PanelArea::Composer => self.composer.read(cx).focus(window, cx),
        }
        cx.notify();
    }

    /// The part of the panel the keyboard is in.
    pub fn focused_area(&self, window: &Window, cx: &App) -> Option<PanelArea> {
        if self.composer.read(cx).is_focused(window, cx) {
            Some(PanelArea::Composer)
        } else if self.messages.read(cx).contains_focus(window, cx) {
            Some(PanelArea::Messages)
        } else if self.header_focus.is_focused(window) {
            Some(PanelArea::Header)
        } else if self.context_focus.is_focused(window) {
            Some(PanelArea::Context)
        } else {
            None
        }
    }

    /// Whether the keyboard is anywhere in the panel (its picker
    /// included).
    pub fn contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.focused_area(window, cx).is_some()
            || self
                .picker
                .as_ref()
                .is_some_and(|picker| picker.focus_handle(cx).contains_focused(window, cx))
    }

    /// `tab` and `shift-tab`: header, context, messages, composer, and
    /// round again.
    pub fn cycle_focus(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        const ORDER: [PanelArea; 4] = [
            PanelArea::Header,
            PanelArea::Context,
            PanelArea::Messages,
            PanelArea::Composer,
        ];
        // From an open picker: on from where the keyboard was before it.
        self.close_context_picker(window, cx);
        let current = self
            .focused_area(window, cx)
            .and_then(|area| ORDER.iter().position(|candidate| *candidate == area));
        let next = match (current, forward) {
            (None, _) => 3,
            (Some(index), true) => (index + 1) % ORDER.len(),
            (Some(index), false) => (index + ORDER.len() - 1) % ORDER.len(),
        };
        self.focus_area(ORDER[next], window, cx);
    }

    /// The marked control of the header.
    pub fn header_control(&self) -> HeaderControl {
        self.header_control
    }

    /// The marked item of the context row: an index into the attachments,
    /// or their number for "Add".
    pub fn context_index(&self) -> usize {
        self.context_index
    }

    pub(crate) fn header_control_enabled(&self, control: HeaderControl) -> bool {
        match control {
            HeaderControl::Effort => {
                picker_has_choices(PickerKind::Effort, &self.catalog(), &self.selection)
            }
            HeaderControl::NewChat => {
                !self.transcript().is_empty() || !self.attachments().is_empty()
            }
            HeaderControl::Clear => !self.transcript().is_empty(),
            HeaderControl::Model | HeaderControl::Close => true,
        }
    }

    /// `left` / `right` in the header or the context row.
    pub(crate) fn move_control(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.context_focus.is_focused(window) {
            let last = self.attachments().len();
            self.context_index = if forward {
                (self.context_index + 1).min(last)
            } else {
                self.context_index.saturating_sub(1)
            };
        } else if self.header_focus.is_focused(window) {
            let all = HeaderControl::ALL;
            let mut index = all
                .iter()
                .position(|control| *control == self.header_control)
                .unwrap_or(0);
            loop {
                let next = if forward {
                    index + 1
                } else {
                    match index.checked_sub(1) {
                        Some(next) => next,
                        None => break,
                    }
                };
                if next >= all.len() {
                    break;
                }
                index = next;
                if self.header_control_enabled(all[index]) {
                    self.header_control = all[index];
                    break;
                }
            }
        }
        cx.notify();
    }

    /// `enter` or `space` in the header or the context row.
    pub(crate) fn press_control(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.context_focus.is_focused(window) {
            if self.context_index >= self.attachments().len() {
                self.open_context_picker(window, cx);
            } else {
                self.remove_marked_context(window, cx);
            }
        } else if self.header_focus.is_focused(window) {
            self.press_header(self.header_control, true, window, cx);
        }
    }

    /// Does what a control of the header does.
    pub fn press_header(
        &mut self,
        control: HeaderControl,
        from_keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.header_control_enabled(control) {
            return;
        }
        match control {
            HeaderControl::Model => self.open_picker(PickerKind::Model, from_keyboard, window, cx),
            HeaderControl::Effort => {
                self.open_picker(PickerKind::Effort, from_keyboard, window, cx)
            }
            HeaderControl::NewChat => {
                self.new_chat(window, cx);
                self.leave_disabled_control(window, cx);
            }
            HeaderControl::Clear => {
                self.clear(window, cx);
                self.leave_disabled_control(window, cx);
            }
            HeaderControl::Close => cx.emit(AssistantPanelEvent::CloseRequested),
        }
    }

    /// The marked control of the header has just switched itself off (the
    /// chat was cleared with "Clear"): the mark moves on to the model
    /// picker, which is always there.
    fn leave_disabled_control(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if !self.header_control_enabled(self.header_control) {
            self.header_control = HeaderControl::Model;
            cx.notify();
        }
    }

    /// `delete` or `backspace` on a chip of the context row.
    pub(crate) fn remove_marked_context(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(attachment) = self.attachments().get(self.context_index).cloned() else {
            return;
        };
        self.detach(attachment.kind, attachment.id, cx);
    }

    // ----- menus and the picker ---------------------------------------------------------

    /// Opens the menu of the model or the effort picker under its control.
    pub fn open_picker(
        &mut self,
        kind: PickerKind,
        from_keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let catalog = self.catalog();
        if !picker_has_choices(kind, &catalog, &self.selection) {
            return;
        }
        let bounds = match kind {
            PickerKind::Model => self.model_bounds.clone(),
            PickerKind::Effort => self.effort_bounds.clone(),
        };
        let panel = cx.weak_entity();
        open_picker_menu(
            kind,
            &catalog,
            &self.selection,
            &bounds,
            from_keyboard,
            move |selection, _, cx| {
                panel
                    .update(cx, |panel, cx| panel.set_selection(selection, cx))
                    .ok();
            },
            window,
            cx,
        );
    }

    /// What the picker of the context row offers: the library's documents
    /// and folders without the open document and without what is attached
    /// already.
    pub fn context_candidates(&self, cx: &App) -> Vec<Attachment> {
        let Some(source) = &self.library_source else {
            return Vec::new();
        };
        source(cx)
            .into_iter()
            .filter(|candidate| {
                !self.is_open_document(candidate)
                    && !self
                        .attachments()
                        .iter()
                        .any(|attached| attached.is_same_item(candidate))
            })
            .collect()
    }

    /// Opens the picker over the library under "Add". The keyboard goes
    /// into its field and comes back to where it was when it closes.
    pub fn open_context_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.is_some() {
            return;
        }
        let candidates = self.context_candidates(cx);
        let panel = cx.weak_entity();
        let picker = cx.new(|cx| {
            ContextPicker::popup(ContextPickerDelegate::new(panel, candidates), window, cx)
                .width(gpui::px(300.))
                .max_visible_rows(9)
        });
        self.picker_return = window.focused(cx).map(|handle| handle.downgrade());
        self._picker_subscription = Some(cx.subscribe_in(
            &picker,
            window,
            |this, _, _: &DismissEvent, window, cx| this.close_context_picker(window, cx),
        ));
        window.focus(&picker.focus_handle(cx));
        self.picker = Some(picker);
        cx.notify();
    }

    /// Closes the picker of the context row; the keyboard goes back to
    /// where it was (or to the composer).
    pub fn close_context_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.picker.take() else {
            return;
        };
        self._picker_subscription = None;
        let had_focus = picker.focus_handle(cx).contains_focused(window, cx);
        let back = self
            .picker_return
            .take()
            .and_then(|handle| handle.upgrade());
        if had_focus {
            match back {
                Some(handle) => window.focus(&handle),
                None => self.composer.read(cx).focus(window, cx),
            }
        }
        cx.notify();
    }
}

/// The notices above the composer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanelNotice {
    /// The agent is not installed, or nobody is signed in. Its button
    /// looks again.
    Availability,
    /// Where the text goes. Its button is "OK".
    Privacy,
    /// Something the agent wants the user to know. Its button dismisses
    /// it.
    Agent,
}

/// The panel that window-wide assistant actions go to: the one of the
/// active window, or, when no window is active (the app is in the
/// background), the only one there is. `None` while that panel is not on
/// screen.
pub(crate) fn panel_for_actions(cx: &App) -> Option<(AnyWindowHandle, Entity<AssistantPanel>)> {
    let panels = cx.try_global::<WindowPanels>()?;
    let live = || {
        panels
            .0
            .iter()
            .rev()
            .filter_map(|(window, panel)| Some((*window, panel.upgrade()?)))
    };
    let found = match cx.active_window() {
        Some(active) => live().find(|(window, _)| *window == active),
        None => None,
    };
    let (window, panel) = match found {
        Some(found) => found,
        None => {
            let (window, panel) = live().next()?;
            if !live().all(|(other, _)| other == window) {
                return None;
            }
            (window, panel)
        }
    };
    panel.read(cx).is_shown().then_some((window, panel))
}
