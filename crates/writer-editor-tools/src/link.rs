//! The link popover (`cmd-k`): a URL field at the caret.
//!
//! `enter` applies, `cmd-enter` opens the URL in the browser,
//! `cmd-shift-k` removes the link, `escape` cancels. With a collapsed caret
//! outside a link, `enter` inserts the URL as linked text.

use gpui::{
    App, Context, Entity, EventEmitter, MouseButton, SharedString, Subscription, Window, div,
    point, prelude::*, px,
};
use writer_core::{Choice, Position, TextRange};
use writer_editor::{Editor, PopupAnchor};
use writer_ui::{
    Divider, FieldAppearance, KeyHint, TextField, TextFieldEvent,
    theme::{ActiveTheme, RADIUS_LG, RADIUS_SM, UI_TEXT_BASE, UI_TEXT_SM, UI_TEXT_XS},
};

use crate::{
    actions::{LINK_CONTEXT, OpenLinkInBrowser, RemoveLink},
    tip::tip,
    tools::{EditorTools, PopupTool},
    util::normalize_url,
};

/// Longest selection looked at as a possible URL to prefill.
const PREFILL_LIMIT: usize = 2_048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkEvent {
    Apply,
    Open,
    Remove,
    Cancel,
    Blurred,
    /// A mouse button went down somewhere outside the popover.
    ClickedOutside,
}

pub(crate) struct LinkPopover {
    pub(crate) field: Entity<TextField>,
    /// The caret or selection is on a link (shows the remove action).
    has_link: bool,
    _subscription: Subscription,
}

impl EventEmitter<LinkEvent> for LinkPopover {}

impl LinkPopover {
    fn new(url: &str, has_link: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = cx.new(|cx| {
            TextField::new(window, cx)
                .with_placeholder("Paste or type a link")
                .with_text(url)
                .with_appearance(FieldAppearance::Borderless)
                .with_text_size(UI_TEXT_BASE)
                .consume_escape()
        });
        field.update(cx, |field, cx| field.select_all(cx));
        let subscription = cx.subscribe(&field, |_, _, event: &TextFieldEvent, cx| match event {
            TextFieldEvent::Submitted => cx.emit(LinkEvent::Apply),
            TextFieldEvent::Cancelled => cx.emit(LinkEvent::Cancel),
            TextFieldEvent::Blurred => cx.emit(LinkEvent::Blurred),
            // "Apply" and "Open" are offered only while there is something
            // to apply or open.
            TextFieldEvent::Changed => cx.notify(),
            TextFieldEvent::Focused => {}
        });
        Self {
            field,
            has_link,
            _subscription: subscription,
        }
    }

    /// Whether the field holds something that can be a URL.
    fn has_url(&self, cx: &App) -> bool {
        normalize_url(self.field.read(cx).text()).is_some()
    }

    /// Whether `button` (a debug selector of the footer) can be used now:
    /// "Open" needs a URL in the field; "Apply" needs one too, unless
    /// there is a link to take off by applying an empty field.
    pub(crate) fn action_enabled(&self, button: &str, cx: &App) -> bool {
        match button {
            "link-open" => self.has_url(cx),
            "link-apply" => self.has_url(cx) || self.has_link,
            _ => true,
        }
    }

    /// A button of the footer: its key, its name, and a tooltip that says
    /// what it does. A button that cannot be used now is dimmed and does
    /// not react.
    fn footer_action(
        &self,
        id: &'static str,
        hint: Option<KeyHint>,
        label: &'static str,
        tooltip: &'static str,
        event: LinkEvent,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let colors = cx.theme().colors;
        let enabled = self.action_enabled(id, cx);
        div()
            .id(id)
            .debug_selector(|| id.to_string())
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(4.))
            .h(px(20.))
            .rounded(RADIUS_SM)
            .children(hint.clone().map(|hint| hint.size(UI_TEXT_XS)))
            .child(SharedString::from(label))
            .map(|el| {
                if enabled {
                    el.cursor_pointer()
                        .hover(|style| style.bg(colors.hover).text_color(colors.ink))
                        .tooltip(tip(tooltip, hint))
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(event)))
                } else {
                    el.opacity(0.4)
                }
            })
    }
}

impl Render for LinkPopover {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let shadow = theme.shadow_popover();
        let font = theme.fonts.ui.clone();
        let apply = self.footer_action(
            "link-apply",
            KeyHint::parse("enter"),
            "Apply",
            if self.has_link && !self.has_url(cx) {
                "Remove the link"
            } else {
                "Link the text to this address"
            },
            LinkEvent::Apply,
            cx,
        );
        let open = self.footer_action(
            "link-open",
            KeyHint::for_action(&OpenLinkInBrowser, cx),
            "Open",
            "Open the address in the browser",
            LinkEvent::Open,
            cx,
        );
        let remove = self.has_link.then(|| {
            self.footer_action(
                "link-remove",
                KeyHint::for_action(&RemoveLink, cx),
                "Remove",
                "Remove the link, keep the text",
                LinkEvent::Remove,
                cx,
            )
        });
        let cancel = self.footer_action(
            "link-cancel",
            KeyHint::parse("escape"),
            "Cancel",
            "Close without changing the link",
            LinkEvent::Cancel,
            cx,
        );
        div()
            .key_context(LINK_CONTEXT)
            .debug_selector(|| "link-popover".into())
            .occlude()
            // Clicks on the panel keep focus in the field.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(LinkEvent::ClickedOutside)))
            .on_action(cx.listener(|_, _: &OpenLinkInBrowser, _, cx| cx.emit(LinkEvent::Open)))
            .w(px(380.))
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(colors.elevated)
            .border_1()
            .border_color(colors.hairline)
            .rounded(RADIUS_LG)
            .shadow(shadow)
            .font_family(font)
            .text_size(UI_TEXT_BASE)
            .line_height(px(18.))
            .text_color(colors.ink)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .h(px(34.))
                    .px(px(10.))
                    .child(
                        div()
                            .flex_none()
                            .text_size(UI_TEXT_SM)
                            .text_color(colors.muted)
                            .child("Link"),
                    )
                    .child(div().flex_1().min_w_0().child(self.field.clone())),
            )
            .child(Divider::horizontal())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .h(px(28.))
                    .px(px(8.))
                    .text_size(UI_TEXT_XS)
                    .text_color(colors.muted)
                    .child(apply)
                    .child(open)
                    .children(remove)
                    .child(div().flex_1())
                    .child(cancel),
            )
    }
}

/// Where the popover of the link at a collapsed caret goes. A link that
/// runs over several rows would be covered from the caret's row down, so
/// the popover hangs from the link's last row instead (from the position
/// that row starts with, or the link starts with on that row). With the
/// caret on the last row, and for a link on one row, it is the caret.
fn link_anchor(editor: &Editor, link: TextRange, caret: Position) -> PopupAnchor {
    let last_row_start = || {
        let rows = editor.range_bounds(link)?;
        let last = rows.last().filter(|_| rows.len() > 1)?;
        let caret_row = editor.position_bounds(caret)?;
        if caret_row.top() >= last.top() - px(1.) {
            return None;
        }
        let row_of = |position: Position| {
            editor
                .position_bounds(position)
                .is_some_and(|bounds| (bounds.top() - last.top()).abs() < px(1.))
        };
        let start = editor.position_at_point(point(last.left() + px(1.), last.center().y))?;
        // The start of a wrapped row is also the end of the row before
        // it; when the editor resolves it to that row, one character
        // further is on the right one.
        if row_of(start) {
            return Some(start);
        }
        let text = editor.document().block(start.block)?.text();
        let next = text.get(start.offset..)?.chars().next()?.len_utf8();
        let after = Position::new(start.block, start.offset + next);
        (after <= link.end && row_of(after)).then_some(after)
    };
    last_row_start().map_or(PopupAnchor::Caret, PopupAnchor::Position)
}

pub(crate) struct LinkTool {
    pub(crate) popover: Entity<LinkPopover>,
    _subscription: Subscription,
}

/// Whether selected text is itself a URL worth prefilling.
fn looks_like_url(text: &str) -> bool {
    !text.is_empty()
        && !text.contains(char::is_whitespace)
        && (text.contains("://") || text.starts_with("www.") || text.starts_with("mailto:"))
}

impl EditorTools {
    /// Opens the link popover at the caret or selection, prefilled with the
    /// link that is there. Does nothing in a read-only editor.
    pub fn open_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only(cx) {
            return;
        }
        self.make_room(window, cx);
        let editor = self.editor.read(cx);
        let selection = editor.selection();
        let collapsed = selection.is_collapsed();
        let mut anchor = if collapsed {
            PopupAnchor::Caret
        } else {
            PopupAnchor::Selection
        };
        let (url, has_link) = if collapsed {
            match editor.document().link_at(selection.head) {
                Some(link) => {
                    anchor = link_anchor(editor, link.range, selection.head);
                    (link.url, true)
                }
                None => (String::new(), false),
            }
        } else {
            match editor.state().format_state().link {
                Choice::Same(Some(url)) => (url, true),
                Choice::Mixed => (String::new(), true),
                Choice::Same(None) => {
                    let range = selection.range();
                    let short = range.start.block == range.end.block
                        && range.end.offset - range.start.offset <= PREFILL_LIMIT;
                    let text = if short {
                        editor.state().selected_text()
                    } else {
                        String::new()
                    };
                    let text = text.trim();
                    (
                        if looks_like_url(text) {
                            text.to_string()
                        } else {
                            String::new()
                        },
                        false,
                    )
                }
            }
        };
        let popover = cx.new(|cx| LinkPopover::new(&url, has_link, window, cx));
        let tool = LinkTool {
            popover: popover.clone(),
            _subscription: cx.subscribe_in(&popover, window, Self::on_link_event),
        };
        popover.read(cx).field.read(cx).focus(window);
        self.show_popup(PopupTool::Link(tool), popover.into(), anchor, cx);
    }

    /// `cmd-k`: opens the link popover, or closes it when it is open.
    pub(crate) fn toggle_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.popup, Some(PopupTool::Link(_))) {
            self.close_popup(true, window, cx);
        } else {
            self.open_link(window, cx);
        }
    }

    /// Whether a button of the open link popover can be used now
    /// (`"link-apply"`, `"link-open"`, `"link-remove"`, `"link-cancel"`):
    /// "Open" and "Apply" are dimmed while the field is empty and there
    /// is nothing to open or apply. `None` when the popover is closed.
    pub fn link_button_enabled(&self, button: &str, cx: &App) -> Option<bool> {
        match &self.popup {
            Some(PopupTool::Link(tool)) => Some(tool.popover.read(cx).action_enabled(button, cx)),
            _ => None,
        }
    }

    /// The text of the link popover's field, when the popover is open.
    pub fn link_field_text(&self, cx: &gpui::App) -> Option<String> {
        match &self.popup {
            Some(PopupTool::Link(tool)) => {
                Some(tool.popover.read(cx).field.read(cx).text().to_string())
            }
            _ => None,
        }
    }

    fn on_link_event(
        &mut self,
        popover: &Entity<LinkPopover>,
        event: &LinkEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(&self.popup, Some(PopupTool::Link(tool)) if tool.popover == *popover) {
            return;
        }
        match event {
            LinkEvent::Apply => self.apply_link(window, cx),
            LinkEvent::Open => self.open_link_in_browser(window, cx),
            LinkEvent::Remove => self.remove_link(window, cx),
            LinkEvent::Cancel => self.close_popup(true, window, cx),
            LinkEvent::Blurred => self.popup_lost_focus(window, cx),
            LinkEvent::ClickedOutside => self.popup_clicked_outside(window, cx),
        }
    }

    /// Applies the field: links the selection, changes the link at the
    /// caret, or inserts the URL as linked text. An empty field removes the
    /// link.
    fn apply_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(typed) = self.link_field_text(cx) else {
            return;
        };
        let url = normalize_url(&typed);
        self.close_popup(true, window, cx);
        self.editor.update(cx, |editor, cx| {
            if editor.is_read_only() {
                return;
            }
            editor.edit(
                |state| match &url {
                    None => state.remove_link(),
                    Some(url) => {
                        let selection = state.selection();
                        let insert = selection.is_collapsed()
                            && state.document().link_at(selection.head).is_none();
                        if insert {
                            state.insert_link(typed.trim(), url)
                        } else {
                            state.set_link(url)
                        }
                    }
                },
                cx,
            );
        });
    }

    /// Removes the link at the caret or from the selection, and closes the
    /// link popover when it is open.
    pub fn remove_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.popup, Some(PopupTool::Link(_))) {
            self.close_popup(true, window, cx);
        }
        self.editor.update(cx, |editor, cx| {
            if !editor.is_read_only() {
                editor.edit(|state| state.remove_link(), cx);
            }
        });
    }

    /// Opens a link in the browser: the URL in the link popover's field
    /// when the popover is open, else the link at the caret.
    pub fn open_link_in_browser(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let url = match self.link_field_text(cx) {
            Some(typed) => normalize_url(&typed),
            None => {
                let editor = self.editor.read(cx);
                editor
                    .document()
                    .link_at(editor.selection().head)
                    .map(|link| link.url)
            }
        };
        if let Some(url) = url {
            cx.open_url(&url);
        }
    }
}
