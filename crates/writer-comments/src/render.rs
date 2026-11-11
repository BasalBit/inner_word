//! How the panel looks, and where its action listeners are attached.
//!
//! ```text
//! root (CommentsPanel)                 action listeners
//! ├── header: "Comments", open count, buttons: add a comment, hide
//! ├── new-comment composer             (CommentComposer > Editor), save and cancel buttons
//! ├── thread list (CommentThreads)     focus handle, scrolls
//! │   ├── thread card ...              buttons on hover and while active: reply, resolve
//! │   ├── "Detached" heading
//! │   └── thread card ...
//! └── footer: show / hide resolved     (a button)
//! ```
//!
//! A reply or an edit puts the composer inside the thread card instead.
//!
//! Everything a key does here a click does too. The buttons are in
//! `controls`; what a click on a card or a reply means is in
//! `panel::pointer`.
//!
//! The thread list is a `gpui::list`: only the cards near the visible part
//! are built, laid out and painted, so a document with a thousand threads
//! costs a frame no more than one with ten.

use gpui::{
    AnyElement, App, ClickEvent, Context, Div, FontWeight, Hsla, InteractiveElement, IntoElement,
    KeyContext, MouseButton, MouseDownEvent, Render, SharedString, Stateful, WeakEntity, Window,
    canvas, div, list, prelude::*, px,
};
use writer_core::{CommentId, Reply, ReplyId};
use writer_ui::{
    IconName, KeyHint, SectionLabel,
    context_menu::OpenContextMenu,
    theme::{
        ActiveTheme, RADIUS_MD, RADIUS_SM, SPACE_0_5, SPACE_1, SPACE_2, SPACE_3, ThemeColors,
        UI_LINE_HEIGHT, UI_TEXT_BASE, UI_TEXT_SM, UI_TEXT_XS,
    },
};

use crate::{
    actions::{
        AddComment, COMPOSER_CONTEXT, CancelComment, CancelDelete, ConfirmDelete, CopyComment,
        CycleFocus, DeleteComment, EditComment, FocusComments, FocusEditor, GoToThread,
        NextComment, PANEL_CONTEXT, PreviousComment, ReplyToThread, SelectFirstThread,
        SelectLastThread, SelectNextReply, SelectNextThread, SelectPreviousReply,
        SelectPreviousThread, SubmitComment, THREADS_CONTEXT, ToggleComments, ToggleResolved,
        ToggleShowResolved,
    },
    controls::{ICON_BUTTON, Look, Press, icon_button, text_button},
    panel::{
        CommentMenuTarget, CommentsPanel, DeleteTarget, DraftKind, Row, menu::MenuAnchors,
        panel_for_actions,
    },
    threads::ThreadEntry,
    time::relative_time,
};

/// Attaches one action listener that forwards to a panel method.
macro_rules! forward {
    ($element:expr, $panel:expr, $($action:ty => $method:ident),+ $(,)?) => {{
        let mut element = $element;
        $(
            element = element.on_action({
                let panel = $panel.clone();
                move |_: &$action, window: &mut Window, cx: &mut App| {
                    panel
                        .update(cx, |panel, cx| panel.$method(window, cx))
                        .ok();
                }
            });
        )+
        element
    }};
}

/// The actions that work from anywhere in the window: starting a comment,
/// comment navigation, visibility, and the commands on the active thread.
/// With focus inside the panel these listeners handle them; from anywhere
/// else the global listeners in `global_actions` do.
fn window_actions<E: InteractiveElement>(element: E, panel: &WeakEntity<CommentsPanel>) -> E {
    let element = element.on_action({
        let panel = panel.clone();
        move |action: &GoToThread, window: &mut Window, cx: &mut App| {
            panel
                .update(cx, |panel, cx| {
                    panel.go_to_thread(action.id(), window, cx);
                })
                .ok();
        }
    });
    forward!(element, panel,
        AddComment => add_comment,
        NextComment => next_comment,
        PreviousComment => previous_comment,
        ToggleComments => toggle_visible,
        FocusComments => focus_panel,
        ToggleShowResolved => toggle_show_resolved,
        ReplyToThread => reply,
        EditComment => edit,
        ToggleResolved => toggle_resolved,
        DeleteComment => request_delete,
        CopyComment => copy_active,
    )
}

/// The actions that only make sense with focus inside the panel.
fn panel_actions<E: InteractiveElement>(element: E, panel: &WeakEntity<CommentsPanel>) -> E {
    forward!(element, panel,
        SelectNextThread => select_next_thread,
        SelectPreviousThread => select_previous_thread,
        SelectFirstThread => select_first_thread,
        SelectLastThread => select_last_thread,
        SelectNextReply => select_next_reply,
        SelectPreviousReply => select_previous_reply,
        ConfirmDelete => confirm_delete,
        CancelDelete => cancel_delete,
        FocusEditor => focus_editor,
        CycleFocus => cycle_focus,
        OpenContextMenu => open_context_menu,
        SubmitComment => submit,
        CancelComment => cancel,
        writer_editor::actions::Undo => undo,
        writer_editor::actions::Redo => redo,
    )
}

/// Registers the global listeners for the window-wide comment actions:
/// whatever has focus in a window, an action that no element handled
/// reaches that window's panel. The panel is updated after the current
/// dispatch has finished, because the window is busy until then.
pub(crate) fn global_actions(cx: &mut App) {
    macro_rules! global {
        ($($action:ty => $method:ident),+ $(,)?) => {
            $(
                cx.on_action(|_: &$action, cx: &mut App| {
                    let Some((window, panel)) = panel_for_actions(cx) else {
                        return;
                    };
                    cx.defer(move |cx| {
                        window
                            .update(cx, |_, window, cx| {
                                panel.update(cx, |panel, cx| panel.$method(window, cx));
                            })
                            .ok();
                    });
                });
            )+
        };
    }
    cx.on_action(|action: &GoToThread, cx: &mut App| {
        let Some((window, panel)) = panel_for_actions(cx) else {
            return;
        };
        let thread = action.id();
        cx.defer(move |cx| {
            window
                .update(cx, |_, window, cx| {
                    panel.update(cx, |panel, cx| {
                        panel.go_to_thread(thread, window, cx);
                    });
                })
                .ok();
        });
    });
    global!(
        AddComment => add_comment,
        NextComment => next_comment,
        PreviousComment => previous_comment,
        ToggleComments => toggle_visible,
        FocusComments => focus_panel,
        ToggleShowResolved => toggle_show_resolved,
        ReplyToThread => reply,
        EditComment => edit,
        ToggleResolved => toggle_resolved,
        DeleteComment => request_delete,
        CopyComment => copy_active,
    );
}

impl Render for CommentsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.is_visible() {
            // Comments are hidden: nothing to draw.
            return div().into_any_element();
        }
        let colors = cx.theme().colors;
        let ui_font = cx.theme().fonts.ui.clone();
        if !self.is_open() {
            // The app normally hides the panel's container in this state
            // (see `CommentsEvent::OpenChanged`); if it does not, say what
            // the empty column is for.
            return self
                .render_no_comments(&colors, ui_font, cx)
                .into_any_element();
        }
        let panel = cx.weak_entity();
        let composer_focused = self.composer.read(cx).is_focused(window);
        // The "more" buttons this frame draws say where they are.
        *self.menu_anchors.borrow_mut() = MenuAnchors::default();
        if self.reveal_pending() {
            // Once this frame has measured the cards: is the active one
            // fully in view?
            cx.defer_in(window, |this, _, cx| this.settle_reveal(cx));
        }
        let open_count = self
            .threads
            .iter()
            .filter(|entry| !entry.is_resolved())
            .count();
        let threads: AnyElement = if self.rows.is_empty() && self.draft.is_none() {
            // Every thread is resolved and hidden.
            self.render_no_open_comments(self.focus_handle.is_focused(window), &colors, cx)
                .into_any_element()
        } else {
            let panel = panel.clone();
            list(self.list.clone(), move |ix, window, cx| {
                panel
                    .update(cx, |this, cx| this.render_row(ix, window, cx))
                    .unwrap_or_else(|_| div().into_any_element())
            })
            .size_full()
            .py(SPACE_2)
            .into_any_element()
        };

        let root = div()
            .id("comments-panel")
            .debug_selector(|| "comments-panel".into())
            .key_context(PANEL_CONTEXT)
            // The container gives the width and the border beside the text.
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.sidebar)
            .font_family(ui_font)
            .text_size(UI_TEXT_BASE)
            .line_height(UI_LINE_HEIGHT)
            .text_color(colors.ink);
        let root = panel_actions(window_actions(root, &panel), &panel)
            // A right-click that no card took: the menu of the panel.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_menu(CommentMenuTarget::Panel, event.position, false, window, cx);
                }),
            );

        root.child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_between()
                .relative()
                .h(px(36.))
                .pl(SPACE_3)
                .pr(SPACE_2)
                .border_b_1()
                .border_color(colors.hairline)
                .child({
                    let anchors = self.menu_anchors.clone();
                    canvas(
                        move |bounds, _, _| anchors.borrow_mut().header = Some(bounds),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full()
                })
                .child(SectionLabel::new("Comments"))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(SPACE_0_5)
                        .child(
                            div()
                                .mr(SPACE_1)
                                .text_size(UI_TEXT_XS)
                                .text_color(colors.muted)
                                .child(match open_count {
                                    1 => "1 open".to_string(),
                                    count => format!("{count} open"),
                                }),
                        )
                        .child(
                            icon_button(
                                "comments-add",
                                "comments-add".into(),
                                IconName::Plus,
                                "Add comment on selection",
                                KeyHint::for_action(&AddComment, cx),
                                Press::KeepFocus,
                                &colors,
                            )
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.add_comment(window, cx);
                                },
                            )),
                        )
                        .child(
                            icon_button(
                                "comments-hide",
                                "comments-hide".into(),
                                IconName::Close,
                                "Hide comments",
                                KeyHint::for_action(&ToggleComments, cx),
                                Press::KeepFocus,
                                &colors,
                            )
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.set_visible(false, window, cx);
                                },
                            )),
                        ),
                ),
        )
        .when_some(
            self.render_new_composer(composer_focused, &colors, cx),
            |root, composer| root.child(composer),
        )
        .child(
            div()
                .id("comment-threads")
                .debug_selector(|| "comment-threads".into())
                .key_context(self.threads_context(composer_focused))
                .track_focus(&self.focus_handle)
                .flex_1()
                .min_h_0()
                .child(threads),
        )
        .when_some(self.render_footer(&colors, cx), |root, footer| {
            root.child(footer)
        })
        .into_any_element()
    }
}

impl CommentsPanel {
    /// One row of the thread list, built when the list needs it.
    fn render_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.rows_built += 1;
        let colors = cx.theme().colors;
        // The list lays its rows out edge to edge and one under the other,
        // so the side margin and the space between two cards belong to
        // the row.
        let row = div().px(SPACE_1);
        match self.rows.get(ix).copied() {
            Some(Row::DetachedHeading) => row
                .child(self.render_detached_heading(&colors))
                .into_any_element(),
            Some(Row::Thread(id)) => {
                let Some(entry) = self.thread(id) else {
                    return row.into_any_element();
                };
                let list_focused = self.focus_handle.is_focused(window);
                let composer_focused = self.composer.read(cx).is_focused(window);
                row.pb(SPACE_1)
                    .child(self.render_thread(
                        entry,
                        list_focused,
                        composer_focused,
                        self.now(),
                        &colors,
                        cx,
                    ))
                    .into_any_element()
            }
            None => row.into_any_element(),
        }
    }

    /// The key context of the thread list: [`THREADS_CONTEXT`], plus
    /// `confirming` while a delete confirmation is showing, plus `composing`
    /// while focus is in a composer inside the list (a reply or an edit).
    /// The list's single-letter bindings exclude `composing`, so that
    /// letters typed into a reply are text. The flag follows the focus, not
    /// the draft: when the list itself has focus beside a draft that was
    /// left open, its keys work.
    fn threads_context(&self, composer_focused: bool) -> KeyContext {
        let mut context = KeyContext::default();
        context.add(THREADS_CONTEXT);
        if self.pending_delete().is_some() {
            context.add("confirming");
        }
        if composer_focused && self.draft().is_some_and(|draft| draft.thread().is_some()) {
            context.add("composing");
        }
        context
    }

    /// What the panel shows in a document without comments.
    fn render_no_comments(
        &self,
        colors: &ThemeColors,
        ui_font: SharedString,
        cx: &Context<Self>,
    ) -> Div {
        div()
            .debug_selector(|| "comments-none".into())
            .size_full()
            .flex()
            .flex_col()
            .gap(SPACE_2)
            .p(SPACE_3)
            .bg(colors.sidebar)
            .font_family(ui_font)
            .text_size(UI_TEXT_BASE)
            .line_height(UI_LINE_HEIGHT)
            .text_color(colors.muted)
            .child(SectionLabel::new("Comments"))
            .child("No comments in this document.")
            .child(
                hint_row(colors).child(
                    text_button(
                        "comments-none-add",
                        "comments-none-add".into(),
                        "Add one on the selected text",
                        KeyHint::for_action(&AddComment, cx),
                        Look::Hint,
                        false,
                        Press::KeepFocus,
                        colors,
                    )
                    .on_click(cx.listener(
                        |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            this.add_comment(window, cx);
                        },
                    )),
                ),
            )
    }

    /// What the list shows when every thread is resolved and resolved
    /// threads are hidden. While it has the keyboard it is marked like an
    /// active card and names the two keys that lead somewhere.
    fn render_no_open_comments(
        &self,
        list_focused: bool,
        colors: &ThemeColors,
        cx: &Context<Self>,
    ) -> Div {
        div().px(SPACE_1).py(SPACE_2).child(
            div()
                .debug_selector(|| "comments-empty".into())
                .flex()
                .flex_col()
                .gap(SPACE_1)
                .py(SPACE_2)
                .pr(SPACE_2)
                .pl(px(6.))
                .rounded_r(RADIUS_MD)
                .border_l_2()
                .border_color(if list_focused {
                    colors.accent
                } else {
                    gpui::transparent_black()
                })
                .when(list_focused, |block| block.bg(colors.accent_subtle))
                .child(div().text_color(colors.muted).child("No open comments."))
                .when(list_focused, |block| {
                    block.child(
                        hint_row(colors)
                            .debug_selector(|| "comments-empty-hints".into())
                            .child(
                                text_button(
                                    "comments-empty-show",
                                    "comments-empty-show".into(),
                                    "Show resolved",
                                    KeyHint::for_action(&ToggleShowResolved, cx),
                                    Look::Hint,
                                    false,
                                    Press::Bubble,
                                    colors,
                                )
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.toggle_show_resolved(window, cx);
                                    },
                                )),
                            )
                            .child(
                                text_button(
                                    "comments-empty-back",
                                    "comments-empty-back".into(),
                                    "Back to the text",
                                    KeyHint::for_action(&FocusEditor, cx),
                                    Look::Hint,
                                    false,
                                    Press::Bubble,
                                    colors,
                                )
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.focus_editor(window, cx);
                                    },
                                )),
                            ),
                    )
                }),
        )
    }

    fn render_detached_heading(&self, colors: &ThemeColors) -> Div {
        div()
            .debug_selector(|| "comments-detached".into())
            .flex_none()
            .flex()
            .flex_col()
            .gap(SPACE_0_5)
            .px(SPACE_2)
            .pt(SPACE_3)
            .pb(SPACE_2)
            .child(SectionLabel::new("Detached"))
            .child(
                div()
                    .text_size(UI_TEXT_XS)
                    .text_color(colors.muted)
                    .child("The commented text was deleted."),
            )
    }

    /// The composer box, wherever the draft puts it.
    fn render_composer_box(&self, focused: bool, colors: &ThemeColors) -> Div {
        div()
            .debug_selector(|| "comment-composer".into())
            .key_context(COMPOSER_CONTEXT)
            // A right-click in the composer is about its text (the
            // composer asks for its own menu), not about the card around.
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .px(SPACE_2)
            .py(SPACE_1)
            .bg(colors.canvas)
            .border_1()
            .border_color(if focused {
                colors.accent
            } else {
                colors.border_strong
            })
            .rounded(RADIUS_MD)
            .child(self.composer.clone())
    }

    /// Under the composer: the buttons that save and discard what is
    /// written, each with its key from the keymap (`enter`, the compact
    /// editor's own submit-on-enter, bound as its `Newline`; `cmd-enter`
    /// saves too), and the key of a line break. The save button is dimmed
    /// while there is nothing it could save.
    fn render_composer_actions(
        &self,
        submit_label: &'static str,
        colors: &ThemeColors,
        cx: &Context<Self>,
    ) -> Div {
        let can_submit = self.can_submit(cx);
        let submit = text_button(
            "comment-submit",
            "comment-submit".into(),
            submit_label,
            KeyHint::for_action(&writer_editor::actions::Newline, cx),
            Look::Primary,
            !can_submit,
            Press::KeepFocus,
            colors,
        );
        div()
            .debug_selector(|| "comment-composer-hints".into())
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(SPACE_2)
            .gap_y(SPACE_1)
            .child(if can_submit {
                submit.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.submit(window, cx);
                }))
            } else {
                // Still stops the click: it is not one on the card.
                submit.on_click(|_, _, cx| cx.stop_propagation())
            })
            .child(
                text_button(
                    "comment-cancel",
                    "comment-cancel".into(),
                    "Cancel",
                    KeyHint::for_action(&CancelComment, cx),
                    Look::Quiet,
                    false,
                    Press::KeepFocus,
                    colors,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.cancel(window, cx);
                })),
            )
            .child(hint_row(colors).flex_1().justify_end().child(hint(
                "Line break",
                KeyHint::for_action(&writer_editor::actions::InsertSoftBreak, cx),
                colors,
            )))
    }

    /// In place of the quote when the text a new comment was started on
    /// was deleted meanwhile: what happened, and the two ways out.
    fn render_target_lost(&self, colors: &ThemeColors, cx: &Context<Self>) -> Div {
        div()
            .debug_selector(|| "comment-target-lost".into())
            .flex()
            .gap(SPACE_2)
            .child(
                div()
                    .w(px(2.))
                    .flex_none()
                    .rounded(px(1.))
                    .bg(colors.danger),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(SPACE_0_5)
                    .child(div().text_size(UI_TEXT_SM).text_color(colors.ink).child(
                        "The text for this comment was deleted. Undo that in the \
                                 text, or point the comment at other text.",
                    ))
                    .child(
                        hint_row(colors).child(
                            text_button(
                                "comment-retarget",
                                "comment-retarget".into(),
                                "Use the selection",
                                KeyHint::for_action(&AddComment, cx),
                                Look::Hint,
                                false,
                                Press::KeepFocus,
                                colors,
                            )
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.add_comment(window, cx);
                                },
                            )),
                        ),
                    ),
            )
    }

    /// The block under the header while a new comment is being written:
    /// the quoted target, the composer, the keys.
    fn render_new_composer(
        &self,
        composer_focused: bool,
        colors: &ThemeColors,
        cx: &Context<Self>,
    ) -> Option<Div> {
        let draft = self.draft.as_ref()?;
        if !matches!(draft.kind, DraftKind::New { .. }) {
            return None;
        }
        Some(
            div()
                .flex()
                .flex_none()
                .flex_col()
                .gap(SPACE_2)
                .p(SPACE_3)
                .border_b_1()
                .border_color(colors.hairline)
                .child(if self.draft_target_lost() {
                    self.render_target_lost(colors, cx)
                } else {
                    quote_line(draft.quote.clone().into(), colors)
                })
                .child(self.render_composer_box(composer_focused, colors))
                .child(self.render_composer_actions("Comment", colors, cx)),
        )
    }

    fn render_thread(
        &self,
        entry: &ThreadEntry,
        list_focused: bool,
        composer_focused: bool,
        now: i64,
        colors: &ThemeColors,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let id = entry.id;
        let is_active = self.active == Some(id);
        let draft = self
            .draft
            .as_ref()
            .filter(|draft| draft.kind.thread() == Some(id));
        let editing_comment = matches!(
            draft.map(|draft| &draft.kind),
            Some(DraftKind::EditComment { .. })
        );
        let editing_reply = match draft.map(|draft| &draft.kind) {
            Some(DraftKind::EditReply { reply, .. }) => Some(*reply),
            _ => None,
        };
        let replying = matches!(
            draft.map(|draft| &draft.kind),
            Some(DraftKind::Reply { .. })
        );
        let pending_delete = self.pending_delete().filter(|target| target.thread() == id);

        let mut card = div()
            .id(("comment-thread", id.0 as usize))
            .debug_selector(|| format!("comment-thread-{}", id.0))
            // The list scrolls; a card keeps its height.
            .flex_none()
            .flex()
            .flex_col()
            .gap(SPACE_1)
            .py(SPACE_2)
            .pr(SPACE_2)
            // With the 2px rule and the list's 4px, the text starts 12px
            // from the panel's edge, in line with the header and the footer.
            .pl(px(6.))
            // The rule is straight: no rounding on its side.
            .rounded_r(RADIUS_MD)
            .border_l_2()
            // Accent only while the keyboard is in the list (as the
            // library marks its selected row); a quiet rule otherwise.
            .border_color(match (is_active, list_focused) {
                (true, true) => colors.accent,
                (true, false) => colors.border_strong,
                (false, _) => gpui::transparent_black(),
            })
            .when(is_active, |card| {
                card.bg(if list_focused {
                    colors.accent_subtle
                } else {
                    colors.hover
                })
            })
            .when(!is_active, |card| {
                card.hover(|style| style.bg(colors.hover))
            })
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                this.hover_thread(id, *hovered, cx);
            }))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.click_thread(id, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_menu(
                        CommentMenuTarget::Thread(id),
                        event.position,
                        false,
                        window,
                        cx,
                    );
                }),
            );

        // A resolved thread steps back: its words are done with.
        let resolved = entry.is_resolved();
        let (name_color, body_color) = if resolved {
            (colors.muted, colors.muted)
        } else {
            (colors.ink, colors.ink)
        };
        if !entry.quote.is_empty() {
            card = card.child(quote_line(entry.quote.clone().into(), colors));
        }
        // The card's buttons sit at the end of the line with the name.
        // Their room is always kept, so the card does not move when they
        // appear under the pointer.
        let controls_shown = is_active || self.hovered == Some(id);
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .child(
                    meta_line(
                        &entry.thread.author,
                        relative_time(entry.thread.created_at, now),
                        resolved,
                        name_color,
                        colors,
                    )
                    .flex_1()
                    .min_w_0(),
                )
                .child(self.render_thread_controls(entry, controls_shown, colors, cx)),
        );
        card = if editing_comment {
            card.child(self.render_composer_box(composer_focused, colors))
                .child(self.render_composer_actions("Save", colors, cx))
        } else {
            card.child(body_text(&entry.thread.body, body_color))
        };

        if !entry.thread.replies.is_empty() {
            let selected = self.selected_reply.filter(|_| is_active);
            let replies = entry.thread.replies.iter().map(|reply| {
                if editing_reply == Some(reply.id) {
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_1)
                        .child(meta_line(
                            &reply.author,
                            relative_time(reply.created_at, now),
                            false,
                            name_color,
                            colors,
                        ))
                        .child(self.render_composer_box(composer_focused, colors))
                        .child(self.render_composer_actions("Save", colors, cx))
                        .into_any_element()
                } else {
                    self.render_reply(
                        id,
                        reply,
                        selected == Some(reply.id),
                        (name_color, body_color),
                        now,
                        colors,
                        cx,
                    )
                    .into_any_element()
                }
            });
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_1)
                    .mt(SPACE_1)
                    .ml(SPACE_1)
                    .pl(SPACE_2)
                    .border_l_1()
                    .border_color(colors.border_strong)
                    .children(replies),
            );
        }

        if replying {
            card = card
                .child(
                    self.render_composer_box(composer_focused, colors)
                        .mt(SPACE_1),
                )
                .child(self.render_composer_actions("Reply", colors, cx));
        }

        if let Some(target) = pending_delete {
            card = card.child(self.render_delete_confirmation(target, entry, colors, cx));
        } else if is_active && list_focused && draft.is_none() {
            card = card.child(self.render_thread_hints(entry, colors, cx));
        }
        card
    }

    /// The buttons of a card: reply, resolve or reopen. They are drawn
    /// while the card is active or under the pointer; their room is kept
    /// otherwise. Each acts on its own card, whichever thread is active.
    fn render_thread_controls(
        &self,
        entry: &ThreadEntry,
        shown: bool,
        colors: &ThemeColors,
        cx: &Context<Self>,
    ) -> Div {
        let id = entry.id;
        let n = id.0 as usize;
        let row = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_end()
            .gap(SPACE_0_5)
            .h(ICON_BUTTON)
            .w(ICON_BUTTON * 3. + SPACE_0_5 * 2.);
        if !shown {
            return row;
        }
        let target = CommentMenuTarget::Thread(id);
        let more = self
            .more_button(
                ("comment-more-button", n),
                format!("comment-more-button-{}", id.0),
                target,
                colors,
                cx,
            )
            .into_any_element();
        if self.editor.read(cx).is_read_only() {
            // Nothing can be changed; the menu still copies the text.
            return row.child(more);
        }
        let resolved = entry.is_resolved();
        row.child(
            icon_button(
                ("comment-reply-button", n),
                format!("comment-reply-button-{}", id.0),
                IconName::Comment,
                "Reply",
                KeyHint::for_action(&ReplyToThread, cx),
                Press::Bubble,
                colors,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                this.reply_to(id, window, cx);
            })),
        )
        .child(
            icon_button(
                ("comment-resolve-button", n),
                format!("comment-resolve-button-{}", id.0),
                IconName::Check,
                if resolved { "Reopen" } else { "Resolve" },
                KeyHint::for_action(&ToggleResolved, cx),
                Press::Bubble,
                colors,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                this.toggle_resolved_of(id, window, cx);
            })),
        )
        .child(more)
    }

    /// The "more" button of a card or a reply: opens its menu, dropping
    /// down from the button. `ctrl-enter` in the list opens the same menu
    /// at the same place.
    fn more_button(
        &self,
        id: impl Into<gpui::ElementId>,
        name: String,
        target: CommentMenuTarget,
        colors: &ThemeColors,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let anchors = self.menu_anchors.clone();
        icon_button(
            id,
            name,
            IconName::More,
            "More",
            KeyHint::for_action(&OpenContextMenu, cx),
            Press::Bubble,
            colors,
        )
        .child(
            canvas(
                move |bounds, _, _| anchors.borrow_mut().more.push((target, bounds)),
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            cx.stop_propagation();
            let anchor = this.menu_anchor(target);
            this.open_menu(target, anchor, false, window, cx);
        }))
    }

    /// One reply. A click selects it, as the arrow keys do.
    #[allow(clippy::too_many_arguments)]
    fn render_reply(
        &self,
        thread: CommentId,
        reply: &Reply,
        selected: bool,
        (name_color, body_color): (Hsla, Hsla),
        now: i64,
        colors: &ThemeColors,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let reply_id = reply.id;
        let ReplyId(n) = reply_id;
        let target = CommentMenuTarget::Reply(thread, reply_id);
        let controls_shown = selected || self.hovered_reply == Some((thread, reply_id));
        div()
            .id(("comment-reply", n as usize))
            .debug_selector(move || format!("comment-reply-{}-{}", thread.0, n))
            .flex()
            .flex_col()
            .gap(SPACE_0_5)
            .px(SPACE_1)
            .py(SPACE_0_5)
            .rounded(RADIUS_SM)
            .when(selected, |block| block.bg(colors.selection))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                this.hover_reply(thread, reply_id, *hovered, cx);
            }))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                this.select_reply(thread, reply_id, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_menu(target, event.position, false, window, cx);
                }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_2)
                    .child(
                        meta_line(
                            &reply.author,
                            relative_time(reply.created_at, now),
                            false,
                            name_color,
                            colors,
                        )
                        .flex_1()
                        .min_w_0(),
                    )
                    // The room of the button is kept, as on the card.
                    .child(
                        div()
                            .flex_none()
                            .size(ICON_BUTTON)
                            .when(controls_shown, |slot| {
                                slot.child(self.more_button(
                                    ("comment-reply-more-button", n as usize),
                                    format!("comment-reply-more-button-{}-{}", thread.0, n),
                                    target,
                                    colors,
                                    cx,
                                ))
                            }),
                    ),
            )
            .child(body_text(&reply.body, body_color))
    }

    /// The commands on the active thread with their keys, shown while the
    /// list has the keyboard. The glyphs come from the keymap, and each
    /// label is a button that does what its key does.
    fn render_thread_hints(
        &self,
        entry: &ThreadEntry,
        colors: &ThemeColors,
        cx: &Context<Self>,
    ) -> Div {
        let on_reply = self.selected_reply.is_some();
        let n = entry.id.0;
        let button = |name: &'static str, label: &'static str, key: Option<KeyHint>| {
            text_button(
                name,
                format!("{name}-{n}"),
                label,
                key,
                Look::Hint,
                false,
                Press::Bubble,
                colors,
            )
        };
        hint_row(colors)
            .debug_selector(|| "comment-thread-hints".into())
            .mt(SPACE_1)
            .child(
                button(
                    "comment-hint-reply",
                    "Reply",
                    KeyHint::for_action(&ReplyToThread, cx),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.reply(window, cx);
                })),
            )
            .child(
                button(
                    "comment-hint-edit",
                    if on_reply { "Edit reply" } else { "Edit" },
                    KeyHint::for_action(&EditComment, cx),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.edit(window, cx);
                })),
            )
            .child(
                button(
                    "comment-hint-resolve",
                    if entry.is_resolved() {
                        "Reopen"
                    } else {
                        "Resolve"
                    },
                    KeyHint::for_action(&ToggleResolved, cx),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.toggle_resolved(window, cx);
                })),
            )
            .child(
                button(
                    "comment-hint-delete",
                    if on_reply { "Delete reply" } else { "Delete" },
                    KeyHint::for_action(&DeleteComment, cx),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.request_delete(window, cx);
                })),
            )
    }

    fn render_delete_confirmation(
        &self,
        target: DeleteTarget,
        entry: &ThreadEntry,
        colors: &ThemeColors,
        cx: &Context<Self>,
    ) -> Div {
        let question = match target {
            DeleteTarget::Reply(..) => "Delete this reply?".to_string(),
            DeleteTarget::Thread(_) => match entry.thread.replies.len() {
                0 => "Delete this comment?".to_string(),
                1 => "Delete this comment and its reply?".to_string(),
                count => format!("Delete this comment and its {count} replies?"),
            },
        };
        div()
            .debug_selector(|| "comment-delete-confirmation".into())
            .flex()
            .flex_col()
            .gap(SPACE_2)
            .mt(SPACE_1)
            .p(SPACE_2)
            .rounded(RADIUS_SM)
            .bg(colors.danger_subtle)
            .child(div().text_color(colors.ink).child(question))
            .child(
                div()
                    .flex()
                    .gap(SPACE_2)
                    // The two buttons leave the keyboard on the list, where
                    // `enter` and `escape` answer the question.
                    .child(
                        text_button(
                            "confirm-delete-comment",
                            "confirm-delete-comment".into(),
                            "Delete",
                            KeyHint::for_action(&ConfirmDelete, cx),
                            Look::Danger,
                            false,
                            Press::Bubble,
                            colors,
                        )
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                this.confirm_delete(window, cx);
                            },
                        )),
                    )
                    .child(
                        text_button(
                            "cancel-delete-comment",
                            "cancel-delete-comment".into(),
                            "Keep",
                            KeyHint::for_action(&CancelDelete, cx),
                            Look::Quiet,
                            false,
                            Press::Bubble,
                            colors,
                        )
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                this.cancel_delete(window, cx);
                            },
                        )),
                    ),
            )
    }

    /// The row at the bottom that shows or hides resolved threads. Absent
    /// when no thread is resolved.
    fn render_footer(&self, colors: &ThemeColors, cx: &Context<Self>) -> Option<Stateful<Div>> {
        let resolved = self.resolved_count();
        if resolved == 0 {
            return None;
        }
        let label = if self.show_resolved {
            "Hide resolved".to_string()
        } else {
            format!("Show resolved ({resolved})")
        };
        Some(
            div()
                .id("comments-resolved-toggle")
                .debug_selector(|| "comments-resolved-toggle".into())
                .flex()
                .flex_none()
                .items_center()
                .justify_between()
                .h(px(32.))
                .px(SPACE_3)
                .border_t_1()
                .border_color(colors.hairline)
                .text_size(UI_TEXT_SM)
                .text_color(colors.muted)
                .cursor_pointer()
                .hover(|style| style.bg(colors.hover))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.toggle_show_resolved(window, cx);
                }))
                .child(label)
                .when_some(KeyHint::for_action(&ToggleShowResolved, cx), |row, hint| {
                    row.child(hint.color(colors.muted))
                }),
        )
    }
}

/// The anchored text, quoted beside a bar in the comment colour.
fn quote_line(quote: SharedString, colors: &ThemeColors) -> Div {
    div()
        .flex()
        .gap(SPACE_2)
        .child(
            div()
                .w(px(2.))
                .flex_none()
                .rounded(px(1.))
                .bg(colors.comment_underline),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(UI_TEXT_SM)
                .text_color(colors.muted)
                .line_clamp(2)
                .child(quote),
        )
}

/// "Author  5 min ago", plus "Resolved" for a resolved thread.
fn meta_line(
    author: &str,
    time: String,
    resolved: bool,
    name_color: Hsla,
    colors: &ThemeColors,
) -> Div {
    let author: SharedString = if author.is_empty() {
        "Unknown".into()
    } else {
        author.to_string().into()
    };
    div()
        .flex()
        .items_baseline()
        .gap(SPACE_2)
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(UI_TEXT_SM)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(name_color)
                .child(author),
        )
        .child(
            div()
                .flex_none()
                .text_size(UI_TEXT_XS)
                .text_color(colors.muted)
                .child(time),
        )
        .when(resolved, |line| {
            line.child(
                div()
                    .flex_none()
                    .text_size(UI_TEXT_XS)
                    .text_color(colors.success)
                    .child("Resolved"),
            )
        })
}

fn body_text(body: &str, color: Hsla) -> Div {
    div()
        .min_w_0()
        .text_color(color)
        .child(SharedString::from(body.to_string()))
}

fn hint_row(colors: &ThemeColors) -> Div {
    div()
        .flex()
        .flex_wrap()
        .gap_x(SPACE_3)
        .gap_y(SPACE_0_5)
        .text_size(UI_TEXT_XS)
        .text_color(colors.muted)
}

/// A label with its key, e.g. "Reply ↩".
fn hint(label: &'static str, key: Option<KeyHint>, colors: &ThemeColors) -> Div {
    let color: Hsla = colors.muted;
    div()
        .flex()
        .items_center()
        .gap(SPACE_1)
        .child(label)
        .when_some(key, |hint, key| {
            hint.child(key.color(color).size(UI_TEXT_XS))
        })
}
