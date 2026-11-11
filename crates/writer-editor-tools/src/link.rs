//! The link popover (`cmd-k`): a URL field at the caret.
//!
//! `enter` applies, `cmd-enter` opens the URL in the browser,
//! `cmd-shift-k` removes the link, `escape` cancels. With a collapsed caret
//! outside a link, `enter` inserts the URL as linked text.

use gpui::{
    Context, Entity, EventEmitter, MouseButton, SharedString, Subscription, Window, div,
    prelude::*, px,
};
use writer_core::Choice;
use writer_editor::PopupAnchor;
use writer_ui::{
    Divider, FieldAppearance, KeyHint, TextField, TextFieldEvent,
    theme::{ActiveTheme, RADIUS_LG, RADIUS_SM, UI_TEXT_BASE, UI_TEXT_SM, UI_TEXT_XS},
};

use crate::{
    actions::{LINK_CONTEXT, OpenLinkInBrowser, RemoveLink},
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
            TextFieldEvent::Changed | TextFieldEvent::Focused => {}
        });
        Self {
            field,
            has_link,
            _subscription: subscription,
        }
    }

    fn footer_action(
        &self,
        id: &'static str,
        hint: Option<KeyHint>,
        label: &'static str,
        event: LinkEvent,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let colors = cx.theme().colors;
        div()
            .id(id)
            .debug_selector(|| id.to_string())
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(4.))
            .h(px(20.))
            .rounded(RADIUS_SM)
            .cursor_pointer()
            .hover(|style| style.bg(colors.hover).text_color(colors.ink))
            .children(hint.map(|hint| hint.size(UI_TEXT_XS)))
            .child(SharedString::from(label))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(event)))
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
            LinkEvent::Apply,
            cx,
        );
        let open = self.footer_action(
            "link-open",
            KeyHint::for_action(&OpenLinkInBrowser, cx),
            "Open",
            LinkEvent::Open,
            cx,
        );
        let remove = self.has_link.then(|| {
            self.footer_action(
                "link-remove",
                KeyHint::for_action(&RemoveLink, cx),
                "Remove",
                LinkEvent::Remove,
                cx,
            )
        });
        let cancel = self.footer_action(
            "link-cancel",
            KeyHint::parse("escape"),
            "Cancel",
            LinkEvent::Cancel,
            cx,
        );
        div()
            .key_context(LINK_CONTEXT)
            .debug_selector(|| "link-popover".into())
            .occlude()
            // Clicks on the panel keep focus in the field.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
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
        self.make_room(false, window, cx);
        let editor = self.editor.read(cx);
        let selection = editor.selection();
        let collapsed = selection.is_collapsed();
        let (url, has_link) = if collapsed {
            match editor.document().link_at(selection.head) {
                Some(link) => (link.url, true),
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
        let anchor = if collapsed {
            PopupAnchor::Caret
        } else {
            PopupAnchor::Selection
        };
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
