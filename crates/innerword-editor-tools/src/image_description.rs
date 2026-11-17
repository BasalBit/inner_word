//! The description of an image: one line that says what the picture
//! shows, written in a small popover under the picture.
//!
//! The exports write it (in a DOCX it is what a screen reader reads), the
//! agents and the assistant see it, and the editor shows it where the
//! picture is missing. The popover opens for the selected image, the one
//! the caret is on ([`crate::actions::EditImageDescription`]; "Description…"
//! in the image's context menu and in the format menu), and has a field
//! and three buttons:
//!
//! - **Done** keeps what the field holds (an empty field removes the
//!   description). Its key is the field's own submit.
//! - **Remove** takes the description away. It is there only while the
//!   image has one.
//! - **Cancel** closes and changes nothing. Its key is the one that
//!   closes a tool.
//!
//! **What was typed is not lost.** The link popover closes without
//! applying when the pointer is pressed outside it or the keyboard goes
//! elsewhere, which is right for an address half typed. A sentence is
//! another matter: here every way of closing keeps the field, except the
//! two that say otherwise (Cancel, and Remove) and a document that was
//! replaced under the popover. One place decides,
//! `EditorTools::settle_image_description`, called when the popup closes.
//!
//! The popover has no key context and no binding of its own: the keys are
//! the text field's (`TextFieldEvent::Submitted` and `Cancelled`), and the
//! buttons are buttons.

use std::cell::Cell;

use gpui::{
    App, Context, Entity, EventEmitter, SharedString, Subscription, Window, div, prelude::*, px,
};
use innerword_core::{BlockId, ImageId};
use innerword_editor::PopupAnchor;
use innerword_ui::{
    ContextMenu, Divider, FieldAppearance, KeyHint, KeyLabel, TextField, TextFieldEvent,
    theme::{ActiveTheme, RADIUS_LG, RADIUS_SM, UI_TEXT_BASE, UI_TEXT_SM, UI_TEXT_XS},
};

use crate::{
    tip::{apply_hint, close_hint, tip},
    tools::{EditorTools, PopupTool},
};

/// The line under the field: where the description goes.
const NOTE: &str = "Written into exports and shown to agents.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DescriptionEvent {
    /// Keep what the field holds.
    Done,
    /// Take the description away.
    Remove,
    /// Close and change nothing.
    Cancel,
    /// The keyboard left the field.
    Blurred,
    /// A mouse button went down somewhere outside the popover.
    ClickedOutside,
}

/// What closing the popover does with the field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// The field becomes the description.
    Keep,
    /// The image is left without a description.
    Remove,
    /// Nothing changes.
    Discard,
}

pub(crate) struct ImageDescriptionPopover {
    pub(crate) field: Entity<TextField>,
    /// The image has a description (shows the remove button).
    has_description: bool,
    _subscription: Subscription,
}

impl EventEmitter<DescriptionEvent> for ImageDescriptionPopover {}

impl ImageDescriptionPopover {
    fn new(description: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = cx.new(|cx| {
            TextField::new(window, cx)
                .with_placeholder("What the image shows")
                .with_text(description)
                .with_appearance(FieldAppearance::Borderless)
                .with_text_size(UI_TEXT_BASE)
                .consume_escape()
        });
        field.update(cx, |field, cx| field.select_all(cx));
        let subscription = cx.subscribe(&field, |_, _, event: &TextFieldEvent, cx| match event {
            TextFieldEvent::Submitted => cx.emit(DescriptionEvent::Done),
            TextFieldEvent::Cancelled => cx.emit(DescriptionEvent::Cancel),
            TextFieldEvent::Blurred => cx.emit(DescriptionEvent::Blurred),
            TextFieldEvent::Changed | TextFieldEvent::Focused => {}
        });
        Self {
            field,
            has_description: !description.is_empty(),
            _subscription: subscription,
        }
    }

    /// The key beside `button` (a debug selector of the footer), asked of
    /// the keymap as it is now. "Remove" has none.
    pub(crate) fn action_key(&self, button: &str, cx: &App) -> Option<KeyHint> {
        match button {
            "image-description-done" => apply_hint(cx),
            "image-description-cancel" => close_hint(cx),
            _ => None,
        }
    }

    /// The buttons of the footer, as debug selectors, in order.
    pub(crate) fn buttons(&self) -> Vec<&'static str> {
        let mut buttons = vec!["image-description-done"];
        if self.has_description {
            buttons.push("image-description-remove");
        }
        buttons.push("image-description-cancel");
        buttons
    }

    /// A button of the footer: its name, its key, and a tooltip that says
    /// what it does (the link popover's buttons, `LinkPopover::footer_action`).
    fn footer_action(
        &self,
        id: &'static str,
        label: &'static str,
        tooltip: &'static str,
        event: DescriptionEvent,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let colors = cx.theme().colors;
        let hint = self.action_key(id, cx);
        div()
            .id(id)
            .debug_selector(|| id.to_string())
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(4.))
            .h(px(20.))
            .rounded(RADIUS_SM)
            .child(KeyLabel::new(SharedString::from(label), hint.clone()))
            .cursor_pointer()
            .hover(|style| style.bg(colors.hover))
            .tooltip(tip(tooltip, hint))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(event)))
    }
}

impl Render for ImageDescriptionPopover {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let shadow = theme.shadow_popover();
        let font = theme.fonts.ui.clone();
        let done = self.footer_action(
            "image-description-done",
            "Done",
            "Keep this description",
            DescriptionEvent::Done,
            cx,
        );
        let remove = self.has_description.then(|| {
            self.footer_action(
                "image-description-remove",
                "Remove",
                "Remove the description",
                DescriptionEvent::Remove,
                cx,
            )
        });
        let cancel = self.footer_action(
            "image-description-cancel",
            "Cancel",
            "Close without changing the description",
            DescriptionEvent::Cancel,
            cx,
        );
        div()
            .debug_selector(|| "image-description-popover".into())
            .occlude()
            // Clicks on the panel keep focus in the field.
            .on_any_mouse_down(|_, window, _| window.prevent_default())
            // (A press while a context menu is open, the field's own for
            // example, belongs to that menu.)
            .on_mouse_down_out(cx.listener(|_, _, window, cx| {
                if !ContextMenu::is_open(window, cx) {
                    cx.emit(DescriptionEvent::ClickedOutside)
                }
            }))
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
                            .child("Description"),
                    )
                    .child(div().flex_1().min_w_0().child(self.field.clone())),
            )
            .child(
                div()
                    .debug_selector(|| "image-description-note".into())
                    .flex_none()
                    .px(px(10.))
                    .pb(px(8.))
                    .text_size(UI_TEXT_XS)
                    .line_height(px(14.))
                    .text_color(colors.muted)
                    .child(NOTE),
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
                    .child(done)
                    .children(remove)
                    .child(div().flex_1())
                    .child(cancel),
            )
    }
}

pub(crate) struct ImageDescriptionTool {
    pub(crate) popover: Entity<ImageDescriptionPopover>,
    /// The block of the image the popover was opened for. A block keeps
    /// its id while others come and go around it (an agent's change).
    block: BlockId,
    /// The stored picture that block named then. Another document can have
    /// a block with the same id: the description is only ever written to
    /// the picture it was typed for.
    image: ImageId,
    outcome: Cell<Outcome>,
    _subscription: Subscription,
}

impl EditorTools {
    /// Opens the description popover under the selected image, the field
    /// holding the description it has, all of it selected, and moves the
    /// keyboard into the field. Returns `false`, and does nothing, when
    /// the caret is not on an image. In a read-only editor nothing opens.
    pub fn open_image_description(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let editor = self.editor.read(cx);
        let Some((index, image)) = editor.selected_image() else {
            return false;
        };
        let Some(block) = editor.document().block(index).map(|block| block.id()) else {
            return false;
        };
        if editor.is_read_only() {
            return true;
        }
        self.make_room(window, cx);
        let popover = cx.new(|cx| ImageDescriptionPopover::new(&image.description, window, cx));
        let tool = ImageDescriptionTool {
            popover: popover.clone(),
            block,
            image: image.id,
            outcome: Cell::new(Outcome::Keep),
            _subscription: cx.subscribe_in(&popover, window, Self::on_image_description_event),
        };
        popover.read(cx).field.read(cx).focus(window);
        // The caret of a selected image is the picture's box: the popover
        // hangs under the picture.
        self.show_popup(
            PopupTool::ImageDescription(tool),
            popover.into(),
            PopupAnchor::Caret,
            cx,
        );
        true
    }

    /// The command: opens the popover, or closes it when it is open (what
    /// the field holds is kept, as on every close that was not a Cancel).
    /// Returns `false` when the caret is not on an image: the action then
    /// travels on.
    pub(crate) fn toggle_image_description(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if matches!(self.popup, Some(PopupTool::ImageDescription(_))) {
            self.close_popup(true, window, cx);
            return true;
        }
        self.open_image_description(window, cx)
    }

    /// The text of the description popover's field, when the popover is
    /// open.
    pub fn image_description_field_text(&self, cx: &App) -> Option<String> {
        match &self.popup {
            Some(PopupTool::ImageDescription(tool)) => {
                Some(tool.popover.read(cx).field.read(cx).text().to_string())
            }
            _ => None,
        }
    }

    /// The buttons the open description popover shows, in order, by their
    /// debug selectors: `"image-description-done"`,
    /// `"image-description-remove"` (only for an image that has a
    /// description) and `"image-description-cancel"`. Empty when the
    /// popover is closed. For tests and automation.
    pub fn image_description_buttons(&self, cx: &App) -> Vec<&'static str> {
        match &self.popup {
            Some(PopupTool::ImageDescription(tool)) => tool.popover.read(cx).buttons(),
            _ => Vec::new(),
        }
    }

    /// The key the open description popover shows beside a button (see
    /// [`EditorTools::image_description_buttons`] for the names), as text.
    /// `None` when the popover is closed, for "Remove", and when the
    /// keymap has no key for it.
    pub fn image_description_button_key(&self, button: &str, cx: &App) -> Option<String> {
        match &self.popup {
            Some(PopupTool::ImageDescription(tool)) => tool
                .popover
                .read(cx)
                .action_key(button, cx)
                .map(|hint| hint.text()),
            _ => None,
        }
    }

    fn on_image_description_event(
        &mut self,
        popover: &Entity<ImageDescriptionPopover>,
        event: &DescriptionEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(PopupTool::ImageDescription(tool)) = &self.popup else {
            return;
        };
        if tool.popover != *popover {
            return;
        }
        match event {
            DescriptionEvent::Done => self.close_popup(true, window, cx),
            DescriptionEvent::Remove => {
                tool.outcome.set(Outcome::Remove);
                self.close_popup(true, window, cx);
            }
            DescriptionEvent::Cancel => {
                tool.outcome.set(Outcome::Discard);
                self.close_popup(true, window, cx);
            }
            DescriptionEvent::Blurred => self.popup_lost_focus(window, cx),
            DescriptionEvent::ClickedOutside => self.popup_clicked_outside(window, cx),
        }
    }

    /// The open description popover closes without changing anything,
    /// whatever closes it next: Cancel, the key that closes a tool, a
    /// document that was replaced under it.
    pub(crate) fn abandon_image_description(&mut self) {
        if let Some(PopupTool::ImageDescription(tool)) = &self.popup {
            tool.outcome.set(Outcome::Discard);
        }
    }

    /// The document or the selection changed under the open description
    /// popover: it closes when the image it was opened for is gone (an
    /// agent took it out), and when the caret is no longer on that image
    /// (the popover hangs from the caret). In the second case what the
    /// field holds is still kept.
    pub(crate) fn sync_image_description(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(PopupTool::ImageDescription(tool)) = &self.popup else {
            return;
        };
        let editor = self.editor.read(cx);
        let still_selected = editor.selected_image().is_some_and(|(index, image)| {
            image.id == tool.image
                && editor
                    .document()
                    .block(index)
                    .is_some_and(|block| block.id() == tool.block)
        });
        if still_selected {
            return;
        }
        let had_focus = tool.popover.read(cx).field.read(cx).is_focused(window);
        self.close_popup(had_focus, window, cx);
    }

    /// Called when the description popover closes: does what was decided
    /// with the field. One edit, so one undo step; nothing at all when the
    /// description stays what it is, when the image is gone, and in an
    /// editor that cannot be changed.
    pub(crate) fn settle_image_description(
        &mut self,
        tool: &ImageDescriptionTool,
        cx: &mut Context<Self>,
    ) {
        let description = match tool.outcome.get() {
            Outcome::Keep => tool.popover.read(cx).field.read(cx).text().to_string(),
            Outcome::Remove => String::new(),
            Outcome::Discard => return,
        };
        self.editor.update(cx, |editor, cx| {
            if editor.is_read_only() {
                return;
            }
            let document = editor.document();
            let Some(index) = document.block_index(tool.block) else {
                return;
            };
            let same_image = document
                .block(index)
                .and_then(|block| block.as_image())
                .is_some_and(|image| image.id == tool.image);
            if same_image {
                editor.edit(|state| state.set_image_description(index, &description), cx);
            }
        });
    }
}
