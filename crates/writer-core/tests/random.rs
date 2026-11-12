//! Randomised tests: long seeded sequences of commands on random (often
//! out-of-range or mid-character) selections.
//!
//! - `random_command_sequences` / `long_random_sequences`: invariants are
//!   checked after every step, change reports are checked against block
//!   revision stamps, then everything is undone (must equal the original
//!   document) and redone (must equal the final one).
//! - `every_command_undoes_exactly`: every single command is undone and
//!   redone on the spot, and must restore the document, the selection and
//!   every block's `(id, revision)` exactly.
//! - Documents and fragments round-trip through JSON; corrupted JSON never
//!   panics.
//!
//! `WRITER_CORE_SEEDS=n` runs `n` seeds instead of the default.

use std::collections::HashMap;

use writer_core::{
    Alignment, Block, BlockId, BlockKind, CommentId, Document, EditorOptions, EditorState,
    Fragment, Highlight, InlineAttrs, InlineStyle, Movement, Position, Rgb, SearchOptions,
    Selection, TextRange,
};

/// xorshift64*: small, deterministic, good enough for test generation.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

fn seeds(default: u64) -> u64 {
    std::env::var("WRITER_CORE_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

const PIECES: &[&str] = &[
    "a",
    "b",
    "word",
    " ",
    "  ",
    "é",
    "e\u{301}",
    "\u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F467}",
    "中文",
    "שלום",
    "\n",
    "\u{1100}",
    "\u{1161}",
    "\u{1F1EB}\u{1F1F7}",
    "*",
    "**",
    "_",
    "`",
    "#",
    "-",
    "\"",
    "'",
    ".",
    "=",
    "~",
    "1.",
    "[ ]",
    ">",
    "x",
    "\t",
    "\r\n",
    "Σ",
    "ς",
    "İ",
    "  \n",
    "\u{0}",
    "\u{2028}",
    "a\u{308}\u{301}",
];

const TYPED: &[&str] = &[
    "a",
    "b",
    "c",
    " ",
    " ",
    " ",
    "*",
    "_",
    "`",
    "~",
    "=",
    "#",
    "-",
    "\"",
    "'",
    ".",
    "1",
    ")",
    "[",
    "]",
    "x",
    ">",
    "é",
    "\u{301}",
    "中",
    "ש",
    "\u{1F600}",
    "\u{200D}",
    "\u{1100}",
    "\u{1161}",
    "+",
    "\u{1F1EB}",
    "(",
    "\t",
];

fn random_kind(r: &mut Rng) -> BlockKind {
    match r.below(12) {
        0 => BlockKind::heading(1 + r.below(6) as u8),
        1 => BlockKind::BulletItem,
        2 => BlockKind::NumberedItem,
        3 => BlockKind::TodoItem {
            checked: r.chance(50),
        },
        4 => BlockKind::Quote,
        5 => BlockKind::CodeBlock {
            language: r.chance(50).then(|| "rust".to_string()),
        },
        6 => BlockKind::Divider,
        7 => BlockKind::PageBreak,
        _ => BlockKind::Paragraph,
    }
}

fn random_attrs(r: &mut Rng) -> InlineAttrs {
    let mut a = InlineAttrs::default();
    if r.chance(60) {
        return a;
    }
    a.bold = r.chance(30);
    a.italic = r.chance(30);
    a.underline = r.chance(15);
    a.strikethrough = r.chance(10);
    a.code = r.chance(10);
    if r.chance(15) {
        a.highlight = Some(*r.pick(&Highlight::ALL));
    }
    if r.chance(10) {
        a.color = Some(Rgb::new(r.below(256) as u8, 0, 9));
    }
    if r.chance(10) {
        a.font = Some(if r.chance(80) {
            "Menlo".into()
        } else {
            "  ".into()
        });
    }
    if r.chance(10) {
        a.link = Some(format!("https://example.com/{}", r.below(3)));
    }
    if r.chance(5) {
        // Anchors to threads that do not exist: must be removed on load.
        a.comments.insert(CommentId(r.below(4) as u64));
    }
    a
}

fn random_text_block(r: &mut Rng, kind: BlockKind) -> Block {
    let mut b = Block::new(kind);
    for _ in 0..r.below(6) {
        let piece = *r.pick(PIECES);
        b = b.with_span(piece, random_attrs(r));
    }
    b.with_alignment(*r.pick(&[
        Alignment::Left,
        Alignment::Center,
        Alignment::Right,
        Alignment::Justify,
    ]))
    .with_indent(if r.chance(30) { r.below(4) as u8 } else { 0 })
    .with_text_size(if r.chance(10) { Some(20.0) } else { None })
}

fn random_document(r: &mut Rng) -> Document {
    let n = 1 + r.below(10);
    let blocks = (0..n)
        .map(|_| {
            let k = random_kind(r);
            random_text_block(r, k)
        })
        .collect();
    Document::from_blocks(blocks)
}

/// A position that is often invalid: past the last block, past the end of
/// the text, inside a multi-byte character or a grapheme cluster.
fn random_position(r: &mut Rng, d: &Document) -> Position {
    if r.chance(2) {
        return Position::new(usize::MAX, usize::MAX);
    }
    let b = r.below(d.block_count() + 1);
    let len = d.block(b.min(d.block_count() - 1)).map_or(0, |b| b.len());
    Position::new(b, r.below(len + 3))
}

fn random_range(r: &mut Rng, d: &Document) -> TextRange {
    let a = random_position(r, d);
    let b = if r.chance(50) {
        Position::new(a.block, r.below(a.offset.saturating_add(6).min(1 << 20)))
    } else {
        random_position(r, d)
    };
    TextRange::new(a, b)
}

fn random_block_range(r: &mut Rng, d: &Document) -> std::ops::Range<usize> {
    let a = r.below(d.block_count() + 2);
    let b = a + r.below(4);
    a..b
}

fn stamps(d: &Document) -> Vec<(BlockId, u64)> {
    d.blocks().iter().map(|b| (b.id(), b.revision())).collect()
}

/// Whether a command goes through the undo history.
#[derive(PartialEq)]
enum Undoable {
    Yes,
    /// Settings changes are not undoable by design.
    No,
    /// Several commands, the first of which only moves the selection: undo
    /// restores the document but the selection only to after that move.
    Compound,
}

struct Runner {
    r: Rng,
    e: EditorState,
    clipboard: Vec<Fragment>,
    depth: usize,
    /// Every (id, revision) seen, with the content it had.
    versions: HashMap<(BlockId, u64), Block>,
    now: i64,
}

impl Runner {
    fn comment_id(&mut self) -> CommentId {
        let ids: Vec<CommentId> = self.e.document().comments().map(|t| t.id).collect();
        if ids.is_empty() || self.r.chance(5) {
            CommentId(self.r.below(50) as u64)
        } else {
            *self.r.pick(&ids)
        }
    }

    /// One random public command (never undo/redo/grouping).
    fn command(&mut self) -> (String, Undoable) {
        let r = &mut self.r;
        let e = &mut self.e;
        let op = r.below(118);
        let name: String = match op {
            0..=9 => {
                let a = random_position(r, e.document());
                let h = match r.below(10) {
                    0..=6 => a,
                    7..=8 => {
                        Position::new(a.block, r.below(a.offset.saturating_add(8).min(1 << 20)))
                    }
                    _ => random_position(r, e.document()),
                };
                e.set_selection(Selection::new(a, h));
                format!("select {a:?} {h:?}")
            }
            10..=29 => {
                let t = *r.pick(TYPED);
                e.insert_text(t);
                format!("type {t:?}")
            }
            30..=31 => {
                let mut s = String::new();
                for _ in 0..1 + r.below(4) {
                    s.push_str(r.pick(PIECES));
                }
                if r.chance(50) {
                    e.insert_text(&s);
                } else {
                    e.paste_plain_text(&s);
                }
                format!("insert {s:?}")
            }
            32..=35 => {
                e.insert_paragraph_break();
                "enter".into()
            }
            36 => {
                e.insert_soft_break();
                "soft break".into()
            }
            37 => {
                e.exit_block();
                "exit block".into()
            }
            38..=42 => {
                e.backspace();
                "backspace".into()
            }
            43..=44 => {
                e.delete_forward();
                "delete".into()
            }
            45 => {
                match r.below(5) {
                    0 => e.delete_word_backward(),
                    1 => e.delete_word_forward(),
                    2 => e.delete_to_line_start(),
                    3 => e.delete_to_line_end(),
                    _ => e.delete_selection(),
                };
                "word/line delete".into()
            }
            46..=50 => {
                let m = *r.pick(&[
                    Movement::Left,
                    Movement::Right,
                    Movement::WordLeft,
                    Movement::WordRight,
                    Movement::LineStart,
                    Movement::LineEnd,
                    Movement::BlockStart,
                    Movement::BlockEnd,
                    Movement::DocStart,
                    Movement::DocEnd,
                ]);
                let ext = r.chance(40);
                e.move_caret(m, ext);
                format!("move {m:?} {ext}")
            }
            51 => {
                match r.below(5) {
                    0 => e.select_all(),
                    1 => {
                        let p = random_position(r, e.document());
                        e.select_word_at(p)
                    }
                    2 => {
                        let i = r.below(e.document().block_count() + 1);
                        e.select_block(i)
                    }
                    3 => {
                        let br = random_block_range(r, e.document());
                        e.select_blocks(br)
                    }
                    _ => {
                        let t = random_range(r, e.document());
                        e.select_range(t)
                    }
                }
                "select special".into()
            }
            52..=55 => {
                let s = *r.pick(&InlineStyle::ALL);
                match r.below(6) {
                    0 => e.toggle_bold(),
                    1 => e.toggle_italic(),
                    2 => e.toggle_underline(),
                    3 => e.toggle_strikethrough(),
                    4 => e.toggle_code(),
                    _ => e.toggle_style(s),
                };
                format!("toggle {s:?}")
            }
            56 => {
                match r.below(8) {
                    0 => e.set_highlight(if r.chance(70) {
                        Some(*r.pick(&Highlight::ALL))
                    } else {
                        None
                    }),
                    1 => e.set_text_color(if r.chance(70) {
                        Some(Rgb::new(1, 2, 3))
                    } else {
                        None
                    }),
                    2 => e.set_font_family(if r.chance(70) {
                        Some("Georgia")
                    } else {
                        Some(" ")
                    }),
                    3 => e.set_link(if r.chance(80) { "https://l.ink" } else { "  " }),
                    4 => e.remove_link(),
                    5 => e.toggle_highlight(*r.pick(&Highlight::ALL)),
                    _ => e.clear_formatting(),
                };
                "attr".into()
            }
            57 => {
                e.insert_link(if r.chance(80) { "link" } else { "a\nb" }, "https://x.y");
                "insert link".into()
            }
            58..=61 => {
                let k = random_kind(r);
                e.set_block_kind(k.clone());
                format!("kind {k:?}")
            }
            62 => {
                match r.below(7) {
                    0 => e.toggle_todo_checked(),
                    1 => e.set_alignment(*r.pick(&[
                        Alignment::Left,
                        Alignment::Center,
                        Alignment::Justify,
                    ])),
                    2 => e.step_text_size(*r.pick(&[1, -2, 30, i32::MIN])),
                    3 => e.set_text_size(Some(*r.pick(&[0.0, 13.0, 500.0, f32::NAN]))),
                    4 => e.reset_text_size(),
                    5 => e.set_code_language(if r.chance(70) { Some("py") } else { None }),
                    _ => e.set_alignment(Alignment::Right),
                };
                "block attr".into()
            }
            63..=64 => {
                if r.chance(50) {
                    e.indent();
                } else {
                    e.outdent();
                }
                "indent".into()
            }
            65..=66 => {
                match r.below(4) {
                    0 => e.move_blocks_up(),
                    1 => e.move_blocks_down(),
                    2 => e.duplicate_blocks(),
                    _ => e.delete_blocks(),
                };
                "block op".into()
            }
            67 => {
                match r.below(4) {
                    0 => e.insert_divider(),
                    1 => e.insert_page_break(),
                    2 => e.insert_block_before(),
                    _ => e.insert_block_after(),
                };
                "insert block".into()
            }
            68..=69 => {
                let f = match r.below(4) {
                    0 => e.cut(),
                    1 => {
                        let br = random_block_range(r, e.document());
                        e.document().block_fragment(br)
                    }
                    2 => {
                        let t = random_range(r, e.document());
                        e.document().fragment(t)
                    }
                    _ => e.copy(),
                };
                // Round-trip through JSON as the clipboard would.
                let back = Fragment::from_json(&f.to_json()).expect("fragment json");
                assert_eq!(back, f);
                assert_eq!(
                    Fragment::from_plain_text(&f.to_plain_text()).to_plain_text(),
                    f.to_plain_text()
                );
                self.clipboard.push(f);
                "copy/cut".into()
            }
            70..=72 => {
                if !self.clipboard.is_empty() {
                    let f = r.pick(&self.clipboard).clone();
                    match r.below(5) {
                        0 => {
                            let i = r.below(e.document().block_count() + 2);
                            e.insert_blocks_at(i, &f);
                        }
                        1 => {
                            e.paste_plain_text(&f.to_plain_text());
                        }
                        _ => {
                            e.paste(&f);
                        }
                    }
                }
                "paste".into()
            }
            73..=75 => {
                self.now += 1;
                e.add_comment("note", " me ", self.now);
                "add comment".into()
            }
            76..=78 => {
                let id = self.comment_id();
                let r = &mut self.r;
                let e = &mut self.e;
                match r.below(7) {
                    0 => {
                        e.edit_comment(id, "edited\r\n");
                    }
                    1 => {
                        e.add_reply(id, "reply", "you", 7);
                    }
                    2 => {
                        let rid = e
                            .document()
                            .comment(id)
                            .and_then(|t| t.replies.first().map(|r| r.id));
                        if let Some(rid) = rid {
                            e.edit_reply(id, rid, "changed");
                        }
                    }
                    3 => {
                        let rid = e
                            .document()
                            .comment(id)
                            .and_then(|t| t.replies.last().map(|r| r.id));
                        if let Some(rid) = rid {
                            e.delete_reply(id, rid);
                        }
                    }
                    4 => {
                        e.resolve_comment(id);
                    }
                    5 => {
                        e.reopen_comment(id);
                    }
                    _ => {
                        e.delete_comment(id);
                    }
                }
                let d = e.document();
                let _ = d.comment_ranges(id);
                let _ = d.is_orphaned(id);
                let p = random_position(r, d);
                let _ = d.next_comment(p, true);
                let _ = d.prev_comment(p, false);
                let _ = d.comments_at(p);
                let _ = d.threads_in_order();
                "comment op".into()
            }
            79..=80 => {
                let q = *r.pick(&["a", "word", " ", "é", "中", "σ", "A", "\n", "e\u{301}"]);
                let rep = *r.pick(&["", "z", "longer text", "\u{1F600}", "Z", "a\nb"]);
                let opts = SearchOptions {
                    case_sensitive: r.chance(50),
                    whole_word: r.chance(30),
                };
                match r.below(3) {
                    0 => {
                        e.replace_all(q, opts, rep);
                    }
                    1 => {
                        let m = e.document().find_all(q, opts);
                        if !m.is_empty() {
                            let t = m[r.below(m.len())];
                            e.replace_range(t, rep);
                        }
                    }
                    _ => {
                        let t = random_range(r, e.document());
                        e.replace_range(t, rep);
                    }
                }
                "replace".into()
            }
            81 => {
                e.set_options(EditorOptions {
                    input_rules: r.chance(70),
                    smart_punctuation: r.chance(70),
                });
                "options".into()
            }
            82 => {
                let t = random_range(r, e.document());
                e.delete_range(t);
                "delete range".into()
            }
            83 => {
                // An IME composition: marked text replaced a few times, then
                // committed, as one undo step.
                let at = e.selection().head;
                e.begin_undo_group();
                let mut range = TextRange::empty(at);
                for s in ["k", "ka", "\u{304B}"] {
                    e.replace_range(range, s);
                    let end = e.selection().head;
                    range = TextRange::new(
                        Position::new(end.block, end.offset - s.len().min(end.offset)),
                        end,
                    );
                }
                e.end_undo_group();
                "ime".into()
            }
            84 => {
                let d = e.document().clone();
                let mut s = d.settings().clone();
                s.page_view = !s.page_view;
                s.typography.base_size = *r.pick(&[17.0, 1.0, 1e9]);
                if r.chance(50) {
                    e.set_page_view(!d.settings().page_view);
                } else {
                    e.set_settings(s);
                }
                return ("settings".into(), Undoable::No);
            }
            85..=96 => {
                // Grow the document: several pieces, sometimes multi-line.
                let mut s = String::new();
                for _ in 0..2 + r.below(8) {
                    s.push_str(r.pick(PIECES));
                    s.push(' ');
                }
                e.paste_plain_text(&s);
                format!("grow {s:?}")
            }
            97..=100 => {
                // Comment a random word.
                let p = random_position(r, e.document());
                e.select_word_at(p);
                self.now += 1;
                e.add_comment("word note", "me", self.now);
                e.move_caret(Movement::Right, false);
                return ("comment word".into(), Undoable::Compound);
            }
            101..=103 => {
                // Toggle a style on a collapsed caret, then type: pending attrs.
                let s = *r.pick(&InlineStyle::ALL);
                e.toggle_style(s);
                if r.chance(30) {
                    let id = self.comment_id();
                    self.e.delete_comment(id);
                }
                let t = *self.r.pick(TYPED);
                self.e.insert_text(t);
                "pending then type".into()
            }
            104..=105 => {
                // A checkbox click on any block, in or out of range.
                let i = r.below(e.document().block_count() + 2);
                let sel = e.selection();
                e.set_todo_checked(i, r.chance(50));
                assert_eq!(e.selection(), sel, "checkbox click moved the selection");
                "todo click".into()
            }
            106..=109 => {
                // Whole-line selections (offset 0 to offset 0 of a later
                // block), then delete, cut and paste back, or type over.
                let n = e.document().block_count();
                let a = r.below(n);
                let b = (a + 1 + r.below(3)).min(n - 1);
                let sel = if r.chance(50) {
                    Selection::new(Position::new(a, 0), Position::new(b, 0))
                } else {
                    Selection::new(Position::new(b, 0), Position::new(a, 0))
                };
                e.set_selection(sel);
                match r.below(4) {
                    0 => {
                        e.delete_selection();
                    }
                    1 => {
                        let f = e.cut();
                        let at = random_position(r, e.document());
                        e.set_caret(at);
                        e.paste(&f);
                    }
                    2 => {
                        e.insert_text("x");
                    }
                    _ => {
                        let _ = e.selected_text();
                        let r0 = e.document().block_range(a);
                        assert_eq!(r0.start, Position::new(a, 0));
                        e.backspace();
                    }
                }
                return ("whole lines".into(), Undoable::Compound);
            }
            _ => {
                self.queries();
                "queries".into()
            }
        };
        (name, Undoable::Yes)
    }

    /// Read-only queries with cross-checks between related answers.
    fn queries(&mut self) {
        let r = &mut self.r;
        let e = &mut self.e;
        let d = e.document();
        let _ = d.title();
        let _ = e.selection_stats();
        let _ = e.format_state();
        let _ = e.current_attrs();
        let p = random_position(r, d);
        let _ = d.link_at(p);
        let _ = d.text_in(TextRange::new(p, d.end()));
        let _ = d.moved(p, Movement::WordLeft);
        let _ = d.find_in_block(p.block, "a", SearchOptions::default());
        let s = e.stats();
        assert_eq!(s, e.document().stats());
        let d = e.document();
        let all = d.numbered_ordinals(0..d.block_count());
        for (i, o) in all.iter().enumerate() {
            assert_eq!(*o, d.numbered_ordinal(i));
        }
        let from = r.below(d.block_count() + 1);
        assert_eq!(
            d.numbered_ordinals(from..d.block_count() + 3),
            all[from.min(all.len())..].to_vec()
        );
        let flat = d.plain_text();
        let p = d.clamp_position(random_position(r, d));
        assert_eq!(d.position_from_flat(d.flat_offset(p)), p);
        assert_eq!(
            &flat[d.flat_offset(Position::ZERO)..d.flat_offset(p)],
            d.text_in(TextRange::new(Position::ZERO, p))
        );
        let b = d.block(p.block).unwrap();
        let u = b.offset_to_utf16(p.offset);
        assert_eq!(b.utf16_to_offset(u), p.offset);
        for m in d.find_all("a", SearchOptions::default()) {
            assert!(d.is_valid_position(m.start) && d.is_valid_position(m.end));
            assert_eq!(d.text_in(m).to_lowercase(), "a");
        }
        for (id, range) in d.all_comment_ranges() {
            assert!(d.comment(id).is_some());
            assert!(!d.is_orphaned(id));
            assert!(!range.is_empty());
        }
        for t in d.comments() {
            assert_eq!(d.is_orphaned(t.id), d.comment_ranges(t.id).is_empty());
        }
        let order = d.threads_in_order();
        assert_eq!(order.len(), d.comment_count());
    }

    fn step(&mut self) -> String {
        let op = self.r.below(100);
        match op {
            0..=5 => {
                self.e.undo();
                self.depth = 0;
                "undo".into()
            }
            6..=8 => {
                self.e.redo();
                self.depth = 0;
                "redo".into()
            }
            9..=10 => {
                self.e.break_undo_group();
                "break".into()
            }
            11 => {
                if self.depth < 2 {
                    self.e.begin_undo_group();
                    self.depth += 1;
                }
                "begin group".into()
            }
            12 => {
                if self.depth > 0 {
                    self.e.end_undo_group();
                    self.depth -= 1;
                }
                "end group".into()
            }
            _ => {
                let (name, undoable) = self.command();
                if undoable == Undoable::No {
                    // Keep the undo-everything check meaningful: put the
                    // settings back (not undoable by design).
                    self.e
                        .set_settings(writer_core::DocumentSettings::default());
                }
                name
            }
        }
    }

    fn record_versions(&mut self) {
        for b in self.e.document().blocks() {
            let key = (b.id(), b.revision());
            match self.versions.get(&key) {
                Some(old) => assert_eq!(old, b, "revision stamp reused for different content"),
                None => {
                    self.versions.insert(key, b.clone());
                }
            }
        }
    }
}

fn runner(seed: u64) -> (Runner, Document) {
    let mut r = Rng::new(seed);
    let original = random_document(&mut r);
    original.check_invariants().unwrap();
    // JSON round trip of the random document.
    assert_eq!(Document::from_json(&original.to_json()).unwrap(), original);
    let mut e = EditorState::new(original.clone());
    e.set_undo_limit(usize::MAX);
    let run = Runner {
        r,
        e,
        clipboard: Vec::new(),
        depth: 0,
        versions: HashMap::new(),
        now: 0,
    };
    (run, original)
}

fn run_seed(seed: u64, steps: usize) {
    let (mut run, original) = runner(seed);
    run.record_versions();
    let mut log: Vec<String> = Vec::new();
    for step in 0..steps {
        let before = stamps(run.e.document());
        let rev_before = run.e.revision();
        let what = run.step();
        log.push(what);
        if let Err(err) = run.e.check_invariants() {
            panic!(
                "seed {seed} step {step}: invariant violated: {err}\nlast ops: {:?}",
                &log[log.len().saturating_sub(8)..]
            );
        }
        // Change reports: blocks outside the reported range are untouched.
        let after = stamps(run.e.document());
        match run.e.take_changes() {
            None => {
                assert_eq!(
                    before,
                    after,
                    "seed {seed} step {step}: unreported change ({:?})",
                    log.last()
                );
                assert_eq!(run.e.revision(), rev_before);
            }
            Some(c) => {
                assert!(run.e.revision() > rev_before);
                assert_eq!(c.revision, run.e.revision());
                if let Some(s) = c.blocks {
                    assert_eq!(before.len() - s.old_len + s.new_len, after.len());
                    assert_eq!(
                        before[..s.start],
                        after[..s.start],
                        "seed {seed} step {step}"
                    );
                    assert_eq!(
                        before[s.start + s.old_len..],
                        after[s.start + s.new_len..],
                        "seed {seed} step {step}: {:?}",
                        log.last()
                    );
                } else {
                    assert_eq!(before, after);
                }
            }
        }
        run.record_versions();
    }
    while run.depth > 0 {
        run.e.end_undo_group();
        run.depth -= 1;
    }
    let final_doc = run.e.document().clone();
    if std::env::var_os("WRITER_CORE_RANDOM_STATS").is_some() {
        let d = &final_doc;
        eprintln!(
            "seed {seed}: revision {} versions {} blocks {} chars {} threads {} anchored {} undo depth {}",
            run.e.revision(),
            run.versions.len(),
            d.block_count(),
            d.plain_text().len(),
            d.comment_count(),
            d.all_comment_ranges().len(),
            run.e.undo_depth(),
        );
    }
    assert_eq!(
        Document::from_json(&final_doc.to_json()).unwrap(),
        final_doc,
        "seed {seed}"
    );
    let mut undos = 0;
    while run.e.undo() {
        undos += 1;
        run.e.check_invariants().unwrap();
        run.record_versions();
    }
    assert_eq!(
        run.e.document(),
        &original,
        "seed {seed}: undo all ({undos} steps)"
    );
    // Redo exactly what was undone (steps undone before the end of the
    // sequence may still be on the redo stack beyond that).
    for _ in 0..undos {
        assert!(run.e.redo(), "seed {seed}: redo available");
        run.e.check_invariants().unwrap();
        run.record_versions();
    }
    assert_eq!(run.e.document(), &final_doc, "seed {seed}: redo all");
}

#[test]
fn random_command_sequences() {
    for seed in 1..=seeds(150) {
        run_seed(seed, 500);
    }
}

#[test]
fn long_random_sequences() {
    run_seed(0xC0FFEE, 4000);
    run_seed(0xBADC0DE, 4000);
}

/// Every command, undone and redone on the spot, restores the document,
/// the selection and the block stamps exactly.
#[test]
fn every_command_undoes_exactly() {
    for seed in 1..=seeds(150) {
        let (mut run, _) = runner(seed ^ 0x5EED);
        for step in 0..300 {
            run.e.break_undo_group();
            let doc0 = run.e.document().clone();
            let sel0 = run.e.selection();
            let stamps0 = stamps(&doc0);
            let depth0 = run.e.undo_depth();
            let (what, undoable) = run.command();
            run.e
                .check_invariants()
                .unwrap_or_else(|err| panic!("seed {seed} step {step} {what}: {err}"));
            if undoable == Undoable::No {
                continue;
            }
            let doc1 = run.e.document().clone();
            let sel1 = run.e.selection();
            let stamps1 = stamps(&doc1);
            let k = run.e.undo_depth() - depth0;
            if doc1 != doc0 {
                assert!(
                    k >= 1,
                    "seed {seed} step {step} {what}: change without an undo step"
                );
            }
            if k == 0 || run.r.chance(30) {
                continue;
            }
            for _ in 0..k {
                assert!(run.e.undo());
                run.e.check_invariants().unwrap();
            }
            let ctx = format!("seed {seed} step {step} {what} ({k} undo steps)");
            assert_eq!(run.e.document(), &doc0, "{ctx}: undo document");
            if undoable == Undoable::Yes {
                assert_eq!(
                    run.e.selection(),
                    run.e.document().clamp_selection(sel0),
                    "{ctx}: undo selection"
                );
            }
            assert_eq!(stamps(run.e.document()), stamps0, "{ctx}: undo stamps");
            for _ in 0..k {
                assert!(run.e.redo());
                run.e.check_invariants().unwrap();
            }
            assert_eq!(run.e.document(), &doc1, "{ctx}: redo document");
            if undoable == Undoable::Yes {
                assert_eq!(run.e.selection(), sel1, "{ctx}: redo selection");
            }
            assert_eq!(stamps(run.e.document()), stamps1, "{ctx}: redo stamps");
        }
    }
}

#[test]
fn random_documents_round_trip() {
    let mut r = Rng::new(42);
    for _ in 0..200 {
        let d = random_document(&mut r);
        let json = d.to_json();
        let back = Document::from_json(&json).unwrap();
        assert_eq!(back, d);
        back.check_invariants().unwrap();
        let pretty: Document = serde_json::from_str(&d.to_json_pretty()).unwrap();
        assert_eq!(pretty, d);
        // A second round trip is byte-identical (stable output).
        assert_eq!(back.to_json(), json);
    }
}

#[test]
fn corrupted_json_never_panics() {
    let mut r = Rng::new(7);
    for _ in 0..2000 {
        let d = random_document(&mut r);
        let mut json = d.to_json().into_bytes();
        // Flip, delete or duplicate random bytes.
        for _ in 0..1 + r.below(4) {
            if json.is_empty() {
                break;
            }
            let i = r.below(json.len());
            match r.below(3) {
                0 => json[i] = *r.pick(b"0123456789{}[]\",:-tfn \\e.E"),
                1 => {
                    json.remove(i);
                }
                _ => {
                    let c = json[i];
                    json.insert(i, c);
                }
            }
        }
        let s = String::from_utf8_lossy(&json);
        if let Ok(doc) = Document::from_json(&s) {
            doc.check_invariants().unwrap();
            // A repaired document is editable and round-trips.
            let back = Document::from_json(&doc.to_json()).unwrap();
            assert_eq!(back, doc);
            let mut e = EditorState::new(doc);
            e.select_all();
            e.toggle_bold();
            e.insert_text("x");
            e.check_invariants().unwrap();
        }
        if let Ok(f) = Fragment::from_json(&s) {
            let mut e = EditorState::new(Document::new());
            e.paste(&f);
            e.check_invariants().unwrap();
        }
    }
}
