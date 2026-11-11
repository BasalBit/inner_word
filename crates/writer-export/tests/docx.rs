//! DOCX export tests: the package is unzipped with an independent ZIP
//! reader, every part is parsed with an XML parser, and the required parts,
//! relationships and document structure are checked.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

use roxmltree::Node;
use writer_core::{
    Alignment, Block, BlockKind, Document, DocumentSettings, EditorState, Highlight, InlineAttrs, InlineStyle,
    Margins, Orientation, PageSetup, PaperSize, Position, Rgb, Selection, Typography,
};
use writer_export::docx::parts;
use writer_export::{DocxOptions, ExportSource, PartTitles, to_docx};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";

// ----- reading the package ----------------------------------------------------------------

/// The parts of a package: name to XML text. Also checks the archive
/// itself: CRCs (the reader verifies them), no duplicate names.
fn unzip(bytes: &[u8]) -> BTreeMap<String, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("a valid ZIP archive");
    let mut parts = BTreeMap::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).expect("a readable entry");
        let mut content = String::new();
        file.read_to_string(&mut content).expect("UTF-8 content with a valid CRC");
        assert!(parts.insert(file.name().to_string(), content).is_none(), "duplicate part {}", file.name());
    }
    parts
}

struct Package {
    parts: BTreeMap<String, String>,
}

impl Package {
    fn new(source: &ExportSource<'_>, options: &DocxOptions) -> Package {
        let bytes = to_docx(source, options);
        let package = Package { parts: unzip(&bytes) };
        package.check();
        package
    }

    fn of(doc: &Document) -> Package {
        Package::new(&ExportSource::single("Test", doc), &DocxOptions::default())
    }

    fn part(&self, name: &str) -> &str {
        self.parts.get(name).unwrap_or_else(|| panic!("missing part {name}"))
    }

    fn has(&self, name: &str) -> bool {
        self.parts.contains_key(name)
    }

    /// `Id -> (Type, Target, external)` of a relationships part.
    fn rels(&self, name: &str) -> BTreeMap<String, (String, String, bool)> {
        let doc = roxmltree::Document::parse(self.part(name)).unwrap();
        doc.root_element()
            .children()
            .filter(|n| n.has_tag_name(("http://schemas.openxmlformats.org/package/2006/relationships", "Relationship")))
            .map(|n| {
                (
                    n.attribute("Id").unwrap().to_string(),
                    (
                        n.attribute("Type").unwrap().to_string(),
                        n.attribute("Target").unwrap().to_string(),
                        n.attribute("TargetMode") == Some("External"),
                    ),
                )
            })
            .collect()
    }

    /// Everything that must hold for any package this crate writes.
    fn check(&self) {
        // Every part is well-formed XML with a declaration.
        for (name, xml) in &self.parts {
            assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\""), "{name} has no declaration");
            roxmltree::Document::parse(xml).unwrap_or_else(|e| panic!("{name} is not well-formed: {e}\n{xml}"));
        }
        // Required parts.
        for name in [
            parts::CONTENT_TYPES,
            parts::ROOT_RELS,
            parts::CORE,
            parts::APP,
            parts::DOCUMENT,
            parts::DOCUMENT_RELS,
            parts::STYLES,
            parts::NUMBERING,
            parts::SETTINGS,
        ] {
            assert!(self.has(name), "missing {name}");
        }
        // The content types name every part, and only parts that exist.
        let types = roxmltree::Document::parse(self.part(parts::CONTENT_TYPES)).unwrap();
        let overrides: BTreeSet<String> = types
            .root_element()
            .children()
            .filter(|n| n.has_tag_name(("http://schemas.openxmlformats.org/package/2006/content-types", "Override")))
            .map(|n| n.attribute("PartName").unwrap().trim_start_matches('/').to_string())
            .collect();
        for name in self.parts.keys() {
            if name != parts::CONTENT_TYPES && !name.ends_with(".rels") {
                assert!(overrides.contains(name), "no content type for {name}");
            }
        }
        for name in &overrides {
            assert!(self.has(name), "content type for missing part {name}");
        }
        // Relationship targets exist.
        for (_, (_, target, external)) in self.rels(parts::ROOT_RELS) {
            assert!(!external && self.has(&target), "root relationship to missing {target}");
        }
        let rels = self.rels(parts::DOCUMENT_RELS);
        for (_, target, external) in rels.values() {
            if !external {
                assert!(self.has(&format!("word/{target}")), "relationship to missing word/{target}");
            }
        }
        let kinds: BTreeSet<&str> = rels.values().map(|(t, _, _)| t.rsplit('/').next().unwrap()).collect();
        for kind in ["styles", "numbering", "settings"] {
            assert!(kinds.contains(kind), "no {kind} relationship");
        }
        // Every r:id, style, numbering instance and comment the document
        // refers to exists.
        let document = roxmltree::Document::parse(self.part(parts::DOCUMENT)).unwrap();
        let styles = roxmltree::Document::parse(self.part(parts::STYLES)).unwrap();
        let style_ids: BTreeSet<&str> = styles.descendants().filter(|n| n.has_tag_name((W, "style"))).filter_map(|n| n.attribute((W, "styleId"))).collect();
        let numbering = roxmltree::Document::parse(self.part(parts::NUMBERING)).unwrap();
        let num_ids: BTreeSet<&str> = numbering.descendants().filter(|n| n.has_tag_name((W, "num"))).filter_map(|n| n.attribute((W, "numId"))).collect();
        let abstract_ids: BTreeSet<&str> = numbering.descendants().filter(|n| n.has_tag_name((W, "abstractNum"))).filter_map(|n| n.attribute((W, "abstractNumId"))).collect();
        for n in numbering.descendants().filter(|n| n.has_tag_name((W, "abstractNumId"))) {
            assert!(abstract_ids.contains(n.attribute((W, "val")).unwrap()));
        }
        let mut starts = Vec::new();
        let mut ends = Vec::new();
        let mut refs = Vec::new();
        for n in document.descendants() {
            if let Some(id) = n.attribute((R, "id")) {
                assert!(rels.contains_key(id), "dangling relationship {id}");
            }
            if n.has_tag_name((W, "pStyle")) || n.has_tag_name((W, "rStyle")) {
                let id = n.attribute((W, "val")).unwrap();
                assert!(style_ids.contains(id), "unknown style {id}");
            }
            if n.has_tag_name((W, "numId")) {
                let id = n.attribute((W, "val")).unwrap();
                assert!(num_ids.contains(id), "unknown numbering {id}");
            }
            let id = || n.attribute((W, "id")).unwrap().to_string();
            if n.has_tag_name((W, "commentRangeStart")) {
                starts.push(id());
            } else if n.has_tag_name((W, "commentRangeEnd")) {
                ends.push(id());
            } else if n.has_tag_name((W, "commentReference")) {
                refs.push(id());
            }
        }
        // Styles refer to styles that exist.
        for n in styles.descendants().filter(|n| n.has_tag_name((W, "basedOn")) || n.has_tag_name((W, "next"))) {
            assert!(style_ids.contains(n.attribute((W, "val")).unwrap()));
        }
        // Comments: each one has exactly one start, end and reference, in
        // that order, and an entry in both comment parts.
        let comment_ids: Vec<String> = if self.has(parts::COMMENTS) {
            let comments = roxmltree::Document::parse(self.part(parts::COMMENTS)).unwrap();
            let ids: Vec<String> = comments.descendants().filter(|n| n.has_tag_name((W, "comment"))).map(|n| n.attribute((W, "id")).unwrap().to_string()).collect();
            let para_ids: BTreeSet<String> = comments.descendants().filter_map(|n| n.attribute((W14, "paraId"))).map(str::to_string).collect();
            assert_eq!(para_ids.len(), ids.len(), "one paragraph id per comment");
            let extended = roxmltree::Document::parse(self.part(parts::COMMENTS_EXTENDED)).unwrap();
            let ex: Vec<Node> = extended.descendants().filter(|n| n.has_tag_name((W15, "commentEx"))).collect();
            assert_eq!(ex.len(), ids.len());
            for e in ex {
                assert!(para_ids.contains(e.attribute((W15, "paraId")).unwrap()));
                if let Some(parent) = e.attribute((W15, "paraIdParent")) {
                    assert!(para_ids.contains(parent));
                }
            }
            assert!(kinds.contains("comments") && kinds.contains("commentsExtended"));
            ids
        } else {
            Vec::new()
        };
        let sorted = |mut v: Vec<String>| {
            v.sort();
            v
        };
        assert_eq!(sorted(starts.clone()), sorted(comment_ids.clone()));
        assert_eq!(sorted(ends.clone()), sorted(comment_ids.clone()));
        assert_eq!(sorted(refs), sorted(comment_ids));
        let xml = self.part(parts::DOCUMENT);
        for id in &starts {
            let s = xml.find(&format!("<w:commentRangeStart w:id=\"{id}\"/>")).unwrap();
            let e = xml.find(&format!("<w:commentRangeEnd w:id=\"{id}\"/>")).unwrap();
            assert!(s < e, "comment {id} ends before it starts");
        }
        // The body ends with section properties, and every paragraph and
        // run holds only elements that may be there.
        let body = document.descendants().find(|n| n.has_tag_name((W, "body"))).unwrap();
        assert!(body.last_element_child().unwrap().has_tag_name((W, "sectPr")));
        for p in body.children().filter(Node::is_element) {
            assert!(p.has_tag_name((W, "p")) || p.has_tag_name((W, "sectPr")), "unexpected {:?} in body", p.tag_name());
        }
        // No character XML forbids, anywhere.
        for (name, xml) in &self.parts {
            assert!(
                xml.chars().all(|c| matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..)),
                "{name} has a forbidden character"
            );
        }
    }

    fn paragraphs(&self) -> Vec<P> {
        let rels = self.rels(parts::DOCUMENT_RELS);
        let document = roxmltree::Document::parse(self.part(parts::DOCUMENT)).unwrap();
        let body = document.descendants().find(|n| n.has_tag_name((W, "body"))).unwrap();
        body.children().filter(|n| n.has_tag_name((W, "p"))).map(|p| P::read(p, &rels)).collect()
    }

    /// The `w:sectPr` elements in document order.
    fn sections(&self) -> Vec<Section> {
        let document = roxmltree::Document::parse(self.part(parts::DOCUMENT)).unwrap();
        document
            .descendants()
            .filter(|n| n.has_tag_name((W, "sectPr")))
            .map(|s| {
                let size = child(s, "pgSz").unwrap();
                let mar = child(s, "pgMar").unwrap();
                let num = |n: Node, a: &str| n.attribute((W, a)).unwrap().parse::<i32>().unwrap();
                (
                    num(size, "w"),
                    num(size, "h"),
                    size.attribute((W, "orient")).unwrap_or("portrait").to_string(),
                    num(mar, "top"),
                    num(mar, "right"),
                    num(mar, "bottom"),
                    num(mar, "left"),
                    child(s, "footerReference").is_some(),
                )
            })
            .collect()
    }
}

/// A section: `(width, height, orientation, top, right, bottom, left, has a footer)`, lengths in twips.
type Section = (i32, i32, String, i32, i32, i32, i32, bool);

fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| c.has_tag_name((W, name)))
}

fn val<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    child(n, name).and_then(|c| c.attribute((W, "val")))
}

/// A run as the tests see it.
#[derive(Debug, Default, Clone, PartialEq)]
struct Run {
    text: String,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    rtl: bool,
    color: Option<String>,
    font: Option<String>,
    size: Option<u32>,
    shade: Option<String>,
    style: Option<String>,
    link: Option<String>,
    page_break: bool,
}

/// A paragraph as the tests see it.
#[derive(Debug, Default, Clone, PartialEq)]
struct P {
    style: String,
    num: Option<(u32, u32)>,
    left: Option<i32>,
    hanging: Option<i32>,
    align: Option<String>,
    bidi: bool,
    rule: bool,
    section: bool,
    runs: Vec<Run>,
}

impl P {
    fn read(p: Node, rels: &BTreeMap<String, (String, String, bool)>) -> P {
        let mut out = P::default();
        if let Some(ppr) = child(p, "pPr") {
            out.style = val(ppr, "pStyle").unwrap_or("").to_string();
            if let Some(num) = child(ppr, "numPr") {
                out.num = Some((val(num, "numId").unwrap().parse().unwrap(), val(num, "ilvl").unwrap().parse().unwrap()));
            }
            if let Some(ind) = child(ppr, "ind") {
                out.left = ind.attribute((W, "left")).map(|v| v.parse().unwrap());
                out.hanging = ind.attribute((W, "hanging")).map(|v| v.parse().unwrap());
            }
            out.align = val(ppr, "jc").map(str::to_string);
            out.bidi = child(ppr, "bidi").is_some();
            out.rule = child(ppr, "pBdr").is_some_and(|b| child(b, "bottom").is_some());
            out.section = child(ppr, "sectPr").is_some();
        }
        let read_run = |r: Node, link: Option<String>| {
            let mut run = Run {
                link,
                ..Run::default()
            };
            for c in r.children().filter(Node::is_element) {
                match c.tag_name().name() {
                    "rPr" => {
                        run.bold = child(c, "b").is_some();
                        assert_eq!(run.bold, child(c, "bCs").is_some(), "bold needs its complex-script twin");
                        run.italic = child(c, "i").is_some();
                        run.underline = child(c, "u").is_some();
                        run.strike = child(c, "strike").is_some();
                        run.rtl = child(c, "rtl").is_some();
                        run.color = val(c, "color").map(str::to_string);
                        run.font = child(c, "rFonts").and_then(|f| f.attribute((W, "ascii"))).map(str::to_string);
                        run.size = val(c, "sz").map(|v| v.parse().unwrap());
                        assert_eq!(run.size, val(c, "szCs").map(|v| v.parse().unwrap()));
                        run.shade = child(c, "shd").and_then(|f| f.attribute((W, "fill"))).map(str::to_string);
                        run.style = val(c, "rStyle").map(str::to_string);
                    }
                    "t" => {
                        let t = c.text().unwrap_or("");
                        if c.attribute(("http://www.w3.org/XML/1998/namespace", "space")) != Some("preserve") {
                            assert_eq!(t, t.trim(), "whitespace needs xml:space");
                        }
                        run.text.push_str(t);
                    }
                    "br" if c.attribute((W, "type")) == Some("page") => run.page_break = true,
                    "br" => run.text.push('\n'),
                    "tab" => run.text.push('\t'),
                    "commentReference" => {}
                    other => panic!("unexpected <w:{other}> in a run"),
                }
            }
            run
        };
        for c in p.children().filter(Node::is_element) {
            match c.tag_name().name() {
                "pPr" | "commentRangeStart" | "commentRangeEnd" => {}
                "r" => out.runs.push(read_run(c, None)),
                "hyperlink" => {
                    let target = rels[c.attribute((R, "id")).unwrap()].1.clone();
                    for r in c.children().filter(Node::is_element) {
                        match r.tag_name().name() {
                            "r" => out.runs.push(read_run(r, Some(target.clone()))),
                            "commentRangeStart" | "commentRangeEnd" => {}
                            other => panic!("unexpected <w:{other}> in a hyperlink"),
                        }
                    }
                }
                other => panic!("unexpected <w:{other}> in a paragraph"),
            }
        }
        out
    }

    fn text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
}

fn paragraphs(blocks: Vec<Block>) -> Vec<P> {
    Package::of(&Document::from_blocks(blocks)).paragraphs()
}

fn plain() -> InlineAttrs {
    InlineAttrs::default()
}

fn link(url: &str) -> InlineAttrs {
    InlineAttrs {
        link: Some(url.to_string()),
        ..plain()
    }
}

// ----- the package ------------------------------------------------------------------------

#[test]
fn package_parts_and_order() {
    let doc = Document::from_blocks(vec![Block::paragraph("Hello")]);
    let bytes = to_docx(&ExportSource::single("T", &doc), &DocxOptions::default());
    assert_eq!(&bytes[..4], b"PK\x03\x04");
    // The first entry is the content types, as packages conventionally start.
    assert_eq!(&bytes[30..30 + parts::CONTENT_TYPES.len()], parts::CONTENT_TYPES.as_bytes());
    let package = Package { parts: unzip(&bytes) };
    package.check();
    let names: Vec<&str> = package.parts.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        vec![
            "[Content_Types].xml",
            "_rels/.rels",
            "docProps/app.xml",
            "docProps/core.xml",
            "word/_rels/document.xml.rels",
            "word/document.xml",
            "word/footer1.xml",
            "word/numbering.xml",
            "word/settings.xml",
            "word/styles.xml",
        ]
    );
    // Without the footer the part, its content type and its relationship go.
    let o = DocxOptions {
        page_numbers: false,
        ..Default::default()
    };
    let package = Package::new(&ExportSource::single("T", &doc), &o);
    assert!(!package.has(parts::FOOTER));
    assert!(!package.part(parts::CONTENT_TYPES).contains("footer"));
    assert!(!package.part(parts::DOCUMENT_RELS).contains("footer"));
    assert!(!package.part(parts::DOCUMENT).contains("footerReference"));
}

#[test]
fn output_is_deterministic() {
    let doc = Document::from_blocks(vec![Block::paragraph("Same"), Block::with_text(BlockKind::NumberedItem, "x")]);
    let o = DocxOptions {
        created: 1_794_391_505_000,
        modified: 1_794_391_505_000,
        ..Default::default()
    };
    let source = ExportSource::single("T", &doc);
    assert_eq!(to_docx(&source, &o), to_docx(&source, &o));
}

#[test]
fn core_properties() {
    let doc = Document::new();
    let o = DocxOptions {
        author: " Ada <Lovelace> & Co ".into(),
        created: 0,
        modified: 1_794_391_505_999,
        ..Default::default()
    };
    let package = Package::new(&ExportSource::single("Fish & <Chips>\u{0} \"quoted\"", &doc), &o);
    let core = roxmltree::Document::parse(package.part(parts::CORE)).unwrap();
    let get = |name: &str| core.descendants().find(|n| n.tag_name().name() == name).and_then(|n| n.text()).unwrap_or("").to_string();
    assert_eq!(get("title"), "Fish & <Chips> \"quoted\"");
    assert_eq!(get("creator"), "Ada <Lovelace> & Co");
    assert_eq!(get("lastModifiedBy"), "Ada <Lovelace> & Co");
    assert_eq!(get("created"), "1970-01-01T00:00:00Z");
    assert_eq!(get("modified"), "2026-11-11T10:05:05Z");
    assert!(package.part(parts::APP).contains("<Application>The Writer</Application>"));
    assert!(package.part(parts::SETTINGS).contains("compatibilityMode"));
}

#[test]
fn styles_follow_the_typography() {
    let settings = DocumentSettings {
        typography: Typography {
            body_font: "Iowan \"Old\" Style".into(),
            base_size: 12.0,
            line_height: 1.5,
            paragraph_spacing: 6.0,
        },
        ..Default::default()
    };
    let doc = Document::from_blocks(vec![Block::paragraph("x")]).with_settings(settings);
    let package = Package::of(&doc);
    let styles = roxmltree::Document::parse(package.part(parts::STYLES)).unwrap();
    let style = |id: &str| styles.descendants().find(|n| n.has_tag_name((W, "style")) && n.attribute((W, "styleId")) == Some(id)).unwrap_or_else(|| panic!("no style {id}"));
    let normal = style("Normal");
    assert_eq!(normal.attribute((W, "default")), Some("1"));
    let rpr = child(normal, "rPr").unwrap();
    assert_eq!(child(rpr, "rFonts").unwrap().attribute((W, "ascii")), Some("Iowan \"Old\" Style"));
    assert_eq!(val(rpr, "sz"), Some("24"));
    let spacing = child(child(normal, "pPr").unwrap(), "spacing").unwrap();
    assert_eq!(spacing.attribute((W, "after")), Some("120"));
    assert_eq!(spacing.attribute((W, "line")), Some("300"));
    assert_eq!(spacing.attribute((W, "lineRule")), Some("auto"));
    // Headings: built-in names, outline levels, the typography's sizes.
    let t = &doc.settings().typography;
    for level in 1..=6u8 {
        let h = style(&format!("Heading{level}"));
        assert_eq!(val(h, "name"), Some(format!("heading {level}").as_str()));
        assert_eq!(val(h, "basedOn"), Some("Normal"));
        let size = (t.size_for(&BlockKind::heading(level)) * 2.0) as u32;
        assert_eq!(val(child(h, "rPr").unwrap(), "sz"), Some(size.to_string().as_str()));
        assert_eq!(val(child(h, "pPr").unwrap(), "outlineLvl"), Some((level - 1).to_string().as_str()));
    }
    assert_eq!(val(style("Heading1"), "name"), Some("heading 1"));
    for id in ["Quote", "Code", "ListParagraph", "Hyperlink", "Footer", "CommentReference", "CommentText"] {
        style(id);
    }
    let code = child(style("Code"), "rPr").unwrap();
    assert_eq!(child(code, "rFonts").unwrap().attribute((W, "ascii")), Some("Menlo"));
    assert_eq!(val(code, "sz"), Some("22"));
    assert!(child(child(style("Code"), "pPr").unwrap(), "shd").is_some());
    assert_eq!(style("Hyperlink").attribute((W, "type")), Some("character"));
}

#[test]
fn text_scale_scales_text_but_not_the_page() {
    let doc = Document::from_blocks(vec![Block::paragraph("x").with_text_size(Some(34.0))]);
    let o = DocxOptions {
        text_scale: 12.0 / 17.0,
        ..Default::default()
    };
    let package = Package::new(&ExportSource::single("T", &doc), &o);
    assert!(package.part(parts::STYLES).contains("<w:sz w:val=\"24\"/>"));
    assert_eq!(package.paragraphs()[0].runs[0].size, Some(48));
    assert_eq!(package.sections()[0].0, 11906);
}

// ----- blocks -----------------------------------------------------------------------------

#[test]
fn headings_paragraphs_quotes_and_code() {
    let ps = paragraphs(vec![
        Block::with_text(BlockKind::heading(1), "One"),
        Block::with_text(BlockKind::heading(6), "Six"),
        Block::paragraph("Body"),
        Block::paragraph(""),
        Block::with_text(BlockKind::Quote, "Wise\nwords"),
        Block::with_text(BlockKind::CodeBlock { language: Some("rust".into()) }, "fn main() {\n\tlet x  =  1;\n}\n"),
    ]);
    let styles: Vec<&str> = ps.iter().map(|p| p.style.as_str()).collect();
    assert_eq!(styles, vec!["Heading1", "Heading6", "", "", "Quote", "Code"]);
    assert_eq!(ps[0].text(), "One");
    assert_eq!(ps[2].text(), "Body");
    // Empty paragraphs are kept: they are the writer's spacing.
    assert!(ps[3].runs.is_empty());
    assert_eq!(ps[4].text(), "Wise\nwords");
    // Code keeps every space, tab and line break, in one paragraph.
    assert_eq!(ps[5].text(), "fn main() {\n\tlet x  =  1;\n}\n");
    assert!(ps[5].runs.iter().all(|r| r.font.is_none() && !r.bold));
}

#[test]
fn lists_use_numbering_with_levels() {
    let ps = paragraphs(vec![
        Block::with_text(BlockKind::BulletItem, "a"),
        Block::with_text(BlockKind::BulletItem, "b").with_indent(1),
        Block::with_text(BlockKind::BulletItem, "c").with_indent(8),
        Block::with_text(BlockKind::NumberedItem, "one"),
        Block::with_text(BlockKind::NumberedItem, "two"),
        Block::with_text(BlockKind::NumberedItem, "nested one").with_indent(1),
        Block::with_text(BlockKind::NumberedItem, "nested two").with_indent(1),
        Block::with_text(BlockKind::NumberedItem, "three"),
    ]);
    assert!(ps.iter().all(|p| p.style == "ListParagraph"));
    assert_eq!(ps[0].num, Some((1, 0)));
    assert_eq!(ps[1].num, Some((1, 1)));
    assert_eq!(ps[2].num, Some((1, 8)));
    // One numbering instance for the outer list, another for the nested one.
    assert_eq!(ps[3].num, Some((2, 0)));
    assert_eq!(ps[4].num, Some((2, 0)));
    assert_eq!(ps[5].num, Some((3, 1)));
    assert_eq!(ps[6].num, Some((3, 1)));
    assert_eq!(ps[7].num, Some((2, 0)));
}

#[test]
fn numbering_restarts_for_each_separate_list() {
    let doc = Document::from_blocks(vec![
        Block::with_text(BlockKind::NumberedItem, "1"),
        Block::with_text(BlockKind::NumberedItem, "2"),
        Block::paragraph("between"),
        Block::with_text(BlockKind::NumberedItem, "1 again"),
        Block::with_text(BlockKind::NumberedItem, "nested").with_indent(1),
        Block::with_text(BlockKind::BulletItem, "bullet at the nested level").with_indent(1),
        Block::with_text(BlockKind::NumberedItem, "nested, restarted").with_indent(1),
        Block::paragraph("child paragraph").with_indent(1),
        Block::with_text(BlockKind::NumberedItem, "2 again"),
    ]);
    // What the document says the numbers are.
    assert_eq!(
        doc.numbered_ordinals(0..9),
        vec![Some(1), Some(2), None, Some(1), Some(1), None, Some(1), None, Some(2)]
    );
    let package = Package::of(&doc);
    let ps = package.paragraphs();
    let nums: Vec<Option<(u32, u32)>> = ps.iter().map(|p| p.num).collect();
    assert_eq!(
        nums,
        vec![Some((2, 0)), Some((2, 0)), None, Some((3, 0)), Some((4, 1)), Some((1, 1)), Some((5, 1)), None, Some((3, 0))]
    );
    // Every numbered list has a numbering instance and a definition of its
    // own, so each counts from 1; instance 1 is the bullets.
    let numbering = roxmltree::Document::parse(package.part(parts::NUMBERING)).unwrap();
    let nums: Vec<Node> = numbering.descendants().filter(|n| n.has_tag_name((W, "num"))).collect();
    assert_eq!(nums.len(), 5);
    let definitions: BTreeSet<&str> = nums.iter().map(|n| val(*n, "abstractNumId").unwrap()).collect();
    assert_eq!(definitions.len(), 5);
    let abstracts: Vec<Node> = numbering.descendants().filter(|n| n.has_tag_name((W, "abstractNum"))).collect();
    assert_eq!(abstracts.len(), 5);
    for (i, a) in abstracts.iter().enumerate() {
        let format = if i == 0 { "bullet" } else { "decimal" };
        let levels: Vec<Node> = a.children().filter(|n| n.has_tag_name((W, "lvl"))).collect();
        assert_eq!(levels.len(), 9);
        assert!(levels.iter().all(|l| val(*l, "numFmt") == Some(format) && val(*l, "start") == Some("1")));
        // Deeper levels are indented further.
        let lefts: Vec<i32> = levels.iter().map(|l| child(child(*l, "pPr").unwrap(), "ind").unwrap().attribute((W, "left")).unwrap().parse().unwrap()).collect();
        assert!(lefts.windows(2).all(|w| w[1] > w[0]));
    }
    assert_eq!(val(abstracts[1].children().find(|n| n.has_tag_name((W, "lvl"))).unwrap(), "lvlText"), Some("%1."));
    // All definitions come before all instances, as the schema requires.
    let xml = package.part(parts::NUMBERING);
    assert!(xml.rfind("<w:abstractNum ").unwrap() < xml.find("<w:num ").unwrap());
    // The paragraph nested in the list is indented one level (24 pt).
    assert_eq!(ps[7].left, Some(480));
}

#[test]
fn todos_have_ballot_boxes() {
    let ps = paragraphs(vec![
        Block::with_text(BlockKind::TodoItem { checked: false }, "open"),
        Block::with_text(BlockKind::TodoItem { checked: true }, "done").with_indent(1),
        Block::new(BlockKind::TodoItem { checked: false }),
    ]);
    assert_eq!(ps[0].text(), "\u{2610}\topen");
    assert_eq!(ps[1].text(), "\u{2611}\tdone");
    assert_eq!(ps[2].text(), "\u{2610}\t");
    assert_eq!(ps[0].style, "ListParagraph");
    assert_eq!(ps[0].num, None);
    // Like list items: 28 pt for the marker, 24 pt more per level.
    assert_eq!((ps[0].left, ps[0].hanging), (Some(560), Some(560)));
    assert_eq!((ps[1].left, ps[1].hanging), (Some(1040), Some(560)));
}

#[test]
fn dividers_and_page_breaks() {
    let ps = paragraphs(vec![
        Block::new(BlockKind::Divider),
        Block::paragraph("one"),
        Block::new(BlockKind::PageBreak),
        Block::paragraph("two"),
        Block::new(BlockKind::PageBreak),
        Block::new(BlockKind::PageBreak),
        Block::with_text(BlockKind::heading(1), "three"),
        Block::new(BlockKind::Divider),
        Block::new(BlockKind::PageBreak),
    ]);
    assert_eq!(ps.len(), 7);
    assert!(ps[0].rule && ps[0].runs.is_empty());
    assert!(!ps[1].rule);
    // The page break is a run at the start of the next paragraph.
    assert!(ps[2].runs[0].page_break && ps[2].text() == "two");
    // Two in a row: an empty page.
    assert!(ps[3].runs[0].page_break && ps[3].text().is_empty());
    assert!(ps[4].runs[0].page_break && ps[4].style == "Heading1");
    assert!(ps[5].rule);
    // A trailing page break stands alone.
    assert!(ps[6].runs[0].page_break && ps[6].runs.len() == 1);
    // Void blocks only.
    let ps = paragraphs(vec![Block::new(BlockKind::PageBreak)]);
    assert_eq!(ps.len(), 1);
    assert!(ps[0].runs[0].page_break);
}

#[test]
fn alignment_and_indent() {
    let ps = paragraphs(vec![
        Block::paragraph("left"),
        Block::paragraph("center").with_alignment(Alignment::Center),
        Block::paragraph("right").with_alignment(Alignment::Right),
        Block::paragraph("justify").with_alignment(Alignment::Justify),
        Block::paragraph("in").with_indent(2),
        Block::with_text(BlockKind::Quote, "quote in").with_indent(1),
        Block::with_text(BlockKind::heading(2), "centred heading").with_alignment(Alignment::Center),
    ]);
    let aligns: Vec<Option<&str>> = ps.iter().map(|p| p.align.as_deref()).collect();
    assert_eq!(aligns, vec![None, Some("center"), Some("right"), Some("both"), None, None, Some("center")]);
    assert_eq!(ps[4].left, Some(960));
    assert_eq!(ps[5].left, Some(880));
    assert_eq!(ps[0].left, None);
}

// ----- runs -------------------------------------------------------------------------------

#[test]
fn run_formatting() {
    let style = |s: InlineStyle| plain().with(s);
    let ps = paragraphs(vec![
        Block::new(BlockKind::Paragraph)
            .with_span("plain ", plain())
            .with_span("bold", style(InlineStyle::Bold))
            .with_span("italic", style(InlineStyle::Italic))
            .with_span("under", style(InlineStyle::Underline))
            .with_span("strike", style(InlineStyle::Strikethrough))
            .with_span("code", style(InlineStyle::Code))
            .with_span("marked", InlineAttrs { highlight: Some(Highlight::Yellow), ..plain() })
            .with_span("red", InlineAttrs { color: Some(Rgb::new(0xcc, 0x33, 0x00)), ..plain() })
            .with_span("font", InlineAttrs { font: Some("Georgia".into()), ..plain() }),
    ]);
    let r = &ps[0].runs;
    assert_eq!(r.len(), 9);
    assert_eq!(r[0], Run { text: "plain ".into(), ..Run::default() });
    assert!(r[1].bold && !r[1].italic);
    assert!(r[2].italic);
    assert!(r[3].underline);
    assert!(r[4].strike);
    assert_eq!(r[5].font.as_deref(), Some("Menlo"));
    assert_eq!(r[5].shade.as_deref(), Some("F1EEE8"));
    assert_eq!(r[6].shade.as_deref(), Some("FFF176"));
    assert_eq!(r[7].color.as_deref(), Some("CC3300"));
    assert_eq!(r[8].font.as_deref(), Some("Georgia"));
    assert!(r.iter().all(|r| r.size.is_none()));
}

#[test]
fn every_attribute_at_once() {
    let all = InlineAttrs {
        bold: true,
        italic: true,
        underline: true,
        strikethrough: true,
        code: true,
        highlight: Some(Highlight::Purple),
        color: Some(Rgb::new(1, 2, 3)),
        font: Some("Charter".into()),
        link: Some("https://example.com/all".into()),
        ..plain()
    };
    let ps = paragraphs(vec![Block::new(BlockKind::heading(2)).with_span("every\nthing", all).with_text_size(Some(20.0)).with_alignment(Alignment::Right).with_indent(1)]);
    assert_eq!(ps[0].style, "Heading2");
    assert_eq!(ps[0].align.as_deref(), Some("right"));
    assert_eq!(ps[0].left, Some(480));
    assert_eq!(
        ps[0].runs,
        vec![Run {
            text: "every\nthing".into(),
            bold: true,
            italic: true,
            underline: true,
            strike: true,
            rtl: false,
            color: Some("010203".into()),
            // Inline code wins the font; the highlight wins the shading.
            font: Some("Menlo".into()),
            size: Some(40),
            shade: Some("D9C5F0".into()),
            style: Some("Hyperlink".into()),
            link: Some("https://example.com/all".into()),
            page_break: false,
        }]
    );
}

#[test]
fn block_text_size_is_on_every_run() {
    let ps = paragraphs(vec![
        Block::paragraph("big ").with_span("bold", plain().with(InlineStyle::Bold)).with_text_size(Some(24.0)),
        Block::paragraph("normal"),
        Block::with_text(BlockKind::TodoItem { checked: false }, "small").with_text_size(Some(10.0)),
    ]);
    assert!(ps[0].runs.iter().all(|r| r.size == Some(48)));
    assert!(ps[1].runs.iter().all(|r| r.size.is_none()));
    assert!(ps[2].runs.iter().all(|r| r.size == Some(20)));
}

#[test]
fn hyperlinks_go_through_relationships() {
    let package = Package::of(&Document::from_blocks(vec![
        Block::paragraph("See ")
            .with_span("this ", link("https://example.com/a?b=1&c=2"))
            .with_span("bold part", InlineAttrs { bold: true, ..link("https://example.com/a?b=1&c=2") })
            .with_span(" and ", plain())
            .with_span("that", link("my file <1>.txt"))
            .with_span("again", link("https://example.com/a?b=1&c=2"))
            .with_span("blank", link("   ")),
    ]));
    let ps = package.paragraphs();
    let links: Vec<Option<&str>> = ps[0].runs.iter().map(|r| r.link.as_deref()).collect();
    assert_eq!(
        links,
        vec![
            None,
            Some("https://example.com/a?b=1&c=2"),
            Some("https://example.com/a?b=1&c=2"),
            None,
            Some("my%20file%20%3C1%3E.txt"),
            Some("https://example.com/a?b=1&c=2"),
            None,
        ]
    );
    assert!(ps[0].runs[1].style.as_deref() == Some("Hyperlink") && ps[0].runs[2].bold);
    // One relationship per distinct target, all external.
    let rels = package.rels(parts::DOCUMENT_RELS);
    let hyper: Vec<_> = rels.values().filter(|(t, _, _)| t.ends_with("/hyperlink")).collect();
    assert_eq!(hyper.len(), 2);
    assert!(hyper.iter().all(|(_, _, external)| *external));
    // The two runs of the first link share one hyperlink element.
    assert_eq!(package.part(parts::DOCUMENT).matches("<w:hyperlink ").count(), 3);
}

#[test]
fn right_to_left_text() {
    let ps = paragraphs(vec![
        Block::paragraph("שלום עולם"),
        Block::paragraph("Latin with עברית inside"),
        Block::paragraph("مرحبا").with_alignment(Alignment::Right),
        Block::paragraph("English").with_alignment(Alignment::Right),
    ]);
    assert!(ps[0].bidi && ps[0].runs.iter().all(|r| r.rtl));
    assert!(!ps[1].bidi);
    let dirs: Vec<(&str, bool)> = ps[1].runs.iter().map(|r| (r.text.as_str(), r.rtl)).collect();
    assert_eq!(dirs, vec![("Latin with ", false), ("עברית ", true), ("inside", false)]);
    // "Right" in a right-to-left paragraph is its leading edge.
    assert!(ps[2].bidi && ps[2].align.as_deref() == Some("left"));
    assert!(!ps[3].bidi && ps[3].align.as_deref() == Some("right"));
}

#[test]
fn hostile_text_is_escaped_and_cleaned() {
    let s = "<w:t>&amp; \"quotes\" 'single' ]]> <!-- --> 👨‍👩‍👧 \u{fffe}\u{ffff}*md* `x`";
    let ps = paragraphs(vec![
        Block::paragraph(s),
        Block::with_text(BlockKind::code(), "</w:t></w:r></w:p> & <script>"),
        Block::new(BlockKind::Paragraph).with_span("x", InlineAttrs { font: Some("A\"<&>'B".into()), ..link("https://x.test/?q=\"<a>\"&r='b'") }),
    ]);
    assert_eq!(ps[0].text(), "<w:t>&amp; \"quotes\" 'single' ]]> <!-- --> 👨‍👩‍👧 *md* `x`");
    assert_eq!(ps[1].text(), "</w:t></w:r></w:p> & <script>");
    assert_eq!(ps[2].runs[0].font.as_deref(), Some("A\"<&>'B"));
    assert_eq!(ps[2].runs[0].link.as_deref(), Some("https://x.test/?q=%22%3Ca%3E%22&r='b'"));
}

#[test]
fn empty_documents() {
    let ps = Package::of(&Document::new()).paragraphs();
    assert_eq!(ps, vec![P::default()]);
    let package = Package::new(&ExportSource::collection("Empty", Vec::<(String, &Document)>::new()), &DocxOptions::default());
    assert_eq!(package.paragraphs().len(), 1);
    assert_eq!(package.sections().len(), 1);
}

// ----- page setup -------------------------------------------------------------------------

#[test]
fn section_properties_carry_the_page_setup() {
    // The default: A4 portrait with one-inch margins.
    let package = Package::of(&Document::from_blocks(vec![Block::paragraph("x")]));
    assert_eq!(package.sections(), vec![(11906, 16838, "portrait".to_string(), 1440, 1440, 1440, 1440, true)]);

    let settings = DocumentSettings {
        page: PageSetup {
            paper: PaperSize::UsLetter,
            orientation: Orientation::Landscape,
            margins: Margins {
                top: 36.0,
                right: 54.0,
                bottom: 90.0,
                left: 108.0,
            },
        },
        ..Default::default()
    };
    let doc = Document::from_blocks(vec![Block::paragraph("x")]).with_settings(settings);
    let o = DocxOptions {
        page_numbers: false,
        ..Default::default()
    };
    let package = Package::new(&ExportSource::single("T", &doc), &o);
    assert_eq!(package.sections(), vec![(15840, 12240, "landscape".to_string(), 720, 1080, 1800, 2160, false)]);

    let custom = DocumentSettings {
        page: PageSetup {
            paper: PaperSize::Custom { width: 360.0, height: 576.0 },
            ..Default::default()
        },
        ..Default::default()
    };
    let doc = Document::from_blocks(vec![Block::paragraph("x")]).with_settings(custom);
    assert_eq!(Package::of(&doc).sections()[0].0, 7200);
    assert_eq!(Package::of(&doc).sections()[0].1, 11520);
}

#[test]
fn page_number_footer() {
    let package = Package::of(&Document::from_blocks(vec![Block::paragraph("x")]));
    let footer = package.part(parts::FOOTER);
    assert!(footer.contains("<w:instrText xml:space=\"preserve\"> PAGE </w:instrText>"));
    assert!(footer.contains("<w:jc w:val=\"center\"/>"));
    let rels = package.rels(parts::DOCUMENT_RELS);
    assert!(rels.values().any(|(t, target, _)| t.ends_with("/footer") && target == "footer1.xml"));
}

// ----- collections ------------------------------------------------------------------------

#[test]
fn collections_are_sections_on_new_pages() {
    let landscape = DocumentSettings {
        page: PageSetup {
            orientation: Orientation::Landscape,
            ..Default::default()
        },
        ..Default::default()
    };
    let one = Document::from_blocks(vec![Block::with_text(BlockKind::heading(1), "Chapter One"), Block::with_text(BlockKind::NumberedItem, "a")]);
    let two = Document::from_blocks(vec![Block::paragraph("Second body"), Block::with_text(BlockKind::NumberedItem, "b")]).with_settings(landscape);
    let three = Document::new();
    let source = ExportSource::collection("The Book", [("Chapter One", &one), ("Chapter Two", &two), ("", &three)]);
    let package = Package::new(&source, &DocxOptions::default());
    let ps = package.paragraphs();
    let texts: Vec<String> = ps.iter().map(P::text).collect();
    // The second and third documents do not open with their titles, so they get them.
    assert_eq!(texts, vec!["Chapter One", "a", "Chapter Two", "Second body", "b", "Untitled", ""]);
    assert_eq!(ps[2].style, "Heading1");
    assert_eq!(ps[5].style, "Heading1");
    // A section ends in the last paragraph of each document but the last.
    let ends: Vec<bool> = ps.iter().map(|p| p.section).collect();
    assert_eq!(ends, vec![false, true, false, false, true, false, false]);
    let sections = package.sections();
    assert_eq!(sections.len(), 3);
    assert_eq!((sections[0].0, sections[0].2.as_str()), (11906, "portrait"));
    assert_eq!((sections[1].0, sections[1].2.as_str()), (16838, "landscape"));
    assert!(package.part(parts::DOCUMENT).matches("<w:type w:val=\"nextPage\"/>").count() == 3);
    // Numbering starts again in each document.
    assert_eq!(ps[1].num, Some((2, 0)));
    assert_eq!(ps[4].num, Some((3, 0)));
    // The title of the whole is the document title.
    assert!(package.part(parts::CORE).contains("<dc:title>The Book</dc:title>"));

    let o = DocxOptions {
        part_titles: PartTitles::Never,
        ..Default::default()
    };
    let texts: Vec<String> = Package::new(&source, &o).paragraphs().iter().map(P::text).collect();
    assert_eq!(texts, vec!["Chapter One", "a", "Second body", "b", ""]);
}

// ----- comments ---------------------------------------------------------------------------

fn commented() -> Document {
    let mut ed = EditorState::new(Document::from_blocks(vec![
        Block::paragraph("The quick brown fox."),
        Block::new(BlockKind::Divider),
        Block::paragraph("Second ").with_span("linked", link("https://example.com")).with_span(" paragraph", plain()),
        Block::with_text(BlockKind::code(), "let x = 1;"),
        Block::paragraph("Deleted soon"),
    ]));
    // A thread inside one paragraph, with a reply, resolved.
    ed.set_selection(Selection::new(Position::new(0, 4), Position::new(0, 9)));
    let first = ed.add_comment("Is it <really> quick?\nSecond & line.", "Ada Lovelace", 1_794_391_505_000).unwrap();
    ed.add_reply(first, "Yes.", "Ben", 1_794_391_506_000).unwrap();
    ed.resolve_comment(first);
    // A thread across blocks, starting and ending inside a link's paragraph.
    ed.set_selection(Selection::new(Position::new(0, 16), Position::new(2, 10)));
    ed.add_comment("Spans blocks", "", 5).unwrap();
    // A thread in a code block.
    ed.set_selection(Selection::new(Position::new(3, 4), Position::new(3, 5)));
    ed.add_comment("Rename", "Ada Lovelace", 6).unwrap();
    // An orphaned thread: its text is deleted.
    ed.set_selection(Selection::new(Position::new(4, 0), Position::new(4, 7)));
    let orphan = ed.add_comment("Orphan", "Ada", 7).unwrap();
    ed.delete_selection();
    assert!(ed.document().is_orphaned(orphan));
    ed.into_document()
}

#[test]
fn comments_are_word_comments_with_replies() {
    let doc = commented();
    let package = Package::of(&doc);
    assert!(package.has(parts::COMMENTS) && package.has(parts::COMMENTS_EXTENDED));
    assert!(package.part(parts::CONTENT_TYPES).contains("comments+xml") && package.part(parts::CONTENT_TYPES).contains("commentsExtended+xml"));

    let comments = roxmltree::Document::parse(package.part(parts::COMMENTS)).unwrap();
    let list: Vec<Node> = comments.descendants().filter(|n| n.has_tag_name((W, "comment"))).collect();
    // Three anchored threads, one with a reply; the orphan is left out.
    assert_eq!(list.len(), 4);
    let attr = |n: Node, a: &str| n.attribute((W, a)).unwrap().to_string();
    let body = |n: Node| -> String {
        n.descendants()
            .filter(|d| d.has_tag_name((W, "t")) || d.has_tag_name((W, "br")))
            .map(|d| if d.has_tag_name((W, "br")) { "\n" } else { d.text().unwrap_or("") })
            .collect()
    };
    assert_eq!(attr(list[0], "author"), "Ada Lovelace");
    assert_eq!(attr(list[0], "initials"), "AL");
    assert_eq!(attr(list[0], "date"), "2026-11-11T10:05:05Z");
    assert_eq!(body(list[0]), "Is it <really> quick?\nSecond & line.");
    assert_eq!((attr(list[1], "author"), body(list[1])), ("Ben".to_string(), "Yes.".to_string()));
    assert_eq!(attr(list[1], "date"), "2026-11-11T10:05:06Z");
    assert_eq!((attr(list[2], "author"), body(list[2])), ("Author".to_string(), "Spans blocks".to_string()));
    assert_eq!(body(list[3]), "Rename");
    assert!(list.iter().all(|c| c.descendants().any(|d| d.has_tag_name((W, "annotationRef")))));

    // Threading and resolved state.
    let extended = roxmltree::Document::parse(package.part(parts::COMMENTS_EXTENDED)).unwrap();
    let ex: Vec<(Option<&str>, &str)> = extended
        .descendants()
        .filter(|n| n.has_tag_name((W15, "commentEx")))
        .map(|n| (n.attribute((W15, "paraIdParent")), n.attribute((W15, "done")).unwrap()))
        .collect();
    let para_id = |n: Node| n.descendants().find_map(|d| d.attribute((W14, "paraId"))).unwrap().to_string();
    assert_eq!(ex[0], (None, "1"));
    assert_eq!(ex[1], (Some(para_id(list[0]).as_str()), "1"));
    assert_eq!(ex[2], (None, "0"));
    assert_eq!(ex[3], (None, "0"));

    // The anchors sit exactly around the commented text.
    let xml = package.part(parts::DOCUMENT);
    assert!(xml.contains(
        "<w:r><w:t xml:space=\"preserve\">The </w:t></w:r><w:commentRangeStart w:id=\"0\"/><w:commentRangeStart w:id=\"1\"/>\
         <w:r><w:t xml:space=\"preserve\">quick</w:t></w:r><w:commentRangeEnd w:id=\"0\"/>"
    ));
    assert!(xml.contains("<w:commentRangeStart w:id=\"3\"/><w:r><w:t xml:space=\"preserve\">x</w:t></w:r><w:commentRangeEnd w:id=\"3\"/>"));
    // The text is unchanged by the anchors.
    let texts: Vec<String> = package.paragraphs().iter().map(P::text).collect();
    assert_eq!(texts, vec!["The quick brown fox.", "", "Second linked paragraph", "let x = 1;", " soon"]);
    // The thread across blocks starts in the first paragraph and ends inside the third, in the link.
    let s = xml.find("<w:commentRangeStart w:id=\"2\"/>").unwrap();
    let e = xml.find("<w:commentRangeEnd w:id=\"2\"/>").unwrap();
    assert!(xml[s..e].contains("</w:p><w:p>") && xml[s..e].contains("<w:hyperlink "));
}

#[test]
fn comments_can_be_left_out() {
    let doc = commented();
    let o = DocxOptions {
        comments: false,
        ..Default::default()
    };
    let package = Package::new(&ExportSource::single("T", &doc), &o);
    assert!(!package.has(parts::COMMENTS) && !package.has(parts::COMMENTS_EXTENDED));
    assert!(!package.part(parts::DOCUMENT).contains("comment"));
    assert!(!package.part(parts::CONTENT_TYPES).contains("comments"));
    assert!(!package.part(parts::DOCUMENT_RELS).contains("comments"));
    // A document without comments has no comment parts either.
    assert!(!Package::of(&Document::from_blocks(vec![Block::paragraph("x")])).has(parts::COMMENTS));
}

// ----- random documents -------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

const PIECES: &[&str] = &[
    "word", " ", " two words ", "\n", "\t", "<", ">", "&", "\"", "'", "]]>", "<!--", "&amp;", "&#0;", "</w:t>", "😀", "👨‍👩‍👧",
    "שלום", "مرحبا", "日本語", "é", "\u{a0}", "\u{200b}", "\u{202e}", "\u{feff}", "*", "`", "1.", "☐",
];

#[test]
fn random_documents_are_valid_packages_and_keep_their_text() {
    let seeds: u64 = std::env::var("WRITER_EXPORT_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(400);
    for seed in 1..=seeds.min(5000) {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut blocks = Vec::new();
        for _ in 0..(1 + rng.below(10)) {
            let kind = match rng.below(12) {
                0..=3 => BlockKind::Paragraph,
                4 => BlockKind::heading(1 + rng.below(6) as u8),
                5 => BlockKind::BulletItem,
                6 => BlockKind::NumberedItem,
                7 => BlockKind::TodoItem { checked: rng.chance(50) },
                8 => BlockKind::Quote,
                9 => BlockKind::code(),
                10 => BlockKind::Divider,
                _ => BlockKind::PageBreak,
            };
            let mut block = Block::new(kind).with_indent(rng.below(9) as u8);
            for _ in 0..rng.below(5) {
                let text: String = (0..1 + rng.below(4)).map(|_| PIECES[rng.below(PIECES.len())]).collect();
                let mut a = plain();
                a.bold = rng.chance(30);
                a.italic = rng.chance(30);
                a.code = rng.chance(15);
                a.underline = rng.chance(15);
                if rng.chance(20) {
                    a.link = Some(["https://example.com", "a b<c>", "#x", "mailto:a@b.c", "%zz \"q\""][rng.below(5)].to_string());
                }
                if rng.chance(15) {
                    a.highlight = Some(Highlight::ALL[rng.below(6)]);
                }
                if rng.chance(15) {
                    a.font = Some("F<o>\"n\"&t".to_string());
                }
                block = block.with_span(&text, a);
            }
            if rng.chance(20) {
                block = block.with_text_size(Some(8.0 + rng.below(40) as f32));
            }
            blocks.push(block);
        }
        let mut ed = EditorState::new(Document::from_blocks(blocks));
        for _ in 0..rng.below(4) {
            let count = ed.document().block_count();
            let pick = |rng: &mut Rng, ed: &EditorState| {
                let b = rng.below(count);
                let len = ed.document().blocks()[b].len();
                ed.document().clamp_position(Position::new(b, rng.below(len + 1)))
            };
            let (a, b) = (pick(&mut rng, &ed), pick(&mut rng, &ed));
            ed.set_selection(Selection::new(a, b));
            if let Some(id) = ed.add_comment("body <&> \u{1}", "A & \"B\"", seed as i64) {
                if rng.chance(50) {
                    ed.add_reply(id, "reply\nline", "", 2);
                }
                if rng.chance(30) {
                    ed.resolve_comment(id);
                }
            }
        }
        let doc = ed.into_document();
        let package = Package::new(&ExportSource::single("T <&>", &doc), &DocxOptions::default());
        // The text of every block comes back from the runs.
        let ps = package.paragraphs();
        let mut ps = ps.iter().peekable();
        let mut pending_break = false;
        for block in doc.blocks() {
            if matches!(block.kind(), BlockKind::PageBreak) {
                if pending_break {
                    assert!(ps.next().unwrap().runs[0].page_break, "seed {seed}");
                }
                pending_break = true;
                continue;
            }
            let p = ps.next().unwrap_or_else(|| panic!("seed {seed}: a paragraph is missing"));
            let mut text = p.text();
            assert_eq!(p.runs.first().is_some_and(|r| r.page_break), pending_break, "seed {seed}");
            pending_break = false;
            if let BlockKind::TodoItem { checked } = block.kind() {
                let mark = if *checked { "\u{2611}\t" } else { "\u{2610}\t" };
                text = text.strip_prefix(mark).unwrap_or_else(|| panic!("seed {seed}: no ballot box")).to_string();
            }
            assert_eq!(text, block.text(), "seed {seed}");
        }
        if pending_break {
            assert!(ps.next().unwrap().runs[0].page_break, "seed {seed}");
        }
        assert!(ps.next().is_none(), "seed {seed}: extra paragraphs");
    }
}
