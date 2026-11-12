//! Shared fixtures.
//!
//! [`TestChat`] is a chat put together from the shared components the way
//! the crate docs tell a host to (it is what the quick chat does): a
//! `ChatSession` on the fake backend, a message list, a composer, the two
//! pickers. The window mounts the menu layer itself, as a window without a
//! toast layer must.

use std::sync::Arc;

use gpui::{
    Bounds, Context, Entity, IntoElement, Modifiers, MouseButton, Pixels, Point, Render,
    TestAppContext, VisualTestContext, Window, div, prelude::*, px,
};
use writer_assistant::{ChatSession, FakeBackend, MessageId, Selection};
use writer_assistant_ui::{
    AnswerAction, ChatKeys, Composer, ComposerEvent, ComposerOptions, ControlBounds, FocusNextArea,
    FocusPreviousArea, MessageList, MessageListEvent, MessageListOptions, PickerKind, StopAnswer,
    open_picker_menu, picker_button,
};
use writer_ui::theme::{self, ActiveTheme, ThemeMode};

/// Width of the column the chat is given.
pub const CHAT_WIDTH: Pixels = px(360.);
/// Height of the column the chat is given.
pub const CHAT_HEIGHT: Pixels = px(520.);

pub fn setup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        theme::init(ThemeMode::Light, cx);
        writer_ui::init(cx);
        writer_editor::init(cx);
        writer_assistant_ui::init(cx);
    });
}

pub struct TestChat {
    pub session: ChatSession,
    pub messages: Entity<MessageList>,
    pub composer: Entity<Composer>,
    pub list_events: Vec<MessageListEvent>,
    pub composer_events: Vec<ComposerEvent>,
    pub selections: Vec<Selection>,
    pub quote: Option<String>,
    pub model_bounds: ControlBounds,
    pub effort_bounds: ControlBounds,
    pub focus_moves: Vec<bool>,
}

impl TestChat {
    pub fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let messages = self.session.transcript().messages.clone();
        self.messages
            .update(cx, |list, cx| list.set_messages(&messages, window, cx));
        let busy = self.session.is_answering();
        self.composer
            .update(cx, |composer, cx| composer.set_busy(busy, cx));
        cx.notify();
    }

    pub fn open_picker(
        &mut self,
        kind: PickerKind,
        from_keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let catalog = self.session.backend().catalog();
        let bounds = match kind {
            PickerKind::Model => self.model_bounds.clone(),
            PickerKind::Effort => self.effort_bounds.clone(),
        };
        let this = cx.weak_entity();
        open_picker_menu(
            kind,
            &catalog,
            &self.session.selection().clone(),
            &bounds,
            from_keyboard,
            move |selection, _, cx| {
                this.update(cx, |this, cx| {
                    this.session.set_selection(selection.clone());
                    this.selections.push(selection);
                    cx.notify();
                })
                .ok();
            },
            window,
            cx,
        );
    }
}

impl Render for TestChat {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        let catalog = self.session.backend().catalog();
        let selection = self.session.selection().clone();
        div()
            .key_context("TestChat")
            .size_full()
            .on_action(cx.listener(|this, _: &FocusNextArea, _, _| this.focus_moves.push(true)))
            .on_action(
                cx.listener(|this, _: &FocusPreviousArea, _, _| this.focus_moves.push(false)),
            )
            .on_action(cx.listener(|this, _: &StopAnswer, _, _| this.session.stop()))
            .child(
                div()
                    .debug_selector(|| "chat".into())
                    .w(CHAT_WIDTH)
                    .h(CHAT_HEIGHT)
                    .flex()
                    .flex_col()
                    .bg(colors.sidebar)
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .child(
                                picker_button(
                                    PickerKind::Model,
                                    &catalog,
                                    &selection,
                                    &self.model_bounds,
                                    false,
                                    &colors,
                                    cx,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.open_picker(PickerKind::Model, false, window, cx)
                                    },
                                )),
                            )
                            .child(
                                picker_button(
                                    PickerKind::Effort,
                                    &catalog,
                                    &selection,
                                    &self.effort_bounds,
                                    false,
                                    &colors,
                                    cx,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.open_picker(PickerKind::Effort, false, window, cx)
                                    },
                                )),
                            ),
                    )
                    .child(div().flex_1().min_h_0().child(self.messages.clone()))
                    .child(div().flex_none().p_2().child(self.composer.clone())),
            )
            .child(writer_ui::ContextMenuLayer)
    }
}

pub struct ChatFixture {
    pub chat: Entity<TestChat>,
    pub messages: Entity<MessageList>,
    pub composer: Entity<Composer>,
    pub fake: FakeBackend,
}

/// Opens an active window with a [`TestChat`] on a new fake backend; the
/// keyboard is in the composer.
pub fn open_chat(cx: &mut TestAppContext) -> (ChatFixture, &mut VisualTestContext) {
    setup(cx);
    let fake = FakeBackend::new();
    open_chat_on(fake, cx)
}

pub fn open_chat_on(
    fake: FakeBackend,
    cx: &mut TestAppContext,
) -> (ChatFixture, &mut VisualTestContext) {
    let backend = Arc::new(fake.clone());
    let (chat, cx) = cx.add_window_view(move |window, cx| {
        let session = ChatSession::quick(backend.clone());
        let keys = ChatKeys::new("assistant_test");
        let info = writer_assistant::Backend::info(&*backend);
        let messages = cx.new(|cx| {
            let mut list =
                MessageList::new(MessageListOptions::new(info).keys(keys.clone()), window, cx);
            list.set_actions(
                vec![
                    AnswerAction::new("replace", "Replace")
                        .tooltip("Replace Selection")
                        .key_of(writer_assistant_ui::ReplaceSelection),
                    AnswerAction::new("insert", "Insert").tooltip("Insert Below"),
                    AnswerAction::new("copy", "Copy")
                        .tooltip("Copy Answer")
                        .key_of(writer_assistant_ui::CopyAnswer),
                ],
                cx,
            );
            list
        });
        let composer =
            cx.new(|cx| Composer::new(ComposerOptions::new("Ask").keys(keys), window, cx));
        cx.subscribe_in(
            &composer,
            window,
            |this: &mut TestChat, _, event: &ComposerEvent, window, cx| {
                this.composer_events.push(event.clone());
                match event {
                    ComposerEvent::Submitted(text) => {
                        let quote = this.quote.clone();
                        this.session.ask_quick(text, quote.as_deref());
                        this.sync(window, cx);
                    }
                    ComposerEvent::StopRequested => this.session.stop(),
                    _ => {}
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &messages,
            window,
            |this: &mut TestChat, _, event: &MessageListEvent, _, _| {
                this.list_events.push(event.clone());
            },
        )
        .detach();
        let events = session.events();
        cx.spawn_in(window, async move |this, cx| {
            while let Some(batch) = events.next_batch().await {
                let alive = this.update_in(cx, |this, window, cx| {
                    let mut changed = false;
                    for event in batch {
                        changed |= this.session.apply(event);
                    }
                    if changed {
                        this.sync(window, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        composer.read(cx).focus(window, cx);
        TestChat {
            session,
            messages,
            composer,
            list_events: Vec::new(),
            composer_events: Vec::new(),
            selections: Vec::new(),
            quote: None,
            model_bounds: ControlBounds::new(),
            effort_bounds: ControlBounds::new(),
            focus_moves: Vec::new(),
        }
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let (messages, composer) =
        chat.read_with(cx, |chat, _| (chat.messages.clone(), chat.composer.clone()));
    (
        ChatFixture {
            chat,
            messages,
            composer,
            fake,
        },
        cx,
    )
}

impl ChatFixture {
    /// Types a message into the composer and sends it.
    pub fn send(&self, text: &str, cx: &mut VisualTestContext) {
        self.composer
            .update_in(cx, |composer, window, cx| composer.focus(window, cx));
        cx.simulate_input(text);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
    }

    pub fn ids(&self, cx: &mut VisualTestContext) -> Vec<MessageId> {
        self.messages.read_with(cx, |list, _| {
            list.messages().iter().map(|message| message.id).collect()
        })
    }

    pub fn last_id(&self, cx: &mut VisualTestContext) -> MessageId {
        *self.ids(cx).last().expect("a message")
    }

    /// The text the editor of an answer shows.
    pub fn answer_text(&self, id: MessageId, cx: &mut VisualTestContext) -> String {
        self.messages.read_with(cx, |list, cx| {
            list.answer_editor(id)
                .map(|editor| editor.read(cx).plain_text())
                .unwrap_or_default()
        })
    }

    pub fn list_events(&self, cx: &mut VisualTestContext) -> Vec<MessageListEvent> {
        self.chat.read_with(cx, |chat, _| chat.list_events.clone())
    }

    pub fn composer_events(&self, cx: &mut VisualTestContext) -> Vec<ComposerEvent> {
        self.chat
            .read_with(cx, |chat, _| chat.composer_events.clone())
    }
}

pub fn try_bounds(
    selector: impl Into<String>,
    cx: &mut VisualTestContext,
) -> Option<Bounds<Pixels>> {
    // `debug_bounds` wants a `&'static str`; a test may leak a few names.
    let selector: &'static str = Box::leak(selector.into().into_boxed_str());
    cx.debug_bounds(selector)
}

pub fn bounds(selector: impl Into<String>, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    let selector = selector.into();
    try_bounds(selector.clone(), cx).unwrap_or_else(|| panic!("nothing was painted for {selector}"))
}

/// Moves the pointer to `position` (no button held).
pub fn move_to(position: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_mouse_move(position, None, Modifiers::none());
    cx.run_until_parked();
}

/// Moves the pointer onto the element and clicks it, as a hand does.
pub fn click(selector: impl Into<String>, cx: &mut VisualTestContext) {
    let center = bounds(selector, cx).center();
    click_at(center, cx);
}

pub fn click_at(position: Point<Pixels>, cx: &mut VisualTestContext) {
    move_to(position, cx);
    cx.simulate_click(position, Modifiers::none());
    cx.run_until_parked();
}

pub fn right_click_at(position: Point<Pixels>, cx: &mut VisualTestContext) {
    move_to(position, cx);
    cx.simulate_mouse_down(position, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(position, MouseButton::Right, Modifiers::none());
    cx.run_until_parked();
}

/// The labels of the open context menu ("-" for a separator).
pub fn menu_labels(cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|window, cx| {
        writer_ui::ContextMenu::current(window, cx)
            .map(|menu| menu.read(cx).labels())
            .unwrap_or_default()
    })
}

pub fn menu_is_open(cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| writer_ui::ContextMenu::is_open(window, cx))
}

/// Chooses an item of the open context menu, as a click on it does.
pub fn choose(title: &str, cx: &mut VisualTestContext) {
    let chosen = cx.update(|window, cx| writer_ui::ContextMenu::choose(title, window, cx));
    assert!(chosen, "the menu has no item {title:?}");
    cx.run_until_parked();
}
