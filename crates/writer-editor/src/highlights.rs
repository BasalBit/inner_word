//! Highlight layers (find matches, Vim search) and comment tints.
//!
//! A layer is a named set of document ranges with one style. Layers are
//! painted behind the text and under the selection, in the order they were
//! first set (a later layer paints over an earlier one), so "find" followed
//! by "find-current" shows the current match on top.
//!
//! Ranges are plain positions: they do not move with edits. A consumer
//! recomputes its ranges on [`crate::EditorEvent::Edited`] and
//! [`crate::EditorEvent::DocumentReplaced`].

use gpui::{Hsla, SharedString};
use writer_core::TextRange;

/// How the ranges of a highlight layer are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HighlightStyle {
    /// Fill behind the text (use a translucent colour such as the theme's
    /// `find_match`).
    pub background: Option<Hsla>,
    /// A 1px line under the text.
    pub underline: Option<Hsla>,
}

impl HighlightStyle {
    /// A background fill.
    pub fn background(color: Hsla) -> Self {
        Self {
            background: Some(color),
            underline: None,
        }
    }

    /// Adds an underline.
    pub fn underline(mut self, color: Hsla) -> Self {
        self.underline = Some(color);
        self
    }
}

pub(crate) struct HighlightLayer {
    pub(crate) name: SharedString,
    /// Ordered ranges, sorted by start.
    pub(crate) ranges: Vec<TextRange>,
    pub(crate) style: HighlightStyle,
    /// The largest number of block boundaries one range crosses.
    max_span: usize,
}

/// All highlight layers of an editor, in paint order.
#[derive(Default)]
pub(crate) struct Highlights {
    layers: Vec<HighlightLayer>,
}

impl Highlights {
    /// Replaces (or creates) a layer. An existing layer keeps its place in
    /// the paint order.
    pub(crate) fn set(&mut self, name: SharedString, ranges: Vec<TextRange>, style: HighlightStyle) {
        let mut ranges: Vec<TextRange> = ranges
            .into_iter()
            .map(|range| TextRange::new(range.start, range.end))
            .filter(|range| !range.is_empty())
            .collect();
        ranges.sort_by_key(|range| (range.start, range.end));
        let max_span = ranges
            .iter()
            .map(|range| range.end.block - range.start.block)
            .max()
            .unwrap_or(0);
        match self.layers.iter_mut().find(|layer| layer.name == name) {
            Some(layer) => {
                layer.ranges = ranges;
                layer.style = style;
                layer.max_span = max_span;
            }
            None => self.layers.push(HighlightLayer {
                name,
                ranges,
                style,
                max_span,
            }),
        }
    }

    /// Removes a layer. Returns whether it existed.
    pub(crate) fn clear(&mut self, name: &str) -> bool {
        let before = self.layers.len();
        self.layers.retain(|layer| layer.name.as_ref() != name);
        self.layers.len() != before
    }

    pub(crate) fn clear_all(&mut self) -> bool {
        let had = !self.layers.is_empty();
        self.layers.clear();
        had
    }

    pub(crate) fn get(&self, name: &str) -> Option<&HighlightLayer> {
        self.layers.iter().find(|layer| layer.name.as_ref() == name)
    }

    pub(crate) fn layers(&self) -> &[HighlightLayer] {
        &self.layers
    }
}

impl HighlightLayer {
    /// The ranges of this layer that touch block `block`.
    pub(crate) fn ranges_in_block(&self, block: usize) -> impl Iterator<Item = &TextRange> {
        let low = self
            .ranges
            .partition_point(|range| range.start.block + self.max_span < block);
        let high = self.ranges.partition_point(|range| range.start.block <= block);
        self.ranges[low..high]
            .iter()
            .filter(move |range| range.end.block >= block)
    }
}
