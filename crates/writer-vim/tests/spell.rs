//! The spelling keys: `]s`, `[s`, `z=`, `zg`, `zug`, and the host they ask.

use crate::harness::*;
use writer_core::{Position, TextRange};
use writer_vim::{
    Key, Mode, SpellCommand, SpellJump, SpellLookahead, VimOptions, walk_misspellings,
};

/// What the mock host's spell checker marks.
const BAD: &[&str] = &["teh", "wrod", "recieve"];

const fn sp(start: &'static str, keys: &'static str, expect: &'static str) -> Case {
    c(start, keys, expect).misspelled(BAD)
}

fn spelling(start: &str) -> Session {
    let mut s = Session::new(start);
    s.host.misspelled = Some(BAD.iter().map(|w| w.to_string()).collect());
    s.clear_effects();
    s
}

#[test]
fn next_and_previous_misspelling() {
    run(&[
        sp("|a teh b wrod c", "]s", "a |teh b wrod c").bell(false),
        sp("a |teh b wrod c", "]s", "a teh b |wrod c"),
        sp("a t|eh b wrod c", "]s", "a teh b |wrod c"),
        sp("|a teh b wrod c", "2]s", "a teh b |wrod c"),
        // Backwards: the word the cursor is in first, then the one before.
        sp("a teh b wr|od c", "[s", "a teh b |wrod c"),
        sp("a teh b |wrod c", "[s", "a |teh b wrod c"),
        sp("a teh b wrod |c", "2[s", "a |teh b wrod c"),
        // `]S` and `[S` are the same here.
        sp("|a teh b wrod c", "]S", "a |teh b wrod c"),
        sp("a teh b wrod |c", "[S", "a teh b |wrod c"),
        // Across paragraphs, soft line breaks, lists and dividers.
        sp(
            "|one\nteh two\nthree wrod",
            "2]s",
            "one\nteh two\nthree |wrod",
        ),
        sp(
            "one\nteh two\nthree |wrod",
            "[s",
            "one\n|teh two\nthree wrod",
        ),
        sp(
            "|one↵two teh\n---\n- wrod",
            "2]s",
            "one↵two teh\n---\n- |wrod",
        ),
        sp("# |title\n> a recieve", "]s", "# title\n> a |recieve"),
        // Multibyte text before the word.
        sp("|ñandú 日本 teh", "]s", "ñandú 日本 |teh"),
        // Code is not checked.
        sp("|x\n~ teh\nwrod", "]s", "x\n~ teh\n|wrod"),
    ]);
}

#[test]
fn misspellings_wrap_around_the_document() {
    run(&[
        sp("a teh b wrod |c", "]s", "a |teh b wrod c")
            .message("search hit BOTTOM, continuing at TOP")
            .bell(false),
        sp("|a teh b wrod c", "[s", "a teh b |wrod c")
            .message("search hit TOP, continuing at BOTTOM")
            .bell(false),
        // A count larger than the number of misspelled words goes round.
        sp("|a teh b wrod c recieve", "3]s", "a teh b wrod c |recieve"),
        sp("|a teh b wrod c recieve", "4]s", "a |teh b wrod c recieve"),
        sp("|a teh b wrod c recieve", "5]s", "a teh b |wrod c recieve"),
        sp(
            "a teh b wrod c recieve |d",
            "5[s",
            "a teh b |wrod c recieve d",
        ),
        sp("|a teh\nb wrod", "999999]s", "a |teh\nb wrod"),
        // The only misspelled word, from anywhere, also from itself.
        sp("a |teh b", "]s", "a |teh b")
            .bell(false)
            .message("BOTTOM"),
        sp("a |teh b", "[s", "a |teh b").bell(false).message("TOP"),
        sp("a t|eh b", "7]s", "a |teh b"),
    ]);
}

#[test]
fn nothing_misspelled_and_no_spell_checker() {
    run(&[
        sp("|a b c", "]s", "|a b c")
            .bell(true)
            .message("No misspelled words"),
        sp("a b |c", "3[s", "a b |c")
            .bell(true)
            .message("No misspelled words"),
        sp("|a b c", "d]s", "|a b c").bell(true),
        // A host without a spell checker: the trait's defaults answer.
        c("|a teh b", "]s", "|a teh b")
            .bell(true)
            .message("E756: Spell checking is off"),
        c("a teh |b", "[s", "a teh |b")
            .bell(true)
            .message("E756: Spell checking is off"),
        c("|a teh b", "d]s", "|a teh b").bell(true).message("E756"),
        c("|a teh b", "z=", "|a teh b").bell(true).message("E756"),
        c("|a teh b", "1z=", "|a teh b").bell(true).message("E756"),
        c("a |teh b", "zg", "a |teh b").bell(true).message("E756"),
        c("a |teh b", "zug", "a |teh b").bell(true).message("E756"),
        // Keys after `]` and `[` that mean nothing.
        sp("|a teh b", "]x", "|a teh b").bell(true),
        sp("|a teh b", "[[", "|a teh b").bell(true),
        // An unfinished `]` is dropped by escape.
        sp("|a teh b", "]<Esc>x", "| teh b").bell(false),
    ]);
}

#[test]
fn misspellings_as_motions_of_operators() {
    run(&[
        sp("|a teh b wrod c", "d]s", "|teh b wrod c").regs(&[('"', "a ")]),
        sp("|a teh b wrod c", "d2]s", "|wrod c"),
        sp("|a teh b wrod c", "2d]s", "|wrod c"),
        sp("a teh b |wrod c", "d[s", "a |wrod c"),
        sp("|a teh b wrod c", "y]s", "|a teh b wrod c").regs(&[('0', "a ")]),
        sp("|a teh b wrod c", "c]sx <Esc>", "x| teh b wrod c"),
        sp("|a teh b wrod c", "gU]s", "|A teh b wrod c"),
        // Exclusive: the misspelled word itself is not taken, also when the
        // motion wraps backwards over the text before the cursor.
        sp("a teh |b", "d]s", "a |b"),
        // Repeated with `.`, with the count it had or a new one.
        sp("|a teh b wrod c", "d]s.", "|wrod c"),
        sp("|a teh b wrod c recieve", "d]s2.", "|recieve"),
        // Undone in one step.
        sp("|a teh b wrod c", "d2]su", "|a teh b wrod c"),
    ]);
}

#[test]
fn misspellings_in_visual_mode() {
    run(&[
        sp("|a teh b wrod c", "v]s", "«a |teh b wrod c").visual(),
        sp("|a teh b wrod c", "v2]s", "«a teh b |wrod c").visual(),
        sp("a teh b |wrod c", "v[s", "a |teh b «wrod c").visual(),
        sp("|a teh b wrod c", "v]sd", "|eh b wrod c"),
        sp("|one\nteh\ntwo", "V]s", "«one\n|teh\ntwo").vline(),
        sp("|one\nteh\ntwo", "V]sd", "|two"),
    ]);
}

#[test]
fn the_engine_asks_what_the_walk_asks() {
    // The questions of `]s` with a count are those of `walk_misspellings`:
    // a host may ask them ahead of the call.
    for (start, keys, forward, count) in [
        ("|a teh b wrod c recieve", "]s", true, 1),
        ("a teh b wr|od c recieve", "3]s", true, 3),
        ("a teh b wr|od c recieve", "8[s", false, 8),
        ("|a b", "4]s", true, 4),
        ("a |teh", "999]s", true, 999),
    ] {
        let mut s = spelling(start);
        let from = s.ed.selection().head;
        let all = s.host.misspellings(s.ed.document());
        let mut asked = Vec::new();
        let expected = walk_misspellings(from, forward, count, |p| {
            asked.push((p, forward));
            if forward {
                all.iter().find(|r| r.start > p).or(all.first()).copied()
            } else {
                all.iter()
                    .rev()
                    .find(|r| r.start < p)
                    .or(all.last())
                    .copied()
            }
        });
        s.keys(keys);
        assert_eq!(s.host.spell_questions, asked, "{start} {keys}");
        assert!(
            asked.len() <= all.len() + 1,
            "{start} {keys}: one round at most"
        );
        if let Some(SpellJump { range, .. }) = expected {
            assert_eq!(s.ed.selection().head, range.start, "{start} {keys}");
        }
    }
}

#[test]
fn walk_misspellings_counts_and_wraps() {
    let r = |s: usize| TextRange::in_block(0, s, s + 2);
    let all = [r(2), r(10), r(20)];
    let next = |p: Position| all.iter().find(|x| x.start > p).or(all.first()).copied();
    let walk = |from: usize, count| walk_misspellings(Position::new(0, from), true, count, next);
    assert_eq!(
        walk(0, 1),
        Some(SpellJump {
            range: r(2),
            wrapped: false
        })
    );
    assert_eq!(
        walk(0, 3),
        Some(SpellJump {
            range: r(20),
            wrapped: false
        })
    );
    assert_eq!(
        walk(0, 4),
        Some(SpellJump {
            range: r(2),
            wrapped: true
        })
    );
    assert_eq!(
        walk(5, 2),
        Some(SpellJump {
            range: r(20),
            wrapped: false
        })
    );
    assert_eq!(
        walk(5, 3),
        Some(SpellJump {
            range: r(2),
            wrapped: true
        })
    );
    assert_eq!(
        walk(25, 1),
        Some(SpellJump {
            range: r(2),
            wrapped: true
        })
    );
    assert_eq!(walk(0, 0), walk(0, 1), "a count of 0 is 1");
    assert_eq!(
        walk(5, 302),
        Some(SpellJump {
            range: r(20),
            wrapped: true
        })
    );
    assert_eq!(walk_misspellings(Position::ZERO, true, 3, |_| None), None);
    let prev = |p: Position| {
        all.iter()
            .rev()
            .find(|x| x.start < p)
            .or(all.last())
            .copied()
    };
    let back = |from: usize, count| walk_misspellings(Position::new(0, from), false, count, prev);
    assert_eq!(
        back(11, 1),
        Some(SpellJump {
            range: r(10),
            wrapped: false
        })
    );
    assert_eq!(
        back(10, 1),
        Some(SpellJump {
            range: r(2),
            wrapped: false
        })
    );
    assert_eq!(
        back(2, 1),
        Some(SpellJump {
            range: r(20),
            wrapped: true
        })
    );
    assert_eq!(
        back(11, 5),
        Some(SpellJump {
            range: r(2),
            wrapped: true
        })
    );
}

#[test]
fn spelling_commands_go_to_the_host() {
    let at = Position::new(0, 3);
    let table: &[(&str, &str, SpellCommand)] = &[
        ("a t|eh b", "z=", SpellCommand::Suggest { at, pick: None }),
        (
            "a t|eh b",
            "1z=",
            SpellCommand::Suggest { at, pick: Some(1) },
        ),
        (
            "a t|eh b",
            "12z=",
            SpellCommand::Suggest { at, pick: Some(12) },
        ),
        (
            "a t|eh b",
            "zg",
            SpellCommand::MarkGood {
                at,
                word: "teh".into(),
            },
        ),
        (
            "a t|eh b",
            "zug",
            SpellCommand::UndoMarkGood {
                at,
                word: "teh".into(),
            },
        ),
        // The word as the engine sees words: with its apostrophe, and the
        // next one when the cursor is on a blank.
        (
            "a d|on't b",
            "zg",
            SpellCommand::MarkGood {
                at,
                word: "don't".into(),
            },
        ),
        (
            "ab |  ñandú",
            "zg",
            SpellCommand::MarkGood {
                at,
                word: "ñandú".into(),
            },
        ),
    ];
    for (start, keys, expected) in table {
        let mut s = spelling(start);
        s.keys(keys);
        assert_eq!(
            s.host.spell_commands,
            std::slice::from_ref(expected),
            "{start} {keys}"
        );
        assert_eq!(s.bells(), 0, "{start} {keys}");
        assert_eq!(s.mode(), Mode::Normal);
        assert_eq!(s.vim.pending_keys(), "", "{start} {keys}");
        assert_eq!(s.buffer(), *start, "the engine changes nothing itself");
        s.check_invariants().unwrap();
    }
}

#[test]
fn spelling_commands_that_are_refused() {
    // Marking a word as wrong is not offered.
    for keys in ["zw", "zuw"] {
        let mut s = spelling("a t|eh b");
        s.keys(keys);
        assert!(s.host.spell_commands.is_empty());
        assert_eq!(s.bells(), 1, "{keys}");
        assert!(
            s.messages().iter().any(|m| m.contains("not supported")),
            "{keys}: {:?}",
            s.messages()
        );
        assert_eq!(s.vim.pending_keys(), "");
    }
    // No word to learn.
    let mut s = spelling("|   ");
    s.keys("zg");
    assert!(s.host.spell_commands.is_empty());
    assert!(s.messages().iter().any(|m| m.contains("E348")));
    let mut s = spelling("a |...");
    s.keys("zug");
    assert!(s.host.spell_commands.is_empty());
    assert_eq!(s.bells(), 1);
    // An unknown key after `zu`.
    let mut s = spelling("a t|eh b");
    s.keys("zux");
    assert!(s.host.spell_commands.is_empty());
    assert_eq!(s.bells(), 1);
    assert_eq!(s.buffer(), "a t|eh b");
    // `zu` shows as pending and escape drops it.
    let mut s = spelling("a t|eh b");
    s.keys("zu");
    assert_eq!(s.vim.pending_keys(), "zu");
    s.keys("<Esc>");
    assert_eq!(s.vim.pending_keys(), "");
    assert_eq!(s.bells(), 0);
    // A read-only document: nothing is replaced, the menu may still open
    // and words may still be learned.
    let read_only = VimOptions {
        read_only: true,
        ..VimOptions::default()
    };
    let mut s = Session::with("a t|eh b", 1000, 10, read_only);
    s.host.misspelled = Some(vec!["teh".into()]);
    s.clear_effects();
    s.keys("1z=");
    assert!(s.host.spell_commands.is_empty());
    assert!(s.messages().iter().any(|m| m.contains("E21")));
    s.keys("z=zg");
    assert_eq!(s.host.spell_commands.len(), 2);
}

#[test]
fn spelling_commands_in_visual_mode_are_for_the_cursor() {
    let mut s = spelling("a |teh b");
    s.keys("viwz=");
    assert_eq!(s.mode(), Mode::Normal);
    assert_eq!(
        s.host.spell_commands,
        [SpellCommand::Suggest {
            at: Position::new(0, 4),
            pick: None
        }]
    );
    assert_eq!(s.buffer(), "a te|h b");
    let mut s = spelling("a |teh b");
    s.keys("viwzg");
    assert_eq!(
        s.host.spell_commands,
        [SpellCommand::MarkGood {
            at: Position::new(0, 4),
            word: "teh".into()
        }]
    );
    assert_eq!(s.mode(), Mode::Normal);
}

#[test]
fn the_scroll_keys_behind_z_still_work() {
    let mut s = spelling("a |teh b");
    s.keys("zzztzb");
    assert_eq!(s.host.scrolls.len(), 3);
    assert!(s.host.spell_commands.is_empty());
    assert_eq!(s.bells(), 0);
}

#[test]
fn spell_lookahead_foresees_the_questions() {
    let key = |k: &str| Key::parse_sequence(k)[0];
    let mut s = spelling("a t|eh b wrod");
    let at = Position::new(0, 3);
    assert_eq!(s.vim.spell_lookahead(key("s"), &s.ed), None);
    assert_eq!(s.vim.spell_lookahead(key("="), &s.ed), None);
    s.keys("3]");
    assert_eq!(
        s.vim.spell_lookahead(key("s"), &s.ed),
        Some(SpellLookahead::Misspellings {
            from: at,
            forward: true,
            count: 3
        })
    );
    assert_eq!(s.vim.spell_lookahead(key("x"), &s.ed), None);
    assert_eq!(s.vim.spell_lookahead(key("<C-s>"), &s.ed), None);
    s.keys("<Esc>2d3[");
    assert_eq!(
        s.vim.spell_lookahead(key("S"), &s.ed),
        Some(SpellLookahead::Misspellings {
            from: at,
            forward: false,
            count: 6
        })
    );
    s.keys("<Esc>z");
    assert_eq!(
        s.vim.spell_lookahead(key("="), &s.ed),
        Some(SpellLookahead::Command)
    );
    assert_eq!(
        s.vim.spell_lookahead(key("g"), &s.ed),
        Some(SpellLookahead::Command)
    );
    assert_eq!(s.vim.spell_lookahead(key("z"), &s.ed), None);
    assert_eq!(s.vim.spell_lookahead(key("u"), &s.ed), None);
    s.keys("u");
    assert_eq!(
        s.vim.spell_lookahead(key("g"), &s.ed),
        Some(SpellLookahead::Command)
    );
    assert_eq!(s.vim.spell_lookahead(key("w"), &s.ed), None);
    s.keys("<Esc>");
    // In Visual mode the walk starts at the cursor end.
    s.keys("vl]");
    assert_eq!(
        s.vim.spell_lookahead(key("s"), &s.ed),
        Some(SpellLookahead::Misspellings {
            from: Position::new(0, 4),
            forward: true,
            count: 1
        })
    );
    s.keys("<Esc><Esc>");
    // A change made with the motion is repeated with it.
    assert_eq!(s.vim.spell_lookahead(key("."), &s.ed), None);
    s.keys("0d]s");
    assert_eq!(s.buffer(), "|teh b wrod");
    assert_eq!(
        s.vim.spell_lookahead(key("."), &s.ed),
        Some(SpellLookahead::Misspellings {
            from: Position::ZERO,
            forward: true,
            count: 1
        })
    );
    s.keys("4");
    assert_eq!(
        s.vim.spell_lookahead(key("."), &s.ed),
        Some(SpellLookahead::Misspellings {
            from: Position::ZERO,
            forward: true,
            count: 4
        })
    );
    s.keys("<Esc>");
    // Typing is not a command.
    s.keys("i");
    assert_eq!(s.vim.spell_lookahead(key("."), &s.ed), None);
    s.keys("<Esc>:");
    assert_eq!(s.vim.spell_lookahead(key("."), &s.ed), None);
}
