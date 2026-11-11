//! Compact mode: the editor as a small plain-text input (comment composer,
//! reply field).
//!
//! A compact editor is created with `EditorConfig::compact(..)`. It differs
//! from the document editor in layout and in what it accepts: interface
//! font and size, no column and no page view, as wide as its container, as
//! tall as its content up to `CompactOptions::max_height` (then it
//! scrolls). It holds plain text: formatting and block commands are
//! ignored (they propagate to the next binding), paste inserts plain text,
//! `tab` and `shift-tab` move focus. With `submit_on_enter`, `enter` emits
//! `EditorEvent::Submitted` and `shift-enter` inserts a line break. Give it
//! padding and a border by wrapping it in a styled div.
//!
//! This module holds the plain-text conveniences and the height
//! measurement the element uses.

use gpui::{App, Context, Pixels, Window, px};
use writer_core::{Document, Fragment};

use crate::{config::EditorMode, editor::Editor};

impl Editor {
    /// The document as plain text: blocks joined by `\n`.
    pub fn plain_text(&self) -> String {
        self.state.document().plain_text()
    }

    /// Whether the document has no text at all.
    pub fn is_empty(&self) -> bool {
        let document = self.state.document();
        document.block_count() == 1
            && document.blocks()[0].is_empty()
            && !document.blocks()[0].is_void()
    }

    /// Replaces the document with plain text, one paragraph per line (see
    /// [`Editor::set_document`] for what else is reset). The caret goes to
    /// the end.
    pub fn set_plain_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let document = if text.is_empty() {
            Document::new()
        } else {
            Fragment::from_plain_text(text).to_document()
        };
        self.set_document(document, cx);
        let end = self.state.document().end();
        self.state.set_caret(end);
        // A fresh document: nothing to undo, no change to report.
        self.state.take_changes();
    }

    /// Empties the editor (after a submit).
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.set_document(Document::new(), cx);
    }

    /// The height a compact editor wants at `width`: its content, at least
    /// one row, at most `CompactOptions::max_height`. Used by the element's
    /// measured layout.
    pub(crate) fn measure_compact_height(
        &mut self,
        width: Pixels,
        window: &mut Window,
        cx: &App,
    ) -> Pixels {
        let EditorMode::Compact(options) = self.config.mode else {
            return px(0.);
        };
        self.install_layout_env(None, width, window, cx);
        let content = self
            .with_layout(|lay| {
                // A composer is small: its real height, not an estimate.
                lay.layout.measure_all(lay.document);
                lay.layout.content_height()
            })
            .unwrap_or(px(0.));
        content.min(options.max_height)
    }
}
