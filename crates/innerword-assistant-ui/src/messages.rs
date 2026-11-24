//! [`MessageList`]: the conversation, as it is read.
//!
//! The user's messages are quiet boxes in the interface font. The
//! assistant's answers are text: each is a read-only, formatted compact
//! `innerword_editor::Editor` over the document the app's own Markdown
//! importer makes of the answer, so a heading, a list, a quote or code in
//! an answer looks as it would in a document, text can be selected with
//! the mouse or the keyboard and copied with its formatting, and a link
//! opens with a `cmd`-click. The document is rebuilt from the Markdown
//! each time a piece of the answer arrives (see `markdown`).
//!
//! Under an answer there is one line: what the assistant is doing while it
//! writes ("Reading Chapter 2"), then the answer's actions, which the host
//! supplies ([`AnswerAction`]: the two chats have different ones). The
//! actions of the latest answer are always shown, those of older answers
//! under the pointer and on the marked message. A stopped answer says so
//! and keeps its actions; a failed one says what happened and what to do
//! in the engine's words, with "Try Again".
//!
//! An action can be offered under some answers and not others, or offered
//! dimmed with the reason: the host says so per answer
//! ([`AnswerAction::state_for`], from the [`AnswerFacts`] of the answer).
//! An action can also stand last in the answer's menu while it is first in
//! the row of buttons ([`AnswerAction::menu_last`]).
//!
//! # Drawings
//!
//! A list made with [`MessageListOptions::drawings`] shows each drawing
//! block of an answer (a fenced block of `innerword_assistant::DRAWING_INFO`)
//! as the one line `innerword_assistant::shown_markdown` writes for it, never
//! its JSON, and hands the drawing read from the blocks to the host with the
//! answer's facts. What a drawing is for (and what it is drawn on) is the
//! host's: it says it with [`MessageList::set_drawings`].
//!
//! The list follows an answer that is being written: while its end is in
//! view it stays in view. Scrolling away stops that, scrolling back to the
//! end (or sending a message) starts it again.
//!
//! # Keys (while the list has the keyboard)
//!
//! | Key | What it does |
//! |---|---|
//! | `up` / `down`, `ctrl-p` / `ctrl-n` | mark the previous / next message |
//! | `home` / `end`, `cmd-up` / `cmd-down` | the first / last message |
//! | `enter`, `ctrl-enter`, `shift-f10` | the menu of the marked message: its actions, copy |
//! | `cmd-c` | copy the marked message ([`MessageListEvent::CopyRequested`]) |
//! | `escape` | [`MessageListEvent::Dismissed`] |
//!
//! In an empty conversation the example prompts are marked the same way
//! and `enter` hands the marked one to the host
//! ([`MessageListEvent::ExampleChosen`]).
//!
//! A click in the text of an answer puts the keyboard there (a caret, the
//! arrows, `shift` to select, `cmd-c`); a click beside it, or on a message
//! of the user, marks the message. A right-click opens the menu.
//!
//! ```ignore
//! let list = cx.new(|cx| MessageList::new(MessageListOptions::new(backend.info()), window, cx));
//! list.update(cx, |list, cx| {
//!     list.set_actions(vec![
//!         AnswerAction::new("replace", "Replace").tooltip("Replace the selection").key_of(ReplaceSelection),
//!         AnswerAction::new("copy", "Copy").tooltip("Copy the answer").key_of(CopyAnswer),
//!     ], cx);
//! });
//! cx.subscribe_in(&list, window, |this, _, event: &MessageListEvent, window, cx| match event {
//!     MessageListEvent::ActionInvoked { action, message } => this.apply(action, *message, window, cx),
//!     MessageListEvent::CopyRequested { message } => this.copy(*message, cx),
//!     MessageListEvent::RetryRequested { message } => this.ask_again(*message, cx),
//!     MessageListEvent::ExampleChosen(text) => this.use_example(text, window, cx),
//!     MessageListEvent::Dismissed => this.close(window, cx),
//! })
//! .detach();
//! // Whenever the session changed (after applying a batch of events, after a send):
//! list.update(cx, |list, cx| list.set_messages(&session.transcript().messages, window, cx));
//! ```

use std::{
    cell::Cell,
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use gpui::{
    Action, Animation, AnimationExt, AnyElement, App, Bounds, ClickEvent, Context, Div, Entity,
    EventEmitter, FocusHandle, Focusable, FontWeight, ListAlignment, ListOffset, ListState,
    MouseButton, MouseDownEvent, Pixels, Point, Render, SharedString, Subscription, Window, div,
    list, point, prelude::*, px,
};
use innerword_assistant::{
    BackendInfo, Drawing, DrawingProblem, DrawingTarget, ElementNames, Message, MessageId,
    MessageState, NamesWhenDrawn, Role, answer_drawing, shown_markdown, without_drawings,
};
use innerword_core::Document;
use innerword_editor::{CompactOptions, ContextMenuRequest, Editor, EditorConfig, EditorEvent};
use innerword_ui::{
    ContextMenu, ContextMenuItem, KeyHint,
    context_menu::OpenContextMenu,
    theme::{
        ActiveTheme, RADIUS_LG, RADIUS_MD, SPACE_1, SPACE_2, SPACE_3, ThemeColors, UI_LINE_HEIGHT,
        UI_TEXT_BASE, UI_TEXT_SM, UI_TEXT_XS,
    },
};

use crate::{
    ChatKeys,
    actions::{
        ANSWER_EDITOR_FLAG, ActivateSelected, CHAT_EDITOR_FLAG, CopyMessage, DismissChat,
        MESSAGES_CONTEXT, SelectFirstMessage, SelectLastMessage, SelectNextMessage,
        SelectPreviousMessage,
    },
    controls::{Look, keep_focus, text_button},
    markdown::{AnswerStyle, answer_document},
    notice::{Notice, notice_row},
};

/// How far beyond the visible part of the list messages are measured.
const LIST_OVERDRAW: Pixels = px(320.);

/// The height of the line under an answer (what the assistant is doing,
/// then the actions).
const FOOTER_HEIGHT: Pixels = px(26.);

/// Whether an action is offered under one answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionState {
    /// Not under this answer, nor in its menu.
    Hidden,
    /// Shown, and does what it says.
    Enabled,
    /// Shown dimmed; its tooltip and its menu item say why (an empty
    /// reason: dimmed without one).
    Disabled(SharedString),
}

/// What the list knows of one answer, made once each time its text
/// changes.
#[derive(Clone, Copy)]
pub struct AnswerFacts<'a> {
    /// The answer.
    pub message: &'a Message,
    /// Its drawing (`innerword_assistant::answer_drawing`), when the list
    /// shows drawings and it has one.
    pub drawing: Option<&'a Result<Drawing, DrawingProblem>>,
    /// Whether it has text besides its drawings.
    pub has_text: bool,
}

/// The words of an action under one answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionWords {
    /// The words on the button.
    pub label: SharedString,
    /// Its tooltip.
    pub tooltip: SharedString,
    /// Its item in the answer's menu.
    pub menu: SharedString,
}

type StateFn = Rc<dyn Fn(&AnswerFacts<'_>) -> ActionState>;
type WordsFn = Rc<dyn Fn(&AnswerFacts<'_>) -> Option<ActionWords>>;

/// One thing that can be done with an answer: a small button under it and
/// an item of its menu. The host supplies the list and carries the action
/// out ([`MessageListEvent::ActionInvoked`]).
pub struct AnswerAction {
    /// The host's name for it; it comes back in the event.
    pub id: SharedString,
    /// The words on the button.
    pub label: SharedString,
    /// What the button does, for its tooltip and the menu.
    pub tooltip: SharedString,
    /// Whether it can be done now (Replace Selection needs a selection).
    pub enabled: bool,
    key_of: Option<Box<dyn Action>>,
    state: Option<StateFn>,
    words: Option<WordsFn>,
    menu_last: bool,
}

impl AnswerAction {
    /// An action with this name and label, enabled, with the label as its
    /// tooltip.
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        let label = label.into();
        Self {
            id: id.into(),
            tooltip: label.clone(),
            label,
            enabled: true,
            key_of: None,
            state: None,
            words: None,
            menu_last: false,
        }
    }

    /// The action's state under each answer. Without it the action is
    /// under every answer that can be applied, enabled as
    /// [`AnswerAction::enabled`] says.
    pub fn state_for(mut self, state: impl Fn(&AnswerFacts<'_>) -> ActionState + 'static) -> Self {
        self.state = Some(Rc::new(state));
        self
    }

    /// Its state under the answer of `facts` (`enabled` false wins over
    /// what [`AnswerAction::state_for`] says is enabled).
    pub fn state(&self, facts: &AnswerFacts<'_>) -> ActionState {
        let state = match &self.state {
            Some(state) => state(facts),
            None => ActionState::Enabled,
        };
        match state {
            ActionState::Enabled if !self.enabled => ActionState::Disabled(SharedString::default()),
            state => state,
        }
    }

    /// Other words under some answers: `words` gives the words under an
    /// answer, or `None` for [`AnswerAction::label`] and
    /// [`AnswerAction::tooltip`] (the menu says the tooltip). "Draw Again"
    /// under an answer that was drawn before.
    pub fn words_for(
        mut self,
        words: impl Fn(&AnswerFacts<'_>) -> Option<ActionWords> + 'static,
    ) -> Self {
        self.words = Some(Rc::new(words));
        self
    }

    /// Its words under the answer of `facts`.
    pub fn words(&self, facts: &AnswerFacts<'_>) -> ActionWords {
        self.words
            .as_ref()
            .and_then(|words| words(facts))
            .unwrap_or_else(|| self.plain_words())
    }

    fn plain_words(&self) -> ActionWords {
        ActionWords {
            label: self.label.clone(),
            tooltip: self.tooltip.clone(),
            menu: self.tooltip.clone(),
        }
    }

    /// In the answer's menu the action comes after the others; the row of
    /// buttons keeps the order of the list of actions.
    pub fn menu_last(mut self) -> Self {
        self.menu_last = true;
        self
    }

    /// Whether the action comes last in the answer's menu.
    pub fn is_menu_last(&self) -> bool {
        self.menu_last
    }

    /// Sets what the tooltip and the menu say.
    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = tooltip.into();
        self
    }

    /// Sets whether it can be done now.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// The GPUI action whose key does the same from the keyboard: the
    /// tooltip and the menu show the key the keymap has for it. (The list
    /// does not dispatch it: the host handles its own action, on the
    /// answer [`MessageList::target_answer`] names.)
    pub fn key_of(mut self, action: impl Action) -> Self {
        self.key_of = Some(Box::new(action));
        self
    }

    /// The key that does the same, from the keymap as a whole (the binding
    /// added last). A chat names the key in its own place:
    /// [`AnswerAction::key_hint_at`].
    pub fn key_hint(&self, cx: &App) -> Option<KeyHint> {
        self.key_of
            .as_ref()
            .and_then(|action| KeyHint::for_action(action.as_ref(), cx))
    }

    /// The key that does the same where `focus` has the keyboard (the
    /// message list of the chat the answer is in), or the keymap-wide one
    /// when that place has none. The actions of an answer are bound once
    /// for each chat, and each of those bindings can be given another key.
    pub fn key_hint_at(&self, focus: &FocusHandle, window: &Window, cx: &App) -> Option<KeyHint> {
        self.key_of
            .as_ref()
            .and_then(|action| KeyHint::for_action_at(action.as_ref(), focus, window, cx))
    }
}

impl Clone for AnswerAction {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            label: self.label.clone(),
            tooltip: self.tooltip.clone(),
            enabled: self.enabled,
            key_of: self.key_of.as_ref().map(|action| action.boxed_clone()),
            state: self.state.clone(),
            words: self.words.clone(),
            menu_last: self.menu_last,
        }
    }
}

impl std::fmt::Debug for AnswerAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnswerAction")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("enabled", &self.enabled)
            .finish()
    }
}

/// What an empty conversation shows instead of messages.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EmptyState {
    /// A short line that says what this chat is for.
    pub title: SharedString,
    /// One or two sentences under it. May be empty.
    pub text: SharedString,
    /// A line of its own above the examples (where the item came from).
    /// May be empty.
    pub note: SharedString,
    /// Example prompts. A click on one, or `enter` on the marked one,
    /// hands it to the host ([`MessageListEvent::ExampleChosen`]).
    pub examples: Vec<SharedString>,
}

/// What a [`MessageList`] is made with.
#[derive(Clone, Debug)]
pub struct MessageListOptions {
    /// The agent the chat runs on: errors are worded with its name.
    pub info: BackendInfo,
    /// How answers are set.
    pub style: AnswerStyle,
    /// The key context flags of the host.
    pub keys: ChatKeys,
    /// Show drawings: each drawing block of an answer as its line, and the
    /// facts of an answer carry its drawing.
    pub drawings: bool,
}

impl MessageListOptions {
    /// Options for a chat on the agent of `info`, with the side chat's
    /// answer style.
    pub fn new(info: BackendInfo) -> Self {
        Self {
            info,
            style: AnswerStyle::default(),
            keys: ChatKeys::default(),
            drawings: false,
        }
    }

    /// Show drawings: each drawing block of an answer as its line (see the
    /// [module docs](self)); the facts carry drawings.
    pub fn drawings(mut self, on: bool) -> Self {
        self.drawings = on;
        self
    }

    /// Sets how answers are set.
    pub fn style(mut self, style: AnswerStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets the key context flags of the host.
    pub fn keys(mut self, keys: ChatKeys) -> Self {
        self.keys = keys;
        self
    }
}

/// What a [`MessageList`] tells its host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageListEvent {
    /// A button under an answer was pressed, or the same item of its menu
    /// chosen. `action` is [`AnswerAction::id`].
    ActionInvoked {
        /// The action.
        action: SharedString,
        /// The answer.
        message: MessageId,
    },
    /// Copy this message (`cmd-c` in the list, "Copy" in the menu of a
    /// message of the user). The host writes the clipboard.
    CopyRequested {
        /// The message.
        message: MessageId,
    },
    /// "Try Again" under a failed answer: ask its question again.
    RetryRequested {
        /// The failed answer.
        message: MessageId,
    },
    /// An example prompt of the empty state was chosen.
    ExampleChosen(SharedString),
    /// `escape`: the keyboard should go back to where the chat was opened
    /// from.
    Dismissed,
}

/// The editor of one answer and what it shows.
struct AnswerView {
    editor: Entity<Editor>,
    /// A fingerprint of the Markdown and the streaming flag the editor's
    /// document was made from (and of the drawing's words, when the list
    /// shows drawings).
    shown: u64,
    /// The answer's drawing, read with its text.
    drawing: Option<Result<Drawing, DrawingProblem>>,
    /// Whether the answer has text besides its drawings.
    has_text: bool,
    _subscription: Subscription,
}

/// What an answer view is made from: the document, the drawing, whether
/// there is text besides it.
struct AnswerParts {
    document: Document,
    drawing: Option<Result<Drawing, DrawingProblem>>,
    has_text: bool,
}

/// The names of the open whiteboard's elements, as the host gives them.
pub type SharedElementNames = Arc<dyn ElementNames + Send + Sync>;

/// See the [module docs](self).
pub struct MessageList {
    messages: Vec<Message>,
    views: HashMap<MessageId, AnswerView>,
    list: ListState,
    /// The list keeps its end in view.
    follow: bool,
    /// Set by the list when the user scrolled it.
    scrolled: Rc<Cell<bool>>,
    selected: Option<MessageId>,
    /// The marked message is marked for the writer to see also while the
    /// keyboard is elsewhere: a command run from outside the chat pointed at
    /// it ([`MessageList::mark_shown`]). Its key acts on it too.
    shown: Option<MessageId>,
    selected_example: usize,
    focus_handle: FocusHandle,
    actions: Vec<AnswerAction>,
    empty: Option<EmptyState>,
    info: BackendInfo,
    style: AnswerStyle,
    keys: ChatKeys,
    /// Whether drawings are shown as their lines.
    drawings: bool,
    /// What a drawing is offered for (the words of its line).
    drawing_target: DrawingTarget,
    /// The names of the elements the line names.
    element_names: Option<SharedElementNames>,
    /// Counts the changes of the two above, for the fingerprints.
    drawing_words: u64,
    /// The left button went down on a control of a row (a button under an
    /// answer): that press is not a press on the row.
    control_pressed: bool,
    /// How many rows were built since the list was made (tests: only the
    /// rows near the visible part are).
    rows_built: usize,
}

impl EventEmitter<MessageListEvent> for MessageList {}

impl Focusable for MessageList {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn fingerprint(markdown: &str, streaming: bool, words: u64, drawn_at: Option<i64>) -> u64 {
    let mut hasher = DefaultHasher::new();
    markdown.hash(&mut hasher);
    streaming.hash(&mut hasher);
    words.hash(&mut hasher);
    drawn_at.hash(&mut hasher);
    hasher.finish()
}

/// `text` cut at `limit` characters, with "…" when it was longer.
pub(crate) fn cut(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(limit.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

impl MessageList {
    /// An empty list.
    pub fn new(options: MessageListOptions, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let list = ListState::new(0, ListAlignment::Top, LIST_OVERDRAW);
        let scrolled = Rc::new(Cell::new(false));
        list.set_scroll_handler({
            let scrolled = scrolled.clone();
            move |_, _, _| scrolled.set(true)
        });
        Self {
            messages: Vec::new(),
            views: HashMap::new(),
            list,
            follow: true,
            scrolled,
            selected: None,
            shown: None,
            selected_example: 0,
            focus_handle: cx.focus_handle(),
            actions: Vec::new(),
            empty: None,
            info: options.info,
            style: options.style,
            keys: options.keys,
            drawings: options.drawings,
            drawing_target: DrawingTarget::None,
            element_names: None,
            drawing_words: 0,
            control_pressed: false,
            rows_built: 0,
        }
    }

    // ----- what is shown -----------------------------------------------------

    /// Shows `messages`: the transcript of the chat as it is now. Call it
    /// whenever the transcript changed. What is already shown is kept
    /// (answers keep their editors, the list its place) when the messages
    /// continue the ones shown before, also after a long transcript has
    /// dropped its oldest messages; otherwise the list starts at the end.
    /// Messages are told apart by their ids, which every transcript counts
    /// from one: before showing another conversation, call
    /// [`clear`](Self::clear).
    pub fn set_messages(
        &mut self,
        messages: &[Message],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let old_ids: Vec<MessageId> = self.messages.iter().map(|message| message.id).collect();
        let new_ids: Vec<MessageId> = messages.iter().map(|message| message.id).collect();
        // The messages shown before, without the oldest ones a long
        // transcript has dropped since.
        let dropped = new_ids
            .first()
            .and_then(|first| old_ids.iter().position(|id| id == first))
            .unwrap_or(0);
        let kept = &old_ids[dropped..];
        let continues = !new_ids.is_empty()
            && new_ids.len() >= kept.len()
            && new_ids[..kept.len()] == *kept
            && (dropped == 0 || !kept.is_empty());
        if continues {
            if dropped > 0 {
                self.list.splice(0..dropped, 0);
            }
            let added = new_ids.len() - kept.len();
            if added > 0 {
                // A new message: the mark shown from outside pointed at an
                // older answer.
                self.shown = None;
                self.list.splice(kept.len()..kept.len(), added);
                // The user's own new message is always brought into view.
                if messages[kept.len()..]
                    .iter()
                    .any(|message| message.role == Role::User)
                {
                    self.follow = true;
                }
            }
        } else {
            self.list.reset(new_ids.len());
            self.shown = None;
            self.follow = true;
            self.scrolled.set(false);
        }
        self.messages = messages.to_vec();
        if self
            .selected
            .is_some_and(|selected| !new_ids.contains(&selected))
        {
            self.selected = None;
        }

        // An editor for every answer that has text; none for what is gone.
        self.views.retain(|id, _| new_ids.contains(id));
        for index in 0..self.messages.len() {
            let message = &self.messages[index];
            if message.role != Role::Assistant || message.text.trim().is_empty() {
                continue;
            }
            let id = message.id;
            match self.views.contains_key(&id) {
                true => self.refresh_view(index, cx),
                false => {
                    let shown = self.shown_of(message);
                    let parts = self.parts_of(message);
                    let view = self.new_answer_view(id, parts, shown, window, cx);
                    self.views.insert(id, view);
                }
            }
        }
        if self.follow {
            self.scroll_to_end();
        }
        cx.notify();
    }

    /// The fingerprint of what the view of `message` shows.
    fn shown_of(&self, message: &Message) -> u64 {
        let words = if self.drawings { self.drawing_words } else { 0 };
        // A drawing applied again keeps the names of that press.
        let drawn_at = if self.drawings {
            message.drawn_at
        } else {
            None
        };
        fingerprint(&message.text, message.is_streaming(), words, drawn_at)
    }

    /// What the view of `message` is made from.
    fn parts_of(&self, message: &Message) -> AnswerParts {
        let streaming = message.is_streaming();
        if !self.drawings {
            return AnswerParts {
                document: answer_document(&message.text, &self.style, streaming),
                drawing: None,
                has_text: !message.text.trim().is_empty(),
            };
        }
        let live = self
            .element_names
            .as_deref()
            .map(|names| names as &dyn ElementNames);
        // A drawn answer's line names what it changed and removed as it was when
        // it was drawn: the whiteboard has none of what it removed since.
        let drawn = message
            .drawn_names
            .as_ref()
            .map(|kept| NamesWhenDrawn { kept, live });
        let names = match &drawn {
            Some(drawn) => Some(drawn as &dyn ElementNames),
            None => live,
        };
        let shown = shown_markdown(&message.text, streaming, self.drawing_target, names);
        let drawing = answer_drawing(&message.text);
        let has_text = match &drawing {
            None => !message.text.trim().is_empty(),
            Some(_) => !without_drawings(&message.text).trim().is_empty(),
        };
        AnswerParts {
            document: answer_document(&shown, &self.style, streaming),
            drawing,
            has_text,
        }
    }

    /// Makes the view of the message at `index` show it as it is now.
    fn refresh_view(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(message) = self.messages.get(index) else {
            return;
        };
        let id = message.id;
        let shown = self.shown_of(message);
        if self.views.get(&id).is_none_or(|view| view.shown == shown) {
            return;
        }
        let parts = self.parts_of(message);
        let Some(view) = self.views.get_mut(&id) else {
            return;
        };
        let document = parts.document;
        view.editor.update(cx, |editor, cx| {
            if editor.document() != &document {
                editor.set_document(document, cx);
            }
        });
        view.shown = shown;
        view.drawing = parts.drawing;
        view.has_text = parts.has_text;
    }

    /// Says what a drawing is offered for (the words of its line,
    /// `innerword_assistant::shown_markdown`) and the names of the elements it
    /// may name. The answers shown are made again. Nothing happens in a list
    /// that does not show drawings.
    pub fn set_drawings(
        &mut self,
        target: DrawingTarget,
        names: Option<SharedElementNames>,
        cx: &mut Context<Self>,
    ) {
        let same_names = match (&self.element_names, &names) {
            (None, None) => true,
            (Some(old), Some(new)) => Arc::ptr_eq(old, new),
            _ => false,
        };
        if self.drawing_target == target && same_names {
            return;
        }
        self.drawing_target = target;
        self.element_names = names;
        self.drawing_words += 1;
        if self.drawings {
            for index in 0..self.messages.len() {
                self.refresh_view(index, cx);
            }
            cx.notify();
        }
    }

    /// What a drawing is offered for, as the host said.
    pub fn drawing_target(&self) -> DrawingTarget {
        self.drawing_target
    }

    /// Whether the list shows drawings.
    pub fn shows_drawings(&self) -> bool {
        self.drawings
    }

    /// Takes `messages` in place of the messages shown when they are the
    /// same messages with the same text (a mark of an answer changed, not
    /// what it says), without a window. Returns whether it did.
    pub(crate) fn update_in_place(&mut self, messages: &[Message], cx: &mut Context<Self>) -> bool {
        let same = messages.len() == self.messages.len()
            && messages.iter().zip(&self.messages).all(|(new, old)| {
                new.id == old.id && new.text == old.text && new.state == old.state
            });
        if same {
            self.messages = messages.to_vec();
            // An answer marked drawn reads with the names kept at the press.
            for index in 0..self.messages.len() {
                self.refresh_view(index, cx);
            }
            cx.notify();
        }
        same
    }

    /// What the list knows of answer `id`, `None` for another message.
    pub fn facts(&self, id: MessageId) -> Option<AnswerFacts<'_>> {
        let message = self.message(id)?;
        if message.role != Role::Assistant {
            return None;
        }
        let view = self.views.get(&id);
        Some(AnswerFacts {
            message,
            drawing: view.and_then(|view| view.drawing.as_ref()),
            has_text: view.is_some_and(|view| view.has_text),
        })
    }

    /// The state of the action `action` under answer `message`
    /// ([`ActionState::Hidden`] for an action the list does not have or a
    /// message that is no answer).
    pub fn action_state(&self, action: &str, message: MessageId) -> ActionState {
        let Some(facts) = self.facts(message) else {
            return ActionState::Hidden;
        };
        self.actions
            .iter()
            .find(|candidate| candidate.id.as_ref() == action)
            .map_or(ActionState::Hidden, |candidate| candidate.state(&facts))
    }

    /// The words of the action `action` under answer `message`.
    pub fn action_words(&self, action: &str, message: MessageId) -> Option<ActionWords> {
        let facts = self.facts(message)?;
        self.actions
            .iter()
            .find(|candidate| candidate.id.as_ref() == action)
            .map(|candidate| candidate.words(&facts))
    }

    fn new_answer_view(
        &mut self,
        id: MessageId,
        parts: AnswerParts,
        shown: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnswerView {
        let config = EditorConfig::compact(
            CompactOptions::default()
                // As tall as the answer: the list scrolls, not the answer.
                .max_height(px(1.0e7))
                .submit_on_enter(false)
                .formatted(true),
        )
        .read_only(true);
        let flag = self.keys.editor_flag.clone();
        let AnswerParts {
            document,
            drawing,
            has_text,
        } = parts;
        let editor = cx.new(|cx| {
            let mut editor = Editor::new(document, config, window, cx);
            editor.add_key_context_flag(CHAT_EDITOR_FLAG, cx);
            editor.add_key_context_flag(ANSWER_EDITOR_FLAG, cx);
            if let Some(flag) = flag {
                editor.add_key_context_flag(flag, cx);
            }
            editor
        });
        let subscription = cx.subscribe_in(
            &editor,
            window,
            move |this, _, event: &EditorEvent, window, cx| match event {
                EditorEvent::ContextMenuRequested(request) => {
                    this.open_answer_text_menu(id, request, window, cx);
                }
                EditorEvent::Focused => {
                    // The keyboard is in this answer: it is the marked one.
                    this.selected = Some(id);
                    cx.notify();
                }
                EditorEvent::Blurred => cx.notify(),
                _ => {}
            },
        );
        AnswerView {
            editor,
            shown,
            drawing,
            has_text,
            _subscription: subscription,
        }
    }

    /// Forgets what is shown: the next [`set_messages`](Self::set_messages)
    /// is another conversation (its message ids may be the same ones), and
    /// starts at its end with nothing marked.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.messages.clear();
        self.views.clear();
        self.list.reset(0);
        self.selected = None;
        self.shown = None;
        self.selected_example = 0;
        self.follow = true;
        self.scrolled.set(false);
        cx.notify();
    }

    /// The messages shown.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// The message with this id.
    pub fn message(&self, id: MessageId) -> Option<&Message> {
        self.messages.iter().find(|message| message.id == id)
    }

    /// The editor that shows the text of an answer (an answer without text
    /// has none).
    pub fn answer_editor(&self, id: MessageId) -> Option<&Entity<Editor>> {
        self.views.get(&id).map(|view| &view.editor)
    }

    /// The actions of an answer.
    pub fn actions(&self) -> &[AnswerAction] {
        &self.actions
    }

    /// Sets the actions of an answer (the same for every answer).
    pub fn set_actions(&mut self, actions: Vec<AnswerAction>, cx: &mut Context<Self>) {
        self.actions = actions;
        cx.notify();
    }

    /// Says whether one of the actions can be done now.
    pub fn set_action_enabled(&mut self, action: &str, enabled: bool, cx: &mut Context<Self>) {
        if let Some(action) = self
            .actions
            .iter_mut()
            .find(|candidate| candidate.id.as_ref() == action)
            && action.enabled != enabled
        {
            action.enabled = enabled;
            cx.notify();
        }
    }

    /// Sets what an empty conversation shows.
    pub fn set_empty_state(&mut self, empty: Option<EmptyState>, cx: &mut Context<Self>) {
        if self.empty != empty {
            self.empty = empty;
            self.selected_example = 0;
            cx.notify();
        }
    }

    /// What an empty conversation shows.
    pub fn empty_state(&self) -> Option<&EmptyState> {
        self.empty.as_ref()
    }

    /// Sets how answers are set. The answers shown are set again.
    pub fn set_style(&mut self, style: AnswerStyle, window: &mut Window, cx: &mut Context<Self>) {
        if self.style != style {
            self.style = style;
            for view in self.views.values_mut() {
                view.shown = 0;
            }
            let messages = std::mem::take(&mut self.messages);
            self.set_messages(&messages, window, cx);
        }
    }

    // ----- scrolling -----------------------------------------------------------

    /// Whether the list keeps its end in view.
    pub fn is_following(&self) -> bool {
        self.follow
    }

    /// Brings the end of the conversation into view and keeps it there.
    pub fn scroll_to_bottom(&mut self, cx: &mut Context<Self>) {
        self.follow = true;
        self.scrolled.set(false);
        self.scroll_to_end();
        cx.notify();
    }

    fn scroll_to_end(&self) {
        self.list.scroll_to(ListOffset {
            item_ix: self.messages.len(),
            offset_in_item: px(0.),
        });
    }

    /// How far the list is scrolled from its top, and how far it can be.
    pub fn scroll_position(&self) -> (Pixels, Pixels) {
        (
            -self.list.scroll_px_offset_for_scrollbar().y,
            self.list.max_offset_for_scrollbar().height,
        )
    }

    /// Scrolls the list as the wheel does (a positive distance moves
    /// towards the end).
    pub fn scroll_by(&mut self, distance: Pixels, cx: &mut Context<Self>) {
        self.list.scroll_by(distance);
        self.scrolled.set(true);
        cx.notify();
    }

    /// After the user scrolled: the list follows again once its end is in
    /// view, and stops following when it is not.
    fn settle_follow(&mut self) {
        if self.scrolled.take() {
            let (offset, max) = self.scroll_position();
            self.follow = offset + px(2.) >= max;
        }
    }

    /// How many rows were built so far (only the rows near the visible
    /// part of a long conversation are).
    pub fn rows_built(&self) -> usize {
        self.rows_built
    }

    // ----- the marked message ----------------------------------------------------

    /// The marked message.
    pub fn selected(&self) -> Option<MessageId> {
        self.selected
    }

    /// The marked example prompt of an empty conversation.
    pub fn selected_example(&self) -> usize {
        self.selected_example
    }

    /// Marks a message (or none) and brings it into view.
    pub fn select(&mut self, message: Option<MessageId>, cx: &mut Context<Self>) {
        self.selected = message.filter(|id| self.index_of(*id).is_some());
        if self.shown != self.selected {
            self.shown = None;
        }
        if let Some(index) = self.selected.and_then(|id| self.index_of(id)) {
            self.list.scroll_to_reveal_item(index);
            // Marking an older message is scrolling away from the end.
            self.follow = index + 1 == self.messages.len() && self.follow;
        }
        cx.notify();
    }

    /// Marks `message`, brings it into view and shows the mark while the
    /// keyboard is elsewhere too: a command run from outside the chat (the
    /// palette's "Draw answer on whiteboard") says which answer it will act
    /// on, and the chat's key acts on it, until another message is marked or
    /// a new one comes. The keyboard does not move.
    pub fn mark_shown(&mut self, message: MessageId, cx: &mut Context<Self>) {
        self.select(Some(message), cx);
        self.shown = self.selected;
    }

    /// The message marked with [`MessageList::mark_shown`], while it is.
    pub fn shown(&self) -> Option<MessageId> {
        self.shown
    }

    /// The mark of [`MessageList::mark_shown`] is no longer shown without the
    /// keyboard (the answer it pointed at was acted on).
    pub fn clear_shown(&mut self, cx: &mut Context<Self>) {
        if self.shown.take().is_some() {
            cx.notify();
        }
    }

    fn index_of(&self, id: MessageId) -> Option<usize> {
        self.messages.iter().position(|message| message.id == id)
    }

    fn selected_index(&self) -> Option<usize> {
        self.selected.and_then(|id| self.index_of(id))
    }

    fn example_count(&self) -> usize {
        self.empty.as_ref().map_or(0, |empty| empty.examples.len())
    }

    fn move_selection(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.messages.is_empty() {
            let count = self.example_count();
            if count > 0 {
                self.selected_example = if forward {
                    (self.selected_example + 1).min(count - 1)
                } else {
                    self.selected_example.saturating_sub(1)
                };
                cx.notify();
            }
            return;
        }
        let last = self.messages.len() - 1;
        let index = match (self.selected_index(), forward) {
            (None, _) => last,
            (Some(index), true) => (index + 1).min(last),
            (Some(index), false) => index.saturating_sub(1),
        };
        self.select(Some(self.messages[index].id), cx);
    }

    /// Marks the message after the marked one (the last one when none is).
    pub fn select_next(&mut self, cx: &mut Context<Self>) {
        self.move_selection(true, cx);
    }

    /// Marks the message before the marked one.
    pub fn select_previous(&mut self, cx: &mut Context<Self>) {
        self.move_selection(false, cx);
    }

    /// Marks the first message.
    pub fn select_first(&mut self, cx: &mut Context<Self>) {
        if self.messages.is_empty() {
            self.selected_example = 0;
            cx.notify();
        } else {
            self.select(self.messages.first().map(|message| message.id), cx);
        }
    }

    /// Marks the last message.
    pub fn select_last(&mut self, cx: &mut Context<Self>) {
        if self.messages.is_empty() {
            self.selected_example = self.example_count().saturating_sub(1);
            cx.notify();
        } else {
            self.select(self.messages.last().map(|message| message.id), cx);
        }
    }

    /// Moves the keyboard to the list. The last message is marked when
    /// none was.
    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        if self.selected.is_none()
            && let Some(last) = self.messages.last()
        {
            self.selected = Some(last.id);
            self.list
                .scroll_to_reveal_item(self.messages.len().saturating_sub(1));
        }
        cx.notify();
    }

    /// Whether the keyboard is in the list itself (not in the text of an
    /// answer).
    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    /// The answer whose text has the keyboard, if one has.
    pub fn focused_answer(&self, window: &Window, cx: &App) -> Option<MessageId> {
        self.views
            .iter()
            .find(|(_, view)| view.editor.read(cx).is_focused(window))
            .map(|(id, _)| *id)
    }

    /// Whether the keyboard is in the list or in the text of an answer.
    pub fn contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.is_focused(window) || self.focused_answer(window, cx).is_some()
    }

    /// The answer a key of the host acts on: the marked answer while the
    /// keyboard is in the list or in the text of an answer, else the
    /// latest answer. `None` when that answer cannot be applied (it is
    /// still being written, it is empty, the marked message is the
    /// user's).
    pub fn target_answer(&self, window: &Window, cx: &App) -> Option<&Message> {
        let marked = if self.contains_focus(window, cx) {
            self.selected.and_then(|id| self.message(id))
        } else {
            self.shown.and_then(|id| self.message(id))
        };
        let message = match marked {
            Some(message) => message,
            None => self
                .messages
                .iter()
                .rev()
                .find(|message| message.role == Role::Assistant)?,
        };
        message.is_applicable().then_some(message)
    }

    // ----- what the user does ----------------------------------------------------

    /// Does what a click on an action's button does: tells the host
    /// ([`MessageListEvent::ActionInvoked`]). Nothing happens for an action
    /// that is disabled or an answer that cannot be applied. Returns
    /// whether the host was told.
    pub fn invoke_action(
        &mut self,
        action: &str,
        message: MessageId,
        cx: &mut Context<Self>,
    ) -> bool {
        let applicable = self.message(message).is_some_and(Message::is_applicable);
        if !applicable || self.action_state(action, message) != ActionState::Enabled {
            return false;
        }
        let Some(action) = self
            .actions
            .iter()
            .find(|candidate| candidate.id.as_ref() == action)
            .map(|candidate| candidate.id.clone())
        else {
            return false;
        };
        cx.emit(MessageListEvent::ActionInvoked { action, message });
        true
    }

    /// Hands an example prompt to the host.
    pub fn choose_example(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(example) = self
            .empty
            .as_ref()
            .and_then(|empty| empty.examples.get(index))
        {
            cx.emit(MessageListEvent::ExampleChosen(example.clone()));
        }
    }

    fn copy_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(message) = self.selected.filter(|id| self.message(*id).is_some()) {
            cx.emit(MessageListEvent::CopyRequested { message });
        }
    }

    fn activate_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.messages.is_empty() {
            self.choose_example(self.selected_example, cx);
        } else {
            self.open_menu_from_keyboard(window, cx);
        }
    }

    /// The left button went down on a row.
    fn row_pressed(
        &mut self,
        id: MessageId,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if std::mem::take(&mut self.control_pressed) {
            // A button of the row was pressed: the keyboard stays where it
            // is.
            return;
        }
        self.selected = Some(id);
        // A press in the text of the answer belongs to its editor, which
        // takes the keyboard itself.
        if !self.point_in_answer_text(id, event.position, cx) {
            window.focus(&self.focus_handle);
        }
        cx.notify();
    }

    fn point_in_answer_text(&self, id: MessageId, position: Point<Pixels>, cx: &App) -> bool {
        self.views
            .get(&id)
            .and_then(|view| view.editor.read(cx).viewport_bounds())
            .is_some_and(|bounds| bounds.contains(&position))
    }

    // ----- menus -----------------------------------------------------------------

    /// The items of the menu of a message. `selection_in` is the editor of
    /// the answer when the menu was asked for in its text with text
    /// selected: "Copy" then copies the selection.
    fn menu_items(
        &self,
        id: MessageId,
        selection_in: Option<FocusHandle>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<ContextMenuItem> {
        let Some(message) = self.message(id) else {
            return Vec::new();
        };
        let list = cx.weak_entity();
        let mut items = Vec::new();
        match message.role {
            Role::User => {
                let list = list.clone();
                items.push(
                    ContextMenuItem::new("Copy", move |_, cx| {
                        list.update(cx, |_, cx| {
                            cx.emit(MessageListEvent::CopyRequested { message: id })
                        })
                        .ok();
                    })
                    .key_hint(KeyHint::for_action_at(
                        &CopyMessage,
                        &self.focus_handle,
                        window,
                        cx,
                    )),
                );
            }
            Role::Assistant => {
                if selection_in.is_some() {
                    items.push(ContextMenuItem::action(
                        "Copy Selection",
                        innerword_editor::actions::Copy,
                    ));
                    items.push(ContextMenuItem::separator());
                }
                let applicable = message.is_applicable();
                let facts = self.facts(id);
                // The actions in their order, those that come last in the
                // menu after the others.
                let ordered = self
                    .actions
                    .iter()
                    .filter(|action| !action.menu_last)
                    .chain(self.actions.iter().filter(|action| action.menu_last));
                for action in ordered {
                    let (state, tooltip) = match &facts {
                        Some(facts) => (action.state(facts), action.words(facts).menu),
                        None => (ActionState::Enabled, action.tooltip.clone()),
                    };
                    let (title, disabled): (SharedString, bool) = match state {
                        ActionState::Hidden => continue,
                        ActionState::Enabled => (tooltip, !applicable),
                        ActionState::Disabled(reason) if reason.is_empty() => (tooltip, true),
                        ActionState::Disabled(reason) => (
                            format!("{tooltip} ({})", cut(reason.trim_end_matches('.'), 60)).into(),
                            true,
                        ),
                    };
                    let list = list.clone();
                    let action_id = action.id.clone();
                    items.push(
                        ContextMenuItem::new(title, move |_, cx| {
                            list.update(cx, |list, cx| {
                                list.invoke_action(&action_id, id, cx);
                            })
                            .ok();
                        })
                        .disabled(disabled)
                        .key_hint(action.key_hint_at(
                            &self.focus_handle,
                            window,
                            cx,
                        )),
                    );
                }
                if matches!(message.state, MessageState::Failed(_)) {
                    let list = list.clone();
                    items.push(ContextMenuItem::separator());
                    items.push(ContextMenuItem::new("Try Again", move |_, cx| {
                        list.update(cx, |_, cx| {
                            cx.emit(MessageListEvent::RetryRequested { message: id })
                        })
                        .ok();
                    }));
                }
            }
        }
        items
    }

    /// Opens the menu of a message at a point (a right-click).
    pub fn open_menu(
        &mut self,
        id: MessageId,
        position: Point<Pixels>,
        from_keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.message(id).is_none() {
            return;
        }
        self.selected = Some(id);
        let items = self.menu_items(id, None, window, cx);
        let menu = ContextMenu::open(position, items, window, cx);
        if from_keyboard {
            menu.update(cx, |menu, cx| menu.select_first(cx));
        }
        cx.notify();
    }

    /// `enter`, `ctrl-enter` or `shift-f10` in the list: the menu of the
    /// marked message, at its top left corner (or the list's, when the
    /// message begins above it).
    fn open_menu_from_keyboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.selected_index() else {
            return;
        };
        let id = self.messages[index].id;
        let viewport = self.list.viewport_bounds();
        let row: Bounds<Pixels> = self.list.bounds_for_item(index).unwrap_or(viewport);
        let position = point(
            row.left() + px(24.),
            row.top().max(viewport.top()) + px(20.),
        );
        self.open_menu(id, position, true, window, cx);
    }

    /// A right-click (or `ctrl-enter`) in the text of an answer.
    fn open_answer_text_menu(
        &mut self,
        id: MessageId,
        request: &ContextMenuRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.views.get(&id) else {
            return;
        };
        self.selected = Some(id);
        let target = view.editor.focus_handle(cx);
        let items = self.menu_items(
            id,
            request.has_selection.then(|| target.clone()),
            window,
            cx,
        );
        let position = request.position;
        let from_keyboard = request.from_keyboard;
        // After this update: the editor is in the middle of emitting.
        window.defer(cx, move |window, cx| {
            let menu = ContextMenu::open_for(&target, position, items, window, cx);
            if from_keyboard {
                menu.update(cx, |menu, cx| menu.select_first(cx));
            }
        });
        cx.notify();
    }

    // ----- drawing ---------------------------------------------------------------

    fn render_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.rows_built += 1;
        let Some(message) = self.messages.get(index).cloned() else {
            return div().into_any_element();
        };
        let colors = cx.theme().colors;
        let id = message.id;
        let last = index + 1 == self.messages.len();
        let text_focused = self
            .views
            .get(&id)
            .is_some_and(|view| view.editor.read(cx).is_focused(window));
        let marked = self.selected == Some(id)
            && (self.focus_handle.is_focused(window) || text_focused || self.shown == Some(id));
        let group = SharedString::from(format!("assistant-message-{}", id.0));
        let selector = group.to_string();
        let content = match message.role {
            Role::User => self.render_question(&message, &colors).into_any_element(),
            Role::Assistant => self
                .render_answer(&message, last, marked, group.clone(), &colors, window, cx)
                .into_any_element(),
        };
        div()
            .id(("assistant-message", id.0 as usize))
            .debug_selector(move || selector)
            .group(group)
            .relative()
            .px(SPACE_3)
            .pt(if index == 0 { SPACE_3 } else { SPACE_1 })
            .when(last, |row| row.pb(SPACE_2))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.row_pressed(id, event, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    // In the text of an answer the editor asks for the menu.
                    if !this.point_in_answer_text(id, event.position, cx) {
                        this.open_menu(id, event.position, false, window, cx);
                    }
                }),
            )
            .when(marked, |row| {
                // The mark of the keyboard: a thin line in the margin.
                row.child(
                    div()
                        .debug_selector(|| "assistant-message-mark".into())
                        .absolute()
                        .left(px(4.))
                        .top(if index == 0 { SPACE_3 } else { SPACE_1 })
                        .bottom(if message.role == Role::Assistant {
                            SPACE_2
                        } else {
                            px(0.)
                        })
                        .w(px(2.))
                        .rounded(px(1.))
                        .bg(colors.accent),
                )
            })
            .child(content)
            .into_any_element()
    }

    /// A message of the user: a quiet box, the quoted selection above the
    /// question when there is one (the quick chat).
    fn render_question(&self, message: &Message, colors: &ThemeColors) -> Div {
        let quote = message
            .quote
            .as_ref()
            .filter(|quote| !quote.trim().is_empty());
        div().pb(SPACE_2).child(
            div()
                .rounded(RADIUS_LG)
                .bg(colors.hover)
                .px(px(10.))
                .py(px(7.))
                .text_size(UI_TEXT_BASE)
                .line_height(UI_LINE_HEIGHT)
                .text_color(colors.ink)
                .children(quote.map(|quote| {
                    div()
                        .mb(px(5.))
                        .pl(SPACE_2)
                        .border_l_2()
                        .border_color(colors.border_strong)
                        .text_size(UI_TEXT_SM)
                        .line_height(px(17.))
                        .text_color(colors.muted)
                        .line_clamp(3)
                        .child(SharedString::from(quote.trim().to_string()))
                }))
                .child(SharedString::from(message.text.clone())),
        )
    }

    /// An answer: its text, what went wrong if something did, and the line
    /// under it.
    #[allow(clippy::too_many_arguments)]
    fn render_answer(
        &self,
        message: &Message,
        last: bool,
        marked: bool,
        group: SharedString,
        colors: &ThemeColors,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let id = message.id;
        let streaming = message.is_streaming();
        let has_text = !message.text.trim().is_empty();
        let mut answer = div().px(px(2.));
        if has_text && let Some(view) = self.views.get(&id) {
            answer = answer.child(view.editor.clone());
        }
        if let Some(error) = message.error() {
            let notice = Notice::for_error(error, &self.info);
            let name = format!("assistant-failed-{}", id.0);
            answer = answer.child(
                div().mt(if has_text { SPACE_2 } else { px(0.) }).child(
                    div()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, _, _| {
                                this.control_pressed = true;
                            }),
                        )
                        .child(notice_row(name, &notice, false, colors).on_action_click(
                            cx.listener(move |_, _: &ClickEvent, _, cx| {
                                cx.emit(MessageListEvent::RetryRequested { message: id });
                            }),
                        )),
                ),
            );
        }

        // The line under the answer.
        let mut footer = div()
            .flex()
            .items_center()
            .gap(SPACE_1)
            .h(FOOTER_HEIGHT)
            .text_size(UI_TEXT_SM)
            .line_height(px(16.))
            .text_color(colors.muted);
        if streaming {
            let label: SharedString = match message.current_activity() {
                Some(activity) => activity.to_string().into(),
                None if has_text => "Writing".into(),
                None => "Thinking".into(),
            };
            let name = format!("assistant-activity-{}", id.0);
            footer = footer.child(
                div()
                    .debug_selector(move || name)
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .size(px(6.))
                            .rounded_full()
                            .bg(colors.accent)
                            .with_animation(
                                "assistant-pulse",
                                Animation::new(Duration::from_millis(1100))
                                    .repeat()
                                    .with_easing(gpui::pulsating_between(0.3, 1.)),
                                |dot, delta| dot.opacity(delta),
                            ),
                    )
                    .child(label),
            );
        } else {
            if message.state == MessageState::Stopped {
                let name = format!("assistant-stopped-{}", id.0);
                footer = footer.child(
                    div()
                        .debug_selector(move || name)
                        .mr(SPACE_1)
                        .text_size(UI_TEXT_XS)
                        .text_color(colors.faint)
                        .child("Stopped"),
                );
            }
            if message.is_applicable() && !self.actions.is_empty() {
                let always = last || marked;
                let mut buttons = div()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    // The words of a ghost button start where the text
                    // starts.
                    .ml(px(-6.))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _, _| {
                            this.control_pressed = true;
                        }),
                    );
                if !always {
                    buttons = buttons
                        .opacity(0.)
                        .group_hover(group, |style| style.opacity(1.));
                }
                let facts = self.facts(id);
                for action in &self.actions {
                    let (state, ActionWords { label, tooltip, .. }) = match &facts {
                        Some(facts) => (action.state(facts), action.words(facts)),
                        None => (ActionState::Enabled, action.plain_words()),
                    };
                    let (enabled, tooltip) = match state {
                        ActionState::Hidden => continue,
                        ActionState::Enabled => (true, tooltip),
                        ActionState::Disabled(reason) if reason.is_empty() => (false, tooltip),
                        ActionState::Disabled(reason) => (false, reason),
                    };
                    let name = format!("assistant-action-{}-{}", action.id, id.0);
                    let action_id = action.id.clone();
                    let button = text_button(
                        SharedString::from(name.clone()),
                        name,
                        label,
                        None,
                        Some((tooltip, action.key_hint_at(&self.focus_handle, window, cx))),
                        Look::Ghost,
                        !enabled,
                        false,
                        colors,
                    );
                    buttons = buttons.child(if enabled {
                        button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.invoke_action(&action_id, id, cx);
                        }))
                    } else {
                        button
                    });
                }
                footer = footer.child(buttons);
            }
        }
        answer.child(footer)
    }

    /// What an empty conversation shows.
    fn render_empty(
        &self,
        list_focused: bool,
        colors: &ThemeColors,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some(empty) = self.empty.as_ref() else {
            return div().size_full();
        };
        let mut body = div()
            .debug_selector(|| "assistant-empty".into())
            .px(SPACE_3)
            .pt(px(28.))
            .child(
                div()
                    .text_size(UI_TEXT_BASE)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors.ink)
                    .child(empty.title.clone()),
            );
        if !empty.text.is_empty() {
            body = body.child(
                div()
                    .mt(SPACE_1)
                    .text_size(UI_TEXT_SM)
                    .line_height(px(17.))
                    .text_color(colors.muted)
                    .child(empty.text.clone()),
            );
        }
        if !empty.note.is_empty() {
            body = body.child(
                div()
                    .debug_selector(|| "assistant-empty-note".into())
                    .mt(SPACE_2)
                    .text_size(UI_TEXT_SM)
                    .line_height(px(17.))
                    .text_color(colors.muted)
                    .italic()
                    .child(empty.note.clone()),
            );
        }
        for (index, example) in empty.examples.iter().enumerate() {
            let name = format!("assistant-example-{index}");
            let marked = list_focused && self.selected_example == index;
            let hover = colors.hover;
            let ink = colors.ink;
            let row = div()
                .id(("assistant-example", index))
                .debug_selector(move || name)
                .mt(if index == 0 { SPACE_3 } else { px(6.) })
                .px(px(10.))
                .py(px(7.))
                .rounded(RADIUS_MD)
                .border_1()
                .border_color(if marked {
                    colors.accent
                } else {
                    colors.hairline
                })
                .when(marked, |row| row.bg(colors.accent_subtle))
                .text_size(UI_TEXT_SM)
                .line_height(px(17.))
                .text_color(if marked { colors.ink } else { colors.muted })
                .cursor_pointer()
                .hover(move |style| style.bg(hover).text_color(ink))
                .child(example.clone());
            body = body.child(keep_focus(row).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.selected_example = index;
                    this.choose_example(index, cx);
                },
            )));
        }
        body
    }
}

impl Render for MessageList {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let ui_font = theme.fonts.ui.clone();
        let list_focused = self.focus_handle.is_focused(window);
        self.settle_follow();
        if self.follow && !self.messages.is_empty() {
            // Whatever made the content taller since (a resize, a font).
            self.scroll_to_end();
        }
        let content: AnyElement = if self.messages.is_empty() {
            self.render_empty(list_focused, &colors, cx)
                .into_any_element()
        } else {
            let this = cx.weak_entity();
            list(self.list.clone(), move |index, window, cx| {
                this.update(cx, |this, cx| this.render_row(index, window, cx))
                    .unwrap_or_else(|_| div().into_any_element())
            })
            .size_full()
            .into_any_element()
        };
        div()
            .id("assistant-messages")
            .debug_selector(|| "assistant-messages".into())
            .key_context(MESSAGES_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &SelectNextMessage, _, cx| this.select_next(cx)))
            .on_action(
                cx.listener(|this, _: &SelectPreviousMessage, _, cx| this.select_previous(cx)),
            )
            .on_action(cx.listener(|this, _: &SelectFirstMessage, _, cx| this.select_first(cx)))
            .on_action(cx.listener(|this, _: &SelectLastMessage, _, cx| this.select_last(cx)))
            .on_action(cx.listener(|this, _: &CopyMessage, _, cx| this.copy_selected(cx)))
            .on_action(cx.listener(|this, _: &ActivateSelected, window, cx| {
                this.activate_selected(window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenContextMenu, window, cx| {
                if this.messages.is_empty() {
                    cx.propagate();
                } else {
                    this.open_menu_from_keyboard(window, cx);
                }
            }))
            .on_action(cx.listener(|_, _: &DismissChat, _, cx| {
                cx.emit(MessageListEvent::Dismissed);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, _| {
                    // Whatever a row made of the press, it is over.
                    this.control_pressed = false;
                }),
            )
            .size_full()
            .font_family(ui_font)
            .text_size(UI_TEXT_BASE)
            .line_height(UI_LINE_HEIGHT)
            .text_color(colors.ink)
            .child(content)
    }
}
