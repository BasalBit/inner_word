//! DOCX export tests: the package is unzipped with an independent ZIP
//! reader, every part is parsed with an XML parser, and the required parts,
//! relationships and document structure are checked.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

use roxmltree::Node;
use writer_core::{
    Alignment, Block, BlockKind, Document, DocumentSettings, EditorState, Highlight, InlineAttrs,
    InlineStyle, Margins, Orientation, PageSetup, PaperSize, Position, Rgb, Selection,
    SpellingLanguage, Typography,
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
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("a valid ZIP archive");
    let mut parts = BTreeMap::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).expect("a readable entry");
        let mut content = String::new();
        file.read_to_string(&mut content)
            .expect("UTF-8 content with a valid CRC");
        assert!(
            parts.insert(file.name().to_string(), content).is_none(),
            "duplicate part {}",
            file.name()
        );
    }
    parts
}

struct Package {
    parts: BTreeMap<String, String>,
}

impl Package {
    fn new(source: &ExportSource<'_>, options: &DocxOptions) -> Package {
        let bytes = to_docx(source, options);
        let package = Package {
            parts: unzip(&bytes),
        };
        package.check();
        package
    }

    fn of(doc: &Document) -> Package {
        Package::new(&ExportSource::single("Test", doc), &DocxOptions::default())
    }

    fn part(&self, name: &str) -> &str {
        self.parts
            .get(name)
            .unwrap_or_else(|| panic!("missing part {name}"))
    }

    fn has(&self, name: &str) -> bool {
        self.parts.contains_key(name)
    }

    /// `Id -> (Type, Target, external)` of a relationships part.
    fn rels(&self, name: &str) -> BTreeMap<String, (String, String, bool)> {
        let doc = roxmltree::Document::parse(self.part(name)).unwrap();
        doc.root_element()
            .children()
            .filter(|n| {
                n.has_tag_name((
                    "http://schemas.openxmlformats.org/package/2006/relationships",
                    "Relationship",
                ))
            })
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
            assert!(
                xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\""),
                "{name} has no declaration"
            );
            roxmltree::Document::parse(xml)
                .unwrap_or_else(|e| panic!("{name} is not well-formed: {e}\n{xml}"));
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
            .filter(|n| {
                n.has_tag_name((
                    "http://schemas.openxmlformats.org/package/2006/content-types",
                    "Override",
                ))
            })
            .map(|n| {
                n.attribute("PartName")
                    .unwrap()
                    .trim_start_matches('/')
                    .to_string()
            })
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
            assert!(
                !external && self.has(&target),
                "root relationship to missing {target}"
            );
        }
        let rels = self.rels(parts::DOCUMENT_RELS);
        for (_, target, external) in rels.values() {
            if !external {
                assert!(
                    self.has(&format!("word/{target}")),
                    "relationship to missing word/{target}"
                );
            }
        }
        let kinds: BTreeSet<&str> = rels
            .values()
            .map(|(t, _, _)| t.rsplit('/').next().unwrap())
            .collect();
        for kind in ["styles", "numbering", "settings"] {
            assert!(kinds.contains(kind), "no {kind} relationship");
        }
        // Every r:id, style, numbering instance and comment the document
        // refers to exists.
        let document = roxmltree::Document::parse(self.part(parts::DOCUMENT)).unwrap();
        let styles = roxmltree::Document::parse(self.part(parts::STYLES)).unwrap();
        let style_ids: BTreeSet<&str> = styles
            .descendants()
            .filter(|n| n.has_tag_name((W, "style")))
            .filter_map(|n| n.attribute((W, "styleId")))
            .collect();
        let numbering = roxmltree::Document::parse(self.part(parts::NUMBERING)).unwrap();
        let num_ids: BTreeSet<&str> = numbering
            .descendants()
            .filter(|n| n.has_tag_name((W, "num")))
            .filter_map(|n| n.attribute((W, "numId")))
            .collect();
        let abstract_ids: BTreeSet<&str> = numbering
            .descendants()
            .filter(|n| n.has_tag_name((W, "abstractNum")))
            .filter_map(|n| n.attribute((W, "abstractNumId")))
            .collect();
        for n in numbering
            .descendants()
            .filter(|n| n.has_tag_name((W, "abstractNumId")))
        {
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
        for n in styles
            .descendants()
            .filter(|n| n.has_tag_name((W, "basedOn")) || n.has_tag_name((W, "next")))
        {
            assert!(style_ids.contains(n.attribute((W, "val")).unwrap()));
        }
        // Comments: each one has exactly one start, end and reference, in
        // that order, and an entry in both comment parts.
        let comment_ids: Vec<String> = if self.has(parts::COMMENTS) {
            let comments = roxmltree::Document::parse(self.part(parts::COMMENTS)).unwrap();
            let ids: Vec<String> = comments
                .descendants()
                .filter(|n| n.has_tag_name((W, "comment")))
                .map(|n| n.attribute((W, "id")).unwrap().to_string())
                .collect();
            let para_ids: BTreeSet<String> = comments
                .descendants()
                .filter_map(|n| n.attribute((W14, "paraId")))
                .map(str::to_string)
                .collect();
            assert_eq!(para_ids.len(), ids.len(), "one paragraph id per comment");
            let extended = roxmltree::Document::parse(self.part(parts::COMMENTS_EXTENDED)).unwrap();
            let ex: Vec<Node> = extended
                .descendants()
                .filter(|n| n.has_tag_name((W15, "commentEx")))
                .collect();
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
            let s = xml
                .find(&format!("<w:commentRangeStart w:id=\"{id}\"/>"))
                .unwrap();
            let e = xml
                .find(&format!("<w:commentRangeEnd w:id=\"{id}\"/>"))
                .unwrap();
            assert!(s < e, "comment {id} ends before it starts");
        }
        // The body ends with section properties, and every paragraph and
        // run holds only elements that may be there.
        let body = document
            .descendants()
            .find(|n| n.has_tag_name((W, "body")))
            .unwrap();
        assert!(
            body.last_element_child()
                .unwrap()
                .has_tag_name((W, "sectPr"))
        );
        for p in body.children().filter(Node::is_element) {
            assert!(
                p.has_tag_name((W, "p")) || p.has_tag_name((W, "sectPr")),
                "unexpected {:?} in body",
                p.tag_name()
            );
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
        let body = document
            .descendants()
            .find(|n| n.has_tag_name((W, "body")))
            .unwrap();
        body.children()
            .filter(|n| n.has_tag_name((W, "p")))
            .map(|p| P::read(p, &rels))
            .collect()
    }

    /// The numbering part: per `w:num`, its definition and start
    /// overrides; per `w:abstractNum`, whether it is a bullet list.
    fn numbering(&self) -> Numbering {
        let xml = roxmltree::Document::parse(self.part(parts::NUMBERING)).unwrap();
        let mut out = Numbering::default();
        for n in xml.root_element().children().filter(Node::is_element) {
            if n.has_tag_name((W, "abstractNum")) {
                let id: u32 = n.attribute((W, "abstractNumId")).unwrap().parse().unwrap();
                let bullet = n
                    .children()
                    .filter(|l| l.has_tag_name((W, "lvl")))
                    .all(|l| val(l, "numFmt") == Some("bullet"));
                assert!(
                    out.bullet.insert(id, bullet).is_none(),
                    "duplicate definition"
                );
            } else if n.has_tag_name((W, "num")) {
                let id: u32 = n.attribute((W, "numId")).unwrap().parse().unwrap();
                let definition: u32 = val(n, "abstractNumId").unwrap().parse().unwrap();
                let mut starts = BTreeMap::new();
                for o in n.children().filter(|o| o.has_tag_name((W, "lvlOverride"))) {
                    let level: u32 = o.attribute((W, "ilvl")).unwrap().parse().unwrap();
                    let start: u32 = val(o, "startOverride").unwrap().parse().unwrap();
                    assert!(starts.insert(level, start).is_none());
                }
                assert!(
                    out.nums.insert(id, (definition, starts)).is_none(),
                    "duplicate instance"
                );
            }
        }
        out
    }

    /// The number every paragraph shows in front of its text, as `reader`
    /// counts: `None` for paragraphs without a number. A number written
    /// as text ("12." and a tab in a hanging indent) counts too.
    fn shown_numbers(&self, reader: Reader) -> Vec<Option<u32>> {
        let numbering = self.numbering();
        // Counters per list, where a list is a definition or an instance.
        let mut counters: BTreeMap<u32, [Option<u32>; 9]> = BTreeMap::new();
        let mut used: BTreeSet<(u32, u32)> = BTreeSet::new();
        self.paragraphs()
            .iter()
            .map(|p| {
                let Some((num, level)) = p.num else {
                    let first = p.runs.first().map(|r| r.text.as_str()).unwrap_or("");
                    return first
                        .strip_suffix(".\t")
                        .and_then(|n| n.strip_prefix('\t'))
                        .filter(|_| p.hanging.is_some())
                        .and_then(|n| n.parse().ok());
                };
                let (definition, starts) = &numbering.nums[&num];
                if numbering.bullet[definition] {
                    return None;
                }
                let first_use = used.insert((num, level));
                let (list, start) = match reader {
                    Reader::Word => (*definition, starts.get(&level).copied()),
                    Reader::LibreOffice => (num, starts.get(&level).copied()),
                    Reader::QuickLook => (*definition, None),
                };
                let c = counters.entry(list).or_default();
                let l = level as usize;
                let shown = match (start, c[l]) {
                    // Word restarts at the override where the instance
                    // first uses the level.
                    (Some(start), _) if first_use && reader == Reader::Word => start,
                    (_, Some(n)) => n + 1,
                    (start, None) => start.unwrap_or(1),
                };
                c[l] = Some(shown);
                for deeper in c.iter_mut().skip(l + 1) {
                    *deeper = None;
                }
                Some(shown)
            })
            .collect()
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
                    size.attribute((W, "orient"))
                        .unwrap_or("portrait")
                        .to_string(),
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

/// `word/numbering.xml` as the tests see it.
#[derive(Default)]
struct Numbering {
    /// Definition id to "is a bullet list".
    bullet: BTreeMap<u32, bool>,
    /// Instance id to its definition and its start override per level.
    nums: BTreeMap<u32, (u32, BTreeMap<u32, u32>)>,
}

/// How a reader counts list items. Word keeps one set of counters per
/// definition and restarts a level where an instance with a start override
/// first uses it. LibreOffice keeps every instance apart. Quick Look counts
/// per definition and ignores overrides. In all three an item restarts the
/// levels below it.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Reader {
    Word,
    LibreOffice,
    QuickLook,
}

/// What the document says every block shows: the ordinal of numbered
/// items.
fn ordinals(doc: &Document) -> Vec<Option<u32>> {
    doc.numbered_ordinals(0..doc.block_count())
        .into_iter()
        .map(|n| n.map(|n| n as u32))
        .collect()
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
    /// Starts on a new page (`w:pageBreakBefore`).
    page_break: bool,
    /// `w:spacing w:before` and `w:after` of the paragraph itself.
    before: Option<i32>,
    after: Option<i32>,
    /// `w:spacing w:line` when the rule is "exact".
    line_exact: Option<i32>,
    /// Tab stops: `(alignment, position)`.
    tabs: Vec<(String, i32)>,
    runs: Vec<Run>,
}

impl P {
    fn read(p: Node, rels: &BTreeMap<String, (String, String, bool)>) -> P {
        let mut out = P::default();
        if let Some(ppr) = child(p, "pPr") {
            out.style = val(ppr, "pStyle").unwrap_or("").to_string();
            if let Some(num) = child(ppr, "numPr") {
                out.num = Some((
                    val(num, "numId").unwrap().parse().unwrap(),
                    val(num, "ilvl").unwrap().parse().unwrap(),
                ));
            }
            if let Some(ind) = child(ppr, "ind") {
                out.left = ind.attribute((W, "left")).map(|v| v.parse().unwrap());
                out.hanging = ind.attribute((W, "hanging")).map(|v| v.parse().unwrap());
            }
            out.align = val(ppr, "jc").map(str::to_string);
            out.bidi = child(ppr, "bidi").is_some();
            out.rule = child(ppr, "pBdr").is_some_and(|b| child(b, "bottom").is_some());
            out.section = child(ppr, "sectPr").is_some();
            out.page_break = child(ppr, "pageBreakBefore").is_some();
            if let Some(spacing) = child(ppr, "spacing") {
                out.before = spacing.attribute((W, "before")).map(|v| v.parse().unwrap());
                out.after = spacing.attribute((W, "after")).map(|v| v.parse().unwrap());
                if spacing.attribute((W, "lineRule")) == Some("exact") {
                    out.line_exact = spacing.attribute((W, "line")).map(|v| v.parse().unwrap());
                }
            }
            if let Some(tabs) = child(ppr, "tabs") {
                out.tabs = tabs
                    .children()
                    .filter(Node::is_element)
                    .map(|t| {
                        (
                            t.attribute((W, "val")).unwrap().to_string(),
                            t.attribute((W, "pos")).unwrap().parse().unwrap(),
                        )
                    })
                    .collect();
            }
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
                        assert_eq!(
                            run.bold,
                            child(c, "bCs").is_some(),
                            "bold needs its complex-script twin"
                        );
                        run.italic = child(c, "i").is_some();
                        run.underline = child(c, "u").is_some();
                        run.strike = child(c, "strike").is_some();
                        run.rtl = child(c, "rtl").is_some();
                        run.color = val(c, "color").map(str::to_string);
                        run.font = child(c, "rFonts")
                            .and_then(|f| f.attribute((W, "ascii")))
                            .map(str::to_string);
                        run.size = val(c, "sz").map(|v| v.parse().unwrap());
                        assert_eq!(run.size, val(c, "szCs").map(|v| v.parse().unwrap()));
                        run.shade = child(c, "shd")
                            .and_then(|f| f.attribute((W, "fill")))
                            .map(str::to_string);
                        run.style = val(c, "rStyle").map(str::to_string);
                    }
                    "t" => {
                        let t = c.text().unwrap_or("");
                        if c.attribute(("http://www.w3.org/XML/1998/namespace", "space"))
                            != Some("preserve")
                        {
                            assert_eq!(t, t.trim(), "whitespace needs xml:space");
                        }
                        run.text.push_str(t);
                    }
                    "br" if c.attribute((W, "type")) == Some("page") => {
                        panic!("a page break is a paragraph property, not a run")
                    }
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
    assert_eq!(
        &bytes[30..30 + parts::CONTENT_TYPES.len()],
        parts::CONTENT_TYPES.as_bytes()
    );
    let package = Package {
        parts: unzip(&bytes),
    };
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
    let doc = Document::from_blocks(vec![
        Block::paragraph("Same"),
        Block::with_text(BlockKind::NumberedItem, "x"),
    ]);
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
    let package = Package::new(
        &ExportSource::single("Fish & <Chips>\u{0} \"quoted\"", &doc),
        &o,
    );
    let core = roxmltree::Document::parse(package.part(parts::CORE)).unwrap();
    let get = |name: &str| {
        core.descendants()
            .find(|n| n.tag_name().name() == name)
            .and_then(|n| n.text())
            .unwrap_or("")
            .to_string()
    };
    assert_eq!(get("title"), "Fish & <Chips> \"quoted\"");
    assert_eq!(get("creator"), "Ada <Lovelace> & Co");
    assert_eq!(get("lastModifiedBy"), "Ada <Lovelace> & Co");
    assert_eq!(get("created"), "1970-01-01T00:00:00Z");
    assert_eq!(get("modified"), "2026-11-11T10:05:05Z");
    assert!(
        package
            .part(parts::APP)
            .contains("<Application>The Writer</Application>")
    );
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
    let style = |id: &str| {
        styles
            .descendants()
            .find(|n| n.has_tag_name((W, "style")) && n.attribute((W, "styleId")) == Some(id))
            .unwrap_or_else(|| panic!("no style {id}"))
    };
    let normal = style("Normal");
    assert_eq!(normal.attribute((W, "default")), Some("1"));
    let rpr = child(normal, "rPr").unwrap();
    assert_eq!(
        child(rpr, "rFonts").unwrap().attribute((W, "ascii")),
        Some("Iowan \"Old\" Style")
    );
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
        assert_eq!(
            val(child(h, "rPr").unwrap(), "sz"),
            Some(size.to_string().as_str())
        );
        assert_eq!(
            val(child(h, "pPr").unwrap(), "outlineLvl"),
            Some((level - 1).to_string().as_str())
        );
    }
    assert_eq!(val(style("Heading1"), "name"), Some("heading 1"));
    for id in [
        "Quote",
        "Code",
        "ListParagraph",
        "Hyperlink",
        "Footer",
        "CommentReference",
        "CommentText",
    ] {
        style(id);
    }
    let code = child(style("Code"), "rPr").unwrap();
    assert_eq!(
        child(code, "rFonts").unwrap().attribute((W, "ascii")),
        Some("Menlo")
    );
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
        Block::with_text(
            BlockKind::CodeBlock {
                language: Some("rust".into()),
            },
            "fn main() {\n\tlet x  =  1;\n}\n",
        ),
    ]);
    let styles: Vec<&str> = ps.iter().map(|p| p.style.as_str()).collect();
    assert_eq!(
        styles,
        vec!["Heading1", "Heading6", "", "", "Quote", "Code"]
    );
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
        Block::with_text(BlockKind::NumberedItem, "nested again").with_indent(1),
    ]);
    assert!(ps.iter().all(|p| p.style == "ListParagraph"));
    assert_eq!(ps[0].num, Some((1, 0)));
    assert_eq!(ps[1].num, Some((1, 1)));
    assert_eq!(ps[2].num, Some((1, 8)));
    // One numbering instance for the whole numbered list: readers restart
    // a nested level after an item of the outer one.
    assert_eq!(ps[3].num, Some((2, 0)));
    assert_eq!(ps[4].num, Some((2, 0)));
    assert_eq!(ps[5].num, Some((2, 1)));
    assert_eq!(ps[6].num, Some((2, 1)));
    assert_eq!(ps[7].num, Some((2, 0)));
    assert_eq!(ps[8].num, Some((2, 1)));
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
    let expected = vec![
        Some(1),
        Some(2),
        None,
        Some(1),
        Some(1),
        None,
        Some(1),
        None,
        Some(2),
    ];
    assert_eq!(ordinals(&doc), expected);
    let package = Package::of(&doc);
    let ps = package.paragraphs();
    let nums: Vec<Option<(u32, u32)>> = ps.iter().map(|p| p.num).collect();
    // Instance 1 is the bullets. The second list restarts in instance 3
    // and its nested list rides along; the nested list after the bullet
    // has nothing that restarts it and takes instance 4.
    assert_eq!(
        nums,
        vec![
            Some((2, 0)),
            Some((2, 0)),
            None,
            Some((3, 0)),
            Some((3, 1)),
            Some((1, 1)),
            Some((4, 1)),
            None,
            Some((3, 0))
        ]
    );
    for reader in [Reader::Word, Reader::LibreOffice, Reader::QuickLook] {
        assert_eq!(package.shown_numbers(reader), expected, "{reader:?}");
    }
    let numbering = package.numbering();
    assert_eq!(numbering.nums.len(), 4);
    // The bullet instance has no overrides; every numbered instance
    // restarts all nine levels at 1.
    assert!(numbering.nums[&1].1.is_empty());
    for id in 2..=4 {
        let (_, starts) = &numbering.nums[&id];
        assert_eq!(starts.len(), 9);
        assert!(starts.values().all(|s| *s == 1));
    }
    let xml = roxmltree::Document::parse(package.part(parts::NUMBERING)).unwrap();
    let abstracts: Vec<Node> = xml
        .descendants()
        .filter(|n| n.has_tag_name((W, "abstractNum")))
        .collect();
    assert_eq!(abstracts.len(), 4);
    for (i, a) in abstracts.iter().enumerate() {
        let format = if i == 0 { "bullet" } else { "decimal" };
        let levels: Vec<Node> = a
            .children()
            .filter(|n| n.has_tag_name((W, "lvl")))
            .collect();
        assert_eq!(levels.len(), 9);
        assert!(
            levels
                .iter()
                .all(|l| val(*l, "numFmt") == Some(format) && val(*l, "start") == Some("1"))
        );
        // Deeper levels are indented further.
        let lefts: Vec<i32> = levels
            .iter()
            .map(|l| {
                child(child(*l, "pPr").unwrap(), "ind")
                    .unwrap()
                    .attribute((W, "left"))
                    .unwrap()
                    .parse()
                    .unwrap()
            })
            .collect();
        assert!(lefts.windows(2).all(|w| w[1] > w[0]));
    }
    assert_eq!(
        val(
            abstracts[1]
                .children()
                .find(|n| n.has_tag_name((W, "lvl")))
                .unwrap(),
            "lvlText"
        ),
        Some("%1.")
    );
    // All definitions come before all instances, as the schema requires.
    let xml = package.part(parts::NUMBERING);
    assert!(xml.rfind("<w:abstractNum ").unwrap() < xml.find("<w:num ").unwrap());
    // The paragraph nested in the list is indented one level (24 pt).
    assert_eq!(ps[7].left, Some(480));
}

/// A document of `lists` separate two-item numbered lists.
fn many_lists(lists: usize) -> Document {
    let mut blocks = Vec::new();
    for i in 0..lists {
        blocks.push(Block::paragraph(&format!("List {i}")));
        blocks.push(Block::with_text(BlockKind::NumberedItem, "first"));
        blocks.push(Block::with_text(BlockKind::NumberedItem, "nested").with_indent(1));
        blocks.push(Block::with_text(BlockKind::NumberedItem, "second"));
    }
    Document::from_blocks(blocks)
}

#[test]
fn list_definitions_and_instances_stay_inside_words_limits() {
    // Word addresses 2,046 numbering instances. However many lists a
    // document has, the export writes at most 1,001 instances (one for
    // the bullets) over at most 129 definitions.
    for (lists, instances, definitions) in [
        (1, 2, 2),
        (127, 128, 128),
        (128, 129, 129),
        (129, 130, 129),
        (1000, 1001, 129),
        (2500, 1001, 129),
    ] {
        let doc = many_lists(lists);
        let package = Package::of(&doc);
        package.check();
        let numbering = package.numbering();
        assert_eq!(numbering.nums.len(), instances, "{lists} lists");
        assert_eq!(numbering.bullet.len(), definitions, "{lists} lists");
        // Every reader that honours start overrides shows the document's
        // numbers, also past the limit, where the numbers are text.
        let expected = ordinals(&doc);
        assert_eq!(package.shown_numbers(Reader::Word), expected);
        assert_eq!(package.shown_numbers(Reader::LibreOffice), expected);
        // Quick Look ignores overrides; it is right while every list has
        // a definition of its own.
        assert_eq!(
            package.shown_numbers(Reader::QuickLook) == expected,
            lists <= 128,
            "{lists} lists"
        );
        let ps = package.paragraphs();
        let literal = ps
            .iter()
            .filter(|p| p.num.is_none() && p.style == "ListParagraph")
            .count();
        assert_eq!(literal, lists.saturating_sub(1000) * 3, "{lists} lists");
        if lists > 1000 {
            // A list past the limit: the number is text in the gutter of
            // a hanging indent, at the indent of a real list item.
            let p = &ps[ps.len() - 3..];
            // The number ends at a right tab stop 7 pt before the text,
            // as a real list number does.
            assert_eq!(p[0].text(), "\t1.\tfirst");
            assert_eq!((p[0].left, p[0].hanging), (Some(520), Some(520)));
            assert_eq!(
                p[0].tabs,
                vec![("right".to_string(), 380), ("left".to_string(), 520)]
            );
            assert_eq!(p[0].runs[0].color.as_deref(), Some("6B645B"));
            assert_eq!(p[1].text(), "\t1.\tnested");
            assert_eq!((p[1].left, p[1].hanging), (Some(1000), Some(520)));
            assert_eq!(
                p[1].tabs,
                vec![("right".to_string(), 860), ("left".to_string(), 1000)]
            );
            assert_eq!(p[2].text(), "\t2.\tsecond");
        }
    }
}

#[test]
fn random_lists_show_the_documents_numbers_in_every_reader() {
    let seeds: u64 = std::env::var("WRITER_EXPORT_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(400)
        .min(20_000);
    for seed in 1..=seeds {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let depth = 1 + rng.below(5);
        let len = 1 + rng.below(80);
        let mut indent = 0usize;
        let blocks: Vec<Block> = (0..len)
            .map(|i| {
                // Mostly small steps in depth, as real lists have.
                indent = match rng.below(6) {
                    0 => rng.below(depth),
                    1 => (indent + 1).min(depth - 1),
                    2 => indent.saturating_sub(1),
                    _ => indent,
                };
                let kind = match rng.below(10) {
                    0 => BlockKind::BulletItem,
                    1 => BlockKind::Paragraph,
                    2 if rng.chance(30) => BlockKind::TodoItem { checked: false },
                    3 if rng.chance(20) => BlockKind::heading(2),
                    _ => BlockKind::NumberedItem,
                };
                Block::with_text(kind, &format!("block {i}")).with_indent(indent as u8)
            })
            .collect();
        let doc = Document::from_blocks(blocks);
        let package = Package::of(&doc);
        let expected = ordinals(&doc);
        for reader in [Reader::Word, Reader::LibreOffice, Reader::QuickLook] {
            assert_eq!(
                package.shown_numbers(reader),
                expected,
                "seed {seed}, {reader:?}"
            );
        }
        // Never more instances than numbered items start at 1.
        let restarts = expected.iter().filter(|n| **n == Some(1)).count();
        assert!(
            package.numbering().nums.len() <= restarts + 1,
            "seed {seed}"
        );
    }
}

#[test]
fn numbering_continues_correctly_across_the_parts_of_a_collection() {
    let chapter = || {
        Document::from_blocks(vec![
            Block::with_text(BlockKind::heading(1), "Chapter"),
            Block::with_text(BlockKind::NumberedItem, "a"),
            Block::with_text(BlockKind::NumberedItem, "b").with_indent(1),
            Block::with_text(BlockKind::NumberedItem, "c"),
        ])
    };
    let (one, two) = (chapter(), chapter());
    let source = ExportSource::collection("Book", [("Chapter", &one), ("Chapter", &two)]);
    let package = Package::new(&source, &DocxOptions::default());
    package.check();
    let expected = [None, Some(1), Some(1), Some(2)].repeat(2);
    for reader in [Reader::Word, Reader::LibreOffice, Reader::QuickLook] {
        assert_eq!(package.shown_numbers(reader), expected, "{reader:?}");
    }
    assert_eq!(package.numbering().nums.len(), 3);
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
    // Like list items: 26 pt for the marker, 24 pt more per level.
    assert_eq!((ps[0].left, ps[0].hanging), (Some(520), Some(520)));
    assert_eq!((ps[1].left, ps[1].hanging), (Some(1000), Some(520)));
    // As in the editor: an open box is muted; a finished to-do has an
    // accent box and faint, struck text.
    assert_eq!(ps[0].runs[0].color.as_deref(), Some("6B645B"));
    assert!(!ps[0].runs[1].strike && ps[0].runs[1].color.is_none());
    assert_eq!(ps[1].runs[0].color.as_deref(), Some("B0503A"));
    assert!(!ps[1].runs[0].strike);
    assert!(ps[1].runs[1].strike);
    assert_eq!(ps[1].runs[1].color.as_deref(), Some("A0988D"));
}

/// The spacing, indents and colours of the app's page view (see
/// `crates/writer-editor/src/layout/style.rs`), for the default
/// typography: 17 pt Charter, line height 1.6 (a 27 pt line), 12 pt
/// between paragraphs.
#[test]
fn spacing_and_colours_follow_the_page_view() {
    let doc = Document::from_blocks(vec![
        Block::with_text(BlockKind::BulletItem, "a"),
        Block::with_text(BlockKind::NumberedItem, "b").with_indent(1),
        Block::with_text(BlockKind::TodoItem { checked: false }, "c"),
        Block::paragraph("after the list"),
        Block::with_text(BlockKind::Quote, "q1"),
        Block::with_text(BlockKind::Quote, "q2"),
        Block::new(BlockKind::Divider),
        Block::with_text(BlockKind::code(), "code").with_indent(1),
        Block::with_text(BlockKind::BulletItem, "last block"),
    ]);
    let package = Package::of(&doc);
    package.check();
    let ps = package.paragraphs();
    // Neighbouring list items of any kind keep the style's small gap; the
    // last one is followed by a paragraph's space.
    assert_eq!(ps[0].after, None);
    assert_eq!(ps[1].after, None);
    assert_eq!(ps[2].after, Some(240));
    assert_eq!(ps[8].after, Some(240));
    // Two quotes in a row are half a paragraph's space apart.
    assert_eq!(ps[4].after, Some(120));
    assert_eq!(ps[5].after, None);
    // The divider is a row of 0.6 lines (16 pt) with the rule in its
    // middle: 8 pt above the rule, 8 pt and a paragraph's space below.
    assert!(ps[6].rule);
    assert_eq!(ps[6].line_exact, Some(160));
    assert_eq!(ps[6].after, Some(160 + 240));
    // An indented code block: one level, plus the room its panel needs
    // to stay inside the column (14 pt padding and the border).
    assert_eq!(ps[7].left, Some(480 + 290));

    let styles = roxmltree::Document::parse(package.part(parts::STYLES)).unwrap();
    let style = |id: &str| {
        styles
            .descendants()
            .find(|n| n.has_tag_name((W, "style")) && n.attribute((W, "styleId")) == Some(id))
            .unwrap()
    };
    let spacing = |id: &str, name: &str| -> Option<i32> {
        child(child(style(id), "pPr")?, "spacing")?
            .attribute((W, name))
            .map(|v| v.parse().unwrap())
    };
    // Body: a 27 pt line of 17 pt Charter, whose own line is 1.2202 times
    // its size: 27 / (17 * 1.2202) * 240.
    assert_eq!(spacing("Normal", "line"), Some(312));
    assert_eq!(spacing("Normal", "after"), Some(240));
    // Headings: space above and below in body lines (0.9 and 0.5 of 27 pt
    // for level 1, 0.6 and 0.3 from level 4), and a line of at most 1.3.
    assert_eq!(spacing("Heading1", "before"), Some(480));
    assert_eq!(spacing("Heading1", "after"), Some(280));
    assert_eq!(spacing("Heading1", "line"), Some(255));
    assert_eq!(spacing("Heading2", "before"), Some(480));
    assert_eq!(spacing("Heading2", "after"), Some(180));
    assert_eq!(spacing("Heading3", "before"), Some(400));
    assert_eq!(spacing("Heading3", "after"), Some(160));
    assert_eq!(spacing("Heading6", "before"), Some(320));
    assert_eq!(spacing("Heading6", "after"), Some(160));
    // List items: a third of the paragraph spacing.
    assert_eq!(spacing("ListParagraph", "after"), Some(80));
    // Quote: muted text; a 3 pt bar that starts 1 pt in, text at 20 pt.
    let quote = style("Quote");
    assert_eq!(val(child(quote, "rPr").unwrap(), "color"), Some("6B645B"));
    let quote_ppr = child(quote, "pPr").unwrap();
    let bar = child(child(quote_ppr, "pBdr").unwrap(), "left").unwrap();
    assert_eq!(bar.attribute((W, "sz")), Some("24"));
    assert_eq!(bar.attribute((W, "space")), Some("16"));
    assert_eq!(bar.attribute((W, "color")), Some("CDC5B8"));
    assert_eq!(
        child(quote_ppr, "ind").unwrap().attribute((W, "left")),
        Some("400")
    );
    // Code: a panel in the hover colour with 14 pt padding at the sides
    // and 10 pt above and below, 4 pt clear of its neighbours, kept
    // inside the column; 15 pt Menlo on a 24 pt line.
    let code_ppr = child(style("Code"), "pPr").unwrap();
    let borders = child(code_ppr, "pBdr").unwrap();
    for (side, space) in [
        ("top", "10"),
        ("left", "14"),
        ("bottom", "10"),
        ("right", "14"),
    ] {
        let b = child(borders, side).unwrap();
        assert_eq!(b.attribute((W, "space")), Some(space), "{side}");
        assert_eq!(b.attribute((W, "color")), Some("EEEAE3"), "{side}");
    }
    assert_eq!(
        child(code_ppr, "shd").unwrap().attribute((W, "fill")),
        Some("EEEAE3")
    );
    let ind = child(code_ppr, "ind").unwrap();
    assert_eq!(ind.attribute((W, "left")), Some("290"));
    assert_eq!(ind.attribute((W, "right")), Some("290"));
    assert_eq!(spacing("Code", "before"), Some(80));
    assert_eq!(spacing("Code", "after"), Some(320));
    assert_eq!(spacing("Code", "line"), Some(330));
    // Links are in the accent colour, the page number is small and faint.
    assert_eq!(
        val(child(style("Hyperlink"), "rPr").unwrap(), "color"),
        Some("B0503A")
    );
    let footer = child(style("Footer"), "rPr").unwrap();
    assert_eq!(val(footer, "color"), Some("A0988D"));
    assert_eq!(val(footer, "sz"), Some("22"));

    // List markers: bullets centred in the 26 pt gutter, numbers flush
    // right 7 pt before the text, both muted.
    let numbering = roxmltree::Document::parse(package.part(parts::NUMBERING)).unwrap();
    let levels: Vec<Node> = numbering
        .descendants()
        .filter(|n| n.has_tag_name((W, "lvl")))
        .collect();
    assert_eq!(levels.len(), 18);
    for (i, l) in levels.iter().enumerate() {
        let bullet = i < 9;
        let level = (i % 9) as i32;
        assert_eq!(
            val(*l, "lvlJc"),
            Some(if bullet { "center" } else { "right" })
        );
        let ind = child(child(*l, "pPr").unwrap(), "ind").unwrap();
        assert_eq!(
            ind.attribute((W, "left")),
            Some((520 + 480 * level).to_string().as_str())
        );
        assert_eq!(
            ind.attribute((W, "hanging")),
            Some(if bullet { "260" } else { "140" })
        );
        assert_eq!(val(child(*l, "rPr").unwrap(), "color"), Some("6B645B"));
    }
}

#[test]
fn space_before_and_after_add_up_and_nothing_stands_above_the_first_line() {
    // The editor adds the space after a block and the space before the
    // next one. Word and LibreOffice only do that with this setting.
    let doc = Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), "Title"),
        Block::paragraph("text"),
        Block::with_text(BlockKind::heading(2), "Section"),
        Block::new(BlockKind::PageBreak),
        Block::with_text(BlockKind::heading(2), "After a break"),
    ]);
    let package = Package::of(&doc);
    package.check();
    let settings = roxmltree::Document::parse(package.part(parts::SETTINGS)).unwrap();
    let compat = settings
        .descendants()
        .find(|n| n.has_tag_name((W, "compat")))
        .unwrap();
    assert!(child(compat, "doNotUseHTMLParagraphAutoSpacing").is_some());
    // The heading that opens the document has no space above it, nor has
    // the one after the page break; other headings keep the style's.
    let ps = package.paragraphs();
    assert_eq!(ps[0].before, Some(0));
    assert_eq!(ps[1].before, None);
    assert_eq!(ps[2].before, None);
    assert_eq!((ps[3].page_break, ps[3].before), (true, Some(0)));
    // So does a code block, and the first heading of every part of a
    // collection, including a title written for it.
    let code = Document::from_blocks(vec![Block::with_text(BlockKind::code(), "x")]);
    assert_eq!(Package::of(&code).paragraphs()[0].before, Some(0));
    let plain = Document::from_blocks(vec![Block::paragraph("x")]);
    assert_eq!(Package::of(&plain).paragraphs()[0].before, None);
    let source = ExportSource::collection("Book", [("One", &doc), ("Two", &plain)]);
    let options = DocxOptions {
        part_titles: PartTitles::Always,
        ..Default::default()
    };
    let ps = Package::new(&source, &options).paragraphs();
    let firsts: Vec<(String, Option<i32>)> = ps
        .iter()
        .filter(|p| p.text() == "One" || p.text() == "Two" || p.text() == "Title")
        .map(|p| (p.text(), p.before))
        .collect();
    assert_eq!(
        firsts,
        vec![
            ("One".to_string(), Some(0)),
            ("Title".to_string(), None),
            ("Two".to_string(), Some(0))
        ]
    );
}

#[test]
fn a_bottom_margin_too_small_for_the_page_number_has_none() {
    let with_bottom = |bottom: f32| {
        let settings = DocumentSettings {
            page: PageSetup {
                margins: Margins {
                    top: 72.0,
                    right: 72.0,
                    bottom,
                    left: 72.0,
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let doc = Document::from_blocks(vec![Block::paragraph("x")]).with_settings(settings);
        let package = Package::of(&doc);
        package.check();
        package.sections()[0].7
    };
    assert!(with_bottom(72.0));
    assert!(with_bottom(23.0));
    assert!(!with_bottom(22.0));
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
    // The page break is a property of the next paragraph.
    assert!(ps[2].page_break && ps[2].text() == "two");
    // Two in a row: an empty page.
    assert!(ps[3].page_break && ps[3].text().is_empty());
    assert!(ps[4].page_break && ps[4].style == "Heading1");
    assert!(ps[5].rule);
    // A trailing page break stands alone.
    assert!(ps[6].page_break && ps[6].runs.is_empty());
    // Void blocks only.
    let ps = paragraphs(vec![Block::new(BlockKind::PageBreak)]);
    assert_eq!(ps.len(), 1);
    assert!(ps[0].page_break);
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
        Block::with_text(BlockKind::heading(2), "centred heading")
            .with_alignment(Alignment::Center),
    ]);
    let aligns: Vec<Option<&str>> = ps.iter().map(|p| p.align.as_deref()).collect();
    assert_eq!(
        aligns,
        vec![
            None,
            Some("center"),
            Some("right"),
            Some("both"),
            None,
            None,
            Some("center")
        ]
    );
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
            .with_span(
                "marked",
                InlineAttrs {
                    highlight: Some(Highlight::Yellow),
                    ..plain()
                },
            )
            .with_span(
                "red",
                InlineAttrs {
                    color: Some(Rgb::new(0xcc, 0x33, 0x00)),
                    ..plain()
                },
            )
            .with_span(
                "font",
                InlineAttrs {
                    font: Some("Georgia".into()),
                    ..plain()
                },
            ),
    ]);
    let r = &ps[0].runs;
    assert_eq!(r.len(), 9);
    assert_eq!(
        r[0],
        Run {
            text: "plain ".into(),
            ..Run::default()
        }
    );
    assert!(r[1].bold && !r[1].italic);
    assert!(r[2].italic);
    assert!(r[3].underline);
    assert!(r[4].strike);
    assert_eq!(r[5].font.as_deref(), Some("Menlo"));
    assert_eq!(r[5].shade.as_deref(), Some("EEEAE3"));
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
    let ps = paragraphs(vec![
        Block::new(BlockKind::heading(2))
            .with_span("every\nthing", all)
            .with_text_size(Some(20.0))
            .with_alignment(Alignment::Right)
            .with_indent(1),
    ]);
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
        }]
    );
}

#[test]
fn block_text_size_is_on_every_run() {
    let ps = paragraphs(vec![
        Block::paragraph("big ")
            .with_span("bold", plain().with(InlineStyle::Bold))
            .with_text_size(Some(24.0)),
        Block::paragraph("normal"),
        Block::with_text(BlockKind::TodoItem { checked: false }, "small")
            .with_text_size(Some(10.0)),
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
            .with_span(
                "bold part",
                InlineAttrs {
                    bold: true,
                    ..link("https://example.com/a?b=1&c=2")
                },
            )
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
    let hyper: Vec<_> = rels
        .values()
        .filter(|(t, _, _)| t.ends_with("/hyperlink"))
        .collect();
    assert_eq!(hyper.len(), 2);
    assert!(hyper.iter().all(|(_, _, external)| *external));
    // The two runs of the first link share one hyperlink element.
    assert_eq!(
        package
            .part(parts::DOCUMENT)
            .matches("<w:hyperlink ")
            .count(),
        3
    );
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
    let dirs: Vec<(&str, bool)> = ps[1]
        .runs
        .iter()
        .map(|r| (r.text.as_str(), r.rtl))
        .collect();
    assert_eq!(
        dirs,
        vec![("Latin with ", false), ("עברית ", true), ("inside", false)]
    );
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
        Block::new(BlockKind::Paragraph).with_span(
            "x",
            InlineAttrs {
                font: Some("A\"<&>'B".into()),
                ..link("https://x.test/?q=\"<a>\"&r='b'")
            },
        ),
    ]);
    assert_eq!(
        ps[0].text(),
        "<w:t>&amp; \"quotes\" 'single' ]]> <!-- --> 👨‍👩‍👧 *md* `x`"
    );
    assert_eq!(ps[1].text(), "</w:t></w:r></w:p> & <script>");
    assert_eq!(ps[2].runs[0].font.as_deref(), Some("A\"<&>'B"));
    assert_eq!(
        ps[2].runs[0].link.as_deref(),
        Some("https://x.test/?q=%22%3Ca%3E%22&r='b'")
    );
}

#[test]
fn empty_documents() {
    let ps = Package::of(&Document::new()).paragraphs();
    assert_eq!(ps, vec![P::default()]);
    let package = Package::new(
        &ExportSource::collection("Empty", Vec::<(String, &Document)>::new()),
        &DocxOptions::default(),
    );
    assert_eq!(package.paragraphs().len(), 1);
    assert_eq!(package.sections().len(), 1);
}

// ----- page setup -------------------------------------------------------------------------

#[test]
fn section_properties_carry_the_page_setup() {
    // The default: A4 portrait with one-inch margins.
    let package = Package::of(&Document::from_blocks(vec![Block::paragraph("x")]));
    assert_eq!(
        package.sections(),
        vec![(
            11906,
            16838,
            "portrait".to_string(),
            1440,
            1440,
            1440,
            1440,
            true
        )]
    );

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
    assert_eq!(
        package.sections(),
        vec![(
            15840,
            12240,
            "landscape".to_string(),
            720,
            1080,
            1800,
            2160,
            false
        )]
    );

    let custom = DocumentSettings {
        page: PageSetup {
            paper: PaperSize::Custom {
                width: 360.0,
                height: 576.0,
            },
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
    assert!(
        rels.values()
            .any(|(t, target, _)| t.ends_with("/footer") && target == "footer1.xml")
    );
}

// ----- collections ------------------------------------------------------------------------

/// What a reader resolves for every paragraph of the body: its text, then
/// space before, space after, line and line rule, the font and size of the
/// paragraph mark, and the font and size of every run. Direct formatting
/// wins over the paragraph style, which wins over the style it is based on.
fn resolved_look(package: &Package) -> Vec<(String, Vec<String>)> {
    let styles = roxmltree::Document::parse(package.part(parts::STYLES)).unwrap();
    let document = roxmltree::Document::parse(package.part(parts::DOCUMENT)).unwrap();
    let style = |id: &str| {
        styles
            .descendants()
            .find(|n| n.has_tag_name((W, "style")) && n.attribute((W, "styleId")) == Some(id))
            .unwrap_or_else(|| panic!("no style {id}"))
    };
    // The chain of property holders of a paragraph: itself, its style, the
    // styles that one is based on.
    let chain = |p: Node<'_, '_>| {
        let mut ids = Vec::new();
        let mut id = child(p, "pPr")
            .and_then(|ppr| val(ppr, "pStyle"))
            .unwrap_or("Normal")
            .to_string();
        loop {
            ids.push(id.clone());
            match val(style(&id), "basedOn") {
                Some(base) => id = base.to_string(),
                None => break,
            }
        }
        ids
    };
    let spacing = |p: Node<'_, '_>, ids: &[String], name: &str| {
        child(p, "pPr")
            .into_iter()
            .chain(ids.iter().filter_map(|id| child(style(id), "pPr")))
            .find_map(|ppr| child(ppr, "spacing")?.attribute((W, name)))
            .unwrap_or("-")
            .to_string()
    };
    let run_look = |rpr: Option<Node<'_, '_>>, ids: &[String]| {
        let holders: Vec<Node<'_, '_>> = rpr
            .into_iter()
            .chain(ids.iter().filter_map(|id| child(style(id), "rPr")))
            .collect();
        let font = holders
            .iter()
            .find_map(|h| child(*h, "rFonts")?.attribute((W, "ascii")))
            .unwrap();
        let size = holders.iter().find_map(|h| val(*h, "sz")).unwrap();
        format!("{font} {size}")
    };
    let body = document
        .descendants()
        .find(|n| n.has_tag_name((W, "body")))
        .unwrap();
    body.children()
        .filter(|n| n.has_tag_name((W, "p")))
        .map(|p| {
            let ids = chain(p);
            let mut look: Vec<String> = ["before", "after", "line", "lineRule"]
                .iter()
                .map(|name| format!("{name} {}", spacing(p, &ids, name)))
                .collect();
            look.push(format!(
                "mark {}",
                run_look(child(p, "pPr").and_then(|ppr| child(ppr, "rPr")), &ids)
            ));
            let mut text = String::new();
            for r in p.descendants().filter(|n| n.has_tag_name((W, "r"))) {
                let t: String = r
                    .children()
                    .filter(|n| n.has_tag_name((W, "t")))
                    .filter_map(|n| n.text())
                    .collect();
                // The comment reference mark has a size of its own.
                if child(r, "commentReference").is_some() {
                    continue;
                }
                look.push(format!("{t:?} {}", run_look(child(r, "rPr"), &ids)));
                text.push_str(&t);
            }
            (text, look)
        })
        .collect()
}

#[test]
fn every_document_of_a_collection_keeps_its_own_typography() {
    let every_kind = |name: &str| {
        Document::from_blocks(vec![
            Block::with_text(BlockKind::heading(1), name),
            Block::paragraph("Body text"),
            Block::paragraph("Larger").with_text_size(Some(30.0)),
            Block::with_text(BlockKind::heading(2), "Two"),
            Block::with_text(BlockKind::heading(3), "Three").with_text_size(Some(40.0)),
            Block::with_text(BlockKind::heading(6), "Six"),
            Block::with_text(BlockKind::BulletItem, "bullet"),
            Block::with_text(BlockKind::BulletItem, "bullet").with_indent(1),
            Block::with_text(BlockKind::NumberedItem, "number"),
            Block::with_text(BlockKind::NumberedItem, "number"),
            Block::with_text(BlockKind::todo(), "open"),
            Block::with_text(BlockKind::TodoItem { checked: true }, "done"),
            Block::with_text(BlockKind::Quote, "quote"),
            Block::with_text(BlockKind::Quote, "quote"),
            Block::with_text(BlockKind::code(), "code();\nmore();"),
            Block::new(BlockKind::Divider),
            Block::paragraph(""),
            Block::new(BlockKind::PageBreak),
            Block::new(BlockKind::PageBreak),
            Block::paragraph("See ")
                .with_span("the site", link("https://example.com"))
                .with_span(" and ", plain())
                .with_span(
                    "code",
                    InlineAttrs {
                        code: true,
                        ..plain()
                    },
                )
                .with_span(
                    " in Futura",
                    InlineAttrs {
                        font: Some("Futura".into()),
                        bold: true,
                        ..plain()
                    },
                ),
            Block::paragraph("Centred").with_alignment(Alignment::Center),
        ])
    };
    let own = DocumentSettings {
        typography: Typography {
            body_font: "Georgia".into(),
            base_size: 19.0,
            line_height: 1.7,
            paragraph_spacing: 14.0,
        },
        page: PageSetup {
            paper: PaperSize::UsLetter,
            ..Default::default()
        },
        ..Default::default()
    };
    let one = every_kind("Chapter One");
    let two = every_kind("Chapter Two").with_settings(own.clone());
    let three = every_kind("Chapter Three");
    let empty = Document::new().with_settings(own);
    let o = DocxOptions::default();

    let alone = |title: &str, doc: &Document| {
        resolved_look(&Package::new(&ExportSource::single(title, doc), &o))
    };
    let book = Package::new(
        &ExportSource::collection(
            "Book",
            [
                ("Chapter One", &one),
                ("Chapter Two", &two),
                ("Chapter Three", &three),
            ],
        ),
        &o,
    );
    let looks = resolved_look(&book);
    let (first, second, third) = (
        alone("Chapter One", &one),
        alone("Chapter Two", &two),
        alone("Chapter Three", &three),
    );
    let n = first.len();
    assert_eq!(looks.len(), 3 * n);
    // Every chapter looks in the book as it does exported by itself.
    for (i, (in_book, by_itself)) in looks
        .iter()
        .zip(first.iter().chain(&second).chain(&third))
        .enumerate()
    {
        assert_eq!(in_book, by_itself, "paragraph {i}");
    }
    // The second chapter really is different: 19 pt Georgia.
    assert!(
        second[1]
            .1
            .contains(&"\"Body text\" Georgia 38".to_string())
    );
    assert!(first[1].1.contains(&"\"Body text\" Charter 34".to_string()));
    assert_ne!(first[1].1, second[1].1);
    // Its paper is its own too, as before.
    let widths: Vec<i32> = book.sections().iter().map(|s| s.0).collect();
    assert_eq!(widths, vec![11906, 12240, 11906]);
    // Its headings are still the built-in heading styles.
    let ps = book.paragraphs();
    assert_eq!(ps[n].style, "Heading1");
    assert_eq!(ps[n + 3].style, "Heading2");

    // Documents with the typography of the styles carry nothing of their
    // own, whatever their place.
    let document = book.part(parts::DOCUMENT);
    // (Every paragraph of the second chapter but the divider, whose height
    // is exact.)
    assert_eq!(document.matches("w:lineRule=\"auto\"").count(), n - 1);
    let uniform = Package::new(
        &ExportSource::collection("Book", [("Chapter One", &one), ("Chapter Three", &three)]),
        &o,
    );
    assert!(
        !uniform
            .part(parts::DOCUMENT)
            .contains("w:lineRule=\"auto\"")
    );
    assert!(!uniform.part(parts::DOCUMENT).contains("Georgia"));

    // A title written above a chapter, and the one paragraph of an empty
    // document, follow that document's typography as well.
    let titled = Document::from_blocks(vec![Block::paragraph("No heading here")])
        .with_settings(two.settings().clone());
    let book = Package::new(
        &ExportSource::collection(
            "Book",
            [
                ("Chapter One", &one),
                ("Second", &titled),
                ("Third", &empty),
            ],
        ),
        &o,
    );
    let looks = resolved_look(&book);
    assert_eq!(looks.len(), n + 4);
    assert_eq!(looks[n].0, "Second");
    assert!(looks[n].1.contains(&"\"Second\" Georgia 76".to_string()));
    assert_eq!(
        looks[n + 1].1,
        second[1].1[..5]
            .iter()
            .cloned()
            .chain(["\"No heading here\" Georgia 38".to_string()])
            .collect::<Vec<_>>()
    );
    assert_eq!(looks[n + 2].0, "Third");
    assert_eq!(looks[n + 3].1[..5], second[1].1[..5]);
}

#[test]
fn collections_are_sections_on_new_pages() {
    let landscape = DocumentSettings {
        page: PageSetup {
            orientation: Orientation::Landscape,
            ..Default::default()
        },
        ..Default::default()
    };
    let one = Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), "Chapter One"),
        Block::with_text(BlockKind::NumberedItem, "a"),
    ]);
    let two = Document::from_blocks(vec![
        Block::paragraph("Second body"),
        Block::with_text(BlockKind::NumberedItem, "b"),
    ])
    .with_settings(landscape);
    let three = Document::new();
    let source = ExportSource::collection(
        "The Book",
        [("Chapter One", &one), ("Chapter Two", &two), ("", &three)],
    );
    let package = Package::new(&source, &DocxOptions::default());
    let ps = package.paragraphs();
    let texts: Vec<String> = ps.iter().map(P::text).collect();
    // The second and third documents do not open with their titles, so they get them.
    assert_eq!(
        texts,
        vec![
            "Chapter One",
            "a",
            "Chapter Two",
            "Second body",
            "b",
            "Untitled",
            ""
        ]
    );
    assert_eq!(ps[2].style, "Heading1");
    assert_eq!(ps[5].style, "Heading1");
    // A section ends in the last paragraph of each document but the last.
    let ends: Vec<bool> = ps.iter().map(|p| p.section).collect();
    assert_eq!(ends, vec![false, true, false, false, true, false, false]);
    let sections = package.sections();
    assert_eq!(sections.len(), 3);
    assert_eq!((sections[0].0, sections[0].2.as_str()), (11906, "portrait"));
    assert_eq!(
        (sections[1].0, sections[1].2.as_str()),
        (16838, "landscape")
    );
    assert!(
        package
            .part(parts::DOCUMENT)
            .matches("<w:type w:val=\"nextPage\"/>")
            .count()
            == 3
    );
    // Numbering starts again in each document.
    assert_eq!(ps[1].num, Some((2, 0)));
    assert_eq!(ps[4].num, Some((3, 0)));
    // The title of the whole is the document title.
    assert!(
        package
            .part(parts::CORE)
            .contains("<dc:title>The Book</dc:title>")
    );

    let o = DocxOptions {
        part_titles: PartTitles::Never,
        ..Default::default()
    };
    let texts: Vec<String> = Package::new(&source, &o)
        .paragraphs()
        .iter()
        .map(P::text)
        .collect();
    assert_eq!(texts, vec!["Chapter One", "a", "Second body", "b", ""]);
}

// ----- comments ---------------------------------------------------------------------------

fn commented() -> Document {
    let mut ed = EditorState::new(Document::from_blocks(vec![
        Block::paragraph("The quick brown fox."),
        Block::new(BlockKind::Divider),
        Block::paragraph("Second ")
            .with_span("linked", link("https://example.com"))
            .with_span(" paragraph", plain()),
        Block::with_text(BlockKind::code(), "let x = 1;"),
        Block::paragraph("Deleted soon"),
    ]));
    // A thread inside one paragraph, with a reply, resolved.
    ed.set_selection(Selection::new(Position::new(0, 4), Position::new(0, 9)));
    let first = ed
        .add_comment(
            "Is it <really> quick?\nSecond & line.",
            "Ada Lovelace",
            1_794_391_505_000,
        )
        .unwrap();
    ed.add_reply(first, "Yes.", "Ben", 1_794_391_506_000)
        .unwrap();
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
    assert!(
        package.part(parts::CONTENT_TYPES).contains("comments+xml")
            && package
                .part(parts::CONTENT_TYPES)
                .contains("commentsExtended+xml")
    );

    let comments = roxmltree::Document::parse(package.part(parts::COMMENTS)).unwrap();
    let list: Vec<Node> = comments
        .descendants()
        .filter(|n| n.has_tag_name((W, "comment")))
        .collect();
    // Three anchored threads, one with a reply; the orphan is left out.
    assert_eq!(list.len(), 4);
    let attr = |n: Node, a: &str| n.attribute((W, a)).unwrap().to_string();
    let body = |n: Node| -> String {
        n.descendants()
            .filter(|d| d.has_tag_name((W, "t")) || d.has_tag_name((W, "br")))
            .map(|d| {
                if d.has_tag_name((W, "br")) {
                    "\n"
                } else {
                    d.text().unwrap_or("")
                }
            })
            .collect()
    };
    assert_eq!(attr(list[0], "author"), "Ada Lovelace");
    assert_eq!(attr(list[0], "initials"), "AL");
    assert_eq!(attr(list[0], "date"), "2026-11-11T10:05:05Z");
    assert_eq!(body(list[0]), "Is it <really> quick?\nSecond & line.");
    assert_eq!(
        (attr(list[1], "author"), body(list[1])),
        ("Ben".to_string(), "Yes.".to_string())
    );
    assert_eq!(attr(list[1], "date"), "2026-11-11T10:05:06Z");
    assert_eq!(
        (attr(list[2], "author"), body(list[2])),
        ("Author".to_string(), "Spans blocks".to_string())
    );
    assert_eq!(body(list[3]), "Rename");
    assert!(list.iter().all(|c| {
        c.descendants()
            .any(|d| d.has_tag_name((W, "annotationRef")))
    }));

    // Threading and resolved state.
    let extended = roxmltree::Document::parse(package.part(parts::COMMENTS_EXTENDED)).unwrap();
    let ex: Vec<(Option<&str>, &str)> = extended
        .descendants()
        .filter(|n| n.has_tag_name((W15, "commentEx")))
        .map(|n| {
            (
                n.attribute((W15, "paraIdParent")),
                n.attribute((W15, "done")).unwrap(),
            )
        })
        .collect();
    let para_id = |n: Node| {
        n.descendants()
            .find_map(|d| d.attribute((W14, "paraId")))
            .unwrap()
            .to_string()
    };
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
    assert_eq!(
        texts,
        vec![
            "The quick brown fox.",
            "",
            "Second linked paragraph",
            "let x = 1;",
            " soon"
        ]
    );
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
    "word",
    " ",
    " two words ",
    "\n",
    "\t",
    "<",
    ">",
    "&",
    "\"",
    "'",
    "]]>",
    "<!--",
    "&amp;",
    "&#0;",
    "</w:t>",
    "😀",
    "👨‍👩‍👧",
    "שלום",
    "مرحبا",
    "日本語",
    "é",
    "\u{a0}",
    "\u{200b}",
    "\u{202e}",
    "\u{feff}",
    "*",
    "`",
    "1.",
    "☐",
];

#[test]
fn random_documents_are_valid_packages_and_keep_their_text() {
    let seeds: u64 = std::env::var("WRITER_EXPORT_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(400);
    for seed in 1..=seeds.min(5000) {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut blocks = Vec::new();
        for _ in 0..(1 + rng.below(10)) {
            let kind = match rng.below(12) {
                0..=3 => BlockKind::Paragraph,
                4 => BlockKind::heading(1 + rng.below(6) as u8),
                5 => BlockKind::BulletItem,
                6 => BlockKind::NumberedItem,
                7 => BlockKind::TodoItem {
                    checked: rng.chance(50),
                },
                8 => BlockKind::Quote,
                9 => BlockKind::code(),
                10 => BlockKind::Divider,
                _ => BlockKind::PageBreak,
            };
            let mut block = Block::new(kind).with_indent(rng.below(9) as u8);
            for _ in 0..rng.below(5) {
                let text: String = (0..1 + rng.below(4))
                    .map(|_| PIECES[rng.below(PIECES.len())])
                    .collect();
                let mut a = plain();
                a.bold = rng.chance(30);
                a.italic = rng.chance(30);
                a.code = rng.chance(15);
                a.underline = rng.chance(15);
                if rng.chance(20) {
                    a.link = Some(
                        [
                            "https://example.com",
                            "a b<c>",
                            "#x",
                            "mailto:a@b.c",
                            "%zz \"q\"",
                        ][rng.below(5)]
                        .to_string(),
                    );
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
                ed.document()
                    .clamp_position(Position::new(b, rng.below(len + 1)))
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
        let package = Package::new(
            &ExportSource::single("T <&>", &doc),
            &DocxOptions::default(),
        );
        // The text of every block comes back from the runs.
        let ps = package.paragraphs();
        let mut ps = ps.iter().peekable();
        let mut pending_break = false;
        for block in doc.blocks() {
            if matches!(block.kind(), BlockKind::PageBreak) {
                if pending_break {
                    assert!(ps.next().unwrap().page_break, "seed {seed}");
                }
                pending_break = true;
                continue;
            }
            let p = ps
                .next()
                .unwrap_or_else(|| panic!("seed {seed}: a paragraph is missing"));
            let mut text = p.text();
            assert_eq!(p.page_break, pending_break, "seed {seed}");
            pending_break = false;
            if let BlockKind::TodoItem { checked } = block.kind() {
                let mark = if *checked { "\u{2611}\t" } else { "\u{2610}\t" };
                text = text
                    .strip_prefix(mark)
                    .unwrap_or_else(|| panic!("seed {seed}: no ballot box"))
                    .to_string();
            }
            assert_eq!(text, block.text(), "seed {seed}");
        }
        if pending_break {
            assert!(ps.next().unwrap().page_break, "seed {seed}");
        }
        assert!(ps.next().is_none(), "seed {seed}: extra paragraphs");
    }
}

// ----- second, independent generator (review pass) ----------------------------------------

const ADV_PIECES: &[&str] = &[
    "word",
    " ",
    "  ",
    " two words ",
    "\n",
    "\n\n",
    "\t",
    "<",
    ">",
    "&",
    "\"",
    "'",
    "]]>",
    "<!--",
    "-->",
    "&amp;",
    "&#0;",
    "&#x1;",
    "</w:t>",
    "</w:p>",
    "<w:br/>",
    "<?xml?>",
    "😀",
    "👨‍👩‍👧",
    "🇳🇱",
    "שלום",
    "مرحبا",
    "日本語",
    "é",
    "e\u{301}",
    "\u{a0}",
    "\u{200b}",
    "\u{200d}",
    "\u{202e}",
    "\u{feff}",
    "\u{2028}",
    "\u{2029}",
    "\u{85}",
    "\u{7f}",
    "\u{9f}",
    "\u{fffd}",
    "\u{fffc}",
    "\u{e000}",
    "\u{10ffff}",
    "\u{1f}",
    "\u{b}",
    "\u{c}",
    "\u{0}",
    "\u{1}",
    "\u{8}",
    "\u{fffe}",
    "\u{ffff}",
    "*",
    "`",
    "1.",
    "☐",
    "☑",
    "123",
    "׳",
    "\u{5d0}1\u{5d1}",
    "a\u{5d0}b",
    ".",
    "!",
    "-",
];

const ADV_LINKS: &[&str] = &[
    "https://example.com",
    "https://example.com/a b",
    "a b<c>",
    "#x",
    "mailto:a@b.c",
    "%zz \"q\"",
    "%",
    "http://",
    "http://[::1]/",
    "http://[bad",
    "]",
    "[",
    "::",
    "http://exa mple.com/ü?q=ö&x=<>",
    "\\\\server\\share",
    "file:///C:/a b.txt",
    "tel:+1 555",
    "javascript:alert('x')",
    "https://日本.jp/パス",
    "https://x.y/😀",
    "x\u{0}y",
    "x\u{1}\u{fffe}y",
    "  padded  ",
    "a\nb",
    "a\tb",
    "?",
    "#",
    "##",
    "a#b#c",
    "{x}",
    "a|b",
    "a^b",
    "http://user:pw@host:99999/",
    "http://x.y/%",
    "http://x.y/%2",
    "http://x.y/%2G",
    "//x.y/z",
    "/",
];

fn adv_document(rng: &mut Rng) -> Document {
    let mut blocks = Vec::new();
    for _ in 0..rng.below(14) {
        let kind = match rng.below(14) {
            0..=3 => BlockKind::Paragraph,
            4 => BlockKind::heading(1 + rng.below(6) as u8),
            5 | 6 => BlockKind::BulletItem,
            7 | 8 => BlockKind::NumberedItem,
            9 => BlockKind::TodoItem {
                checked: rng.chance(50),
            },
            10 => BlockKind::Quote,
            11 => BlockKind::CodeBlock {
                language: rng.chance(50).then(|| "ru\"st<&>".to_string()),
            },
            12 => BlockKind::Divider,
            _ => BlockKind::PageBreak,
        };
        let mut block = Block::new(kind).with_indent(rng.below(9) as u8);
        for _ in 0..rng.below(6) {
            let text: String = (0..1 + rng.below(4))
                .map(|_| ADV_PIECES[rng.below(ADV_PIECES.len())])
                .collect();
            let mut a = plain();
            if rng.chance(60) {
                a.bold = rng.chance(30);
                a.italic = rng.chance(30);
                a.code = rng.chance(15);
                a.underline = rng.chance(15);
                a.strikethrough = rng.chance(15);
                if rng.chance(25) {
                    a.link = Some(ADV_LINKS[rng.below(ADV_LINKS.len())].to_string());
                }
                if rng.chance(15) {
                    a.highlight = Some(Highlight::ALL[rng.below(6)]);
                }
                if rng.chance(15) {
                    a.color = Some(Rgb::new(rng.below(256) as u8, 0, 255));
                }
                if rng.chance(15) {
                    a.font = Some(
                        [
                            "F<o>\"n\"&t",
                            "",
                            " ",
                            "Menlo",
                            "a\u{0}b\nc",
                            "日本 Font",
                            "x;y",
                        ][rng.below(7)]
                        .to_string(),
                    );
                }
            }
            block = block.with_span(&text, a);
        }
        if rng.chance(20) {
            block = block.with_text_size(Some(
                [1.0, 6.0, 8.5, 17.0, 33.3, 144.0, 1.0e9, -3.0, f32::NAN][rng.below(9)],
            ));
        }
        if rng.chance(30) {
            block = block.with_alignment(
                [
                    Alignment::Left,
                    Alignment::Center,
                    Alignment::Right,
                    Alignment::Justify,
                ][rng.below(4)],
            );
        }
        blocks.push(block);
    }
    let mut settings = DocumentSettings::default();
    if rng.chance(50) {
        settings.page = PageSetup {
            paper: [
                PaperSize::A4,
                PaperSize::UsLetter,
                PaperSize::Custom {
                    width: 1.0,
                    height: 1.0e9,
                },
                PaperSize::Custom {
                    width: f32::NAN,
                    height: -5.0,
                },
                PaperSize::Custom {
                    width: 300.0,
                    height: 400.5,
                },
            ][rng.below(5)],
            orientation: if rng.chance(50) {
                Orientation::Landscape
            } else {
                Orientation::Portrait
            },
            margins: [
                Margins::uniform(72.0),
                Margins::uniform(0.0),
                Margins::uniform(1.0e6),
                Margins::uniform(-10.0),
                Margins {
                    top: 10.5,
                    right: 20.25,
                    bottom: f32::NAN,
                    left: f32::INFINITY,
                },
            ][rng.below(5)],
        };
        settings.typography = Typography {
            body_font: ["Charter", "", "A \"B\" <C> & D", "x\u{0}y"][rng.below(4)].to_string(),
            base_size: [17.0, 1.0, 400.0, f32::NAN, 12.5][rng.below(5)],
            line_height: [1.6, 0.0, 9.0, f32::NAN, 1.15][rng.below(5)],
            paragraph_spacing: [12.0, 0.0, 1.0e6, -4.0, f32::NAN][rng.below(5)],
        };
    }
    let mut ed = EditorState::new(Document::from_blocks(blocks).with_settings(settings));
    for _ in 0..rng.below(6) {
        let count = ed.document().block_count();
        let pick = |rng: &mut Rng, ed: &EditorState| {
            let b = rng.below(count);
            let len = ed.document().blocks()[b].len();
            ed.document()
                .clamp_position(Position::new(b, rng.below(len + 1)))
        };
        let (a, b) = (pick(rng, &ed), pick(rng, &ed));
        ed.set_selection(Selection::new(a, b));
        let body = [
            "body <&> \u{1}",
            "",
            "a\n\nb\tc",
            "]]> </w:t> 😀 שלום",
            "\u{fffe}",
        ][rng.below(5)];
        let author = ["A & \"B\"", "", "  ", "x\ny", "<a>\u{0}"][rng.below(5)];
        if let Some(id) = ed.add_comment(
            body,
            author,
            [0, -1, i64::MAX, i64::MIN, 1_794_391_505_000][rng.below(5)],
        ) {
            for _ in 0..rng.below(3) {
                ed.add_reply(id, "reply\nline", author, 2);
            }
            if rng.chance(30) {
                ed.resolve_comment(id);
            }
        }
    }
    ed.into_document()
}

/// The characters of `s` that XML can hold, as the exporter writes them.
fn xml_text(s: &str) -> String {
    s.chars()
        .filter(|c| matches!(c, '\t' | '\n' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..))
        .collect()
}

/// Hostile documents, settings, options and collections: every package
/// passes the structural checks and keeps the text of every block. With
/// `WRITER_EXPORT_DUMP_DIR` set, the packages are also written there (as
/// `<seed>.docx` and the expected text as `<seed>.txt`) for checks with
/// external tools.
#[test]
fn adversarial_packages_are_valid_and_keep_their_text() {
    let seeds: u64 = std::env::var("WRITER_EXPORT_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let dump = std::env::var("WRITER_EXPORT_DUMP_DIR").ok();
    for seed in 1..=seeds.min(20_000) {
        let mut rng = Rng(seed.wrapping_mul(0xC2B2_AE3D_27D4_EB4F) | 1);
        let docs: Vec<Document> = (0..if rng.chance(30) { rng.below(4) } else { 1 })
            .map(|_| adv_document(&mut rng))
            .collect();
        let single = docs.len() == 1 && rng.chance(70);
        let source = if single {
            ExportSource::single("T <&> \u{1}", &docs[0])
        } else {
            ExportSource::collection(
                "Book\u{0}",
                docs.iter()
                    .enumerate()
                    .map(|(i, d)| (format!("Part {i} <&>"), d)),
            )
        };
        let options = DocxOptions {
            comments: rng.chance(80),
            page_numbers: rng.chance(60),
            author: ["", "Ada <&> \"L\"", "x\u{0}\ny"][rng.below(3)].to_string(),
            created: [0, -1, i64::MAX, i64::MIN, 1_794_391_505_000][rng.below(5)],
            modified: [0, 1_794_391_505_000, i64::MAX][rng.below(3)],
            text_scale: [1.0, 12.0 / 17.0, 0.0, -1.0, 1.0e9, f32::NAN][rng.below(6)],
            code_font: ["Menlo", "", "A \"B\" <C>"][rng.below(3)].to_string(),
            part_titles: [
                PartTitles::Never,
                PartTitles::WhenMissing,
                PartTitles::Always,
            ][rng.below(3)],
        };
        let result = std::panic::catch_unwind(|| {
            let package = Package::new(&source, &options);
            let ps = package.paragraphs();
            let mut ps = ps.iter().peekable();
            let mut expected_text = String::new();
            let count = source.parts().len();
            for (index, part) in source.parts().iter().enumerate() {
                let doc = part.document;
                // Page breaks at the edge towards another part are dropped.
                let is_break = |b: &Block| matches!(b.kind(), BlockKind::PageBreak);
                let blocks = doc.blocks();
                let from = if index == 0 {
                    0
                } else {
                    blocks.iter().take_while(|b| is_break(b)).count()
                };
                let to = if index + 1 == count {
                    blocks.len()
                } else {
                    blocks.len() - blocks.iter().rev().take_while(|b| is_break(b)).count()
                };
                let blocks = if from < to {
                    &blocks[from..to]
                } else {
                    &blocks[..0]
                };
                let titled = !single
                    && match options.part_titles {
                        PartTitles::Never => false,
                        PartTitles::Always => true,
                        // The title "Part N <&>" never occurs in the text.
                        PartTitles::WhenMissing => true,
                    };
                if titled {
                    let p = ps.next().expect("a title paragraph");
                    assert_eq!(p.style, "Heading1");
                    assert_eq!(p.text(), part.title);
                    expected_text.push_str(&p.text());
                    expected_text.push('\n');
                }
                let mut pending_break = false;
                for block in blocks {
                    if matches!(block.kind(), BlockKind::PageBreak) {
                        if pending_break {
                            assert!(ps.next().unwrap().page_break);
                        }
                        pending_break = true;
                        continue;
                    }
                    let p = ps.next().expect("a paragraph for every block");
                    let mut text = p.text();
                    assert_eq!(p.page_break, pending_break);
                    pending_break = false;
                    if let BlockKind::TodoItem { checked } = block.kind() {
                        let mark = if *checked { "\u{2611}\t" } else { "\u{2610}\t" };
                        text = text.strip_prefix(mark).expect("a ballot box").to_string();
                    }
                    assert_eq!(text, xml_text(block.text()));
                    expected_text.push_str(&text);
                    expected_text.push('\n');
                }
                if pending_break {
                    assert!(ps.next().unwrap().page_break);
                }
                if blocks.is_empty() && !titled {
                    // A part with nothing in it is one empty paragraph.
                    assert_eq!(ps.next().expect("an empty paragraph").text(), "");
                }
            }
            if source.parts().is_empty() {
                ps.next();
            }
            assert!(ps.next().is_none(), "extra paragraphs");
            expected_text
        });
        match result {
            Ok(text) => {
                if let Some(dir) = &dump {
                    let bytes = to_docx(&source, &options);
                    std::fs::write(format!("{dir}/{seed}.docx"), bytes).unwrap();
                    std::fs::write(format!("{dir}/{seed}.txt"), text).unwrap();
                }
            }
            Err(e) => {
                eprintln!("seed {seed} failed");
                std::panic::resume_unwind(e);
            }
        }
    }
}

// ----- review pass: edge cases ------------------------------------------------------------

#[test]
fn page_breaks_between_the_parts_of_a_collection_leave_no_empty_page() {
    let chapter = |name: &str| {
        Document::from_blocks(vec![
            Block::new(BlockKind::PageBreak),
            Block::with_text(BlockKind::heading(1), name),
            Block::new(BlockKind::PageBreak),
            Block::paragraph("Body"),
            Block::new(BlockKind::PageBreak),
            Block::new(BlockKind::PageBreak),
        ])
    };
    let (a, b, c) = (chapter("A"), chapter("B"), chapter("C"));
    let only_breaks = Document::from_blocks(vec![Block::new(BlockKind::PageBreak)]);
    let source = ExportSource::collection(
        "Book",
        [("A", &a), ("B", &b), ("", &only_breaks), ("C", &c)],
    );
    let o = DocxOptions {
        part_titles: PartTitles::Never,
        ..Default::default()
    };
    let ps = Package::new(&source, &o).paragraphs();
    let seen: Vec<(String, bool, bool)> = ps
        .iter()
        .map(|p| (p.text(), p.page_break, p.section))
        .collect();
    let row = |t: &str, page_break: bool, section: bool| (t.to_string(), page_break, section);
    assert_eq!(
        seen,
        vec![
            // The first part keeps its leading break, the last its trailing ones.
            row("A", true, false),
            row("Body", true, true),
            row("B", false, false),
            row("Body", true, true),
            row("", false, true),
            row("C", false, false),
            row("Body", true, false),
            row("", true, false),
            row("", true, false),
        ]
    );
    // A single document is exported as it is.
    let ps = Package::of(&a).paragraphs();
    assert_eq!(ps.len(), 4);
    assert!(ps[0].page_break && ps[2].page_break && ps[3].page_break);
}

#[test]
fn unusable_values_are_left_out() {
    // A font name that is empty once cleaned is not written.
    for name in ["", "  ", "\u{0}\u{1}"] {
        let ps = paragraphs(vec![Block::new(BlockKind::Paragraph).with_span(
            "x",
            InlineAttrs {
                font: Some(name.to_string()),
                ..plain()
            },
        )]);
        assert_eq!(ps[0].runs[0].font, None, "{name:?}");
    }
    // A comment without a time has no date; a real time is written.
    let mut ed = EditorState::new(Document::from_blocks(vec![Block::paragraph("one two")]));
    ed.set_selection(Selection::new(Position::new(0, 0), Position::new(0, 3)));
    let id = ed.add_comment("no time", "Ada", 0).unwrap();
    ed.add_reply(id, "negative", "Ben", -5);
    ed.add_reply(id, "real", "Cy", 1_794_391_505_000);
    let package = Package::of(&ed.into_document());
    let comments = roxmltree::Document::parse(package.part(parts::COMMENTS)).unwrap();
    let dates: Vec<Option<&str>> = comments
        .descendants()
        .filter(|n| n.has_tag_name((W, "comment")))
        .map(|n| n.attribute((W, "date")))
        .collect();
    assert_eq!(dates, vec![None, None, Some("2026-11-11T10:05:05Z")]);
    // Times before 1970 in the options are written as 1970.
    let doc = Document::new();
    let package = Package::new(
        &ExportSource::single("T", &doc),
        &DocxOptions {
            created: i64::MIN,
            modified: -1,
            ..Default::default()
        },
    );
    assert_eq!(
        package
            .part(parts::CORE)
            .matches("1970-01-01T00:00:00Z")
            .count(),
        2
    );
}

#[test]
fn pages_larger_than_word_allows_are_cut_to_its_limit() {
    let settings = DocumentSettings {
        page: PageSetup {
            paper: PaperSize::Custom {
                width: 5000.0,
                height: 14_000.0,
            },
            orientation: Orientation::Portrait,
            margins: Margins {
                top: 6000.0,
                right: 100.0,
                bottom: 6000.0,
                left: 3000.0,
            },
        },
        ..Default::default()
    };
    let doc = Document::new().with_settings(settings);
    let s = &Package::of(&doc).sections()[0];
    // 22 inches is 31680 twips.
    assert_eq!((s.0, s.1), (31_680, 31_680));
    let (top, right, bottom, left) = (s.3, s.4, s.5, s.6);
    assert!(left + right <= 31_680 - 1440, "{left} + {right}");
    assert!(top + bottom <= 31_680 - 1440, "{top} + {bottom}");
    assert!(top > 0 && left > right);
    // An ordinary page is untouched.
    let s = &Package::of(&Document::new()).sections()[0];
    assert_eq!(
        (s.0, s.1, s.3, s.4, s.5, s.6),
        (11_906, 16_838, 1440, 1440, 1440, 1440)
    );
}

// ----- the proofing language ---------------------------------------------------------------------

/// A document with a spelling language (`None`: it follows the app).
fn in_language(language: Option<&str>, blocks: Vec<Block>) -> Document {
    let settings = DocumentSettings {
        spelling_language: language.and_then(SpellingLanguage::language),
        ..DocumentSettings::default()
    };
    Document::from_blocks(blocks).with_settings(settings)
}

/// The `w:lang` of the document defaults, as (attribute, tag) pairs.
fn default_languages(package: &Package) -> Vec<(String, String)> {
    let styles = roxmltree::Document::parse(package.part(parts::STYLES)).unwrap();
    let defaults = styles
        .descendants()
        .find(|n| n.has_tag_name((W, "rPrDefault")))
        .expect("document defaults");
    languages(defaults)
}

/// Every `w:lang` below `node`, as (attribute, tag) pairs.
fn languages(node: Node<'_, '_>) -> Vec<(String, String)> {
    node.descendants()
        .filter(|n| n.has_tag_name((W, "lang")))
        .flat_map(|n| {
            n.attributes()
                .map(|a| (a.name().to_string(), a.value().to_string()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// For every run with text: its text and its `w:lang` value, if any.
fn run_languages(package: &Package) -> Vec<(String, Option<String>)> {
    let document = roxmltree::Document::parse(package.part(parts::DOCUMENT)).unwrap();
    document
        .descendants()
        .filter(|n| n.has_tag_name((W, "r")))
        .filter_map(|r| {
            let text: String = r
                .children()
                .filter(|n| n.has_tag_name((W, "t")))
                .filter_map(|n| n.text())
                .collect();
            let lang = languages(r).into_iter().next().map(|(_, tag)| tag);
            (!text.is_empty()).then_some((text, lang))
        })
        .collect()
}

#[test]
fn a_fixed_spelling_language_is_the_proofing_language() {
    let text = || vec![Block::paragraph("Colour and harbour")];
    let pair = |attribute: &str, tag: &str| vec![(attribute.to_string(), tag.to_string())];
    for (setting, expected) in [
        (Some("en_GB"), pair("val", "en-GB")),
        (Some("pt_BR"), pair("val", "pt-BR")),
        // A language without a region gets the one Word's tools are named
        // after.
        (Some("de"), pair("val", "de-DE")),
        (Some("en"), pair("val", "en-US")),
        (Some("nb"), pair("val", "nb-NO")),
        // The script decides which of Word's three languages it is.
        (Some("he"), pair("bidi", "he-IL")),
        (Some("ars"), pair("bidi", "ar-SA")),
        (Some("ko"), pair("eastAsia", "ko-KR")),
        // Not fixed: the reader decides, as before.
        (None, vec![]),
        (Some("auto"), vec![]),
        // An identifier that is no language.
        (Some("x_YY"), vec![]),
        (Some("Multilingual"), vec![]),
    ] {
        let doc = in_language(setting, text());
        let package = Package::of(&doc);
        assert_eq!(default_languages(&package), expected, "{setting:?}");
        // The styles carry it: no run repeats it.
        assert_eq!(
            run_languages(&package),
            [("Colour and harbour".to_string(), None)],
            "{setting:?}"
        );
    }
    // Without a language the package is byte for byte what it was.
    let plain_doc = Document::from_blocks(text());
    let source = ExportSource::single("Test", &plain_doc);
    let automatic = in_language(Some("auto"), text());
    assert_eq!(
        to_docx(&source, &DocxOptions::default()),
        to_docx(
            &ExportSource::single("Test", &automatic),
            &DocxOptions::default()
        )
    );
    assert!(
        !Package::of(&plain_doc)
            .part(parts::STYLES)
            .contains("w:lang")
    );
    // `w:lang` is the last child of the default run properties, where the
    // schema wants it (after the sizes).
    let package = Package::of(&in_language(Some("en_GB"), text()));
    assert!(
        package
            .part(parts::STYLES)
            .contains("<w:szCs w:val=\"34\"/><w:lang w:val=\"en-GB\"/></w:rPr></w:rPrDefault>"),
        "{}",
        &package.part(parts::STYLES)[..600]
    );
}

#[test]
fn every_document_of_a_collection_keeps_its_own_proofing_language() {
    let chapter = |language: Option<&str>, name: &str| {
        in_language(
            language,
            vec![
                Block::with_text(BlockKind::heading(1), name),
                Block::paragraph(&format!("{name} body ")).with_span(
                    "code",
                    InlineAttrs {
                        code: true,
                        ..plain()
                    },
                ),
                Block::with_text(BlockKind::todo(), &format!("{name} todo")),
                Block::with_text(BlockKind::code(), &format!("{name} block")),
            ],
        )
    };
    // Every chapter in the same language: the styles carry it, no run does.
    let (a, b) = (chapter(Some("en_GB"), "one"), chapter(Some("en_GB"), "two"));
    let package = Package::new(
        &ExportSource::collection("Book", [("One", &a), ("Two", &b)]),
        &DocxOptions::default(),
    );
    assert_eq!(
        default_languages(&package),
        [("val".to_string(), "en-GB".to_string())]
    );
    assert!(run_languages(&package).iter().all(|(_, l)| l.is_none()));

    // Different languages, one chapter without: nothing in the styles, and
    // each chapter with a language says so on its own runs of text.
    let (fr, none, de, auto) = (
        chapter(Some("fr"), "un"),
        chapter(None, "plain"),
        chapter(Some("de_CH"), "eins"),
        chapter(Some("auto"), "any"),
    );
    let source = ExportSource::collection(
        "Book",
        [("Un", &fr), ("Plain", &none), ("Eins", &de), ("Any", &auto)],
    );
    let options = DocxOptions {
        part_titles: PartTitles::Always,
        ..Default::default()
    };
    let package = Package::new(&source, &options);
    assert_eq!(default_languages(&package), []);
    let runs = run_languages(&package);
    let of = |text: &str| -> Option<String> {
        runs.iter()
            .find(|(t, _)| t == text)
            .unwrap_or_else(|| panic!("no run {text:?} in {runs:?}"))
            .1
            .clone()
    };
    for (text, expected) in [
        ("un", Some("fr-FR")),
        ("un body ", Some("fr-FR")),
        ("un todo", Some("fr-FR")),
        ("plain", None),
        ("plain body ", None),
        ("eins", Some("de-CH")),
        ("eins body ", Some("de-CH")),
        ("eins todo", Some("de-CH")),
        ("any", None),
        ("any body ", None),
        // Code is not prose: no language (it is marked as not to be
        // proofed), nor has the ballot box of a to-do.
        ("code", None),
        ("un block", None),
        ("eins block", None),
        ("\u{2610}", None),
    ] {
        assert_eq!(of(text).as_deref(), expected, "{text}");
    }
    // The title put above a chapter is in the chapter's language.
    assert_eq!(of("Un").as_deref(), Some("fr-FR"));
    assert_eq!(of("Eins").as_deref(), Some("de-CH"));
    assert_eq!(of("Plain"), None);
    // `w:lang` closes the run properties, after everything else.
    let document = package.part(parts::DOCUMENT);
    assert!(document.contains("<w:lang w:val=\"fr-FR\"/></w:rPr><w:t"));
    // A chapter alone is the same text with the language in the styles.
    let alone = Package::of(&de);
    assert_eq!(
        default_languages(&alone),
        [("val".to_string(), "de-CH".to_string())]
    );
    assert!(run_languages(&alone).iter().all(|(_, l)| l.is_none()));
}
