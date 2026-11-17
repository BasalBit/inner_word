//! `word/styles.xml` and `word/numbering.xml`.

use innerword_core::{BlockKind, Typography};

use super::lang::Lang;
use super::numbering::Instance;
use super::xml::{DECLARATION, attr};
use super::{Metrics, W_NS};

// The colours of the app's light theme ("Paper"), so that the export looks
// like the page view. They are repeated here because this crate cannot
// depend on the interface crates.

/// Fill colour of code blocks and inline code.
pub(crate) const CODE_FILL: &str = "EEEAE3";
/// Colour of hyperlinks: the accent.
pub(crate) const LINK_COLOR: &str = "B0503A";
/// Colour of the quote bar and of dividers.
pub(crate) const RULE_COLOR: &str = "CDC5B8";
/// Colour of quote text, list markers and open to-do boxes.
pub(crate) const MUTED_COLOR: &str = "6B645B";
/// Colour of finished to-dos and of the page number.
pub(crate) const FAINT_COLOR: &str = "A0988D";
/// Colour of the box of a finished to-do: the accent.
pub(crate) const CHECKED_COLOR: &str = LINK_COLOR;
/// Size of the page number in points (the interface's smallest text).
const PAGE_NUMBER_SIZE: f32 = 11.0;

/// The bullet numbering instance. Numbered lists use the ids from
/// [`FIRST_DECIMAL_NUM_ID`].
pub(crate) const BULLET_NUM_ID: u32 = 1;
pub(crate) const FIRST_DECIMAL_NUM_ID: u32 = 2;
const BULLET_ABSTRACT: u32 = 0;
const FIRST_DECIMAL_ABSTRACT: u32 = 1;

/// Space before and after a heading of `level` in twips: more room above
/// than below, measured in body lines, as in the editor.
pub(crate) fn heading_spacing(typography: &Typography, m: &Metrics, level: u8) -> (i32, i32) {
    let base_line = m.base_line(typography);
    let (before, after) = match level {
        0 | 1 => (0.9, 0.5),
        2 => (0.9, 0.35),
        3 => (0.75, 0.3),
        _ => (0.6, 0.3),
    };
    (
        m.twips((base_line * before).round()),
        m.twips((base_line * after).round()),
    )
}

/// `w:line` and the space after a code block in twips.
pub(crate) fn code_spacing(typography: &Typography, m: &Metrics) -> (i32, i32) {
    let points = typography.size_for(&BlockKind::code());
    (
        m.line(&m.code_font, points, typography.line_height),
        m.paragraph_gap(typography) + m.twips(4.0),
    )
}

fn fonts(name: &str) -> String {
    let n = attr(name);
    format!("<w:rFonts w:ascii=\"{n}\" w:hAnsi=\"{n}\" w:cs=\"{n}\"/>")
}

fn size(half_points: u32) -> String {
    format!("<w:sz w:val=\"{half_points}\"/><w:szCs w:val=\"{half_points}\"/>")
}

/// `word/styles.xml`: Normal from the document typography, Heading 1 to 6,
/// Quote, Code, List Paragraph, Hyperlink, and the small styles comments
/// and the footer need.
pub(crate) fn styles_xml(typography: &Typography, m: &Metrics, language: Option<&Lang>) -> String {
    // The proofing language of the whole export, when it has one.
    let lang = language.map(Lang::xml).unwrap_or_default();
    let body_font = fonts(&typography.body_font);
    let body_size = size(m.half_points(typography.base_size));
    let after = m.paragraph_gap(typography);
    let line = m.line(
        &typography.body_font,
        typography.base_size,
        typography.line_height,
    );
    let mut s = String::with_capacity(8192);
    s.push_str(DECLARATION);
    s.push_str(&format!("<w:styles xmlns:w=\"{W_NS}\">"));
    s.push_str(&format!(
        "<w:docDefaults><w:rPrDefault><w:rPr>{body_font}{body_size}{lang}</w:rPr></w:rPrDefault>\
         <w:pPrDefault><w:pPr><w:spacing w:after=\"{after}\" w:line=\"{line}\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>"
    ));
    s.push_str(&format!(
        "<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/>\
         <w:pPr><w:spacing w:after=\"{after}\" w:line=\"{line}\" w:lineRule=\"auto\"/></w:pPr>\
         <w:rPr>{body_font}{body_size}</w:rPr></w:style>"
    ));
    s.push_str(
        "<w:style w:type=\"character\" w:default=\"1\" w:styleId=\"DefaultParagraphFont\">\
         <w:name w:val=\"Default Paragraph Font\"/><w:uiPriority w:val=\"1\"/><w:semiHidden/><w:unhideWhenUsed/></w:style>\
         <w:style w:type=\"table\" w:default=\"1\" w:styleId=\"TableNormal\"><w:name w:val=\"Normal Table\"/>\
         <w:uiPriority w:val=\"99\"/><w:semiHidden/><w:unhideWhenUsed/><w:tblPr><w:tblInd w:w=\"0\" w:type=\"dxa\"/>\
         <w:tblCellMar><w:top w:w=\"0\" w:type=\"dxa\"/><w:left w:w=\"108\" w:type=\"dxa\"/><w:bottom w:w=\"0\" w:type=\"dxa\"/>\
         <w:right w:w=\"108\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr></w:style>\
         <w:style w:type=\"numbering\" w:default=\"1\" w:styleId=\"NoList\"><w:name w:val=\"No List\"/>\
         <w:uiPriority w:val=\"99\"/><w:semiHidden/><w:unhideWhenUsed/></w:style>",
    );
    // Headings as in the editor: sizes from the typography, a tighter
    // line, and more room above than below, measured in body lines.
    for level in 1..=6u8 {
        let pt = typography.size_for(&BlockKind::heading(level));
        let (before, after) = heading_spacing(typography, m, level);
        let heading_line = m.heading_line(typography, pt);
        s.push_str(&format!(
            "<w:style w:type=\"paragraph\" w:styleId=\"Heading{level}\"><w:name w:val=\"heading {level}\"/>\
             <w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:uiPriority w:val=\"9\"/><w:qFormat/>\
             <w:pPr><w:keepNext/><w:keepLines/>\
             <w:spacing w:before=\"{before}\" w:after=\"{after}\" w:line=\"{heading_line}\" w:lineRule=\"auto\"/>\
             <w:outlineLvl w:val=\"{}\"/></w:pPr><w:rPr><w:b/><w:bCs/>{}</w:rPr></w:style>",
            level - 1,
            size(m.half_points(pt)),
        ));
    }
    // The quote bar is 3 pt wide and starts 1 pt into the block; the text
    // starts at the quote gutter. Quote text is muted.
    let step = m.quote_gutter();
    let bar_space = ((step as f32 / 20.0) - 4.0).round().clamp(0.0, 31.0) as i32;
    s.push_str(&format!(
        "<w:style w:type=\"paragraph\" w:styleId=\"Quote\"><w:name w:val=\"Quote\"/><w:basedOn w:val=\"Normal\"/>\
         <w:uiPriority w:val=\"29\"/><w:qFormat/><w:pPr><w:pBdr><w:left w:val=\"single\" w:sz=\"24\" w:space=\"{bar_space}\" w:color=\"{RULE_COLOR}\"/></w:pBdr>\
         <w:ind w:left=\"{step}\"/></w:pPr><w:rPr><w:color w:val=\"{MUTED_COLOR}\"/></w:rPr></w:style>"
    ));
    // A code block is a panel: the fill reaches the borders, which have
    // the fill's colour and stand off the text by the panel's padding.
    // The indents keep the panel inside the text column.
    let code_font = fonts(&m.code_font);
    let code_points = typography.size_for(&BlockKind::code());
    let code_size = size(m.half_points(code_points));
    let (code_line, code_after) = code_spacing(typography, m);
    let (pad, pad_v) = m.code_padding();
    let code_indent = pad * 20 + 10;
    let code_before = m.twips(4.0);
    let side = |name: &str, space: i32| {
        format!(
            "<w:{name} w:val=\"single\" w:sz=\"4\" w:space=\"{space}\" w:color=\"{CODE_FILL}\"/>"
        )
    };
    s.push_str(&format!(
        "<w:style w:type=\"paragraph\" w:styleId=\"Code\"><w:name w:val=\"Code\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/>\
         <w:pPr><w:pBdr>{}{}{}{}</w:pBdr><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{CODE_FILL}\"/><w:suppressAutoHyphens/>\
         <w:spacing w:before=\"{code_before}\" w:after=\"{code_after}\" w:line=\"{code_line}\" w:lineRule=\"auto\"/>\
         <w:ind w:left=\"{code_indent}\" w:right=\"{code_indent}\"/><w:jc w:val=\"left\"/></w:pPr>\
         <w:rPr>{code_font}<w:noProof/>{code_size}</w:rPr></w:style>",
        side("top", pad_v),
        side("left", pad),
        side("bottom", pad_v),
        side("right", pad),
    ));
    // List items sit closer together than paragraphs, as in the editor;
    // the last item of a list gets the full paragraph spacing in the body.
    let list_after = m.list_gap(typography);
    s.push_str(&format!(
        "<w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/><w:basedOn w:val=\"Normal\"/>\
         <w:uiPriority w:val=\"34\"/><w:qFormat/><w:pPr><w:spacing w:after=\"{list_after}\"/></w:pPr></w:style>"
    ));
    s.push_str(&format!(
        "<w:style w:type=\"character\" w:styleId=\"Hyperlink\"><w:name w:val=\"Hyperlink\"/><w:basedOn w:val=\"DefaultParagraphFont\"/>\
         <w:uiPriority w:val=\"99\"/><w:unhideWhenUsed/><w:rPr><w:color w:val=\"{LINK_COLOR}\"/><w:u w:val=\"single\"/></w:rPr></w:style>"
    ));
    let small = size(m.half_points(PAGE_NUMBER_SIZE));
    s.push_str(&format!(
        "<w:style w:type=\"paragraph\" w:styleId=\"Footer\"><w:name w:val=\"footer\"/><w:basedOn w:val=\"Normal\"/>\
         <w:uiPriority w:val=\"99\"/><w:unhideWhenUsed/><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/><w:jc w:val=\"center\"/></w:pPr>\
         <w:rPr><w:color w:val=\"{FAINT_COLOR}\"/>{small}</w:rPr></w:style>\
         <w:style w:type=\"character\" w:styleId=\"CommentReference\"><w:name w:val=\"annotation reference\"/>\
         <w:basedOn w:val=\"DefaultParagraphFont\"/><w:uiPriority w:val=\"99\"/><w:semiHidden/><w:unhideWhenUsed/>\
         <w:rPr><w:sz w:val=\"16\"/><w:szCs w:val=\"16\"/></w:rPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"CommentText\"><w:name w:val=\"annotation text\"/><w:basedOn w:val=\"Normal\"/>\
         <w:uiPriority w:val=\"99\"/><w:unhideWhenUsed/><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr>\
         <w:rPr><w:sz w:val=\"20\"/><w:szCs w:val=\"20\"/></w:rPr></w:style>"
    ));
    s.push_str("</w:styles>");
    s
}

/// `word/numbering.xml`: the multi-level bullet definition and a bounded
/// number of identical multi-level decimal definitions.
///
/// All bullets use one numbering instance. Every entry of `numbered` is an
/// instance of a decimal definition (ids from [`FIRST_DECIMAL_NUM_ID`])
/// that restarts every level at the given number through
/// `w:lvlOverride` / `w:startOverride`. The body decides which list items
/// share an instance and which instances share a definition; see
/// [`super::numbering`] for the limits behind that.
pub(crate) fn numbering_xml(m: &Metrics, numbered: &[Instance]) -> String {
    // As in the editor: the text of an item starts one marker gutter in.
    // A bullet is centred in the gutter; a number is set flush right and
    // ends a little before the text, so the dots line up.
    let gutter = m.marker_gutter();
    let bullet = m.bullet_offset();
    let number = m.number_gap();
    let mut s = String::with_capacity(4096);
    s.push_str(DECLARATION);
    s.push_str(&format!("<w:numbering xmlns:w=\"{W_NS}\">"));
    // Word tells list definitions apart by their `nsid`.
    let nsid = |id: u32| format!("<w:nsid w:val=\"{:08X}\"/>", 0x5752_0000 + id);
    s.push_str(&format!(
        "<w:abstractNum w:abstractNumId=\"{BULLET_ABSTRACT}\">{}<w:multiLevelType w:val=\"hybridMultilevel\"/>",
        nsid(BULLET_ABSTRACT)
    ));
    for level in 0..9u32 {
        let glyph = ["\u{2022}", "\u{25e6}", "\u{25aa}"][level as usize % 3];
        s.push_str(&format!(
            "<w:lvl w:ilvl=\"{level}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"bullet\"/><w:lvlText w:val=\"{glyph}\"/><w:lvlJc w:val=\"center\"/>\
             <w:pPr><w:ind w:left=\"{}\" w:hanging=\"{bullet}\"/></w:pPr>\
             <w:rPr><w:rFonts w:ascii=\"Arial\" w:hAnsi=\"Arial\" w:cs=\"Arial\" w:hint=\"default\"/><w:color w:val=\"{MUTED_COLOR}\"/></w:rPr></w:lvl>",
            m.indent(level as u8) + gutter,
        ));
    }
    s.push_str("</w:abstractNum>");
    let mut decimal_levels = String::new();
    for level in 0..9u32 {
        decimal_levels.push_str(&format!(
            "<w:lvl w:ilvl=\"{level}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"decimal\"/><w:lvlText w:val=\"%{}.\"/><w:lvlJc w:val=\"right\"/>\
             <w:pPr><w:ind w:left=\"{}\" w:hanging=\"{number}\"/></w:pPr>\
             <w:rPr><w:color w:val=\"{MUTED_COLOR}\"/></w:rPr></w:lvl>",
            level + 1,
            m.indent(level as u8) + gutter,
        ));
    }
    let definitions = numbered.iter().map(|i| i.definition + 1).max().unwrap_or(0) as u32;
    for i in 0..definitions {
        s.push_str(&format!(
            "<w:abstractNum w:abstractNumId=\"{}\">{}<w:multiLevelType w:val=\"hybridMultilevel\"/>{decimal_levels}</w:abstractNum>",
            FIRST_DECIMAL_ABSTRACT + i,
            nsid(FIRST_DECIMAL_ABSTRACT + i),
        ));
    }
    s.push_str(&format!(
        "<w:num w:numId=\"{BULLET_NUM_ID}\"><w:abstractNumId w:val=\"{BULLET_ABSTRACT}\"/></w:num>"
    ));
    for (i, instance) in numbered.iter().enumerate() {
        s.push_str(&format!(
            "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"{}\"/>",
            FIRST_DECIMAL_NUM_ID + i as u32,
            FIRST_DECIMAL_ABSTRACT + instance.definition as u32,
        ));
        for (level, start) in instance.starts.iter().enumerate() {
            s.push_str(&format!(
                "<w:lvlOverride w:ilvl=\"{level}\"><w:startOverride w:val=\"{start}\"/></w:lvlOverride>"
            ));
        }
        s.push_str("</w:num>");
    }
    s.push_str("</w:numbering>");
    s
}
