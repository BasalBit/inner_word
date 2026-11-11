//! Differential test against a real Vim.
//!
//! `tests/fixtures/vim_corpus.tsv` holds key sequences with the buffer,
//! cursor and registers a real Vim (9.1, `-u NONE -N`) ended with; it is
//! written by `tests/fixtures/gen_vim_corpus.py`, which runs every case in
//! its own Vim process. This test replays the sequences through the engine,
//! over plain paragraphs (one per line) and a host that never wraps, and
//! compares. Vim is not needed to run it.
//!
//! Where the engine differs from Vim on purpose, [`deviation`] names the
//! rule; such cases are skipped and counted. Everything else must match.
//!
//! Environment variables for investigating:
//! - `VIMDIFF_CORPUS=path`: replay another corpus file;
//! - `VIMDIFF_DUMP=path`: write every difference to that file.

use writer_core::Position;
use writer_vim::VimOptions;

use crate::harness::Session;

const REGISTERS: [char; 7] = ['"', '0', '1', '2', '-', 'a', 'b'];

struct VimCase {
    start: Vec<String>,
    line: usize,
    col: usize,
    keys: String,
    lines: Vec<String>,
    end_line: usize,
    end_col: usize,
    /// Type (`v`, `V`, or empty for an unset register) and text.
    regs: Vec<(String, String)>,
    /// What the generator noted about the case.
    tags: Vec<String>,
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

/// Splits an escaped list of lines at the `\n` escapes.
fn unescape_lines(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            cur.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push(std::mem::take(&mut cur)),
            Some('t') => cur.push('\t'),
            Some('r') => cur.push('\r'),
            Some(o) => cur.push(o),
            None => {}
        }
    }
    out.push(cur);
    out
}

fn parse(corpus: &str) -> Vec<VimCase> {
    corpus
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            assert!(
                f.len() == 8 + 2 * REGISTERS.len(),
                "malformed corpus line: {l:?}"
            );
            VimCase {
                start: unescape_lines(f[0]),
                line: f[1].parse().unwrap(),
                col: f[2].parse().unwrap(),
                keys: unescape(f[3]),
                lines: unescape_lines(f[4]),
                end_line: f[5].parse().unwrap(),
                end_col: f[6].parse().unwrap(),
                regs: (0..REGISTERS.len())
                    .map(|i| (f[7 + 2 * i].to_string(), unescape(f[8 + 2 * i])))
                    .collect(),
                tags: f[7 + 2 * REGISTERS.len()]
                    .split(',')
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .collect(),
            }
        })
        .collect()
}

/// What the engine ended with, in the corpus's terms.
struct Got {
    lines: Vec<String>,
    line: usize,
    col: usize,
    regs: Vec<(String, String)>,
}

fn replay(case: &VimCase) -> Got {
    let lines: Vec<&str> = case.start.iter().map(String::as_str).collect();
    let options = VimOptions {
        // Vim's own word rule, for the comparison.
        apostrophe_in_words: false,
        ..VimOptions::default()
    };
    let mut s = Session::plain(&lines, Position::new(case.line - 1, case.col - 1), options);
    s.vim_arrows = true;
    s.keys(&case.keys);
    // The generator ends every sequence with an escape.
    s.keys("<Esc>");
    let caret = s.vim.caret(&s.ed);
    Got {
        lines: s
            .ed
            .document()
            .blocks()
            .iter()
            .map(|b| b.text().to_string())
            .collect(),
        line: caret.block + 1,
        col: caret.offset + 1,
        regs: REGISTERS
            .iter()
            .map(|r| match s.vim.register(*r) {
                Some(c) => (
                    if c.linewise { "V" } else { "v" }.to_string(),
                    c.to_plain_text(),
                ),
                None => (String::new(), String::new()),
            })
            .collect(),
    }
}

/// The differences between Vim and the engine for one case.
fn differences(case: &VimCase, got: &Got) -> Vec<String> {
    let mut d = Vec::new();
    if got.lines != case.lines {
        d.push(format!(
            "buffer: vim {:?}\n         got {:?}",
            case.lines, got.lines
        ));
    }
    if (got.line, got.col) != (case.end_line, case.end_col) {
        d.push(format!(
            "cursor: vim {}:{}  got {}:{}",
            case.end_line, case.end_col, got.line, got.col
        ));
    }
    for (i, name) in REGISTERS.iter().enumerate() {
        let (vt, vx) = &case.regs[i];
        let (gt, gx) = &got.regs[i];
        // An unset register and an empty characterwise one read the same.
        let empty = |t: &str, x: &str| x.is_empty() && t != "V";
        if (vt, vx) != (gt, gx) && !(empty(vt, vx) && empty(gt, gx)) {
            d.push(format!("register {name}: vim {vt}{vx:?}  got {gt}{gx:?}"));
        }
    }
    d
}

/// The rule under which the engine differs from Vim on purpose for this
/// case, if any.
fn deviation(case: &VimCase) -> Option<&'static str> {
    let has = |tag: &str| case.tags.iter().any(|t| t == tag);
    let undoes =
        case.keys.contains('u') || case.keys.contains("<C-r>") || case.keys.contains(":red");
    if has("noop-undo") && undoes {
        return Some(
            "a command that leaves the text as it was (`r` with the same character, `gU` \
             on capitals) is an undo step in Vim and none in the engine",
        );
    }
    None
}

/// Replays `corpus`: the cases that differ from Vim (with what differs),
/// and how many were skipped as intentional deviations.
fn run_corpus(cases: &[VimCase]) -> (Vec<(usize, String)>, usize) {
    let mut differing = Vec::new();
    let mut skipped = 0usize;
    for (i, case) in cases.iter().enumerate() {
        if deviation(case).is_some() {
            skipped += 1;
            continue;
        }
        let result = std::panic::catch_unwind(|| {
            let got = replay(case);
            differences(case, &got)
        });
        let diffs = match result {
            Ok(d) => d,
            Err(_) => vec!["PANIC".to_string()],
        };
        if !diffs.is_empty() {
            differing.push((
                i,
                format!(
                    "keys {:?}  at {}:{}  in {:?}\n    {}\n",
                    case.keys,
                    case.line,
                    case.col,
                    case.start,
                    diffs.join("\n    ")
                ),
            ));
        }
    }
    (differing, skipped)
}

#[test]
fn engine_matches_vim() {
    let path = std::env::var("VIMDIFF_CORPUS").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/vim_corpus.tsv").to_string()
    });
    let corpus = std::fs::read_to_string(&path).expect("corpus file");
    let cases = parse(&corpus);
    assert!(!cases.is_empty());
    let (differing, skipped) = run_corpus(&cases);
    let report: String = differing.iter().map(|(_, r)| format!("{r}\n")).collect();
    if let Ok(dump) = std::env::var("VIMDIFF_DUMP") {
        std::fs::write(&dump, &report).expect("dump file");
    }
    if let Ok(dir) = std::env::var("VIMDIFF_SPLIT") {
        // For curating the fixtures: the corpus lines the engine agrees
        // with, and the ones it does not.
        let lines: Vec<&str> = corpus
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        let bad: std::collections::HashSet<usize> = differing.iter().map(|(i, _)| *i).collect();
        let pick = |want: bool| -> String {
            lines
                .iter()
                .enumerate()
                .filter(|(i, _)| bad.contains(i) == want)
                .map(|(_, l)| format!("{l}\n"))
                .collect()
        };
        std::fs::write(format!("{dir}/matching.tsv"), pick(false)).expect("split file");
        std::fs::write(format!("{dir}/differing.tsv"), pick(true)).expect("split file");
    }
    if !differing.is_empty() {
        let shown: String = differing
            .iter()
            .take(40)
            .map(|(_, r)| format!("{r}\n"))
            .collect();
        panic!(
            "{} of {} cases differ from Vim ({skipped} skipped as intentional):\n\n{shown}\n",
            differing.len(),
            cases.len()
        );
    }
}

/// `tests/fixtures/vim_deviations.tsv` holds cases in which the engine is
/// known to end differently from Vim, grouped under `# reason` lines. They
/// document the deviations with real examples; here they only have to run
/// without a panic and leave the editor consistent.
#[test]
fn known_deviations_run_cleanly() {
    let corpus = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/vim_deviations.tsv"
    ))
    .expect("deviations file");
    let cases = parse(&corpus);
    let reasons = corpus.lines().filter(|l| l.starts_with("# ")).count();
    assert!(reasons > 0 && !cases.is_empty());
    let mut failures = Vec::new();
    for case in &cases {
        let ok = std::panic::catch_unwind(|| {
            let lines: Vec<&str> = case.start.iter().map(String::as_str).collect();
            let mut s = Session::plain(
                &lines,
                Position::new(case.line - 1, case.col - 1),
                VimOptions {
                    apostrophe_in_words: false,
                    ..VimOptions::default()
                },
            );
            s.vim_arrows = true;
            s.keys(&case.keys);
            s.keys("<Esc>");
            s.check_invariants()
        });
        match ok {
            Ok(Ok(())) => {}
            Ok(Err(e)) => failures.push(format!("{:?}: {e}", case.keys)),
            Err(_) => failures.push(format!("{:?}: PANIC", case.keys)),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
