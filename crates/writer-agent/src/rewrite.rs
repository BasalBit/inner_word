//! Changing the text of a document in place, through the editing engine of `writer-core`.
//!
//! Everything here works on an [`EditorState`] and changes as little as the new text
//! allows: a block that reads the same is not touched, a paragraph that was edited is
//! changed word by word, and only what is left over is removed and inserted. What is not
//! touched keeps everything it has: its id, the formatting Markdown cannot say (colours,
//! fonts, alignment, text size) and the comments anchored in it.

use std::collections::HashMap;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;
use writer_core::{
    Alignment, Block, BlockKind, DocumentSettings, EditorState, Fragment, InlineAttrs, InlineStyle,
    Position, Rgb, TextRange,
};

use crate::diff;
use crate::markdown_in;

/// The largest stretch of changed blocks (old × new) in which edited paragraphs are
/// looked for by comparing every old block with every new one.
const PAIRING_CELLS: usize = 10_000;

/// How much of two blocks has to be the same words for one to count as the other,
/// edited: the share of the longer one.
const SAME_PARAGRAPH: f64 = 0.5;

/// Or: this share of the shorter one's words are in the longer one, and they make up at
/// least this share of the longer one.
const CONTAINED: f64 = 0.75;
const CONTAINED_SHARE: f64 = 0.25;

const STYLES: [InlineStyle; 5] = [
    InlineStyle::Bold,
    InlineStyle::Italic,
    InlineStyle::Underline,
    InlineStyle::Strikethrough,
    InlineStyle::Code,
];

/// The character formatting Markdown says (with the exporter's `<u>` and `<mark>`). The
/// colour of a highlight is not part of it: Markdown only says that there is one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct Marks {
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    code: bool,
    highlight: bool,
    link: Option<String>,
}

impl Marks {
    fn of(attrs: &InlineAttrs) -> Marks {
        Marks {
            bold: attrs.bold,
            italic: attrs.italic,
            underline: attrs.underline,
            strikethrough: attrs.strikethrough,
            code: attrs.code,
            highlight: attrs.highlight.is_some(),
            link: attrs.link.clone(),
        }
    }
}

/// What a block reads as in Markdown: two blocks with the same key are the same block to
/// somebody who reads and writes Markdown.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct BlockKey {
    kind: BlockKind,
    indent: u8,
    text: String,
    runs: Vec<(usize, Marks)>,
}

impl BlockKey {
    fn of(block: &Block) -> BlockKey {
        let code = block.kind().is_code();
        let text = block.text();
        // Markdown does not say the white space at the ends of a line.
        let (start, end) = if code {
            (0, text.len())
        } else {
            let start = text.len() - text.trim_start().len();
            (start, text.trim_end().len().max(start))
        };
        let mut runs: Vec<(usize, Marks)> = Vec::new();
        if !code {
            for (range, attrs) in block.spans() {
                let length = range.end.min(end).saturating_sub(range.start.max(start));
                if length == 0 {
                    continue;
                }
                let marks = Marks::of(attrs);
                match runs.last_mut() {
                    Some((sum, last)) if *last == marks => *sum += length,
                    _ => runs.push((length, marks)),
                }
            }
        }
        // Plain text is plain text however its runs are cut.
        if runs.iter().all(|(_, marks)| *marks == Marks::default()) {
            runs.clear();
        }
        BlockKey {
            kind: block.kind().clone(),
            indent: block.indent(),
            text: text[start..end].to_string(),
            runs,
        }
    }
}

/// Whether a block is a paragraph without text: what a writer leaves between two
/// paragraphs for space, and what Markdown does not say.
fn is_blank(block: &Block) -> bool {
    *block.kind() == BlockKind::Paragraph && block.text().trim().is_empty()
}

/// What a block says beyond the Markdown an agent ordinarily writes: alignment, text
/// size, colours and fonts (the importer reads them from the exporter's HTML).
#[derive(PartialEq)]
struct Extras {
    alignment: Alignment,
    text_size: Option<f32>,
    runs: Vec<(usize, Option<Rgb>, Option<String>)>,
}

impl Extras {
    fn of(block: &Block) -> Extras {
        let mut runs: Vec<(usize, Option<Rgb>, Option<String>)> = Vec::new();
        for (range, attrs) in block.spans() {
            match runs.last_mut() {
                Some((length, color, font)) if *color == attrs.color && *font == attrs.font => {
                    *length += range.len();
                }
                _ => runs.push((range.len(), attrs.color, attrs.font.clone())),
            }
        }
        if runs
            .iter()
            .all(|(_, color, font)| color.is_none() && font.is_none())
        {
            runs.clear();
        }
        Extras {
            alignment: block.alignment(),
            text_size: block.text_size(),
            runs,
        }
    }

    fn is_plain(&self) -> bool {
        self.alignment == Alignment::Left && self.text_size.is_none() && self.runs.is_empty()
    }
}

/// Whether the new block `new` can stand for the old block `old` as it is: it says
/// nothing beyond Markdown, or says the same as the old one.
fn extras_agree(old: &Block, new: &Block) -> bool {
    let wanted = Extras::of(new);
    wanted.is_plain() || wanted == Extras::of(old)
}

/// Brings the blocks `range` of the document to `new`, changing as little as possible.
pub(crate) fn rewrite_blocks(ed: &mut EditorState, range: Range<usize>, new: &[Block]) {
    let count = ed.document().block_count();
    let range = range.start.min(count)..range.end.min(count);
    let old: Vec<Block> = ed.document().blocks()[range.clone()].to_vec();
    let old_keys: Vec<BlockKey> = old.iter().map(BlockKey::of).collect();
    let new_keys: Vec<BlockKey> = new.iter().map(BlockKey::of).collect();
    let mut kept = diff::common(&old_keys, &new_keys);
    kept.retain(|&(o, n)| extras_agree(&old[o], &new[n]));

    // The steps, in the order of the document.
    enum Step {
        /// The old block is edited into the new one.
        Edit(usize, usize),
        /// The old blocks are removed and the new ones inserted in their place.
        Replace(Range<usize>, Range<usize>),
    }
    let mut steps: Vec<Step> = Vec::new();
    let mut at = (0, 0);
    let stretch = |from: (usize, usize), to: (usize, usize), steps: &mut Vec<Step>| {
        let (o, n) = (from.0..to.0, from.1..to.1);
        if o.is_empty() && n.is_empty() {
            return;
        }
        let mut inner = (o.start, n.start);
        for (i, j) in edited_pairs(&old[o.clone()], &new[n.clone()]) {
            let (i, j) = (o.start + i, n.start + j);
            if inner.0 < i || inner.1 < j {
                steps.push(Step::Replace(inner.0..i, inner.1..j));
            }
            steps.push(Step::Edit(i, j));
            inner = (i + 1, j + 1);
        }
        if inner.0 < o.end || inner.1 < n.end {
            steps.push(Step::Replace(inner.0..o.end, inner.1..n.end));
        }
    };
    for &(o, n) in &kept {
        stretch(at, (o, n), &mut steps);
        at = (o + 1, n + 1);
    }
    stretch(at, (old.len(), new.len()), &mut steps);

    // Empty paragraphs are not in Markdown, so their absence from the new text does not
    // mean that they should go: those at the edges of a replaced stretch stay (unless
    // everything goes, or there was nothing but them).
    let keep_blanks = !new.is_empty() && !old.iter().all(is_blank);

    // From the end, so that the blocks before a step are where they were.
    for step in steps.into_iter().rev() {
        match step {
            Step::Edit(o, n) => edit_block(ed, range.start + o, &new[n]),
            Step::Replace(mut o, mut n) => {
                if keep_blanks {
                    let was = o.clone();
                    while o.start < o.end && is_blank(&old[o.start]) {
                        o.start += 1;
                    }
                    while o.start < o.end && is_blank(&old[o.end - 1]) {
                        o.end -= 1;
                    }
                    // Where one stays, the new text's own (a separator between two
                    // lists) is not put beside it.
                    while o.start > was.start && n.start < n.end && is_blank(&new[n.start]) {
                        n.start += 1;
                    }
                    while o.end < was.end && n.start < n.end && is_blank(&new[n.end - 1]) {
                        n.end -= 1;
                    }
                }
                if o.is_empty() && n.is_empty() {
                    continue;
                }
                replace_block_range(ed, range.start + o.start..range.start + o.end, &new[n]);
            }
        }
    }
}

/// Removes the blocks `range` and puts `blocks` in their place. Removing every block of
/// a document without putting any in leaves one empty paragraph.
pub(crate) fn replace_block_range(ed: &mut EditorState, range: Range<usize>, blocks: &[Block]) {
    if !blocks.is_empty() {
        ed.insert_blocks_at(range.end, &Fragment::new(blocks.to_vec()));
    }
    if !range.is_empty() {
        ed.select_blocks(range);
        ed.delete_blocks();
    }
}

/// Which old blocks of a stretch of changed blocks are which new ones, edited: pairs of
/// indices into `old` and `new`, in order.
fn edited_pairs(old: &[Block], new: &[Block]) -> Vec<(usize, usize)> {
    if old.is_empty() || new.is_empty() {
        return Vec::new();
    }
    let old_words: Vec<HashMap<String, usize>> = old.iter().map(bag).collect();
    let new_words: Vec<HashMap<String, usize>> = new.iter().map(bag).collect();
    let likeness = |i: usize, j: usize| -> f64 {
        let (a, b) = (&old[i], &new[j]);
        let same_kind = a.kind() == b.kind()
            || matches!(
                (a.kind(), b.kind()),
                (BlockKind::TodoItem { .. }, BlockKind::TodoItem { .. })
                    | (BlockKind::CodeBlock { .. }, BlockKind::CodeBlock { .. })
            );
        if !same_kind || a.is_void() || a.indent() != b.indent() || !extras_agree(a, b) {
            return 0.0;
        }
        let total = |words: &HashMap<String, usize>| words.values().sum::<usize>();
        let (longer, shorter) = {
            let (a, b) = (total(&old_words[i]), total(&new_words[j]));
            (a.max(b), a.min(b))
        };
        if shorter == 0 {
            return 0.0;
        }
        let shared: usize = old_words[i]
            .iter()
            .map(|(word, count)| (*count).min(new_words[j].get(word).copied().unwrap_or(0)))
            .sum();
        let share = shared as f64 / longer as f64;
        let contained = shared as f64 / shorter as f64;
        // The same words for the most part, or one of them nearly all inside the other
        // (a line that grew, a heading that got a word more).
        if share >= SAME_PARAGRAPH || (contained >= CONTAINED && share >= CONTAINED_SHARE) {
            share
        } else {
            0.0
        }
    };
    if old.len().saturating_mul(new.len()) > PAIRING_CELLS {
        // Too many to compare each with each: block for block when the numbers agree.
        if old.len() != new.len() {
            return Vec::new();
        }
        return (0..old.len())
            .filter(|&i| likeness(i, i) > 0.0)
            .map(|i| (i, i))
            .collect();
    }
    // The pairing in order with the greatest likeness in total.
    let (rows, columns) = (old.len(), new.len());
    let cell = |i: usize, j: usize| i * (columns + 1) + j;
    let mut best = vec![0.0f64; (rows + 1) * (columns + 1)];
    let mut like = vec![0.0f64; rows * columns];
    for i in (0..rows).rev() {
        for j in (0..columns).rev() {
            let value = likeness(i, j);
            like[i * columns + j] = value;
            let mut score = best[cell(i + 1, j)].max(best[cell(i, j + 1)]);
            if value > 0.0 {
                score = score.max(value + best[cell(i + 1, j + 1)]);
            }
            best[cell(i, j)] = score;
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < rows && j < columns {
        let value = like[i * columns + j];
        if value > 0.0 && best[cell(i, j)] == value + best[cell(i + 1, j + 1)] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if best[cell(i + 1, j)] >= best[cell(i, j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// The words of a block, counted, without regard to case.
fn bag(block: &Block) -> HashMap<String, usize> {
    let mut words = HashMap::new();
    for word in block.text().unicode_words() {
        *words.entry(word.to_lowercase()).or_insert(0) += 1;
    }
    words
}

/// A word, a space or a punctuation mark of a block, with its formatting.
struct Piece {
    range: Range<usize>,
    marks: Marks,
}

/// A block cut into pieces at word boundaries and wherever the formatting changes.
fn pieces(block: &Block) -> Vec<Piece> {
    let code = block.kind().is_code();
    let mut pieces = Vec::new();
    for (range, text, attrs) in block.text_spans() {
        let marks = if code {
            Marks::default()
        } else {
            Marks::of(attrs)
        };
        for (offset, word) in text.split_word_bound_indices() {
            let start = range.start + offset;
            pieces.push(Piece {
                range: start..start + word.len(),
                marks: marks.clone(),
            });
        }
    }
    pieces
}

/// The part `range` of `block` as a paragraph of its own, formatting included.
fn slice(block: &Block, range: Range<usize>) -> Block {
    let mut piece = Block::new(BlockKind::Paragraph);
    for (span, text, attrs) in block.text_spans() {
        let start = span.start.max(range.start);
        let end = span.end.min(range.end);
        if start < end {
            piece = piece.with_span(&text[start - span.start..end - span.start], attrs.clone());
        }
    }
    piece
}

/// Edits the block at `index` into `new`: the words that are the same stay as they are
/// (with their comments and everything else they carry), the others are replaced.
fn edit_block(ed: &mut EditorState, index: usize, new: &Block) {
    let old = ed.document().blocks()[index].clone();
    let old_pieces = pieces(&old);
    let new_pieces = pieces(new);
    let old_keys: Vec<(&str, &Marks)> = old_pieces
        .iter()
        .map(|piece| (&old.text()[piece.range.clone()], &piece.marks))
        .collect();
    let new_keys: Vec<(&str, &Marks)> = new_pieces
        .iter()
        .map(|piece| (&new.text()[piece.range.clone()], &piece.marks))
        .collect();
    let mut same = diff::common(&old_keys, &new_keys);
    same.push((old_pieces.len(), new_pieces.len()));
    // The stretches between what is the same.
    let mut gaps: Vec<(Range<usize>, Range<usize>)> = Vec::new();
    let mut at = (0, 0);
    for (o, n) in same {
        if at.0 < o || at.1 < n {
            gaps.push((at.0..o, at.1..n));
        }
        at = (o + 1, n + 1);
    }
    let bytes = |pieces: &[Piece], gap: &Range<usize>, length: usize| -> Range<usize> {
        let start = pieces
            .get(gap.start)
            .map_or(length, |piece| piece.range.start);
        let end = if gap.is_empty() {
            start
        } else {
            pieces[gap.end - 1].range.end
        };
        start..end
    };
    for (old_gap, new_gap) in gaps.into_iter().rev() {
        let from = bytes(&old_pieces, &old_gap, old.len());
        let to = bytes(&new_pieces, &new_gap, new.len());
        let target = TextRange::in_block(index, from.start, from.end);
        if to.is_empty() {
            ed.delete_range(target);
            continue;
        }
        // Words in one formatting replaced by words in the same formatting keep what the
        // old words carried beyond it: a comment, a colour.
        let marks = &old_pieces
            .get(old_gap.start)
            .map(|piece| piece.marks.clone());
        let uniform = !old_gap.is_empty()
            && old_pieces[old_gap.clone()]
                .iter()
                .all(|piece| Some(&piece.marks) == marks.as_ref())
            && new_pieces[new_gap.clone()]
                .iter()
                .all(|piece| Some(&piece.marks) == marks.as_ref());
        if uniform {
            ed.replace_range(target, &new.text()[to]);
        } else if from.is_empty() {
            // New words between old ones: they come with the formatting they have.
            ed.set_caret(target.start);
            ed.paste(&Fragment::new(vec![slice(new, to)]));
        } else {
            // Other words, or the same words in another formatting: the text first (it
            // takes what the replaced text carried, a comment among it), then exactly
            // the formatting the new text has.
            let piece = slice(new, to.clone());
            ed.replace_range(target, &new.text()[to]);
            apply_marks(ed, index, from.start, &piece, true);
        }
    }
    match (old.kind(), new.kind()) {
        (BlockKind::TodoItem { checked: was }, BlockKind::TodoItem { checked })
            if was != checked =>
        {
            ed.set_todo_checked(index, *checked);
        }
        (BlockKind::CodeBlock { language: was }, BlockKind::CodeBlock { language })
            if was != language =>
        {
            ed.set_caret(Position::new(index, 0));
            ed.set_code_language(language.as_deref());
        }
        _ => {}
    }
}

/// Replaces the text in `range` with `new_text`, the way [`EditRequest`](crate::EditRequest)
/// describes. `exact` says that the formatting of the replacement is all in `new_text`
/// (the old text was given as Markdown); otherwise words without markup take the
/// formatting of what they replace. Returns what in `new_text` could not be kept as
/// written.
pub(crate) fn replace_text(
    ed: &mut EditorState,
    range: TextRange,
    new_text: &str,
    literal: bool,
    exact: bool,
    settings: &DocumentSettings,
) -> Vec<String> {
    let range = TextRange::new(
        ed.document().clamp_position(range.start),
        ed.document().clamp_position(range.end),
    );
    let last = &ed.document().blocks()[range.end.block];
    let whole_blocks = range.start.offset == 0 && range.end.offset == last.len();
    let blocks = range.start.block..range.end.block + 1;
    let normalized = writer_core::text::normalize_input(new_text).into_owned();

    if normalized.is_empty() {
        if whole_blocks {
            // The text of whole blocks is deleted: the blocks go with it.
            ed.select_blocks(blocks);
            ed.delete_blocks();
        } else {
            ed.delete_range(range);
        }
        return Vec::new();
    }
    let single = range.start.block == range.end.block;
    // A line break at an end of the new text ends the block there: the text before the
    // match stays a block of its own, or the text after it does.
    let break_before = normalized.starts_with('\n') && !whole_blocks;
    let break_after = normalized.ends_with('\n') && !whole_blocks;
    let paste = |ed: &mut EditorState, blocks: &[Block]| {
        let mut all: Vec<Block> = Vec::with_capacity(blocks.len() + 2);
        if break_before {
            all.push(Block::new(BlockKind::Paragraph));
        }
        all.extend(blocks.iter().cloned());
        if break_after {
            all.push(Block::new(BlockKind::Paragraph));
        }
        ed.select_range(range);
        ed.paste(&Fragment::new(all));
    };
    let inline = |ed: &mut EditorState, piece: &Block, lead: &str, trail: &str, exact: bool| {
        if break_before || break_after {
            paste(ed, std::slice::from_ref(piece));
        } else if single {
            splice_block(ed, range, piece, lead, trail, exact);
        } else {
            replace_across(ed, range, piece, lead, trail, exact);
        }
    };

    if literal {
        let lines = normalized.trim_matches('\n');
        if lines.contains('\n') || break_before || break_after {
            let blocks: Vec<Block> = lines.split('\n').map(Block::paragraph).collect();
            paste(ed, &blocks);
        } else {
            inline(ed, &Block::paragraph(lines), "", "", false);
        }
        return Vec::new();
    }
    if !exact && is_plain_text(&normalized) {
        inline(ed, &Block::paragraph(&normalized), "", "", false);
        return Vec::new();
    }

    let space = |c: char| c == ' ' || c == '\t';
    let trimmed = normalized.trim_matches(|c: char| space(c) || c == '\n');
    let lead = &normalized[..normalized.len() - normalized.trim_start_matches(space).len()];
    let trail = &normalized[normalized.trim_end_matches(space).len()..];
    let imported = markdown_in::import(trimmed, settings);
    let warnings = imported.warnings.clone();
    let read = imported.blocks();
    if read.is_empty() {
        // White space only, or markup that leaves nothing (an HTML comment).
        let spaces: String = normalized.chars().filter(|&c| space(c)).collect();
        if trimmed.is_empty() && !spaces.is_empty() {
            inline(ed, &Block::paragraph(&spaces), "", "", false);
        } else if whole_blocks {
            ed.select_blocks(blocks);
            ed.delete_blocks();
        } else {
            ed.delete_range(range);
        }
        return warnings;
    }
    let one_paragraph = read.len() == 1 && *read[0].kind() == BlockKind::Paragraph;
    if one_paragraph {
        inline(ed, &read[0], lead, trail, exact);
        return warnings;
    }
    if whole_blocks {
        // The match is whole blocks: the new text says what kinds of block they become.
        rewrite_blocks(ed, blocks, read);
        return warnings;
    }
    if read.len() == 1 && !read[0].is_void() {
        // In the middle of a block a leading "#", "-", ">" or "1." is the characters it
        // is: read the text again behind a word, and take the word away.
        const GUARD: &str = "x ";
        let guarded = markdown_in::import(&format!("{GUARD}{trimmed}"), settings);
        let again = guarded.blocks();
        if again.len() == 1
            && *again[0].kind() == BlockKind::Paragraph
            && again[0].text().starts_with(GUARD)
        {
            let piece = slice(&again[0], GUARD.len()..again[0].len());
            inline(ed, &piece, lead, trail, exact);
            return warnings;
        }
    }
    // Several blocks into the middle of one: the block is split around them.
    paste(ed, read);
    warnings
}

/// Whether `text` is words and nothing a Markdown reader would take for markup.
fn is_plain_text(text: &str) -> bool {
    if text.contains([
        '\n', '*', '_', '`', '[', ']', '<', '>', '#', '~', '\\', '!', '=', '|', '&',
    ]) || text.contains("://")
        || text.contains("www.")
    {
        return false;
    }
    let start = text.trim_start();
    if start.starts_with(['-', '+']) {
        return false;
    }
    let digits = start.bytes().take_while(u8::is_ascii_digit).count();
    !(digits > 0 && start[digits..].starts_with(['.', ')']))
}

/// The formatting a piece of new text gets where it replaces text formatted as `base`:
/// its own markup on top of the formatting of what it replaces, or with `exact` its own
/// markup only. A colour and a font, which Markdown does not say, are kept either way.
fn merged(attrs: &InlineAttrs, base: &InlineAttrs, exact: bool) -> InlineAttrs {
    let mut out = attrs.clone();
    out.comments.clear();
    if !exact {
        for style in STYLES {
            if base.has_style(style) {
                out.set_style(style, true);
            }
        }
        if out.link.is_none() {
            out.link = base.link.clone();
        }
        if out.highlight.is_none() {
            out.highlight = base.highlight;
        }
    }
    if out.color.is_none() {
        out.color = base.color;
    }
    if out.font.is_none() {
        out.font = base.font.clone();
    }
    out
}

/// Replaces `range`, which lies in one block, with the text of the paragraph `piece`
/// (between `lead` and `trail` spaces): the block is edited word by word into what it
/// reads with the new text in place, so words that the old and the new text share are
/// not touched, and a comment on a replaced word stays on its replacement.
fn splice_block(
    ed: &mut EditorState,
    range: TextRange,
    piece: &Block,
    lead: &str,
    trail: &str,
    exact: bool,
) {
    let index = range.start.block;
    let old = ed.document().blocks()[index].clone();
    let (from, to) = (range.start.offset, range.end.offset);
    let base = old.attrs_at(from).cloned().unwrap_or_default();
    let mut new = Block::new(old.kind().clone());
    for (span, text, attrs) in old.text_spans() {
        if span.start < from {
            let end = span.end.min(from);
            new = new.with_span(&text[..end - span.start], attrs.clone());
        }
    }
    let around = merged(&InlineAttrs::default(), &base, exact);
    if !lead.is_empty() {
        new = new.with_span(lead, around.clone());
    }
    for (_, text, attrs) in piece.text_spans() {
        new = new.with_span(text, merged(attrs, &base, exact));
    }
    if !trail.is_empty() {
        new = new.with_span(trail, around);
    }
    for (span, text, attrs) in old.text_spans() {
        if span.end > to {
            let start = span.start.max(to);
            new = new.with_span(&text[start - span.start..], attrs.clone());
        }
    }
    edit_block(ed, index, &new);
}

/// Replaces `range`, which runs across blocks, with the text of the paragraph `piece`
/// (between `lead` and `trail` spaces) and gives it the formatting `piece` has: on top of
/// what the replaced text had, or with `exact` in place of it. The blocks are joined
/// into the first, and a comment anchored at the start of the replaced text stays on the
/// replacement.
fn replace_across(
    ed: &mut EditorState,
    range: TextRange,
    piece: &Block,
    lead: &str,
    trail: &str,
    exact: bool,
) {
    let text = format!("{lead}{}{trail}", piece.text());
    let start = range.start;
    if text.is_empty() {
        ed.delete_range(range);
        return;
    }
    ed.replace_range(range, &text);
    let base = start.offset + lead.len();
    apply_marks(ed, start.block, base, piece, exact);
    let end = Position::new(start.block, base + piece.len() + trail.len());
    ed.set_caret(ed.document().clamp_position(end));
}

/// Gives the text at `base` in block `block` the formatting of `piece`, whose text it
/// is: the styles, link and highlight `piece` has are set; with `exact` those it has not
/// are removed. A colour and a font are only ever set.
fn apply_marks(ed: &mut EditorState, block: usize, base: usize, piece: &Block, exact: bool) {
    if ed.document().blocks()[block].kind().is_code() {
        return;
    }
    for (span, attrs) in piece.spans() {
        if !exact && attrs.is_plain() {
            continue;
        }
        ed.select_range(TextRange::in_block(
            block,
            base + span.start,
            base + span.end,
        ));
        for style in STYLES {
            let wanted = attrs.has_style(style);
            // A toggle sets the style unless all of the selection has it.
            if wanted && !ed.format_state().style(style).is_on() {
                ed.toggle_style(style);
            } else if !wanted && exact {
                if ed.format_state().style(style) == writer_core::Toggle::Mixed {
                    ed.toggle_style(style);
                }
                if ed.format_state().style(style).is_on() {
                    ed.toggle_style(style);
                }
            }
        }
        match &attrs.link {
            Some(url) => {
                ed.set_link(url);
            }
            None if exact => {
                ed.remove_link();
            }
            None => {}
        }
        let highlighted = ed.selection_is_highlighted();
        match attrs.highlight {
            Some(highlight) if !highlighted => {
                ed.set_highlight(Some(highlight));
            }
            None if exact => {
                ed.set_highlight(None);
            }
            _ => {}
        }
        if attrs.color.is_some() {
            ed.set_text_color(attrs.color);
        }
        if attrs.font.is_some() {
            ed.set_font_family(attrs.font.as_deref());
        }
    }
}

/// What this module asks of the selection beyond the engine's own queries.
trait SelectionQueries {
    /// Whether all of the selected text is highlighted, in whatever colour.
    fn selection_is_highlighted(&self) -> bool;
}

impl SelectionQueries for EditorState {
    fn selection_is_highlighted(&self) -> bool {
        let range = self.selection().range();
        if range.start.block != range.end.block {
            return false;
        }
        let block = &self.document().blocks()[range.start.block];
        block
            .spans()
            .filter(|(span, _)| span.start < range.end.offset && span.end > range.start.offset)
            .all(|(_, attrs)| attrs.highlight.is_some())
    }
}
