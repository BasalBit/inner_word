//! `word/styles.xml` and `word/numbering.xml`.

use writer_core::{BlockKind, Typography};

use super::xml::{DECLARATION, attr};
use super::{Metrics, W_NS};

/// Fill colour of code blocks and inline code.
pub(crate) const CODE_FILL: &str = "F1EEE8";
/// Colour of hyperlinks.
pub(crate) const LINK_COLOR: &str = "0563C1";
/// Colour of the quote bar and of dividers.
pub(crate) const RULE_COLOR: &str = "B5AFA5";

/// The bullet numbering instance. Numbered lists use the ids above it.
pub(crate) const BULLET_NUM_ID: u32 = 1;
const BULLET_ABSTRACT: u32 = 0;
const FIRST_DECIMAL_ABSTRACT: u32 = 1;

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
pub(crate) fn styles_xml(typography: &Typography, m: &Metrics) -> String {
    let body_font = fonts(&typography.body_font);
    let body_size = size(m.half_points(typography.base_size));
    let after = m.twips(typography.paragraph_spacing);
    let line = m.line(typography.line_height);
    let mut s = String::with_capacity(8192);
    s.push_str(DECLARATION);
    s.push_str(&format!("<w:styles xmlns:w=\"{W_NS}\">"));
    s.push_str(&format!(
        "<w:docDefaults><w:rPrDefault><w:rPr>{body_font}{body_size}</w:rPr></w:rPrDefault>\
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
    // Headings: sizes from the typography and, as in the editor, extra
    // room above of 0.35 lines.
    for level in 1..=6u8 {
        let pt = typography.size_for(&BlockKind::heading(level));
        let before = m.twips(pt * typography.line_height * 0.35);
        s.push_str(&format!(
            "<w:style w:type=\"paragraph\" w:styleId=\"Heading{level}\"><w:name w:val=\"heading {level}\"/>\
             <w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:uiPriority w:val=\"9\"/><w:qFormat/>\
             <w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before=\"{before}\"/>\
             <w:outlineLvl w:val=\"{}\"/></w:pPr><w:rPr><w:b/><w:bCs/>{}</w:rPr></w:style>",
            level - 1,
            size(m.half_points(pt)),
        ));
    }
    let step = m.quote_gutter();
    s.push_str(&format!(
        "<w:style w:type=\"paragraph\" w:styleId=\"Quote\"><w:name w:val=\"Quote\"/><w:basedOn w:val=\"Normal\"/>\
         <w:uiPriority w:val=\"29\"/><w:qFormat/><w:pPr><w:pBdr><w:left w:val=\"single\" w:sz=\"18\" w:space=\"12\" w:color=\"{RULE_COLOR}\"/></w:pBdr>\
         <w:ind w:left=\"{step}\"/></w:pPr></w:style>"
    ));
    let code_font = fonts(&m.code_font);
    let code_size = size(m.half_points(typography.size_for(&BlockKind::code())));
    s.push_str(&format!(
        "<w:style w:type=\"paragraph\" w:styleId=\"Code\"><w:name w:val=\"Code\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/>\
         <w:pPr><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{CODE_FILL}\"/><w:suppressAutoHyphens/>\
         <w:jc w:val=\"left\"/></w:pPr>\
         <w:rPr>{code_font}<w:noProof/>{code_size}</w:rPr></w:style>"
    ));
    // List items sit closer together than paragraphs, as in the editor.
    let list_after = m.twips(typography.paragraph_spacing * 0.35);
    s.push_str(&format!(
        "<w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/><w:basedOn w:val=\"Normal\"/>\
         <w:uiPriority w:val=\"34\"/><w:qFormat/><w:pPr><w:spacing w:after=\"{list_after}\"/></w:pPr></w:style>"
    ));
    s.push_str(&format!(
        "<w:style w:type=\"character\" w:styleId=\"Hyperlink\"><w:name w:val=\"Hyperlink\"/><w:basedOn w:val=\"DefaultParagraphFont\"/>\
         <w:uiPriority w:val=\"99\"/><w:unhideWhenUsed/><w:rPr><w:color w:val=\"{LINK_COLOR}\"/><w:u w:val=\"single\"/></w:rPr></w:style>"
    ));
    let small = size(m.half_points(typography.base_size * 0.8));
    s.push_str(&format!(
        "<w:style w:type=\"paragraph\" w:styleId=\"Footer\"><w:name w:val=\"footer\"/><w:basedOn w:val=\"Normal\"/>\
         <w:uiPriority w:val=\"99\"/><w:unhideWhenUsed/><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/><w:jc w:val=\"center\"/></w:pPr>\
         <w:rPr>{small}</w:rPr></w:style>\
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

/// `word/numbering.xml`: one multi-level bullet definition, and one
/// multi-level decimal definition per numbered list.
///
/// `numbered_lists` numbering instances (ids from `BULLET_NUM_ID + 1`) each
/// get a definition of their own, so every separate list in the document
/// counts from 1. Sharing one definition and restarting with
/// `w:startOverride` is enough for Word, but Quick Look and other readers
/// then number all lists as one.
pub(crate) fn numbering_xml(m: &Metrics, numbered_lists: u32) -> String {
    let gutter = m.marker_gutter();
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
            "<w:lvl w:ilvl=\"{level}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"bullet\"/><w:lvlText w:val=\"{glyph}\"/><w:lvlJc w:val=\"left\"/>\
             <w:pPr><w:ind w:left=\"{}\" w:hanging=\"{gutter}\"/></w:pPr>\
             <w:rPr><w:rFonts w:ascii=\"Arial\" w:hAnsi=\"Arial\" w:cs=\"Arial\" w:hint=\"default\"/></w:rPr></w:lvl>",
            m.indent(level as u8) + gutter,
        ));
    }
    s.push_str("</w:abstractNum>");
    let mut decimal_levels = String::new();
    for level in 0..9u32 {
        decimal_levels.push_str(&format!(
            "<w:lvl w:ilvl=\"{level}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"decimal\"/><w:lvlText w:val=\"%{}.\"/><w:lvlJc w:val=\"left\"/>\
             <w:pPr><w:ind w:left=\"{}\" w:hanging=\"{gutter}\"/></w:pPr></w:lvl>",
            level + 1,
            m.indent(level as u8) + gutter,
        ));
    }
    for i in 0..numbered_lists {
        s.push_str(&format!(
            "<w:abstractNum w:abstractNumId=\"{}\">{}<w:multiLevelType w:val=\"hybridMultilevel\"/>{decimal_levels}</w:abstractNum>",
            FIRST_DECIMAL_ABSTRACT + i,
            nsid(FIRST_DECIMAL_ABSTRACT + i),
        ));
    }
    s.push_str(&format!("<w:num w:numId=\"{BULLET_NUM_ID}\"><w:abstractNumId w:val=\"{BULLET_ABSTRACT}\"/></w:num>"));
    for i in 0..numbered_lists {
        s.push_str(&format!(
            "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"{}\"/></w:num>",
            BULLET_NUM_ID + 1 + i,
            FIRST_DECIMAL_ABSTRACT + i
        ));
    }
    s.push_str("</w:numbering>");
    s
}
