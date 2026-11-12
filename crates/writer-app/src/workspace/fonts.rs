//! The fonts of the settings in the workspace: the searchable list behind
//! the two font rows ([`crate::font_choice`]) and what a choice does.

use gpui::{App, AppContext as _, Context, SharedString, Window, px};
use writer_editor::Autoscroll;
use writer_editor_tools::FontList;
use writer_ui::Picker;

use super::{MODAL_WIDTH, Workspace, picker_rows};
use crate::{
    font_choice::{FontChoiceDelegate, FontChoicePicker, FontTarget},
    settings::keys,
};

impl Workspace {
    /// Opens the searchable list of fonts for `target` in the place of the
    /// settings; it goes back to its row there, chosen or not. Nothing
    /// happens for the open document when there is none or it cannot be
    /// changed.
    pub(crate) fn show_font_list(
        &mut self,
        target: FontTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = match target {
            FontTarget::NewDocuments => self.settings.new_document_font.clone(),
            FontTarget::Document => {
                if self.session.is_none() || self.editor.read(cx).is_read_only() {
                    return;
                }
                self.document_font(cx)
            }
        };
        let workspace = cx.weak_entity();
        let fonts = FontList::current(cx);
        self.modal_layer.update(cx, |layer, cx| {
            let view = cx.new(|cx| {
                let delegate = FontChoiceDelegate::new(workspace, target, current, fonts);
                let rows = picker_rows(window, 14);
                Picker::modal(delegate, window, cx)
                    .width(px(MODAL_WIDTH))
                    .max_visible_rows(rows)
            });
            layer.show(view, window, cx);
        });
    }

    /// The body font of the open document.
    pub fn document_font(&self, cx: &App) -> String {
        self.editor
            .read(cx)
            .document()
            .settings()
            .typography
            .body_font
            .clone()
    }

    /// A font was chosen in the list of the settings. `installed`: it is a
    /// family of this Mac (not the row of a font that is set but missing),
    /// and so one of the recently used families from now on, as a family
    /// chosen in the font picker of the text is.
    pub(crate) fn choose_font(
        &mut self,
        target: FontTarget,
        font: &str,
        installed: bool,
        cx: &mut Context<Self>,
    ) {
        match target {
            FontTarget::NewDocuments => self.set_new_document_font(font, cx),
            FontTarget::Document => self.set_document_font(font, cx),
        }
        if installed {
            writer_editor_tools::remember_font(&SharedString::from(font.to_string()), cx);
        }
    }

    /// Sets the body font a new document starts with.
    pub fn set_new_document_font(&mut self, font: &str, cx: &mut Context<Self>) {
        if font.trim().is_empty() || font == self.settings.new_document_font {
            return;
        }
        self.write_setting(keys::NEW_DOCUMENT_FONT, font, cx);
    }

    /// Sets the body font of the open document. A change of the document's
    /// settings like any other: saved with the text, not a step of undo.
    pub fn set_document_font(&mut self, font: &str, cx: &mut Context<Self>) {
        if self.session.is_none() || font.trim().is_empty() {
            return;
        }
        let font = font.to_string();
        self.editor.update(cx, |editor, cx| {
            if editor.is_read_only() || editor.document().settings().typography.body_font == font {
                return;
            }
            editor.edit_with(
                Autoscroll::None,
                |state| {
                    let mut settings = state.document().settings().clone();
                    settings.typography.body_font = font;
                    state.set_settings(settings.sanitized())
                },
                cx,
            );
        });
    }

    /// The open list of fonts of the settings, for the `state` step of a
    /// scripted run: whose font, what is typed, how many rows (headings
    /// not counted), the marked row, the font in use and whether it is
    /// missing on this Mac. `None` while the list is not open.
    pub fn describe_font_list(&self, cx: &App) -> Option<String> {
        let picker = self.modal_layer.read(cx).active::<FontChoicePicker>()?;
        let picker = picker.read(cx);
        let delegate = &picker.delegate;
        let fonts = (0..delegate.rows().len())
            .filter(|ix| delegate.font_at(*ix).is_some())
            .count();
        let selected = picker
            .selected_index()
            .and_then(|ix| delegate.font_at(ix))
            .unwrap_or_default();
        Some(format!(
            "target={} query={:?} rows={fonts} selected={selected:?} in_use={:?} missing={}",
            match delegate.target() {
                FontTarget::NewDocuments => "new-documents",
                FontTarget::Document => "this-document",
            },
            delegate.query(),
            delegate.current(),
            delegate.missing().is_some(),
        ))
    }
}
