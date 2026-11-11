//! `word/document.xml`: paragraphs, runs, hyperlinks, comment anchors and
//! section properties.

use std::collections::{BTreeMap, HashMap};

use writer_core::{Alignment, Block, BlockKind, Document, InlineAttrs, Orientation};

use super::numbering::{Allocator, Instance, Slot};
use super::styles::{BULLET_NUM_ID, CODE_FILL, FIRST_DECIMAL_NUM_ID, RULE_COLOR};
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
    bidi: bool,
    /// Left indent and hanging indent in twips.
    indent: Option<(i32, i32)>,
    align: Option<&'static str>,
    /// Size of the paragraph mark in half-points, for blocks with their
    /// own text size.
    mark_size: Option<u32>,
    /// Starts on a new page.
    page_break: bool,
    /// Section properties, on the last paragraph of a section.
    section: Option<String>,
    content: String,
}

impl Para {
    fn xml(&self) -> String {
        // Children in schema order: pStyle, numPr, pBdr, bidi, ind, jc, rPr, sectPr.
        let mut p = String::new();
        if let Some(style) = self.style {
            p.push_str(&format!("<w:pStyle w:val=\"{style}\"/>"));
        }
        if let Some((num, level)) = self.numbering {
            p.push_str(&format!(
                "<w:numPr><w:ilvl w:val=\"{level}\"/><w:numId w:val=\"{num}\"/></w:numPr>"
            ));
        }
        if self.rule {
            p.push_str(&format!(
                "<w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:space=\"1\" w:color=\"{RULE_COLOR}\"/></w:pBdr>"
            ));
        }
        if self.bidi {
            p.push_str("<w:bidi/>");
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
        if let Some(size) = self.mark_size {
            p.push_str(&format!(
                "<w:rPr><w:sz w:val=\"{size}\"/><w:szCs w:val=\"{size}\"/></w:rPr>"
            ));
        }
        if let Some(section) = &self.section {
            p.push_str(section);
        }
        let mut out = String::from("<w:p>");
        if !p.is_empty() {
            out.push_str(&format!("<w:pPr>{p}</w:pPr>"));
        }
        if self.page_break {
            out.push_str("<w:r><w:br w:type=\"page\"/></w:r>");
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
    fn run_props(&self, attrs: &InlineAttrs, size: Option<u32>, code_block: bool) -> RunProps {
        if code_block {
            // The Code style carries the formatting.
            return RunProps {
                size,
                ..RunProps::default()
            };
        }
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
                .map(str::to_string),
            shade: attrs.highlight.map(|h| {
                h.reference_rgb()
                    .to_hex()
                    .trim_start_matches('#')
                    .to_uppercase()
            }),
            size,
            ..RunProps::default()
        };
        if attrs.code {
            p.font = Some(self.metrics.code_font.clone());
            p.no_proof = true;
            if p.shade.is_none() {
                p.shade = Some(CODE_FILL.to_string());
            }
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
                let mut props = self.run_props(attrs, size, code_block);
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
        let typography = &doc.settings().typography;
        let ordinals = ordinals(doc);
        let markers = self.comment_markers(doc);
        let none: Vec<Marker> = Vec::new();
        let mut paras: Vec<Para> = Vec::new();
        if part_needs_title(source, part, self.options.part_titles) {
            paras.push(Para {
                style: Some("Heading1"),
                bidi: starts_rtl(&part.title),
                content: text_runs(&part.title, &RunProps::default()),
                ..Para::default()
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
                        ..Para::default()
                    });
                }
                pending_break = true;
                continue;
            }
            let level = block.indent().min(8);
            let size = block
                .text_size()
                .map(|_| self.metrics.half_points(typography.size_of(block)));
            let block_markers = markers.get(&index).unwrap_or(&none);
            let bidi = starts_rtl(block.text());
            let mut para = Para {
                bidi,
                mark_size: size,
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
                ..Para::default()
            };
            let indented = (level > 0).then(|| (self.metrics.indent(level), 0));
            // A list marker written as text: a to-do's box, or the number
            // of a list item that has no numbering instance.
            let mut literal_marker: Option<String> = None;
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
                }
                BlockKind::CodeBlock { .. } => {
                    para.style = Some("Code");
                    para.indent = indented;
                    para.bidi = false;
                    para.align = None;
                }
                BlockKind::Divider => {
                    para.rule = true;
                    para.indent = indented;
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
                            let gutter = self.metrics.marker_gutter();
                            para.indent = Some((self.metrics.indent(level) + gutter, gutter));
                            literal_marker = Some(format!("{ordinal}."));
                        }
                    }
                }
                BlockKind::TodoItem { .. } => {
                    para.style = Some("ListParagraph");
                    let gutter = self.metrics.marker_gutter();
                    para.indent = Some((self.metrics.indent(level) + gutter, gutter));
                }
                BlockKind::PageBreak => {}
            }
            if let BlockKind::TodoItem { checked } = block.kind() {
                literal_marker = Some(if *checked { "\u{2611}" } else { "\u{2610}" }.to_string());
            }
            if let Some(marker) = literal_marker {
                let props = RunProps {
                    size,
                    ..RunProps::default()
                };
                para.content.push_str(&format!(
                    "<w:r>{}<w:t>{marker}</w:t><w:tab/></w:r>",
                    props.xml()
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
                ..Para::default()
            });
        }
        paras
    }
}

/// The longest page side Word accepts, in points (22 inches).
const MAX_PAGE: f32 = 1584.0;

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
    let footer = if page_numbers {
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
pub(crate) fn build(source: &ExportSource<'_>, options: &DocxOptions, metrics: &Metrics) -> Body {
    let mut builder = Builder {
        options,
        metrics,
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
        if paras.is_empty() {
            paras.push(Para::default());
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
        };
        assert_eq!(
            p.xml(),
            "<w:rPr><w:rStyle w:val=\"Hyperlink\"/><w:rFonts w:ascii=\"A &quot;B&quot;\" w:hAnsi=\"A &quot;B&quot;\" w:cs=\"A &quot;B&quot;\"/>\
             <w:b/><w:bCs/><w:i/><w:iCs/><w:strike/><w:noProof/><w:color w:val=\"FF0000\"/><w:sz w:val=\"34\"/><w:szCs w:val=\"34\"/>\
             <w:u w:val=\"single\"/><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"FFF176\"/><w:rtl/></w:rPr>"
        );
        assert_eq!(RunProps::default().xml(), "");
    }
}
