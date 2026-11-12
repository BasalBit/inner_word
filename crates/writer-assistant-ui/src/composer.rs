//! [`Composer`]: where a message is written.
//!
//! A compact editor in a quiet box that grows with what is typed, up to a
//! limit, and a round button beside it: Send, or Stop while an answer is
//! being written.
//!
//! | Key | What it does |
//! |---|---|
//! | `enter` | sends ([`ComposerEvent::Submitted`]); nothing happens while the field is empty |
//! | `shift-enter` | a new line |
//! | `escape` | hands the keyboard back ([`ComposerEvent::Dismissed`]); what was typed stays |
//! | `cmd-.` | stops the answer ([`ComposerEvent::StopRequested`]) |
//! | `tab`, `shift-tab` | [`FocusNextArea`](crate::FocusNextArea), [`FocusPreviousArea`](crate::FocusPreviousArea): not handled here, the host moves the keyboard |
//! | `ctrl-enter`, a right-click | the menu of a text field: cut, copy, paste, select all |
//!
//! A message can be sent while an answer is still being written: the
//! engine answers them in order.
//!
//! ```ignore
//! let composer = cx.new(|cx| {
//!     Composer::new(ComposerOptions::new("Ask about the selection").keys(keys.clone()), window, cx)
//! });
//! cx.subscribe_in(&composer, window, |this, composer, event: &ComposerEvent, window, cx| match event {
//!     ComposerEvent::Submitted(text) => this.send(text.clone(), window, cx),
//!     ComposerEvent::StopRequested => this.stop(cx),
//!     ComposerEvent::Dismissed => this.close(window, cx),
//!     ComposerEvent::Changed => {}
//! })
//! .detach();
//! // while an answer is being written:
//! composer.update(cx, |composer, cx| composer.set_busy(true, cx));
//! // render: .child(composer.clone())
//! ```

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, Pixels,
    Render, SharedString, Subscription, Window, div, prelude::*, px,
};
use writer_core::Document;
use writer_editor::{CompactOptions, ContextMenuRequest, Editor, EditorConfig, EditorEvent};
use writer_ui::{
    ContextMenu, ContextMenuItem, Icon, IconName, KeyHint, Tooltip,
    theme::{ActiveTheme, RADIUS_LG, SPACE_1, SPACE_2, UI_LINE_HEIGHT, UI_TEXT_BASE, focus_ring},
};

use crate::{
    ChatKeys,
    actions::{CHAT_EDITOR_FLAG, COMPOSER_CONTEXT, COMPOSER_EDITOR_FLAG, DismissChat, StopAnswer},
    controls::keep_focus,
};

/// How tall the field grows before it scrolls.
pub const COMPOSER_MAX_HEIGHT: Pixels = px(168.);

/// What a [`Composer`] is made with.
#[derive(Clone, Debug)]
pub struct ComposerOptions {
    /// Shown while the field is empty.
    pub placeholder: SharedString,
    /// How tall the field grows before it scrolls.
    pub max_height: Pixels,
    /// The key context flags of the host.
    pub keys: ChatKeys,
}

impl ComposerOptions {
    /// Options with this placeholder.
    pub fn new(placeholder: impl Into<SharedString>) -> Self {
        Self {
            placeholder: placeholder.into(),
            max_height: COMPOSER_MAX_HEIGHT,
            keys: ChatKeys::default(),
        }
    }

    /// Sets how tall the field grows before it scrolls.
    pub fn max_height(mut self, max_height: Pixels) -> Self {
        self.max_height = max_height;
        self
    }

    /// Sets the key context flags of the host.
    pub fn keys(mut self, keys: ChatKeys) -> Self {
        self.keys = keys;
        self
    }
}

/// What a [`Composer`] tells its host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComposerEvent {
    /// `enter` or the Send button, with text in the field. The field has
    /// been emptied. The text is trimmed at its ends.
    Submitted(String),
    /// `cmd-.` or the Stop button.
    StopRequested,
    /// `escape`: the keyboard should go back to where the chat was opened
    /// from.
    Dismissed,
    /// The text changed.
    Changed,
}

type Accessory = Box<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// See the [module docs](self).
pub struct Composer {
    editor: Entity<Editor>,
    busy: bool,
    enabled: bool,
    accessory: Option<Accessory>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ComposerEvent> for Composer {}

impl Focusable for Composer {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Composer {
    /// A composer with an empty field.
    pub fn new(options: ComposerOptions, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = EditorConfig::compact(
            CompactOptions::default()
                .max_height(options.max_height)
                .submit_on_enter(true),
        )
        .placeholder(options.placeholder.clone());
        let editor = cx.new(|cx| {
            let mut editor = Editor::new(Document::new(), config, window, cx);
            editor.add_key_context_flag(CHAT_EDITOR_FLAG, cx);
            editor.add_key_context_flag(COMPOSER_EDITOR_FLAG, cx);
            if let Some(flag) = options.keys.editor_flag.clone() {
                editor.add_key_context_flag(flag, cx);
            }
            editor
        });
        let subscriptions = vec![cx.subscribe_in(
            &editor,
            window,
            |this, _, event: &EditorEvent, window, cx| match event {
                EditorEvent::Submitted => this.submit(window, cx),
                EditorEvent::Edited { .. } | EditorEvent::DocumentReplaced => {
                    cx.emit(ComposerEvent::Changed);
                    cx.notify();
                }
                EditorEvent::Focused | EditorEvent::Blurred => cx.notify(),
                EditorEvent::ContextMenuRequested(request) => this.open_menu(request, window, cx),
                _ => {}
            },
        )];
        Self {
            editor,
            busy: false,
            enabled: true,
            accessory: None,
            _subscriptions: subscriptions,
        }
    }

    /// The editor the message is typed into.
    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    /// What is in the field.
    pub fn text(&self, cx: &App) -> String {
        self.editor.read(cx).plain_text()
    }

    /// Whether the field holds nothing but white space.
    pub fn is_blank(&self, cx: &App) -> bool {
        self.text(cx).trim().is_empty()
    }

    /// Puts `text` into the field, the caret after it.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.set_plain_text(text, cx));
        cx.notify();
    }

    /// Empties the field.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| editor.clear(cx));
        cx.notify();
    }

    /// Changes the words shown while the field is empty.
    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let placeholder = placeholder.into();
        self.editor.update(cx, |editor, cx| {
            if editor.config().placeholder != placeholder {
                editor.update_config(|config| config.placeholder = placeholder, cx);
            }
        });
    }

    /// Moves the keyboard into the field.
    pub fn focus(&self, window: &mut Window, cx: &App) {
        self.editor.read(cx).focus(window);
    }

    /// Whether the keyboard is in the field.
    pub fn is_focused(&self, window: &Window, cx: &App) -> bool {
        self.editor.read(cx).is_focused(window)
    }

    /// Whether an answer is being written (the button is Stop).
    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// Says whether an answer is being written: the button is Stop while
    /// one is, Send otherwise.
    pub fn set_busy(&mut self, busy: bool, cx: &mut Context<Self>) {
        if self.busy != busy {
            self.busy = busy;
            cx.notify();
        }
    }

    /// Whether messages can be sent.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Says whether messages can be sent. While they cannot (the agent is
    /// not signed in) the field still takes text, `enter` does nothing and
    /// the Send button is dimmed; what was typed is kept.
    pub fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.enabled != enabled {
            self.enabled = enabled;
            cx.notify();
        }
    }

    /// Small controls drawn at the left of the row under the field (the
    /// quick chat puts its model and effort pickers there). Called every
    /// frame.
    pub fn set_accessory(
        &mut self,
        accessory: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
        cx: &mut Context<Self>,
    ) {
        self.accessory = Some(Box::new(accessory));
        cx.notify();
    }

    /// Whether Send would send: there is text and sending is possible.
    pub fn can_submit(&self, cx: &App) -> bool {
        self.enabled && !self.is_blank(cx)
    }

    /// What `enter` and the Send button do: emits
    /// [`ComposerEvent::Submitted`] with the text and empties the field.
    /// Nothing happens while the field is blank or sending is not possible.
    pub fn submit(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if !self.can_submit(cx) {
            return;
        }
        let text = self.text(cx).trim().to_string();
        self.editor.update(cx, |editor, cx| editor.clear(cx));
        cx.emit(ComposerEvent::Submitted(text));
        cx.notify();
    }

    /// What `cmd-.` and the Stop button do.
    pub fn request_stop(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            cx.emit(ComposerEvent::StopRequested);
        }
    }

    /// A right-click (or `ctrl-enter`) in the field: the menu of a text
    /// field, with the field's own cut, copy, paste and select all.
    fn open_menu(
        &mut self,
        request: &ContextMenuRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use writer_editor::actions::{Copy, Cut, Paste, SelectAll};
        let editor = self.editor.read(cx);
        let target = editor.focus_handle(cx);
        let empty = editor.is_empty();
        let items = vec![
            ContextMenuItem::action("Cut", Cut).disabled(!request.has_selection),
            ContextMenuItem::action("Copy", Copy).disabled(!request.has_selection),
            ContextMenuItem::action("Paste", Paste),
            ContextMenuItem::separator(),
            ContextMenuItem::action("Select All", SelectAll).disabled(empty),
        ];
        let position = request.position;
        let from_keyboard = request.from_keyboard;
        // After this update: the editor is in the middle of emitting.
        window.defer(cx, move |window, cx| {
            let menu = ContextMenu::open_for(&target, position, items, window, cx);
            if from_keyboard {
                menu.update(cx, |menu, cx| menu.select_first(cx));
            }
        });
    }
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        let ui_font = cx.theme().fonts.ui.clone();
        let focused = self.editor.read(cx).is_focused(window);
        let can_submit = self.can_submit(cx);
        let busy = self.busy;

        // One round button: Stop while an answer is being written, Send
        // otherwise.
        let button = {
            let (name, icon, label, hint, active) = if busy {
                (
                    "assistant-stop",
                    IconName::Stop,
                    "Stop",
                    KeyHint::for_action(&StopAnswer, cx),
                    true,
                )
            } else {
                (
                    "assistant-send",
                    IconName::Send,
                    "Send",
                    KeyHint::parse("enter"),
                    can_submit,
                )
            };
            let hover = colors.accent_hover;
            let button = div()
                .id(name)
                .debug_selector(move || name.into())
                .flex_none()
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .map(|button| {
                    if active {
                        button
                            .bg(colors.accent)
                            .cursor_pointer()
                            .hover(move |style| style.bg(hover))
                    } else {
                        button.bg(colors.hover).cursor_default()
                    }
                })
                .child(Icon::new(icon).size(px(14.)).color(if active {
                    colors.on_accent
                } else {
                    colors.faint
                }))
                .tooltip(Tooltip::new(label).key_hint(hint).build());
            keep_focus(button).on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                if this.busy {
                    this.request_stop(cx);
                } else {
                    this.submit(window, cx);
                }
            }))
        };

        let accessory = self
            .accessory
            .as_ref()
            .map(|accessory| accessory(window, cx));

        div()
            .id("assistant-composer")
            .debug_selector(|| "assistant-composer".into())
            .key_context(COMPOSER_CONTEXT)
            .on_action(cx.listener(|this, _: &StopAnswer, _, cx| {
                if this.busy {
                    this.request_stop(cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|_, _: &DismissChat, _, cx| {
                cx.emit(ComposerEvent::Dismissed);
            }))
            // A click on the box beside the text puts the caret in the text.
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.focus(window, cx);
            }))
            .w_full()
            // A flex column, so that the rows in it are as wide as the box
            // (as blocks they came out as wide as their content while the
            // conversation above was empty, and the button stood in the
            // middle of the box).
            .flex()
            .flex_col()
            .rounded(RADIUS_LG)
            .bg(colors.elevated)
            .border_1()
            .border_color(if focused {
                colors.accent
            } else {
                colors.border_strong
            })
            .when(focused, |composer| composer.shadow(focus_ring(&colors)))
            .cursor_text()
            .font_family(ui_font)
            .text_size(UI_TEXT_BASE)
            .line_height(UI_LINE_HEIGHT)
            .text_color(colors.ink)
            .map(|composer| match accessory {
                // With controls of the host under the field: the text
                // above, a row with the controls and the button below.
                Some(accessory) => composer
                    .child(
                        // A plain block: the editor measures itself at the
                        // width it is given.
                        div().pt(SPACE_2).px(px(10.)).child(self.editor.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(SPACE_1)
                            .px(px(6.))
                            .pt(SPACE_1)
                            .pb(px(6.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(SPACE_1)
                                    .min_w_0()
                                    .child(accessory),
                            )
                            .child(button),
                    ),
                // Alone: the button beside the text, at its last line.
                None => composer.child(
                    div()
                        .flex()
                        .items_end()
                        .gap(SPACE_2)
                        .pl(px(10.))
                        .pr(px(6.))
                        .py(px(6.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                // The text sits on the middle line of the
                                // button while it is one line.
                                .py(px(3.))
                                .child(self.editor.clone()),
                        )
                        .child(button),
                ),
            })
    }
}
