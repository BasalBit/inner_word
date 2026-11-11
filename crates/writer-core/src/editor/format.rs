//! Inline formatting commands and the formatting state query.

use crate::attrs::{Highlight, InlineAttrs, InlineStyle, Rgb};
use crate::block::{Alignment, BlockKind};
use crate::document::Document;
use crate::history::EditKind;
use crate::position::{Position, TextRange};
use crate::runs;

use super::EditorState;

/// The state of an on/off attribute over the selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Toggle {
    /// Off everywhere.
    #[default]
    Off,
    /// On everywhere.
    On,
    /// On in some places only.
    Mixed,
}

impl Toggle {
    fn add(self, on: bool, first: bool) -> Toggle {
        let v = if on { Toggle::On } else { Toggle::Off };
        if first || self == v { v } else { Toggle::Mixed }
    }

    /// Whether the attribute is on everywhere.
    pub fn is_on(self) -> bool {
        self == Toggle::On
    }
}

/// The state of a valued attribute over the selection.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice<T> {
    /// The same value everywhere.
    Same(T),
    /// Different values in different places.
    Mixed,
}

impl<T: PartialEq> Choice<T> {
    fn add(self, v: T) -> Choice<T> {
        match self {
            Choice::Same(cur) if cur == v => Choice::Same(cur),
            _ => Choice::Mixed,
        }
    }

    /// The value, when it is the same everywhere.
    pub fn same(&self) -> Option<&T> {
        match self {
            Choice::Same(v) => Some(v),
            Choice::Mixed => None,
        }
    }
}

/// Formatting state of the selection, for menus and toolbars: per inline
/// attribute on / off / mixed (or the common value), plus the block-level
/// properties of the touched blocks.
///
/// With a collapsed caret the inline part describes what the next typed
/// character would get. Code blocks and void blocks are ignored for the
/// inline part when the selection also covers other text.
#[derive(Clone, Debug, PartialEq)]
pub struct FormatState {
    /// Bold.
    pub bold: Toggle,
    /// Italic.
    pub italic: Toggle,
    /// Underline.
    pub underline: Toggle,
    /// Strikethrough.
    pub strikethrough: Toggle,
    /// Inline code.
    pub code: Toggle,
    /// Highlight colour.
    pub highlight: Choice<Option<Highlight>>,
    /// Text colour.
    pub color: Choice<Option<Rgb>>,
    /// Font family.
    pub font: Choice<Option<String>>,
    /// Link URL.
    pub link: Choice<Option<String>>,
    /// Whether any selected text (or the caret position) carries a comment anchor.
    pub has_comments: bool,
    /// Kind of the touched blocks.
    pub kind: Choice<BlockKind>,
    /// Alignment of the touched blocks.
    pub alignment: Choice<Alignment>,
    /// Text size override of the touched blocks.
    pub text_size: Choice<Option<f32>>,
    /// Indent of the touched blocks.
    pub indent: Choice<u8>,
}

impl FormatState {
    /// The state of a toggle style.
    pub fn style(&self, style: InlineStyle) -> Toggle {
        match style {
            InlineStyle::Bold => self.bold,
            InlineStyle::Italic => self.italic,
            InlineStyle::Underline => self.underline,
            InlineStyle::Strikethrough => self.strikethrough,
            InlineStyle::Code => self.code,
        }
    }
}

/// A link and the text range it covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkInfo {
    /// The URL.
    pub url: String,
    /// The maximal contiguous range carrying this URL.
    pub range: TextRange,
}

impl Document {
    /// The link at `pos`: on the character before it, else the character
    /// after it. The range is the whole contiguous run of text with that URL.
    pub fn link_at(&self, pos: Position) -> Option<LinkInfo> {
        let pos = self.clamp_position(pos);
        let b = &self.blocks[pos.block];
        let url = b
            .attrs_before(pos.offset)
            .and_then(|a| a.link.clone())
            .or_else(|| b.attrs_at(pos.offset).and_then(|a| a.link.clone()))?;
        // Find the run containing a char with this url adjacent to pos.
        let spans: Vec<_> = b.spans().map(|(r, a)| (r, a.link.as_deref() == Some(&url))).collect();
        let idx = spans.iter().position(|(r, m)| {
            *m && ((r.start < pos.offset && pos.offset <= r.end) || r.start == pos.offset)
        })?;
        let mut s = idx;
        while s > 0 && spans[s - 1].1 {
            s -= 1;
        }
        let mut e = idx;
        while e + 1 < spans.len() && spans[e + 1].1 {
            e += 1;
        }
        Some(LinkInfo {
            url,
            range: TextRange::in_block(pos.block, spans[s].0.start, spans[e].0.end),
        })
    }
}

impl EditorState {
    /// Applies `f` to every formattable selected range (code and void blocks
    /// skipped) as one transaction.
    fn apply_to_selection(&mut self, f: impl Fn(&mut InlineAttrs)) -> bool {
        let ranges = self.selected_ranges(false);
        if ranges.is_empty() {
            return false;
        }
        self.edit(EditKind::Other, |ed| {
            for (i, r) in ranges {
                ed.update_block(i, |b| b.apply_attrs(r, &f));
            }
        })
    }

    /// Changes the pending attributes (collapsed caret). Also ends the
    /// current typing undo group, so text typed before and after the toggle
    /// undoes separately.
    fn change_pending(&mut self, f: impl FnOnce(&mut InlineAttrs)) {
        let mut a = self.current_attrs();
        f(&mut a);
        self.pending = Some(a);
        self.history.close();
    }

    /// Toggles a style over the selection: if all selected text already has
    /// it, removes it, otherwise applies it. With a collapsed caret, toggles
    /// it in the pending attributes used by the next insertion (returns
    /// false: the document did not change).
    pub fn toggle_style(&mut self, style: InlineStyle) -> bool {
        if self.selection.is_collapsed() {
            self.change_pending(|a| {
                let on = a.has_style(style);
                a.set_style(style, !on);
            });
            return false;
        }
        let ranges = self.selected_ranges(false);
        let all = ranges.iter().all(|(i, r)| {
            runs::spans_in(&self.doc.blocks[*i].runs, r.clone()).all(|(_, a)| a.has_style(style))
        });
        self.apply_to_selection(|a| a.set_style(style, !all))
    }

    /// Toggles bold (see [`EditorState::toggle_style`]).
    pub fn toggle_bold(&mut self) -> bool {
        self.toggle_style(InlineStyle::Bold)
    }

    /// Toggles italic.
    pub fn toggle_italic(&mut self) -> bool {
        self.toggle_style(InlineStyle::Italic)
    }

    /// Toggles underline.
    pub fn toggle_underline(&mut self) -> bool {
        self.toggle_style(InlineStyle::Underline)
    }

    /// Toggles strikethrough.
    pub fn toggle_strikethrough(&mut self) -> bool {
        self.toggle_style(InlineStyle::Strikethrough)
    }

    /// Toggles inline code.
    pub fn toggle_code(&mut self) -> bool {
        self.toggle_style(InlineStyle::Code)
    }

    /// Sets (or with `None` clears) the highlight over the selection, or in
    /// the pending attributes with a collapsed caret.
    pub fn set_highlight(&mut self, highlight: Option<Highlight>) -> bool {
        if self.selection.is_collapsed() {
            self.change_pending(|a| a.highlight = highlight);
            return false;
        }
        self.apply_to_selection(|a| a.highlight = highlight)
    }

    /// Toggles the highlight: clears it if all selected text has `highlight`,
    /// otherwise applies it.
    pub fn toggle_highlight(&mut self, highlight: Highlight) -> bool {
        let state = self.format_state();
        if state.highlight == Choice::Same(Some(highlight)) {
            self.set_highlight(None)
        } else {
            self.set_highlight(Some(highlight))
        }
    }

    /// Sets (or clears) the text colour.
    pub fn set_text_color(&mut self, color: Option<Rgb>) -> bool {
        if self.selection.is_collapsed() {
            self.change_pending(|a| a.color = color);
            return false;
        }
        self.apply_to_selection(|a| a.color = color)
    }

    /// Sets (or clears) the font family. Empty names clear.
    pub fn set_font_family(&mut self, family: Option<&str>) -> bool {
        let family = family.map(str::trim).filter(|f| !f.is_empty()).map(str::to_string);
        if self.selection.is_collapsed() {
            self.change_pending(|a| a.font = family.clone());
            return false;
        }
        self.apply_to_selection(|a| a.font = family.clone())
    }

    /// Clear formatting: removes bold, italic, underline, strikethrough,
    /// inline code, highlight, text colour and font family. Keeps links and
    /// comment anchors, and does not touch block kind, alignment or size.
    /// With a collapsed caret it clears the pending attributes.
    pub fn clear_formatting(&mut self) -> bool {
        if self.selection.is_collapsed() {
            self.change_pending(|a| *a = a.cleared());
            return false;
        }
        self.apply_to_selection(|a| *a = a.cleared())
    }

    /// Sets the link URL. With a selection, links the selected text; with a
    /// collapsed caret inside a link, changes the URL of that whole link
    /// (returns false when the caret is not in a link). An empty URL removes
    /// the link.
    pub fn set_link(&mut self, url: &str) -> bool {
        let url = url.trim();
        if url.is_empty() {
            return self.remove_link();
        }
        let url = url.to_string();
        if self.selection.is_collapsed() {
            let Some(info) = self.doc.link_at(self.selection.head) else {
                return false;
            };
            let r = info.range;
            return self.edit(EditKind::Other, |ed| {
                ed.update_block(r.start.block, |b| {
                    b.apply_attrs(r.start.offset..r.end.offset, |a| a.link = Some(url.clone()))
                });
            });
        }
        self.apply_to_selection(|a| a.link = Some(url.clone()))
    }

    /// Removes links from the selection, or the whole link at a collapsed caret.
    pub fn remove_link(&mut self) -> bool {
        if self.selection.is_collapsed() {
            let Some(info) = self.doc.link_at(self.selection.head) else {
                return false;
            };
            let r = info.range;
            return self.edit(EditKind::Other, |ed| {
                ed.update_block(r.start.block, |b| {
                    b.apply_attrs(r.start.offset..r.end.offset, |a| a.link = None)
                });
            });
        }
        self.apply_to_selection(|a| a.link = None)
    }

    /// Replaces the selection with `text` linked to `url` (current attributes
    /// otherwise). Line breaks in `text` become spaces. One undo step.
    pub fn insert_link(&mut self, text: &str, url: &str) -> bool {
        let url = url.trim();
        let text: String = crate::text::normalize_input(text).replace('\n', " ");
        if text.is_empty() || url.is_empty() {
            return false;
        }
        let mut attrs = self.current_attrs();
        attrs.link = Some(url.to_string());
        self.edit(EditKind::Other, |ed| {
            if !ed.selection.is_collapsed() {
                ed.delete_selection_impl();
            }
            let pos = ed.leave_void(ed.selection.head);
            let p = ed.insert_in_block(pos, &text, &attrs);
            ed.caret_to(p);
            ed.pending = None;
        })
    }

    /// The formatting state of the selection (see [`FormatState`]).
    pub fn format_state(&self) -> FormatState {
        let mut st = FormatState {
            bold: Toggle::Off,
            italic: Toggle::Off,
            underline: Toggle::Off,
            strikethrough: Toggle::Off,
            code: Toggle::Off,
            highlight: Choice::Same(None),
            color: Choice::Same(None),
            font: Choice::Same(None),
            link: Choice::Same(None),
            has_comments: false,
            kind: Choice::Mixed,
            alignment: Choice::Mixed,
            text_size: Choice::Mixed,
            indent: Choice::Mixed,
        };
        let mut first = true;
        let mut add = |st: &mut FormatState, a: &InlineAttrs| {
            st.bold = st.bold.add(a.bold, first);
            st.italic = st.italic.add(a.italic, first);
            st.underline = st.underline.add(a.underline, first);
            st.strikethrough = st.strikethrough.add(a.strikethrough, first);
            st.code = st.code.add(a.code, first);
            if first {
                st.highlight = Choice::Same(a.highlight);
                st.color = Choice::Same(a.color);
                st.font = Choice::Same(a.font.clone());
                st.link = Choice::Same(a.link.clone());
            } else {
                st.highlight = st.highlight.clone().add(a.highlight);
                st.color = st.color.clone().add(a.color);
                st.font = st.font.clone().add(a.font.clone());
                st.link = st.link.clone().add(a.link.clone());
            }
            st.has_comments |= !a.comments.is_empty();
            first = false;
        };
        let ranges = if self.selection.is_collapsed() {
            Vec::new()
        } else {
            self.selected_ranges(false)
        };
        if ranges.is_empty() {
            let a = self.current_attrs();
            add(&mut st, &a);
            if self.selection.is_collapsed() {
                st.has_comments = !self.doc.comments_at(self.selection.head).is_empty();
            }
        } else {
            for (i, r) in &ranges {
                for (_, a) in runs::spans_in(&self.doc.blocks[*i].runs, r.clone()) {
                    add(&mut st, a);
                }
            }
        }
        let touched = self.touched_blocks();
        for (n, i) in touched.enumerate() {
            let b = &self.doc.blocks[i];
            if n == 0 {
                st.kind = Choice::Same(b.kind.clone());
                st.alignment = Choice::Same(b.alignment);
                st.text_size = Choice::Same(b.text_size);
                st.indent = Choice::Same(b.indent);
            } else {
                st.kind = st.kind.add(b.kind.clone());
                st.alignment = st.alignment.add(b.alignment);
                st.text_size = st.text_size.add(b.text_size);
                st.indent = st.indent.add(b.indent);
            }
        }
        st
    }
}
