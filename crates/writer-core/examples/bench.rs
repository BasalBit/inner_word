//! Rough timings of the editing engine on book-length documents.
//!
//! ```text
//! cargo run -p writer-core --release --example bench
//! ```
//!
//! Builds documents of 1,000, 10,000 and 100,000 blocks (about 100 bytes
//! each, with headings, lists, formatting and comments) and times typing,
//! caret movement, undo, Enter, replace-all, statistics and JSON. Per-
//! keystroke costs must stay flat as the document grows.

use std::time::{Duration, Instant};

use writer_core::*;

fn build(blocks: usize) -> Document {
    let words = [
        "the", "quick", "brown", "fox", "jumps", "over", "lazy", "dog", "while", "writers",
        "keep", "typing", "chapters", "of", "a", "very", "long", "book", "café", "naïve",
    ];
    let bold = InlineAttrs {
        bold: true,
        ..Default::default()
    };
    let mut out = Vec::with_capacity(blocks);
    for i in 0..blocks {
        let mut text = String::new();
        let mut k = i;
        while text.len() < 90 {
            k = k.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            text.push_str(words[(k >> 33) % words.len()]);
            text.push(' ');
        }
        let kind = match i % 50 {
            0 => BlockKind::heading(2),
            1..=5 => BlockKind::NumberedItem,
            6..=8 => BlockKind::BulletItem,
            _ => BlockKind::Paragraph,
        };
        let cut = |n: usize| text::floor_char_boundary(&text, n);
        let (a, c) = (cut(40), cut(60));
        let b = Block::with_text(kind, &text[..a])
            .with_span(&text[a..c], bold.clone())
            .with_span(&text[c..], InlineAttrs::default());
        out.push(b);
    }
    Document::from_blocks(out)
}

fn time<R>(f: impl FnOnce() -> R) -> (R, Duration) {
    let t = Instant::now();
    let r = f();
    (r, t.elapsed())
}

fn per(d: Duration, n: usize) -> String {
    format!("{:>8.2} µs/op", d.as_secs_f64() * 1e6 / n as f64)
}

fn run(blocks: usize) {
    let (doc, t_build) = time(|| build(blocks));
    let bytes = doc.plain_text().len();
    println!("\n== {blocks} blocks, {:.2} MB of text (built in {:?})", bytes as f64 / 1e6, t_build);
    let mut e = EditorState::new(doc);
    // Comments spread through the document.
    for i in (0..blocks).step_by(97) {
        e.select_range(TextRange::in_block(i, 4, 20));
        e.add_comment("note", "me", i as i64);
    }
    let mid = blocks / 2;
    e.set_caret(Position::new(mid, 10));

    let text = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ";
    let chars: Vec<String> = text.chars().cycle().take(1000).map(|c| c.to_string()).collect();
    let (_, t) = time(|| {
        for c in &chars {
            e.insert_text(c);
        }
    });
    println!("type 1,000 chars mid-document     {}  total {:?}", per(t, 1000), t);

    let start_e = e.clone();
    let mut e0 = start_e.clone();
    e0.set_caret(Position::new(0, 0));
    let (_, t) = time(|| {
        for c in &chars {
            e0.insert_text(c);
        }
    });
    println!("type 1,000 chars at block 0       {}", per(t, 1000));

    let moves = [Movement::Right, Movement::WordRight, Movement::Left, Movement::WordLeft, Movement::LineEnd, Movement::BlockStart];
    let (_, t) = time(|| {
        for i in 0..1000 {
            if i % 10 == 9 {
                let b = (i * 7919) % blocks;
                e.set_caret(Position::new(b, 5));
            } else {
                e.move_caret(moves[i % moves.len()], i % 3 == 0);
            }
        }
    });
    println!("1,000 caret movements             {}", per(t, 1000));

    let (_, t) = time(|| {
        for _ in 0..200 {
            e.insert_paragraph_break();
            e.insert_text("x");
        }
    });
    println!("200 × (Enter + type)              {}", per(t, 200));

    let (_, t) = time(|| {
        for _ in 0..500 {
            e.undo();
        }
    });
    println!("500 undos                         {}", per(t, 500));
    let (_, t) = time(|| {
        for _ in 0..500 {
            e.redo();
        }
    });
    println!("500 redos                         {}", per(t, 500));

    let (_, t) = time(|| {
        for i in 0..1000 {
            let _ = e.format_state();
            let _ = e.document().numbered_ordinals(i % blocks..(i % blocks + 60).min(blocks));
            let _ = e.take_changes();
        }
    });
    println!("1,000 × (format_state + visible ordinals + take_changes) {}", per(t, 1000));

    let (n, t) = time(|| e.replace_all("fox", SearchOptions::default(), "wolf"));
    println!("replace_all {n} matches            {:?}", t);
    let (_, t) = time(|| e.undo());
    println!("undo replace_all                  {:?}", t);
    let (m, t) = time(|| e.document().find_all("Quick", SearchOptions::default()).len());
    println!("find_all ({m}, case-insensitive)  {:?}", t);
    let (m, t) = time(|| e.document().find_all("quick", SearchOptions { case_sensitive: true, whole_word: true }).len());
    println!("find_all ({m}, whole word)        {:?}", t);

    let (_, t) = time(|| e.document().stats());
    println!("Document::stats                   {:?}", t);
    let (_, t1) = time(|| e.stats());
    e.set_caret(Position::new(mid, 3));
    e.insert_text("z");
    let (_, t2) = time(|| e.stats());
    println!("EditorState::stats first / after one edit {:?} / {:?}", t1, t2);
    let (_, t) = time(|| e.document().all_comment_ranges().len());
    println!("all_comment_ranges                {:?}", t);
    let (_, t) = time(|| e.document().next_comment(Position::new(mid, 0), true));
    println!("next_comment                      {:?}", t);

    let (json, t) = time(|| e.document().to_json());
    println!("to_json ({:.2} MB)                {:?}", json.len() as f64 / 1e6, t);
    let (back, t) = time(|| Document::from_json(&json).unwrap());
    println!("from_json                         {:?}", t);
    assert_eq!(&back, e.document());
    let (_, t) = time(|| back.check_invariants().unwrap());
    println!("check_invariants                  {:?}", t);

    e.select_all();
    let (_, t) = time(|| e.toggle_bold());
    println!("select all + bold                 {:?}", t);
    let (_, t) = time(|| e.insert_text("gone"));
    println!("select all + type over            {:?}", t);
    let (_, t) = time(|| e.undo());
    println!("undo that                         {:?}", t);
}

fn main() {
    for n in [1_000, 10_000, 100_000] {
        run(n);
    }
}
