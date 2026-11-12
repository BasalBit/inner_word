//! How the side chat looks, and where its action listeners are attached.
//!
//! ```text
//! root (AssistantPanel)                    action listeners
//! ├── header (AssistantHeader)             the assistant and its engine; model, effort,
//! │                                        new chat, clear, close
//! ├── context row (AssistantContext)       the open document, a chip per added item, Add
//! ├── message list (AssistantMessages)     the conversation; scrolls
//! ├── notices                              not signed in, where the text goes, the agent's own
//! ├── composer (AssistantComposer > Editor)
//! └── the picker over the library          floats under "Add" while it is open
//! ```
//!
//! The header and the context row are one stop of the keyboard each: the
//! arrows move a mark between their controls, `enter` presses the marked
//! one. Every control is also a button for the pointer, and a press on one
//! never moves the keyboard.

use gpui::{
    AnyElement, App, ClickEvent, Context, Corner, Div, FontWeight, MouseButton, MouseDownEvent,
    Pixels, Point, Render, SharedString, Stateful, Window, anchored, deferred, div, point,
    prelude::*, px,
};
use writer_assistant::{Attachment, AttachmentKind};
use writer_ui::{
    ContextMenu, ContextMenuItem, Icon, IconName, KeyHint, Tooltip,
    theme::{
        ActiveTheme, RADIUS_SM, SPACE_1, SPACE_2, SPACE_3, ThemeColors, UI_LINE_HEIGHT,
        UI_TEXT_BASE, UI_TEXT_SM, UI_TEXT_XS,
    },
};

use super::{
    ApplyKind, AssistantPanel, AssistantPanelEvent, HeaderControl, PanelArea, PanelNotice,
    panel_for_actions,
};
use crate::{
    actions::{
        AddContext, CONTEXT_ROW_CONTEXT, ChooseEffort, ChooseModel, ClearChat, CloseAssistant,
        CopyAnswer, DismissChat, FocusComposer, FocusNextArea, FocusPreviousArea, HEADER_CONTEXT,
        InsertAtCaret, NewChat, NextControl, PANEL_CONTEXT, PressControl, PreviousControl,
        RemoveContext, ReplaceSelection, StopAnswer,
    },
    controls::{Look, icon_button, keep_focus, keyboard_mark, text_button},
    notice::notice_row,
    pickers::{PickerKind, picker_button},
};

/// The height of the header.
pub(crate) const HEADER_HEIGHT: Pixels = px(44.);

/// Registers the global listeners for the assistant's window-wide actions:
/// whatever has the keyboard in a window, an action that no element
/// handled reaches that window's panel while it is on screen. The panel is
/// updated after the current dispatch has finished, because the window is
/// busy until then.
pub(crate) fn global_actions(cx: &mut App) {
    macro_rules! global {
        ($($action:ty => |$panel:ident, $window:ident, $cx:ident| $body:expr),+ $(,)?) => {
            $(
                cx.on_action(|_: &$action, cx: &mut App| {
                    let Some((window, panel)) = panel_for_actions(cx) else {
                        return;
                    };
                    cx.defer(move |cx| {
                        window
                            .update(cx, |_, $window, cx| {
                                panel.update(cx, |$panel, $cx| $body);
                            })
                            .ok();
                    });
                });
            )+
        };
    }
    global!(
        NewChat => |panel, window, cx| panel.new_chat(window, cx),
        ClearChat => |panel, window, cx| panel.clear(window, cx),
        StopAnswer => |panel, _window, cx| panel.stop(cx),
        ChooseModel => |panel, window, cx| panel.open_picker(PickerKind::Model, true, window, cx),
        ChooseEffort => |panel, window, cx| panel.open_picker(PickerKind::Effort, true, window, cx),
        AddContext => |panel, window, cx| panel.open_context_picker(window, cx),
        InsertAtCaret => |panel, window, cx| {
            panel.apply_to_target(ApplyKind::InsertAtCaret, window, cx);
        },
        ReplaceSelection => |panel, window, cx| {
            panel.apply_to_target(ApplyKind::ReplaceSelection, window, cx);
        },
        CopyAnswer => |panel, window, cx| {
            panel.apply_to_target(ApplyKind::Copy, window, cx);
        },
    );
}

impl Render for AssistantPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        let ui_font = cx.theme().fonts.ui.clone();
        let area = self.focused_area(window, cx);

        let header = self.render_header(area == Some(PanelArea::Header), &colors, cx);
        let context = self.render_context_row(area == Some(PanelArea::Context), &colors, cx);
        let notices = self.render_notices(&colors, cx);
        let picker = self.render_picker(cx);

        div()
            .id("assistant-panel")
            .debug_selector(|| "assistant-panel".into())
            .key_context(PANEL_CONTEXT)
            .on_action(cx.listener(|_, _: &DismissChat, _, cx| {
                cx.emit(AssistantPanelEvent::FocusTextRequested);
            }))
            .on_action(
                cx.listener(|this, _: &FocusNextArea, window, cx| {
                    this.cycle_focus(true, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &FocusPreviousArea, window, cx| {
                this.cycle_focus(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &FocusComposer, window, cx| this.focus(window, cx)))
            .on_action(cx.listener(|this, _: &StopAnswer, _, cx| this.stop(cx)))
            .on_action(cx.listener(|this, _: &NewChat, window, cx| this.new_chat(window, cx)))
            .on_action(cx.listener(|this, _: &ClearChat, window, cx| this.clear(window, cx)))
            .on_action(cx.listener(|this, _: &ChooseModel, window, cx| {
                this.open_picker(PickerKind::Model, true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ChooseEffort, window, cx| {
                this.open_picker(PickerKind::Effort, true, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &AddContext, window, cx| {
                    this.open_context_picker(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &InsertAtCaret, window, cx| {
                this.apply_to_target(ApplyKind::InsertAtCaret, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ReplaceSelection, window, cx| {
                this.apply_to_target(ApplyKind::ReplaceSelection, window, cx);
            }))
            .on_action(cx.listener(|this, _: &CopyAnswer, window, cx| {
                this.apply_to_target(ApplyKind::Copy, window, cx);
            }))
            .on_action(cx.listener(|_, _: &CloseAssistant, _, cx| {
                cx.emit(AssistantPanelEvent::CloseRequested);
            }))
            .on_action(
                cx.listener(|this, _: &NextControl, window, cx| {
                    this.move_control(true, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &PreviousControl, window, cx| {
                this.move_control(false, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &PressControl, window, cx| this.press_control(window, cx)),
            )
            .on_action(cx.listener(|this, _: &RemoveContext, window, cx| {
                this.remove_marked_context(window, cx)
            }))
            // A right-click that nothing nearer took: the menu of the panel.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_panel_menu(event.position, window, cx);
                }),
            )
            // The container gives the width and the border beside the text.
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.sidebar)
            .font_family(ui_font)
            .text_size(UI_TEXT_BASE)
            .line_height(UI_LINE_HEIGHT)
            .text_color(colors.ink)
            .child(header)
            .child(context)
            .child(div().flex_1().min_h_0().child(self.messages.clone()))
            .children(notices)
            .child(
                div()
                    .flex_none()
                    // A flex column, so that the composer fills the width
                    // (as a block in a block it came out at the width of
                    // an editor that was never given one, while the
                    // conversation was empty).
                    .flex()
                    .flex_col()
                    .px(SPACE_3)
                    .pt(SPACE_1)
                    .pb(SPACE_3)
                    .child(self.composer.clone()),
            )
            .children(picker)
    }
}

impl AssistantPanel {
    /// The assistant and the engine it runs on; the pickers; new chat,
    /// clear, close.
    fn render_header(
        &mut self,
        focused: bool,
        colors: &ThemeColors,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let catalog = self.catalog();
        let marked = |control: HeaderControl| focused && self.header_control == control;
        let account = self
            .availability
            .as_ref()
            .and_then(|availability| availability.account())
            .map(str::to_string);
        let engine: SharedString = self.info.name.clone().into();
        let engine_tooltip: SharedString = match account {
            Some(account) => format!("Runs on {} ({account})", self.info.name).into(),
            None => format!("Runs on {}, with your own sign-in", self.info.name).into(),
        };

        let model = picker_button(
            PickerKind::Model,
            &catalog,
            &self.selection,
            &self.model_bounds,
            marked(HeaderControl::Model),
            colors,
            cx,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            cx.stop_propagation();
            this.press_header(HeaderControl::Model, false, window, cx);
        }));
        let effort = picker_button(
            PickerKind::Effort,
            &catalog,
            &self.selection,
            &self.effort_bounds,
            marked(HeaderControl::Effort),
            colors,
            cx,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            cx.stop_propagation();
            this.press_header(HeaderControl::Effort, false, window, cx);
        }));

        let button = |this: &Self,
                      control: HeaderControl,
                      name: &'static str,
                      icon: IconName,
                      label: &'static str,
                      hint: Option<KeyHint>,
                      cx: &mut Context<Self>| {
            let enabled = this.header_control_enabled(control);
            let button = icon_button(
                name,
                name,
                icon,
                label,
                hint,
                !enabled,
                focused && this.header_control == control,
                colors,
            );
            if enabled {
                button.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.press_header(control, false, window, cx);
                }))
            } else {
                button
            }
        };
        let new_chat = button(
            self,
            HeaderControl::NewChat,
            "assistant-new-chat",
            IconName::Plus,
            "New Chat",
            KeyHint::for_action(&NewChat, cx),
            cx,
        );
        let clear = button(
            self,
            HeaderControl::Clear,
            "assistant-clear",
            IconName::Trash,
            "Clear Chat",
            KeyHint::for_action(&ClearChat, cx),
            cx,
        );
        let close = button(
            self,
            HeaderControl::Close,
            "assistant-close",
            IconName::Close,
            "Close",
            KeyHint::for_action(&CloseAssistant, cx),
            cx,
        );

        div()
            .id("assistant-header")
            .debug_selector(|| "assistant-header".into())
            .key_context(HEADER_CONTEXT)
            .track_focus(&self.header_focus)
            // A stop of the keyboard only: the pointer presses the
            // controls and never moves the keyboard here.
            .on_any_mouse_down(|_, window, _| window.prevent_default())
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap(SPACE_2)
            .h(HEADER_HEIGHT)
            .pl(SPACE_3)
            .pr(SPACE_2)
            .border_b_1()
            .border_color(colors.hairline)
            .child(
                div()
                    .id("assistant-title")
                    .debug_selector(|| "assistant-title".into())
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .text_size(UI_TEXT_BASE)
                            .line_height(px(17.))
                            .font_weight(FontWeight::MEDIUM)
                            .whitespace_nowrap()
                            .child("Assistant"),
                    )
                    .child(
                        div()
                            .text_size(UI_TEXT_XS)
                            .line_height(px(14.))
                            .text_color(colors.muted)
                            .whitespace_nowrap()
                            .child(engine),
                    )
                    .tooltip(Tooltip::text(engine_tooltip)),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(1.))
                    .child(model)
                    .child(effort)
                    .child(div().w(SPACE_1))
                    .child(new_chat)
                    .child(clear)
                    .child(close),
            )
    }

    /// The open document, a chip for each added item, and "Add".
    fn render_context_row(
        &mut self,
        focused: bool,
        colors: &ThemeColors,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let attachments: Vec<Attachment> = self.attachments().to_vec();
        let add_marked = focused && self.context_index >= attachments.len();

        let document_chip = {
            let (title, tooltip, known): (SharedString, SharedString, bool) = match &self.document {
                Some(document) => (
                    if document.title.trim().is_empty() {
                        "Untitled".into()
                    } else {
                        document.title.clone().into()
                    },
                    "The open document goes with every message, as it is at that moment".into(),
                    true,
                ),
                None => (
                    "No document open".into(),
                    "Open a document to ask about it".into(),
                    false,
                ),
            };
            chip(
                "assistant-context-document",
                "assistant-context-document".to_string(),
                IconName::Page,
                title,
                known,
                false,
                colors,
            )
            .tooltip(Tooltip::text(tooltip))
        };

        let mut row = div()
            .id("assistant-context")
            .debug_selector(|| "assistant-context".into())
            .key_context(CONTEXT_ROW_CONTEXT)
            .track_focus(&self.context_focus)
            .on_any_mouse_down(|_, window, _| window.prevent_default())
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap(SPACE_1)
            .px(SPACE_3)
            .py(px(7.))
            .border_b_1()
            .border_color(colors.hairline)
            .child(document_chip);

        for (index, attachment) in attachments.iter().enumerate() {
            let marked = focused && self.context_index == index;
            let name = format!("assistant-context-{}", attachment.reference());
            let remove_name = format!("{name}-remove");
            let (kind, id) = (attachment.kind, attachment.id);
            let folder = super::context::folder_of(attachment);
            let tooltip: SharedString = match (attachment.kind, folder.is_empty()) {
                (AttachmentKind::Folder, true) => "A folder of the library".into(),
                (AttachmentKind::Folder, false) => format!("A folder in {folder}").into(),
                (AttachmentKind::Document, true) => "A document of the library".into(),
                (AttachmentKind::Document, false) => format!("A document in {folder}").into(),
            };
            let hover = colors.hover;
            let ink = colors.ink;
            let remove = keep_focus(
                div()
                    .id(SharedString::from(remove_name.clone()))
                    .debug_selector(move || remove_name)
                    .flex_none()
                    .size(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover).text_color(ink))
                    .child(Icon::new(IconName::Close).size(px(10.)).color(colors.muted))
                    .tooltip(
                        Tooltip::new("Remove from the context")
                            .key_hint(KeyHint::for_action(&RemoveContext, cx))
                            .build(),
                    ),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.detach(kind, id, cx);
            }));
            row = row.child(
                chip(
                    SharedString::from(name.clone()),
                    name,
                    match attachment.kind {
                        AttachmentKind::Document => IconName::Document,
                        AttachmentKind::Folder => IconName::Folder,
                    },
                    attachment.title.clone().into(),
                    true,
                    marked,
                    colors,
                )
                .pr(px(3.))
                .child(remove)
                .tooltip(Tooltip::text(tooltip))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.open_chip_menu(kind, id, event.position, window, cx);
                    }),
                ),
            );
        }

        let add = text_button(
            "assistant-add-context",
            "assistant-add-context",
            "Add",
            Some(IconName::Plus),
            Some((
                "Add a document or a folder as context".into(),
                KeyHint::for_action(&AddContext, cx),
            )),
            Look::Ghost,
            false,
            add_marked,
            colors,
        )
        .relative()
        .child(self.add_bounds.probe())
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            cx.stop_propagation();
            this.open_context_picker(window, cx);
        }));
        row.child(add)
    }

    /// The notices above the composer.
    fn render_notices(&mut self, colors: &ThemeColors, cx: &mut Context<Self>) -> Option<Div> {
        let notices = self.notices();
        if notices.is_empty() {
            return None;
        }
        // Flex columns all the way down: a notice fills the width of the
        // panel however short its text is (as a block in a block it came
        // out as wide as its text).
        let mut column = div().flex_none().flex().flex_col().px(SPACE_3).pt(SPACE_1);
        for (kind, mut notice) in notices {
            let name = match kind {
                PanelNotice::Availability => "assistant-notice-availability",
                PanelNotice::Privacy => "assistant-notice-privacy",
                PanelNotice::Agent => "assistant-notice-agent",
            };
            if kind == PanelNotice::Availability && self.checking {
                notice.action = Some("Checking".into());
            }
            column = column.child(div().flex().flex_col().pb(SPACE_2).child(
                notice_row(name, &notice, false, colors).on_action_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        this.press_notice(kind, window, cx);
                    },
                )),
            ));
        }
        Some(column)
    }

    /// The picker over the library, floating under "Add".
    fn render_picker(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let picker = self.picker.clone()?;
        let position = match self.add_bounds.get() {
            Some(bounds) => point(bounds.left(), bounds.bottom() + px(4.)),
            None => point(px(8.), HEADER_HEIGHT + px(40.)),
        };
        Some(
            deferred(
                anchored()
                    .position(position)
                    .anchor(Corner::TopLeft)
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .debug_selector(|| "assistant-context-picker".into())
                            .occlude()
                            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                                this.close_context_picker(window, cx);
                            }))
                            .child(picker),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// The menu of the panel itself: a right-click on its header or its
    /// context row.
    pub fn open_panel_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel = cx.weak_entity();
        let item =
            |title: &'static str,
             hint: Option<KeyHint>,
             run: fn(&mut AssistantPanel, &mut Window, &mut Context<AssistantPanel>)| {
                let panel = panel.clone();
                ContextMenuItem::new(title, move |window, cx| {
                    panel.update(cx, |panel, cx| run(panel, window, cx)).ok();
                })
                .key_hint(hint)
            };
        let items = vec![
            item(
                "New Chat",
                KeyHint::for_action(&NewChat, cx),
                |panel, window, cx| panel.new_chat(window, cx),
            )
            .disabled(!self.header_control_enabled(HeaderControl::NewChat)),
            item(
                "Clear Chat",
                KeyHint::for_action(&ClearChat, cx),
                |panel, window, cx| panel.clear(window, cx),
            )
            .disabled(!self.header_control_enabled(HeaderControl::Clear)),
            ContextMenuItem::separator(),
            item(
                "Add Context…",
                KeyHint::for_action(&AddContext, cx),
                |panel, window, cx| panel.open_context_picker(window, cx),
            ),
            ContextMenuItem::separator(),
            item(
                "Close",
                KeyHint::for_action(&CloseAssistant, cx),
                |_, _, cx| cx.emit(AssistantPanelEvent::CloseRequested),
            ),
        ];
        ContextMenu::open(position, items, window, cx);
    }

    /// The menu of an added item's chip.
    fn open_chip_menu(
        &mut self,
        kind: AttachmentKind,
        id: i64,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel = cx.weak_entity();
        let items = vec![
            ContextMenuItem::new("Remove from the Context", move |_, cx| {
                panel
                    .update(cx, |panel, cx| {
                        panel.detach(kind, id, cx);
                    })
                    .ok();
            })
            .key_hint(KeyHint::for_action(&RemoveContext, cx)),
        ];
        ContextMenu::open(position, items, window, cx);
    }
}

/// A chip of the context row: an icon and a name that is cut off when it
/// is long.
fn chip(
    id: impl Into<gpui::ElementId>,
    name: String,
    icon: IconName,
    title: SharedString,
    known: bool,
    marked: bool,
    colors: &ThemeColors,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || name)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .h(px(22.))
        .max_w(px(190.))
        .pl(px(6.))
        .pr(px(8.))
        .rounded(RADIUS_SM)
        .border_1()
        .border_color(colors.hairline)
        .bg(colors.elevated)
        .text_size(UI_TEXT_SM)
        .line_height(px(16.))
        .text_color(if known { colors.ink } else { colors.muted })
        .map(|chip| keyboard_mark(chip, marked, colors))
        .child(Icon::new(icon).size(px(12.)).color(colors.muted))
        .child(div().min_w_0().truncate().child(title))
}
