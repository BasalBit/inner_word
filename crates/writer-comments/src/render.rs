//! How the panel looks, and where its action listeners are attached.
//!
//! ```text
//! root (CommentsPanel)                 action listeners
//! ├── header: "Comments", open count
//! ├── new-comment composer             (CommentComposer > Editor)
//! ├── thread list (CommentThreads)     focus handle, scrolls
//! │   ├── thread card ...
//! │   ├── "Detached" heading
//! │   └── thread card ...
//! └── footer: show / hide resolved
//! ```
//!
//! A reply or an edit puts the composer inside the thread card instead.

use gpui::{
    AnyElement, App, ClickEvent, Context, Div, FontWeight, Hsla, InteractiveElement, IntoElement,
    KeyContext, Render, SharedString, Stateful, WeakEntity, Window, div, prelude::*, px,
};
use writer_core::{CommentId, Reply, ReplyId};
use writer_ui::{
    Button, ButtonSize, KeyHint, SectionLabel,
    theme::{
        ActiveTheme, RADIUS_MD, RADIUS_SM, SPACE_0_5, SPACE_1, SPACE_2, SPACE_3, ThemeColors,
        UI_LINE_HEIGHT, UI_TEXT_BASE, UI_TEXT_SM, UI_TEXT_XS,
    },
};

use crate::{
    actions::{
        AddComment, COMPOSER_CONTEXT, CancelComment, CancelDelete, ConfirmDelete, DeleteComment,
        EditComment, FocusComments, FocusEditor, NextComment, PANEL_CONTEXT, PreviousComment,
        ReplyToThread, SelectFirstThread, SelectLastThread, SelectNextReply, SelectNextThread,
        SelectPreviousReply, SelectPreviousThread, SubmitComment, THREADS_CONTEXT, ToggleComments,
        ToggleResolved, ToggleShowResolved,
    },
    panel::{CommentsPanel, DeleteTarget, DraftKind, panel_for_actions},
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
        let list_focused = self.focus_handle.is_focused(window);
        let composer_focused = self.composer.read(cx).is_focused(window);
        let now = self.now();

        let mut rows: Vec<AnyElement> = Vec::new();
        let mut detached_heading = false;
        let entries: Vec<ThreadEntry> = self.listed_threads().cloned().collect();
        for entry in &entries {
            if entry.is_detached() && !detached_heading {
                detached_heading = true;
                rows.push(self.render_detached_heading(&colors).into_any_element());
            }
            rows.push(
                self.render_thread(entry, list_focused, composer_focused, now, &colors, cx)
                    .into_any_element(),
            );
        }
        let open_count = self
            .threads
            .iter()
            .filter(|entry| !entry.is_resolved())
            .count();
        if rows.is_empty() && self.draft.is_none() {
            // Every thread is resolved and hidden. The list still holds the
            // focus; the footer below leads back to the resolved threads.
            rows.push(
                div()
                    .debug_selector(|| "comments-empty".into())
                    .flex_none()
                    .px(SPACE_1)
                    .py(SPACE_1)
                    .text_color(colors.muted)
                    .child("No open comments.")
                    .into_any_element(),
            );
        }

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
        let root = panel_actions(window_actions(root, &panel), &panel);

        root.child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_between()
                .h(px(36.))
                .px(SPACE_3)
                .border_b_1()
                .border_color(colors.hairline)
                .child(SectionLabel::new("Comments"))
                .child(div().text_size(UI_TEXT_XS).text_color(colors.muted).child(
                    match open_count {
                        1 => "1 open".to_string(),
                        count => format!("{count} open"),
                    },
                )),
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
                .flex()
                .flex_col()
                .gap(SPACE_1)
                .p(SPACE_2)
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .children(rows),
        )
        .when_some(self.render_footer(&colors, cx), |root, footer| {
            root.child(footer)
        })
        .into_any_element()
    }
}

impl CommentsPanel {
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
    fn render_no_comments(&self, colors: &ThemeColors, ui_font: SharedString, cx: &App) -> Div {
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
            .child(hint_row(colors).child(hint(
                "Add one on the selected text",
                KeyHint::for_action(&AddComment, cx),
                colors,
            )))
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
            .pb(SPACE_1)
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

    fn render_composer_hints(
        &self,
        submit_label: &'static str,
        colors: &ThemeColors,
        cx: &App,
    ) -> Div {
        hint_row(colors)
            // `enter` submits through the compact editor's own binding.
            .child(hint(
                submit_label,
                KeyHint::for_action(&writer_editor::actions::Newline, cx),
                colors,
            ))
            .child(hint(
                "Cancel",
                KeyHint::for_action(&CancelComment, cx),
                colors,
            ))
    }

    /// The block under the header while a new comment is being written:
    /// the quoted target, the composer, the keys.
    fn render_new_composer(
        &self,
        composer_focused: bool,
        colors: &ThemeColors,
        cx: &App,
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
                .child(quote_line(draft.quote.clone().into(), colors))
                .child(self.render_composer_box(composer_focused, colors))
                .child(self.render_composer_hints("Comment", colors, cx)),
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
            .pl(SPACE_2)
            .rounded(RADIUS_MD)
            .border_l_2()
            .border_color(if is_active {
                colors.accent
            } else {
                gpui::transparent_black()
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
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.click_thread(id, window, cx);
            }));

        if !entry.quote.is_empty() {
            card = card.child(quote_line(entry.quote.clone().into(), colors));
        }
        card = card.child(meta_line(
            &entry.thread.author,
            relative_time(entry.thread.created_at, now),
            entry.is_resolved(),
            colors,
        ));
        card = if editing_comment {
            card.child(self.render_composer_box(composer_focused, colors))
                .child(self.render_composer_hints("Save", colors, cx))
        } else {
            card.child(body_text(&entry.thread.body, colors))
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
                            colors,
                        ))
                        .child(self.render_composer_box(composer_focused, colors))
                        .child(self.render_composer_hints("Save", colors, cx))
                } else {
                    reply_block(id, reply, selected == Some(reply.id), now, colors)
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
                .child(self.render_composer_hints("Reply", colors, cx));
        }

        if let Some(target) = pending_delete {
            card = card.child(self.render_delete_confirmation(target, entry, colors, cx));
        } else if is_active && list_focused && draft.is_none() {
            card = card.child(self.render_thread_hints(entry, colors, cx));
        }
        card
    }

    /// The keys that act on the active thread, shown while the list has
    /// focus. The glyphs come from the keymap.
    fn render_thread_hints(&self, entry: &ThreadEntry, colors: &ThemeColors, cx: &App) -> Div {
        let on_reply = self.selected_reply.is_some();
        hint_row(colors)
            .mt(SPACE_1)
            .child(hint(
                "Reply",
                KeyHint::for_action(&ReplyToThread, cx),
                colors,
            ))
            .child(hint(
                if on_reply { "Edit reply" } else { "Edit" },
                KeyHint::for_action(&EditComment, cx),
                colors,
            ))
            .child(hint(
                if entry.is_resolved() {
                    "Reopen"
                } else {
                    "Resolve"
                },
                KeyHint::for_action(&ToggleResolved, cx),
                colors,
            ))
            .child(hint(
                if on_reply { "Delete reply" } else { "Delete" },
                KeyHint::for_action(&DeleteComment, cx),
                colors,
            ))
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
                    .child(
                        Button::new("confirm-delete-comment", "Delete")
                            .danger()
                            .size(ButtonSize::Compact)
                            .key_hint(KeyHint::for_action(&ConfirmDelete, cx))
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.confirm_delete(window, cx);
                            })),
                    )
                    .child(
                        Button::new("cancel-delete-comment", "Keep")
                            .size(ButtonSize::Compact)
                            .key_hint(KeyHint::for_action(&CancelDelete, cx))
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.cancel_delete(window, cx);
                            })),
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

    /// A click on a thread: it becomes active, the editor scrolls to its
    /// anchor, and the list takes focus (unless a composer is open, which
    /// keeps it).
    fn click_thread(&mut self, id: CommentId, window: &mut Window, cx: &mut Context<Self>) {
        self.select_thread(Some(id), cx);
        if self.draft.is_none() {
            window.focus(&self.focus_handle);
        }
        cx.notify();
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
fn meta_line(author: &str, time: String, resolved: bool, colors: &ThemeColors) -> Div {
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
                .text_color(colors.ink)
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

fn body_text(body: &str, colors: &ThemeColors) -> Div {
    div()
        .min_w_0()
        .text_color(colors.ink)
        .child(SharedString::from(body.to_string()))
}

fn reply_block(
    thread: CommentId,
    reply: &Reply,
    selected: bool,
    now: i64,
    colors: &ThemeColors,
) -> Div {
    let ReplyId(reply_id) = reply.id;
    div()
        .debug_selector(|| format!("comment-reply-{}-{}", thread.0, reply_id))
        .flex()
        .flex_col()
        .gap(SPACE_0_5)
        .px(SPACE_1)
        .py(SPACE_0_5)
        .rounded(RADIUS_SM)
        .when(selected, |block| block.bg(colors.selection))
        .child(meta_line(
            &reply.author,
            relative_time(reply.created_at, now),
            false,
            colors,
        ))
        .child(body_text(&reply.body, colors))
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
