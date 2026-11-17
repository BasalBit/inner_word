//! The character-limit indicator of the status bar, counted block by block.
//!
//! A network counts a post as one text: trimmed at both ends, in grapheme
//! clusters, links at a fixed length ([`innerword_export::SocialNetwork`]).
//! Counting the whole text again after every keystroke is fine for a post
//! and wasteful for a book that happens to have the indicator on, so
//! [`LimitCounter`] keeps one count per block, keyed by the block's id and
//! revision stamp, and after an edit recounts only the blocks that changed.
//!
//! The sum over blocks is exactly the count of the whole text because a
//! block boundary is a line break: no grapheme cluster and no link reaches
//! across it, and every network counts it as one character. Trimming only
//! touches the first and the last block that have text.
//!
//! Two shapes of text would make a per-block sum differ from the whole
//! count (a combining mark right after leading white space, a carriage
//! return at a block edge). A block like that is noticed and the counter
//! falls back to counting the whole text, so the number is always the one
//! [`SocialNetwork::count`] gives for [`Document::plain_text`].

use unicode_segmentation::GraphemeCursor;
use innerword_core::{BlockId, Document};
use innerword_export::{SocialCount, SocialNetwork};

/// The counts of one block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    id: BlockId,
    revision: u64,
    /// The block has no text besides white space.
    blank: bool,
    /// The length of the text without the white space at its ends.
    inner: usize,
    /// The length of the white space at the start.
    lead: usize,
    /// The length of the white space at the end.
    trail: usize,
    /// Whether `lead + inner + trail` is the length of the whole block.
    additive: bool,
}

/// Counts a document against a network's post limit, cheaply after small
/// edits. See the [module docs](self).
#[derive(Clone, Debug)]
pub struct LimitCounter {
    network: SocialNetwork,
    entries: Vec<Entry>,
    /// How many blocks the last [`LimitCounter::count`] had to count.
    recounted: usize,
}

impl LimitCounter {
    /// A counter for `network`, with nothing counted yet.
    pub fn new(network: SocialNetwork) -> Self {
        Self {
            network,
            entries: Vec::new(),
            recounted: 0,
        }
    }

    /// The network this counter counts for.
    pub fn network(&self) -> SocialNetwork {
        self.network
    }

    /// How many blocks the last call of [`LimitCounter::count`] counted
    /// (the others were taken from the previous call).
    pub fn recounted(&self) -> usize {
        self.recounted
    }

    /// The length of `document` as the network counts it. The same result
    /// as `network.count(&document.plain_text())`.
    ///
    /// The counter remembers the blocks of the document it was last given:
    /// use one counter per open document and drop it when another document
    /// is opened (revision stamps start over in a loaded document).
    pub fn count(&mut self, document: &Document) -> SocialCount {
        self.sync(document);
        let length = match self.sum() {
            Some(length) => length,
            None => self.network.length(&document.plain_text()),
        };
        SocialCount {
            network: self.network,
            length,
            limit: self.network.limit(),
        }
    }

    /// Brings the entries in line with the blocks: whatever matches at the
    /// start and at the end is kept, the stretch between is counted.
    fn sync(&mut self, document: &Document) {
        let blocks = document.blocks();
        let same = |entry: &Entry, block: &innerword_core::Block| {
            entry.id == block.id() && entry.revision == block.revision()
        };
        let prefix = self
            .entries
            .iter()
            .zip(blocks)
            .take_while(|(entry, block)| same(entry, block))
            .count();
        let suffix = self.entries[prefix..]
            .iter()
            .rev()
            .zip(blocks[prefix..].iter().rev())
            .take_while(|(entry, block)| same(entry, block))
            .count();
        let stale = prefix..self.entries.len() - suffix;
        let fresh = &blocks[prefix..blocks.len() - suffix];
        self.recounted = fresh.len();
        if stale.is_empty() && fresh.is_empty() {
            return;
        }
        let network = self.network;
        self.entries
            .splice(stale, fresh.iter().map(|block| entry(network, block)));
    }

    /// The length of the whole text from the entries, or `None` when a
    /// block cannot be summed.
    fn sum(&self) -> Option<usize> {
        if self.entries.iter().any(|entry| !entry.additive) {
            return None;
        }
        let Some(first) = self.entries.iter().position(|entry| !entry.blank) else {
            return Some(0);
        };
        let last = self
            .entries
            .iter()
            .rposition(|entry| !entry.blank)
            .unwrap_or(first);
        if first == last {
            return Some(self.entries[first].inner);
        }
        let between: usize = self.entries[first + 1..last]
            .iter()
            .map(|entry| entry.lead + entry.inner + entry.trail)
            .sum();
        let head = &self.entries[first];
        let tail = &self.entries[last];
        // One line break between each pair of neighbours.
        Some(head.inner + head.trail + between + tail.lead + tail.inner + (last - first))
    }
}

fn entry(network: SocialNetwork, block: &innerword_core::Block) -> Entry {
    let text = block.text();
    let inner_text = text.trim();
    let blank = inner_text.is_empty();
    let lead_text = if blank {
        text
    } else {
        &text[..text.len() - text.trim_start().len()]
    };
    let trail_text = if blank {
        ""
    } else {
        &text[text.trim_end().len()..]
    };
    // A piece counts on its own only if the text may be cut where it ends:
    // not inside a grapheme cluster. A carriage return could pair with the
    // line break between blocks.
    let cut_ok = |at: usize| {
        at == 0
            || at == text.len()
            || GraphemeCursor::new(at, text.len(), true)
                .is_boundary(text, 0)
                .unwrap_or(false)
    };
    let additive =
        !text.contains('\r') && cut_ok(lead_text.len()) && cut_ok(text.len() - trail_text.len());
    Entry {
        id: block.id(),
        revision: block.revision(),
        blank,
        inner: network.length(inner_text),
        lead: white_space_length(network, lead_text),
        trail: white_space_length(network, trail_text),
        additive,
    }
}

/// The length of a run of white space. [`SocialNetwork::length`] trims its
/// argument, so the run is counted between two full stops (each counts as
/// one on every network and never joins a neighbour).
fn white_space_length(network: SocialNetwork, white_space: &str) -> usize {
    if white_space.is_empty() {
        return 0;
    }
    network
        .length(&format!(".{white_space}."))
        .saturating_sub(2)
}
