//! Random key sequences: nothing panics, the document and the caret stay
//! valid, and undoing everything gives the original document back.

use crate::harness::*;
use writer_vim::Key;

const BUFFERS: &[&str] = &[
    "|",
    "|hello world",
    "one two\nth|ree four\n\nfive. Six! Seven?",
    "# Title\n|para one (with [brackets] and \"quotes\")\n---\n- item a\n- item b\n{1}- nested\n===\nlast",
    "~ fn main() {↵    let x = 1;↵    pr|int(x);↵}\nafter",
    "naïve |café 日本語 かな 👩‍👩‍👧 e\u{301}\u{301} 🇦🇱\nПривет мир",
    "a↵b|c↵↵d\n\n\n> quote\n1. one\n1. two\n[x] done\n[ ] todo",
    "**bold** __it|alic__ plain **__both__**\n   indented   \n\t\ttabs",
    "|---",
    "---\n===\n|---",
    "x\n|\n\n\ny",
];

const KEYS: &[&str] = &[
    "h", "j", "k", "l", "w", "b", "e", "W", "B", "E", "0", "$", "^", "G", "g", "x", "X", "d", "D", "c", "C",
    "y", "Y", "p", "P", "u", "U", "r", "R", "J", "o", "O", "i", "I", "a", "A", "s", "S", "~", ".", "<lt>",
    ">", "v", "V", "\"", "'", "`", "m", "f", "F", "t", "T", ";", ",", "%", "{", "}", "(", ")", "*", "#", "n",
    "N", "/", "?", ":", "1", "2", "3", "5", "9", " ", "z", "Z", "&", "-", "_", "+", "|", "H", "M", "L", "q",
    "é", "👍", "[", "]", "=", "!", "@",
    "<Esc>", "<Esc>", "<Esc>", "<CR>", "<BS>", "<Del>", "<Tab>", "<Left>", "<Right>", "<Up>", "<Down>",
    "<Home>", "<End>", "<PageUp>", "<PageDown>", "<S-CR>", "<S-Right>", "<S-Left>", "<A-BS>", "<A-x>",
    "<C-r>", "<C-o>", "<C-d>", "<C-u>", "<C-w>", "<C-e>", "<C-y>", "<C-f>", "<C-b>", "<C-i>", "<C-l>",
    "<C-g>", "<C-h>", "<C-c>", "<C-[>", "<C-n>", "<C-p>", "<C-a>", "<C-k>", "<C-v>", "<C-m>", "<C-j>",
];

/// A small deterministic generator (xorshift).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn sequence(rng: &mut Rng, len: usize) -> Vec<&'static str> {
    (0..len).map(|_| KEYS[rng.below(KEYS.len())]).collect()
}

fn run_sequence(buffer: &str, keys: &[&str], width: usize) -> Result<(), String> {
    let mut s = Session::with(buffer, width, 4, Default::default());
    let original = s.ed.document().clone();
    for (i, k) in keys.iter().enumerate() {
        for key in Key::parse_sequence(k) {
            s.key(key);
        }
        s.check_invariants()
            .map_err(|e| format!("after key {i} ({k}): {e}"))?;
        // The Vim cursor is always a valid position.
        let caret = s.vim.caret(&s.ed);
        if !s.ed.document().is_valid_position(caret) {
            return Err(format!("after key {i} ({k}): invalid caret {caret:?}"));
        }
    }
    // Back to Normal mode, then undo everything.
    s.keys("<Esc><Esc><Esc>");
    s.check_invariants().map_err(|e| format!("after escape: {e}"))?;
    let mut steps = 0;
    while s.ed.undo() {
        steps += 1;
        if steps > 10_000 {
            return Err("undo does not terminate".into());
        }
    }
    if s.ed.document() != &original {
        return Err(format!(
            "undoing everything gave {:?}, not {:?}",
            render(s.ed.document(), None, None),
            render(&original, None, None)
        ));
    }
    Ok(())
}

#[test]
fn random_keys() {
    let rounds: usize = std::env::var("WRITER_VIM_FUZZ")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut failures = Vec::new();
    for round in 0..rounds {
        let buffer = BUFFERS[round % BUFFERS.len()];
        let len = 5 + rng.below(60);
        let keys = sequence(&mut rng, len);
        let width = if round % 3 == 0 { 7 } else { 1000 };
        let result = std::panic::catch_unwind(|| run_sequence(buffer, &keys, width));
        let problem = match result {
            Ok(Ok(())) => continue,
            Ok(Err(e)) => e,
            Err(_) => "PANIC".to_string(),
        };
        failures.push(format!(
            "round {round}: {problem}\n    buffer {buffer:?}\n    keys {:?}",
            keys.concat()
        ));
        if failures.len() >= 5 {
            break;
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn every_key_alone_and_after_an_operator() {
    let mut failures = Vec::new();
    for buffer in BUFFERS {
        for prefix in ["", "d", "c", "y", "v", "V", "g", "2", "i", "R", ":", "/", "\"a", ">", "gU", "z", "vi", "da", "r", "m"] {
            for key in KEYS {
                let keys = [prefix, key];
                let result = std::panic::catch_unwind(|| run_sequence(buffer, &keys, 1000));
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => failures.push(format!("{buffer:?} {prefix}{key}: {e}")),
                    Err(_) => failures.push(format!("{buffer:?} {prefix}{key}: PANIC")),
                }
            }
        }
    }
    failures.truncate(12);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
