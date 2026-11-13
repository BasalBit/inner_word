//! `word/document.xml`: paragraphs, runs, hyperlinks, comment anchors and
//! section properties.

use std::collections::{BTreeMap, HashMap};

use writer_core::{Alignment, Block, BlockKind, Document, InlineAttrs, Orientation, Typography};

use super::lang::Lang;
use super::numbering::{Allocator, Instance, Slot};
use super::styles::{
    BULLET_NUM_ID, CHECKED_COLOR, CODE_FILL, FAINT_COLOR, FIRST_DECIMAL_NUM_ID, MUTED_COLOR,
    RULE_COLOR, code_spacing, heading_spacing,
};
use super::xml::{DECLARATION, attr, text, uri};
use super::{Comment, DocxOptions, FIRST_LINK_REL, FOOTER_REL, Metrics, R_NS, W_NS, page_twips};
use crate::common::{first_comment_ranges, ordinals, part_needs_title, thread_entries};
use crate::{ExportPart, ExportSource};

/// Everything the package needs from the body.
pub(crate) struct Body {
    /// `word/document.xml`.
    pub document: String,
    /// Hyperlink targets; the relationship id of `links[i]` is
    /// `rId{FIRST_LINK_REL + i}`.
    pub links: Vec<String>,
    /// The numbering instances of the numbered lists; `numbered[i]` has
    /// the id `FIRST_DECIMAL_NUM_ID + i`.
    pub numbered: Vec<Instance>,
    /// The comments, in id order.
    pub comments: Vec<Comment>,
}

/// One paragraph: its properties and its content (runs) as XML.
#[derive(Default)]
struct Para {
    style: Option<&'static str>,
    /// `(numId, ilvl)`.
    numbering: Option<(u32, u8)>,
    /// A bottom border: the divider.
    rule: bool,
    /// Tab stops: `(alignment, position in twips)`.
    tabs: Vec<(&'static str, i32)>,
    bidi: bool,
    /// Space before and after the paragraph in twips, where they differ
    /// from the style's.
    space_before: Option<i32>,
    space_after: Option<i32>,
    /// An exact line height in twips.
    line_exact: Option<i32>,
    /// A line height as a multiple of the font's own (`w:line` with the
    /// rule "auto"), where it differs from the style's.
    line_auto: Option<i32>,
    /// Left indent and hanging indent in twips.
    indent: Option<(i32, i32)>,
    align: Option<&'static str>,
    /// Size of the paragraph mark in half-points, for blocks with their
    /// own text size.
    mark_size: Option<u32>,
    /// Font of the paragraph mark, where it differs from the style's.
    mark_font: Option<String>,
    /// Starts on a new page.
    page_break: bool,
    /// Section properties, on the last paragraph of a section.
    section: Option<String>,
    content: String,
}

impl Para {
    fn xml(&self) -> String {
        // Children in schema order: pStyle, pageBreakBefore, numPr, pBdr,
        // tabs, bidi, spacing, ind, jc, rPr, sectPr.
        let mut p = String::new();
        if let Some(style) = self.style {
            p.push_str(&format!("<w:pStyle w:val=\"{style}\"/>"));
        }
        if self.page_break {
            p.push_str("<w:pageBreakBefore/>");
        }
        if let Some((num, level)) = self.numbering {
            p.push_str(&format!(
                "<w:numPr><w:ilvl w:val=\"{level}\"/><w:numId w:val=\"{num}\"/></w:numPr>"
            ));
        }
        if self.rule {
            p.push_str(&format!(
                "<w:pBdr><w:bottom w:val=\"single\" w:sz=\"8\" w:space=\"0\" w:color=\"{RULE_COLOR}\"/></w:pBdr>"
            ));
        }
        if !self.tabs.is_empty() {
            p.push_str("<w:tabs>");
            for (align, pos) in &self.tabs {
                p.push_str(&format!("<w:tab w:val=\"{align}\" w:pos=\"{pos}\"/>"));
            }
            p.push_str("</w:tabs>");
        }
        if self.bidi {
            p.push_str("<w:bidi/>");
        }
        if self.space_before.is_some()
            || self.space_after.is_some()
            || self.line_exact.is_some()
            || self.line_auto.is_some()
        {
            p.push_str("<w:spacing");
            if let Some(before) = self.space_before {
                p.push_str(&format!(" w:before=\"{before}\""));
            }
            if let Some(after) = self.space_after {
                p.push_str(&format!(" w:after=\"{after}\""));
            }
            if let Some(line) = self.line_exact {
                p.push_str(&format!(" w:line=\"{line}\" w:lineRule=\"exact\""));
            } else if let Some(line) = self.line_auto {
                p.push_str(&format!(" w:line=\"{line}\" w:lineRule=\"auto\""));
            }
            p.push_str("/>");
        }
        if let Some((left, hanging)) = self.indent {
            if hanging > 0 {
                p.push_str(&format!(
                    "<w:ind w:left=\"{left}\" w:hanging=\"{hanging}\"/>"
                ));
            } else {
                p.push_str(&format!("<w:ind w:left=\"{left}\"/>"));
            }
        }
        if let Some(align) = self.align {
            p.push_str(&format!("<w:jc w:val=\"{align}\"/>"));
        }
        if self.mark_size.is_some() || self.mark_font.is_some() {
            // The paragraph mark carries what the list marker is set in.
            let mark = RunProps {
                font: self.mark_font.clone(),
                size: self.mark_size,
                ..RunProps::default()
            };
            p.push_str(&mark.xml());
        }
        if let Some(section) = &self.section {
            p.push_str(section);
        }
        let mut out = String::from("<w:p>");
        if !p.is_empty() {
            out.push_str(&format!("<w:pPr>{p}</w:pPr>"));
        }
        out.push_str(&self.content);
        out.push_str("</w:p>");
        out
    }
}

/// Character formatting of one run.
#[derive(Clone, Default, PartialEq)]
struct RunProps {
    style: Option<&'static str>,
    font: Option<String>,
    bold: bool,
    italic: bool,
    strike: bool,
    no_proof: bool,
    color: Option<String>,
    /// Half-points.
    size: Option<u32>,
    underline: bool,
    /// Background fill, `RRGGBB`.
    shade: Option<String>,
    rtl: bool,
    /// The proofing language, where the styles do not carry it.
    lang: Option<Lang>,
}

impl RunProps {
    fn xml(&self) -> String {
        // Children in schema order.
        let mut p = String::new();
        if let Some(style) = self.style {
            p.push_str(&format!("<w:rStyle w:val=\"{style}\"/>"));
        }
        if let Some(font) = &self.font {
            let f = attr(font);
            p.push_str(&format!(
                "<w:rFonts w:ascii=\"{f}\" w:hAnsi=\"{f}\" w:cs=\"{f}\"/>"
            ));
        }
        if self.bold {
            p.push_str("<w:b/><w:bCs/>");
        }
        if self.italic {
            p.push_str("<w:i/><w:iCs/>");
        }
        if self.strike {
            p.push_str("<w:strike/>");
        }
        if self.no_proof {
            p.push_str("<w:noProof/>");
        }
        if let Some(color) = &self.color {
            p.push_str(&format!("<w:color w:val=\"{color}\"/>"));
        }
        if let Some(size) = self.size {
            p.push_str(&format!(
                "<w:sz w:val=\"{size}\"/><w:szCs w:val=\"{size}\"/>"
            ));
        }
        if self.underline {
            p.push_str("<w:u w:val=\"single\"/>");
        }
        if let Some(fill) = &self.shade {
            p.push_str(&format!(
                "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{fill}\"/>"
            ));
        }
        if self.rtl {
            p.push_str("<w:rtl/>");
        }
        if let Some(lang) = &self.lang {
            p.push_str(&lang.xml());
        }
        if p.is_empty() {
            String::new()
        } else {
            format!("<w:rPr>{p}</w:rPr>")
        }
    }
}

/// Whether the character belongs to a right-to-left script (Hebrew, Arabic,
/// Syriac, Thaana, N'Ko and the related presentation forms).
fn is_rtl(c: char) -> bool {
    matches!(u32::from(c), 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF)
}

/// Whether the first strong character of `s` is right-to-left.
fn starts_rtl(s: &str) -> bool {
    s.chars().find(|c| c.is_alphabetic()).is_some_and(is_rtl)
}

/// Splits `s` into runs of one direction. Characters without a direction
/// (spaces, digits, punctuation) stay with the run before them.
fn direction_runs(s: &str) -> Vec<(&str, bool)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut current: Option<bool> = None;
    for (i, c) in s.char_indices() {
        if !c.is_alphabetic() {
            continue;
        }
        let rtl = is_rtl(c);
        match current {
            Some(dir) if dir != rtl => {
                out.push((&s[start..i], dir));
                start = i;
                current = Some(rtl);
            }
            None => current = Some(rtl),
            _ => {}
        }
    }
    if start < s.len() {
        out.push((&s[start..], current.unwrap_or(false)));
    }
    out
}

/// Runs for `s` with the given formatting: line breaks become `<w:br/>`,
/// tabs `<w:tab/>`, and right-to-left stretches get their own runs.
fn text_runs(s: &str, props: &RunProps) -> String {
    let mut out = String::new();
    for (piece, rtl) in direction_runs(s) {
        let mut props = props.clone();
        props.rtl = rtl;
        out.push_str("<w:r>");
        out.push_str(&props.xml());
        let mut buf = String::new();
        let flush = |buf: &mut String, out: &mut String| {
            if !buf.is_empty() {
                out.push_str(&format!("<w:t xml:space=\"preserve\">{}</w:t>", text(buf)));
                buf.clear();
            }
        };
        for c in piece.chars() {
            match c {
                '\n' => {
                    flush(&mut buf, &mut out);
                    out.push_str("<w:br/>");
                }
                '\t' => {
                    flush(&mut buf, &mut out);
                    out.push_str("<w:tab/>");
                }
                c => buf.push(c),
            }
        }
        flush(&mut buf, &mut out);
        out.push_str("</w:r>");
    }
    out
}

/// A zero-width piece of XML at a byte offset of a block (a comment range
/// start or end).
struct Marker {
    offset: usize,
    /// Ends sort before starts at the same offset.
    end: bool,
    xml: String,
}

struct Builder<'a> {
    options: &'a DocxOptions,
    metrics: &'a Metrics,
    /// The typography the styles are written from: the first document's.
    typography: Typography,
    /// The typography of the document being written, when it is not the
    /// one the styles carry: its paragraphs and runs then say so
    /// themselves, so that every document of a collection keeps its own
    /// text size, font, line height and spacing.
    local: Option<Typography>,
    /// The proofing language the styles are written with: the one every
    /// document of the export names, if they all name the same.
    language: Option<Lang>,
    /// The proofing language of the document being written, when the
    /// styles do not carry it: its runs then say so themselves.
    local_language: Option<Lang>,
    links: Vec<String>,
    link_ids: HashMap<String, usize>,
    numbering: Allocator,
    comments: Vec<Comment>,
}

impl Builder<'_> {
    fn link_rel(&mut self, url: &str) -> String {
        let target = uri(url);
        let index = match self.link_ids.get(&target) {
            Some(i) => *i,
            None => {
                self.links.push(target.clone());
                self.link_ids.insert(target, self.links.len() - 1);
                self.links.len() - 1
            }
        };
        format!("rId{}", FIRST_LINK_REL + index)
    }

    /// Registers the comments of `doc` and returns the range markers per
    /// block.
    fn comment_markers(&mut self, doc: &Document) -> BTreeMap<usize, Vec<Marker>> {
        let mut markers: BTreeMap<usize, Vec<Marker>> = BTreeMap::new();
        if !self.options.comments {
            return markers;
        }
        for (id, range) in first_comment_ranges(doc) {
            let Some(thread) = doc.comment(id) else {
                continue;
            };
            let root = self.comments.len() as u32;
            let mut starts = String::new();
            let mut ends = String::new();
            for (i, (author, body)) in thread_entries(thread).into_iter().enumerate() {
                let cid = root + i as u32;
                let date = if i == 0 {
                    thread.created_at
                } else {
                    thread.replies[i - 1].created_at
                };
                self.comments.push(Comment {
                    id: cid,
                    author: if author.is_empty() {
                        "Author".to_string()
                    } else {
                        author.to_string()
                    },
                    date,
                    body: body.to_string(),
                    parent: (i > 0).then_some(root),
                    resolved: thread.resolved,
                });
                starts.push_str(&format!("<w:commentRangeStart w:id=\"{cid}\"/>"));
                ends.push_str(&format!(
                    "<w:commentRangeEnd w:id=\"{cid}\"/><w:r><w:rPr><w:rStyle w:val=\"CommentReference\"/></w:rPr>\
                     <w:commentReference w:id=\"{cid}\"/></w:r>"
                ));
            }
            markers.entry(range.start.block).or_default().push(Marker {
                offset: range.start.offset,
                end: false,
                xml: starts,
            });
            markers.entry(range.end.block).or_default().push(Marker {
                offset: range.end.offset,
                end: true,
                xml: ends,
            });
        }
        for list in markers.values_mut() {
            list.sort_by_key(|m| (m.offset, !m.end));
        }
        markers
    }

    /// The formatting of a run with `attrs` in a block whose text size
    /// override is `size`.
    fn run_props(&self, attrs: &InlineAttrs, size: Option<u32>, block: &Block) -> RunProps {
        if block.kind().is_code() {
            // The Code style carries the formatting.
            return RunProps {
                size,
                ..RunProps::default()
            };
        }
        let local_font = self
            .local
            .as_ref()
            .filter(|t| t.body_font != self.typography.body_font)
            .map(|t| t.body_font.clone());
        let mut p = RunProps {
            bold: attrs.bold,
            italic: attrs.italic,
            strike: attrs.strikethrough,
            underline: attrs.underline,
            color: attrs
                .color
                .map(|c| c.to_hex().trim_start_matches('#').to_uppercase()),
            font: attrs
                .font
                .as_deref()
                .map(str::trim)
                .filter(|f| !super::xml::attr(f).trim().is_empty())
                .map(str::to_string)
                .or(local_font),
            shade: attrs.highlight.map(|h| {
                h.reference_rgb()
                    .to_hex()
                    .trim_start_matches('#')
                    .to_uppercase()
            }),
            size,
            lang: self.local_language.clone(),
            ..RunProps::default()
        };
        if attrs.code {
            p.font = Some(self.metrics.code_font.clone());
            p.no_proof = true;
            p.lang = None;
            if p.shade.is_none() {
                p.shade = Some(CODE_FILL.to_string());
            }
        }
        if matches!(block.kind(), BlockKind::TodoItem { checked: true }) {
            // A finished to-do is struck through and faint, as in the
            // editor, whatever colour its text has.
            p.strike = true;
            p.color = Some(FAINT_COLOR.to_string());
        }
        p
    }

    /// The runs of a block, with hyperlinks and comment markers.
    fn runs(&mut self, block: &Block, markers: &[Marker], size: Option<u32>) -> String {
        let code_block = block.kind().is_code();
        let mut out = String::new();
        let mut open_link: Option<String> = None;
        let mut next_marker = 0;
        for (range, span_text, attrs) in block.text_spans() {
            // Cut the span where markers fall inside it.
            let mut cuts = vec![range.start];
            cuts.extend(
                markers
                    .iter()
                    .map(|m| m.offset)
                    .filter(|o| *o > range.start && *o < range.end),
            );
            cuts.push(range.end);
            cuts.dedup();
            let link = if code_block {
                None
            } else {
                attrs
                    .link
                    .as_deref()
                    .map(str::trim)
                    .filter(|u| !u.is_empty())
            };
            for w in cuts.windows(2) {
                let has_marker = markers.get(next_marker).is_some_and(|m| m.offset <= w[0]);
                // A hyperlink element ends where the target changes; markers
                // between two links go between the elements.
                if open_link.as_deref() != link {
                    if open_link.take().is_some() {
                        out.push_str("</w:hyperlink>");
                    }
                    while let Some(m) = markers.get(next_marker).filter(|m| m.offset <= w[0]) {
                        out.push_str(&m.xml);
                        next_marker += 1;
                    }
                    if let Some(url) = link {
                        out.push_str(&format!(
                            "<w:hyperlink r:id=\"{}\" w:history=\"1\">",
                            self.link_rel(url)
                        ));
                        open_link = Some(url.to_string());
                    }
                } else if has_marker {
                    while let Some(m) = markers.get(next_marker).filter(|m| m.offset <= w[0]) {
                        out.push_str(&m.xml);
                        next_marker += 1;
                    }
                }
                let mut props = self.run_props(attrs, size, block);
                if link.is_some() {
                    props.style = Some("Hyperlink");
                }
                let piece = &span_text[w[0] - range.start..w[1] - range.start];
                out.push_str(&text_runs(piece, &props));
            }
        }
        if open_link.is_some() {
            out.push_str("</w:hyperlink>");
        }
        for m in &markers[next_marker..] {
            out.push_str(&m.xml);
        }
        out
    }

    /// A paragraph with what a document with its own typography adds to
    /// the style of a block of `kind`: the size and font of the paragraph
    /// mark, the line height and the space around the paragraph. Nothing
    /// for a document that has the typography of the styles.
    fn para(&self, kind: &BlockKind, text_size: Option<u32>) -> Para {
        let mut para = Para {
            mark_size: text_size,
            ..Para::default()
        };
        let Some(t) = &self.local else {
            return para;
        };
        let m = self.metrics;
        para.mark_size = text_size.or(Some(m.half_points(t.size_for(kind))));
        if t.body_font != self.typography.body_font && !kind.is_code() {
            para.mark_font = Some(t.body_font.clone());
        }
        let body_line = m.line(&t.body_font, t.base_size, t.line_height);
        match kind {
            BlockKind::Heading { level } => {
                let (before, after) = heading_spacing(t, m, (*level).clamp(1, 6));
                para.space_before = Some(before);
                para.space_after = Some(after);
                para.line_auto = Some(m.heading_line(t, t.size_for(kind)));
            }
            BlockKind::CodeBlock { .. } => {
                let (line, after) = code_spacing(t, m);
                para.space_after = Some(after);
                para.line_auto = Some(line);
            }
            // A divider sets its own exact height.
            BlockKind::Divider => {}
            k if k.is_list_item() => {
                para.space_after = Some(m.list_gap(t));
                para.line_auto = Some(body_line);
            }
            _ => {
                para.space_after = Some(m.paragraph_gap(t));
                para.line_auto = Some(body_line);
            }
        }
        para
    }

    /// The text size of a run or a paragraph mark of `block` in
    /// half-points, where the style does not give it: the block's own
    /// size, or the size of its kind in a document with its own typography.
    fn size(&self, block: &Block) -> Option<u32> {
        match (block.text_size(), &self.local) {
            (Some(size), _) => Some(self.metrics.half_points(size)),
            (None, Some(t)) => Some(self.metrics.half_points(t.size_for(block.kind()))),
            (None, None) => None,
        }
    }

    /// The paragraphs of one document.
    /// `first` and `last` say where the part sits in the export: page
    /// breaks at the edge towards another part are dropped, because the
    /// section break between two parts already starts a new page and the
    /// two together would leave an empty one.
    fn document(
        &mut self,
        source: &ExportSource<'_>,
        part: &ExportPart<'_>,
        first: bool,
        last: bool,
    ) -> Vec<Para> {
        let doc = part.document;
        let is_break = |b: &Block| matches!(b.kind(), BlockKind::PageBreak);
        let blocks = doc.blocks();
        let skip_until = if first {
            0
        } else {
            blocks.iter().take_while(|b| is_break(b)).count()
        };
        let keep_until = if last {
            blocks.len()
        } else {
            blocks.len() - blocks.iter().rev().take_while(|b| is_break(b)).count()
        };
        let typography = doc.settings().typography.clone().sanitized();
        self.local = (typography != self.typography).then(|| typography.clone());
        self.local_language = Lang::of(doc).filter(|l| Some(l) != self.language.as_ref());
        let ordinals = ordinals(doc);
        let paragraph_gap = self.metrics.paragraph_gap(&typography);
        let quote_gap = self.metrics.quote_gap(&typography);
        let divider_half = self
            .metrics
            .twips((self.metrics.base_line(&typography) * 0.3).round());
        let heading = BlockKind::heading(1);
        let body = BlockKind::Paragraph;
        let markers = self.comment_markers(doc);
        let none: Vec<Marker> = Vec::new();
        let mut paras: Vec<Para> = Vec::new();
        if part_needs_title(source, part, self.options.part_titles) {
            let title = Block::with_text(heading.clone(), "");
            let props = self.run_props(&InlineAttrs::default(), self.size(&title), &title);
            paras.push(Para {
                style: Some("Heading1"),
                bidi: starts_rtl(&part.title),
                content: text_runs(&part.title, &props),
                ..self.para(&heading, None)
            });
        }
        let mut pending_break = false;

        for (index, block) in doc.blocks().iter().enumerate() {
            if index < skip_until || index >= keep_until {
                continue;
            }
            if matches!(block.kind(), BlockKind::PageBreak) {
                if pending_break {
                    // Two page breaks in a row: an empty page.
                    paras.push(Para {
                        page_break: true,
                        ..self.para(&body, None)
                    });
                }
                pending_break = true;
                continue;
            }
            let level = block.indent().min(8);
            let size = self.size(block);
            let block_markers = markers.get(&index).unwrap_or(&none);
            let bidi = starts_rtl(block.text());
            let mut para = Para {
                bidi,
                page_break: std::mem::take(&mut pending_break),
                // In a right-to-left paragraph Word reads "left" as the
                // leading edge, which is the right.
                align: match (block.alignment(), bidi) {
                    (Alignment::Left, _) => None,
                    (Alignment::Center, _) => Some("center"),
                    (Alignment::Justify, _) => Some("both"),
                    (Alignment::Right, false) => Some("right"),
                    (Alignment::Right, true) => Some("left"),
                },
                ..self.para(block.kind(), size)
            };
            let indented = (level > 0).then(|| (self.metrics.indent(level), 0));
            // What follows on the same page: blocks of one group (list
            // items, quotes) sit closer together than paragraphs.
            let next = blocks
                .get(index + 1)
                .filter(|_| index + 1 < keep_until)
                .map(Block::kind);
            let next_in_list = next.is_some_and(BlockKind::is_list_item);
            let gutter = self.metrics.marker_gutter();
            let item_left = self.metrics.indent(level) + gutter;
            // A list marker written as text: a to-do's box, or the number
            // of a list item that has no numbering instance.
            // `(text, colour, set flush right before the text)`.
            let mut literal_marker: Option<(String, &str, bool)> = None;
            match block.kind() {
                BlockKind::Paragraph => para.indent = indented,
                BlockKind::Heading { level: n } => {
                    para.style = Some(
                        [
                            "Heading1", "Heading2", "Heading3", "Heading4", "Heading5", "Heading6",
                        ][(*n).clamp(1, 6) as usize - 1],
                    );
                    para.indent = indented;
                }
                BlockKind::Quote => {
                    para.style = Some("Quote");
                    // The style already keeps the text clear of the bar.
                    para.indent = (level > 0)
                        .then(|| (self.metrics.indent(level) + self.metrics.quote_gutter(), 0));
                    if matches!(next, Some(BlockKind::Quote)) {
                        para.space_after = Some(quote_gap);
                    }
                }
                BlockKind::CodeBlock { .. } => {
                    para.style = Some("Code");
                    // The style keeps the panel inside the column.
                    let (pad, _) = self.metrics.code_padding();
                    para.indent =
                        (level > 0).then(|| (self.metrics.indent(level) + pad * 20 + 10, 0));
                    para.bidi = false;
                    para.align = None;
                }
                BlockKind::Divider => {
                    // A short row with the rule in its middle.
                    para.rule = true;
                    para.indent = indented;
                    para.line_exact = Some(divider_half.max(20));
                    para.space_after = Some(divider_half + paragraph_gap);
                }
                BlockKind::BulletItem => {
                    para.style = Some("ListParagraph");
                    para.numbering = Some((BULLET_NUM_ID, level));
                }
                BlockKind::NumberedItem => {
                    para.style = Some("ListParagraph");
                    let ordinal = ordinals[index].unwrap_or(1);
                    let ordinal = u32::try_from(ordinal).unwrap_or(u32::MAX);
                    match self.numbering.item(level as usize, ordinal) {
                        Slot::Instance(i) => {
                            para.numbering = Some((FIRST_DECIMAL_NUM_ID + i as u32, level));
                        }
                        Slot::Literal => {
                            // No numbering instance left (see `numbering`):
                            // the number is text in the marker gutter.
                            // It ends where a real number ends: a right
                            // tab stop there, then the text's left edge.
                            para.indent = Some((item_left, gutter));
                            para.tabs = vec![
                                ("right", item_left - self.metrics.number_gap()),
                                ("left", item_left),
                            ];
                            literal_marker = Some((format!("{ordinal}."), MUTED_COLOR, true));
                        }
                    }
                }
                BlockKind::TodoItem { .. } => {
                    para.style = Some("ListParagraph");
                    para.indent = Some((item_left, gutter));
                }
                // (An image is an empty paragraph here.)
                BlockKind::PageBreak | BlockKind::Image(_) => {}
            }
            if block.kind().is_list_item() && !next_in_list {
                // The last item of a list is followed by a paragraph's
                // space.
                para.space_after = Some(paragraph_gap);
            }
            if let BlockKind::TodoItem { checked } = block.kind() {
                literal_marker = Some(if *checked {
                    ("\u{2611}".to_string(), CHECKED_COLOR, false)
                } else {
                    ("\u{2610}".to_string(), MUTED_COLOR, false)
                });
            }
            if let Some((marker, color, flush_right)) = literal_marker {
                let props = RunProps {
                    color: Some(color.to_string()),
                    // A ballot box or a number is not a word.
                    lang: None,
                    ..self.run_props(&InlineAttrs::default(), size, &Block::paragraph(""))
                };
                para.content.push_str(&format!(
                    "<w:r>{}{}<w:t>{marker}</w:t><w:tab/></w:r>",
                    props.xml(),
                    if flush_right { "<w:tab/>" } else { "" },
                ));
            }
            para.content
                .push_str(&self.runs(block, block_markers, size));
            paras.push(para);
        }
        if pending_break {
            // A page break with nothing after it.
            paras.push(Para {
                page_break: true,
                ..self.para(&body, None)
            });
        }
        if paras.is_empty() {
            // A document without blocks is still a section.
            paras.push(self.para(&body, None));
        }
        paras
    }
}

/// The longest page side Word accepts, in points (22 inches).
const MAX_PAGE: f32 = 1584.0;

/// The smallest bottom margin that still shows a page number, in points.
const MIN_FOOTER_MARGIN: f32 = 23.0;

/// Section properties: page size, orientation, margins and the footer.
fn section_xml(doc: &Document, page_numbers: bool) -> String {
    let page = doc.settings().page.sanitized();
    let (w, h) = page.page_size();
    // Word's largest page is 22 inches each way; a larger one is cut to
    // that, and the margins with it so that an inch of text area is left.
    let (w, h) = (w.min(MAX_PAGE), h.min(MAX_PAGE));
    let mut m = page.margins;
    let fit = |a: &mut f32, b: &mut f32, room: f32| {
        let room = (room - 72.0).max(0.0);
        if *a + *b > room {
            let scale = room / (*a + *b);
            *a *= scale;
            *b *= scale;
        }
    };
    fit(&mut m.left, &mut m.right, w);
    fit(&mut m.top, &mut m.bottom, h);
    let orient = match page.orientation {
        Orientation::Landscape => " w:orient=\"landscape\"",
        Orientation::Portrait => "",
    };
    // As in the page view, a bottom margin with no room for the number
    // has none.
    let footer = if page_numbers && m.bottom >= MIN_FOOTER_MARGIN {
        format!("<w:footerReference w:type=\"default\" r:id=\"{FOOTER_REL}\"/>")
    } else {
        String::new()
    };
    // The header and footer sit in the middle of their margin.
    let edge = |margin: f32| page_twips(margin / 2.0).clamp(0, 720);
    format!(
        "<w:sectPr>{footer}<w:type w:val=\"nextPage\"/><w:pgSz w:w=\"{}\" w:h=\"{}\"{orient}/>\
         <w:pgMar w:top=\"{}\" w:right=\"{}\" w:bottom=\"{}\" w:left=\"{}\" w:header=\"{}\" w:footer=\"{}\" w:gutter=\"0\"/></w:sectPr>",
        page_twips(w),
        page_twips(h),
        page_twips(m.top),
        page_twips(m.right),
        page_twips(m.bottom),
        page_twips(m.left),
        edge(m.top),
        edge(m.bottom),
    )
}

/// Builds the document body of `source`.
pub(crate) fn build(
    source: &ExportSource<'_>,
    options: &DocxOptions,
    metrics: &Metrics,
    language: Option<&Lang>,
) -> Body {
    let mut builder = Builder {
        options,
        metrics,
        typography: source
            .parts()
            .first()
            .map(|p| p.document.settings().typography.clone().sanitized())
            .unwrap_or_default(),
        local: None,
        language: language.cloned(),
        local_language: None,
        links: Vec::new(),
        link_ids: HashMap::new(),
        numbering: Allocator::new(),
        comments: Vec::new(),
    };
    let mut xml = String::from(DECLARATION);
    xml.push_str(&format!(
        "<w:document xmlns:w=\"{W_NS}\" xmlns:r=\"{R_NS}\"><w:body>"
    ));
    let parts = source.parts();
    let mut last_section = None;
    for (i, part) in parts.iter().enumerate() {
        let mut paras = builder.document(source, part, i == 0, i + 1 == parts.len());
        let section = section_xml(part.document, options.page_numbers);
        // As in the page view, nothing stands above the first line of a
        // page: a heading or a code block that opens the document or
        // follows a page break gets no space before. (Readers drop that
        // space where a page ends by itself, but not at the start of a
        // document or a section, or after a page break.)
        for (i, para) in paras.iter_mut().enumerate() {
            if (i == 0 || para.page_break)
                && para
                    .style
                    .is_some_and(|s| s.starts_with("Heading") || s == "Code")
            {
                para.space_before = Some(0);
            }
        }
        if i + 1 < parts.len() {
            // Every document but the last ends its section in its last
            // paragraph; the next one starts on a new page.
            if let Some(last) = paras.last_mut() {
                last.section = Some(section);
            }
        } else {
            last_section = Some(section);
        }
        for para in &paras {
            xml.push_str(&para.xml());
        }
    }
    match last_section {
        Some(section) => xml.push_str(&section),
        None => {
            // An empty collection is still a valid, empty document.
            xml.push_str("<w:p/>");
            xml.push_str(&section_xml(&Document::new(), options.page_numbers));
        }
    }
    xml.push_str("</w:body></w:document>");
    Body {
        document: xml,
        links: builder.links,
        numbered: builder.numbering.into_instances(),
        comments: builder.comments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directions() {
        assert_eq!(direction_runs("hello world"), vec![("hello world", false)]);
        assert_eq!(direction_runs("שלום עולם"), vec![("שלום עולם", true)]);
        assert_eq!(
            direction_runs("say שלום, world!"),
            vec![("say ", false), ("שלום, ", true), ("world!", false)]
        );
        assert_eq!(direction_runs("123 !?"), vec![("123 !?", false)]);
        assert_eq!(direction_runs(""), Vec::<(&str, bool)>::new());
        assert!(starts_rtl("  123 مرحبا world"));
        assert!(!starts_rtl("hello שלום"));
        assert!(!starts_rtl("123"));
    }

    #[test]
    fn run_text() {
        let plain = RunProps::default();
        assert_eq!(
            text_runs("a<b", &plain),
            "<w:r><w:t xml:space=\"preserve\">a&lt;b</w:t></w:r>"
        );
        assert_eq!(
            text_runs("a\n\tb", &plain),
            "<w:r><w:t xml:space=\"preserve\">a</w:t><w:br/><w:tab/><w:t xml:space=\"preserve\">b</w:t></w:r>"
        );
        assert_eq!(text_runs("", &plain), "");
        let bold = RunProps {
            bold: true,
            ..RunProps::default()
        };
        assert_eq!(
            text_runs("אב", &bold),
            "<w:r><w:rPr><w:b/><w:bCs/><w:rtl/></w:rPr><w:t xml:space=\"preserve\">אב</w:t></w:r>"
        );
    }

    #[test]
    fn run_property_order() {
        let p = RunProps {
            style: Some("Hyperlink"),
            font: Some("A \"B\"".into()),
            bold: true,
            italic: true,
            strike: true,
            no_proof: true,
            color: Some("FF0000".into()),
            size: Some(34),
            underline: true,
            shade: Some("FFF176".into()),
            rtl: true,
            lang: Lang::from_identifier("en_GB"),
        };
        assert_eq!(
            p.xml(),
            "<w:rPr><w:rStyle w:val=\"Hyperlink\"/><w:rFonts w:ascii=\"A &quot;B&quot;\" w:hAnsi=\"A &quot;B&quot;\" w:cs=\"A &quot;B&quot;\"/>\
             <w:b/><w:bCs/><w:i/><w:iCs/><w:strike/><w:noProof/><w:color w:val=\"FF0000\"/><w:sz w:val=\"34\"/><w:szCs w:val=\"34\"/>\
             <w:u w:val=\"single\"/><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"FFF176\"/><w:rtl/><w:lang w:val=\"en-GB\"/></w:rPr>"
        );
        assert_eq!(RunProps::default().xml(), "");
    }
}
