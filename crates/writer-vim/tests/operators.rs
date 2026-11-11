//! Operators with motions, and the simple change commands.

use crate::harness::*;

#[test]
fn delete_characters() {
    run(&[
        c("a|bc", "x", "a|c").regs(&[('"', "b"), ('-', "b")]),
        c("ab|c", "x", "a|b"),
        c("|abc", "2x", "|c"),
        c("a|bc", "9x", "|a"),
        c("ab|c", "X", "a|c"),
        c("ab|c", "5X", "|c"),
        c("|abc", "X", "|abc").bell(true),
        c("|", "x", "|").bell(true),
        // x never joins lines.
        c("a|b\ncd", "3x", "|a\ncd"),
        c("a|ñb", "x", "a|b"),
        c("|👩‍👩‍👧x", "x", "|x"),
        c("|e\u{301}x", "x", "|x"),
        c("a|bc", "<Del>", "a|c"),
        c("a|bc", "dl", "a|c"),
        c("ab|c", "dl", "a|b"),
        c("ab|c", "dh", "a|c"),
        c("a|bc", "d<Space>", "a|c"),
        c("ab|cd", "d2h", "|cd"),
    ]);
}

#[test]
fn delete_words() {
    run(&[
        c("|foo bar", "dw", "|bar"),
        c("foo |bar baz", "dw", "foo |baz"),
        c("foo |bar", "dw", "foo| "),
        // The last word of a line does not take the line break along.
        c("foo |bar\nbaz", "dw", "foo| \nbaz"),
        c("|a b c d", "d2w", "|c d"),
        c("|a b c d", "2dw", "|c d"),
        c("|a b c d e", "2d2w", "|e"),
        c("foo|   bar", "dw", "foo|bar"),
        c("|foo.bar", "dw", "|.bar"),
        c("|foo.bar baz", "dW", "|baz"),
        c("|foo bar", "de", "| bar"),
        c("|foo.bar baz", "dE", "| baz"),
        c("foo |bar", "db", "|bar"),
        c("foo ba|r", "db", "foo |r"),
        c("foo ba|r", "dge", "f|o"),
        c("foo.bar ba|z", "dB", "foo.bar |z"),
        // Across a block boundary the blocks merge.
        c("foo ba|r\nbaz qux", "d2w", "foo ba|qux"),
        // A lone word and its trailing blanks: the line goes.
        c("|foo  \nbar", "dw", "|bar"),
        c("|\nbar", "dw", "|bar"),
        c("|naïve café", "dw", "|café"),
        c("|don't stop", "dw", "|stop"),
    ]);
}

#[test]
fn delete_in_line() {
    run(&[
        c("foo |bar baz", "D", "foo| "),
        c("foo |bar baz", "d$", "foo| "),
        c("|foo", "D", "|"),
        c("a|bc\ndef\nghi", "2D", "|a\nghi"),
        c("foo |bar", "d0", "|bar"),
        c("  foo |bar", "d^", "  |bar"),
        c("|hello world", "dfo", "| world"),
        c("|hello world", "dto", "|o world"),
        c("hello wor|ld", "dFo", "hello w|ld"),
        c("hello wor|ld", "dTo", "hello wo|ld"),
        c("|a.b.c", "f.d;", "a|c"),
        c("a |(b c) d", "d%", "a | d"),
        c("a (b c|) d", "d%", "a | d"),
        c("ab↵c|d↵ef", "D", "ab↵|c↵ef"),
        c("ab↵c|d↵ef", "d0", "ab↵|d↵ef"),
        c("|abc", "d|", "|abc"),
        c("abc|def", "d2|", "a|def"),
        c("|hello", "dfz", "|hello").bell(true),
    ]);
}

#[test]
fn delete_lines() {
    run(&[
        c("one\n|two\nthree", "dd", "one\n|three").regs(&[('"', "two\n"), ('1', "two\n")]),
        c("a\n|b\nc\nd", "2dd", "a\n|d"),
        c("a\n|b\nc\nd", "d2d", "a\n|d"),
        c("a\n|b", "dd", "|a"),
        c("|a", "dd", "|"),
        c("a\n|b\nc", "5dd", "|a"),
        c("|a\n  b", "dd", "  |b"),
        c("a\n|b\nc", "dG", "|a"),
        c("a\n|b\nc", "dgg", "|c"),
        c("a\n|b\nc\nd", "d3G", "a\n|d"),
        c("|a\nb\nc", "dj", "|c"),
        c("a\nb\n|c", "dk", "|a"),
        c("a\n|b", "dj", "a\n|b").bell(true),
        c("|a\nb", "dk", "|a\nb").bell(true),
        c("|a\nb\nc\nd", "d2j", "|d"),
        c("a\n|b\nc\nd", "majd'a", "a\n|d"),
        c("ab|cd\nefgh", "majd`a", "ab|gh"),
        // j as an operator motion means blocks, however long they wrap.
        c("ab|cdefghij\nxyz\nend", "dj", "|end").width(5),
        c("|a\nb\nc\nd\ne", "4dd", "|e").message("4 fewer lines"),
        c("a\n|b\nc", "d+", "|a"),
        c("a\n|b\nc", "d-", "|c"),
        c("a\n|b\nc", "d_", "a\n|c"),
    ]);
}

#[test]
fn delete_paragraphs_and_sentences() {
    run(&[
        c("|a\nb\n\nc", "d}", "|\nc"),
        c("a|x\nb\n\nc", "d}", "|a\n\nc"),
        // From the start of a line only the blank line above goes (the
        // range ends in column one, so it becomes linewise).
        c("a\nb\n\n|c\nd", "d{", "a\nb\n|c\nd"),
        c("a\nb\n\nc\n|d", "d{", "a\nb\n|d"),
        c("a\nb\n\nc\nd|e", "d{", "a\nb\n|e"),
        c("a\n|b\n\nc", "d}", "a\n|\nc"),
        c("|One. Two. Three.", "d)", "|Two. Three."),
        c("One. Two. |Three.", "d(", "One. |Three."),
        c("One. T|wo. Three.", "d)", "One. T|Three."),
        c("|a\nb", "d}", "|"),
    ]);
}

#[test]
fn change() {
    run(&[
        c("|foo bar", "cwnew<Esc>", "ne|w bar"),
        // cw on a word stops at its end.
        c("fo|o bar", "cwX<Esc>", "fo|X bar"),
        c("foo|  bar", "cwX<Esc>", "foo|Xbar"),
        c("|a b c", "c2wX<Esc>", "|X c"),
        c("|foo.bar baz", "cWX<Esc>", "|X baz"),
        c("|foo bar", "cw<Esc>", "| bar"),
        c("foo |bar baz", "ceX<Esc>", "foo |X baz"),
        c("foo |bar", "cbX<Esc>", "|Xbar"),
        c("foo |bar", "CX<Esc>", "foo |X"),
        c("foo |bar", "c$X<Esc>", "foo |X"),
        c("foo |bar", "c0X<Esc>", "|Xbar"),
        c("|hello world", "cfoX<Esc>", "|X world"),
        c("|hello world", "ctoX<Esc>", "|Xo world"),
        c("|foo", "sX<Esc>", "|Xoo"),
        c("|foo", "2sX<Esc>", "|Xo"),
        c("f|oo", "9sX<Esc>", "f|X"),
        c("|foo", "cw", "|").insert(),
        c("|foo bar", "cw", "| bar").insert().regs(&[('"', "foo")]),
        c("|foo\nbar", "c2wX<Esc>", "|X"),
    ]);
}

#[test]
fn change_lines() {
    run(&[
        c("a\n|foo\nb", "ccX<Esc>", "a\n|X\nb").regs(&[('"', "foo\n")]),
        c("|foo", "SX<Esc>", "|X"),
        c("|a\nb\nc", "2ccX<Esc>", "|X\nc"),
        c("|a\nb\nc", "cjX<Esc>", "|X\nc"),
        c("a\nb\n|c", "ckX<Esc>", "a\n|X"),
        // The block stays what it was.
        c("# |Title\nb", "ccNew<Esc>", "# Ne|w\nb"),
        c("- |one\n- two", "ccx<Esc>", "- |x\n- two"),
        c("{2}- |one", "ccx<Esc>", "{2}- |x"),
        c("|foo", "cc", "|").insert(),
        c("|", "ccX<Esc>", "|X"),
        c("a\n|b\nc", "cGX<Esc>", "a\n|X"),
        c("|a\nb\n\nc", "c}X<Esc>", "|X\n\nc"),
    ]);
}

#[test]
fn yank() {
    run(&[
        c("|foo bar", "yw", "|foo bar").regs(&[('"', "foo "), ('0', "foo ")]),
        c("foo |bar", "yy", "foo |bar").regs(&[('"', "foo bar\n"), ('0', "foo bar\n")]),
        c("foo |bar", "Y", "foo |bar").regs(&[('"', "foo bar\n")]),
        c("foo |bar baz", "y$", "foo |bar baz").regs(&[('"', "bar baz")]),
        // A backward yank leaves the caret at the start of the text.
        c("foo |bar", "yb", "|foo bar").regs(&[('"', "foo ")]),
        c("foo ba|r", "y0", "|foo bar").regs(&[('"', "foo ba")]),
        c("|a\nb\nc", "yj", "|a\nb\nc").regs(&[('"', "a\nb\n")]),
        c("a\n|b", "yk", "|a\nb").regs(&[('"', "a\nb\n")]),
        c("|a\nb\nc", "2yy", "|a\nb\nc").regs(&[('"', "a\nb\n")]),
        c("|a\nb\nc", "2Y", "|a\nb\nc").regs(&[('"', "a\nb\n")]),
        c("a\n|b\nc", "yG", "a\n|b\nc").regs(&[('"', "b\nc\n")]),
        c("a\n|b\nc", "ygg", "|a\nb\nc").regs(&[('"', "a\nb\n")]),
        c("|a\nb\nc", "3yy", "|a\nb\nc").message("3 lines yanked"),
        c("|hello world", "yfo", "|hello world").regs(&[('"', "hello")]),
        c("|hello world", "yto", "|hello world").regs(&[('"', "hell")]),
        c("a |(b c) d", "y%", "a |(b c) d").regs(&[('"', "(b c)")]),
        c("fo|o\nbar", "yw", "fo|o\nbar").regs(&[('"', "o")]),
        c("|a\nb\n\nc", "y}", "|a\nb\n\nc").regs(&[('"', "a\nb\n")]),
        c("|añb", "yl", "|añb").regs(&[('"', "a")]),
        c("a|ñb", "yl", "a|ñb").regs(&[('"', "ñ")]),
    ]);
}

#[test]
fn shift() {
    run(&[
        c("|a\nb", ">>", "{1}|a\nb"),
        c("{2}|a", "<<", "{1}|a"),
        c("|a\nb\nc", "2>>", "{1}|a\n{1}b\nc"),
        c("|a\nb\nc", ">j", "{1}|a\n{1}b\nc"),
        c("a\n|b\nc", ">k", "{1}|a\n{1}b\nc"),
        c("|a", "<<", "|a"),
        c("|a\nb\n\nc", ">ip", "{1}|a\n{1}b\n\nc"),
        c("|foo bar", ">w", "{1}|foo bar"),
        c("{8}|a", ">>", "{8}|a"),
        c("  a|b", ">>", "{1}  |ab"),
        c("{1}a\n{1}|b\n{1}c", "<G", "{1}a\n|b\nc"),
        c("- |one\n- two", ">>", "{1}- |one\n- two"),
        c("|a\nb\nc\nd", "3>>", "{1}|a\n{1}b\n{1}c\nd").message("3 lines >ed 1 time"),
        c("|a\nb", ">}", "{1}|a\n{1}b"),
    ]);
}

#[test]
fn change_case() {
    run(&[
        c("|abc", "~", "A|bc"),
        c("|abc", "3~", "AB|C"),
        c("|aBc", "9~", "Ab|C"),
        c("ab|c", "~", "ab|C"),
        c("|1a", "~", "1|a"),
        c("|", "~", "|").bell(true),
        c("|foo bar", "g~w", "|FOO bar"),
        c("|foo bar", "gUw", "|FOO bar"),
        c("|FOO bar", "guw", "|foo bar"),
        c("fo|o bar", "gUiw", "|FOO bar"),
        c("fo|o Bar", "g~~", "FO|O bAR"),
        c("fo|o bar", "gUU", "FO|O BAR"),
        c("FO|O", "guu", "fo|o"),
        c("fo|o Bar", "g~g~", "FO|O bAR"),
        c("fo|o", "gUgU", "FO|O"),
        c("FO|O", "gugu", "fo|o"),
        c("|a\nb\nc", "gUj", "|A\nB\nc"),
        c("a\n|b\nc", "gUk", "|A\nB\nc"),
        c("|a\nb\nc", "2gUU", "|A\nB\nc"),
        c("fo|o bar", "gU$", "fo|O BAR"),
        // Characters that change length.
        c("|straße", "gUU", "|STRASSE"),
        c("|ǆ", "~", "|Ǆ"),
        c("|élan", "gUiw", "|ÉLAN"),
        c("|ÉLAN", "4~", "éla|n"),
    ]);
}

#[test]
fn replace_character() {
    run(&[
        c("|abc", "rx", "|xbc"),
        c("|abc", "2rx", "x|xc"),
        c("|abc", "3rx", "xx|x"),
        c("|abc", "4rx", "|abc").bell(true),
        c("ab|c", "rx", "ab|x"),
        c("|a b", "r<CR>", "\n| b"),
        c("a|b c", "2r<CR>", "a\n|c"),
        c("|añb", "lrx", "a|xb"),
        c("|abc", "rñ", "|ñbc"),
        c("|abc", "3rñ", "ññ|ñ"),
        c("|abc", "r<Esc>", "|abc"),
        c("|", "rx", "|").bell(true),
        c("|a\nb", "2rx", "|a\nb").bell(true),
        c("|👩‍👩‍👧b", "rx", "|xb"),
        c("|abc", "r ", "| bc"),
    ]);
}

#[test]
fn join() {
    run(&[
        c("|a\nb", "J", "a| b"),
        c("|a\n  b", "J", "a| b"),
        c("|a \nb", "J", "a |b"),
        c("|a\n)", "J", "a|)"),
        c("|a\n\nb", "J", "|a\nb"),
        c("|a\nb\nc", "3J", "a b| c"),
        c("|a\nb\nc", "JJ", "a b| c"),
        c("|\nb", "J", "|b"),
        c("a\n|b", "J", "a\n|b").bell(true),
        c("|a\nb\nc", "2J", "a| b\nc"),
        c("|a\nb", "5J", "a| b"),
        c("|a\n  b", "gJ", "a|  b"),
        c("|a\nb\nc", "3gJ", "ab|c"),
        // The first block keeps what it is.
        c("# |Title\nrest", "J", "# Title| rest"),
        c("- |one\ntwo", "J", "- one| two"),
        c("ab|c\ndef", "J", "abc| def"),
    ]);
}

#[test]
fn counts_on_both_sides() {
    run(&[
        c("|a b c d e f g", "2d3w", "|g"),
        c("|a b c d e f g", "3d2w", "|g"),
        c("|abcdefgh", "2d2l", "|efgh"),
        c("|a\nb\nc\nd\ne", "2d2d", "|e"),
        c("|abcdefgh", "2y3l", "|abcdefgh").regs(&[('"', "abcdef")]),
        c("|a b c d e f g", "2c3wX<Esc>", "|X g"),
        c("|a\nb\nc\nd\ne\nf", "2>2j", "{1}|a\n{1}b\n{1}c\n{1}d\n{1}e\nf"),
        // A zero after a digit is part of the count.
        c("|abcdefghijklm", "10x", "|klm"),
        c("|abcdefghijklm", "d10l", "|klm"),
    ]);
}

#[test]
fn cancelled_and_invalid() {
    run(&[
        c("|foo bar", "d<Esc>w", "foo |bar"),
        c("|foo bar", "2<Esc>x", "|oo bar"),
        c("|foo bar", "dx", "|foo bar").bell(true),
        c("|foo bar", "dy", "|foo bar").bell(true),
        c("|foo bar", "d<C-[>x", "|oo bar"),
        c("|foo bar", "\"<Esc>x", "|oo bar"),
        c("|foo bar", "gq", "|foo bar").bell(true),
        c("|foo bar", "Q", "|foo bar").bell(true),
        c("|foo bar", "zq", "|foo bar").bell(true),
    ]);
}

#[test]
fn more_motions_after_operators() {
    run(&[
        c("a\n|b\nc\nd", "dL", "a\n|d").height(3),
        c("a\n|b\nc\nd", "dH", "|c\nd").height(3),
        c("a\n|b\nc\nd", "yM", "a\n|b\nc\nd").height(3).regs(&[('"', "b\n")]),
        c("a |bc  ", "dg_", "a |  "),
        c("abcde|fghij", "dg0", "abcde|fghij").width(5),
        c("abcdef|ghij", "dg0", "abcde|ghij").width(5),
        c("a|bcdefghij", "dg$", "a|fghij").width(5),
        c("a\n|b\nc", "d<CR>", "|a"),
        c("ab\n|cd", "2<BS>", "|ab\ncd"),
        c("ab\n|cd", "d<BS>", "ab|cd"),
        c("foo |bar↵baz", "dw", "foo| ↵baz"),
        c("foo|, bar", "cwX<Esc>", "foo|X bar"),
        c("foo ba|r", "cwX<Esc>", "foo ba|X"),
        c("|ab", "xp", "b|a"),
        c("|foo bar", "dwwP", "bafoo| r"),
        c("a\n|b", "Yp", "a\nb\n|b"),
        c("|a\nb\nc", "jyyGp`[", "a\nb\nc\n|b"),
        c("|a\nb\nc", "yjG'[", "|a\nb\nc"),
        c("|a b", "ywG`]", "a |b"),
        c("a|b cd", "ix<Esc>`.", "a|xb cd"),
        c("|foo bar", "/bar<CR>''", "|foo bar"),
        // The jump list keeps one entry per block, as Vim keeps one per
        // line: a jump inside the block is undone with `` instead.
        c("|foo bar", "/bar<CR><C-o>", "foo |bar").bell(true),
        c("|foo\nbar", "/bar<CR><C-o>", "|foo\nbar"),
        c("|a\nb\nc\nd", "jmajjmb:'a,'bs/$/!/<CR>", "a\nb\nc\n|d").message("E486"),
        c("|a\nb\nc\nd", "jmajmb:'a,'bs/b/B/<CR>", "a\n|B\nc\nd"),
    ]);
}

/// Every operator with every motion: nothing breaks, a yank changes
/// nothing, a change ends in Insert mode, and one undo takes it back.
#[test]
fn every_operator_with_every_motion() {
    use writer_vim::Mode;
    let buffer = "One two. Three (four) five\nsec|ond \"line\" here. More\n\n  last one";
    let operators = ["d", "c", "y", ">", "<", "gu", "gU", "g~"];
    let motions = [
        "h", "l", "j", "k", "w", "W", "b", "B", "e", "E", "ge", "gE", "0", "^", "$", "g_", "g0", "g$", "gg",
        "G", "2G", "{", "}", "(", ")", "fe", "Fe", "te", "Te", ";", ",", "%", "H", "M", "L", "*", "#", "n",
        "N", "+", "-", "_", "<Space>", "<BS>", "gj", "gk", "|", "3|", "2w", "3l", "'a", "`a", "/one<CR>",
        "?One<CR>", "iw", "aw", "iW", "aW", "is", "as", "ip", "ap", "i\"", "a\"", "i(", "a(", "ib",
    ];
    let mut failures = Vec::new();
    for op in operators {
        for m in motions {
            let mut s = Session::new(buffer);
            // A mark, a find and a search to refer to.
            s.keys("ggma/four<CR>fe<C-o><C-o>");
            let before = s.text();
            let depth = s.ed.undo_depth();
            s.clear_effects();
            s.keys(&format!("{op}{m}"));
            let what = format!("{op}{m}");
            if let Err(e) = s.check_invariants() {
                failures.push(format!("{what}: {e}"));
                continue;
            }
            let failed = s.bells() > 0 || s.messages().iter().any(|m| m.starts_with('E'));
            match op {
                "y" => {
                    if s.text() != before || s.mode() != Mode::Normal {
                        failures.push(format!("{what}: a yank changed the buffer or the mode"));
                    }
                }
                "c" => {
                    if !failed && s.mode() != Mode::Insert {
                        failures.push(format!("{what}: not in Insert mode ({:?})", s.mode()));
                    }
                }
                _ => {
                    if s.mode() != Mode::Normal {
                        failures.push(format!("{what}: ended in {:?}", s.mode()));
                    }
                }
            }
            s.keys("<Esc>");
            if s.ed.undo_depth() > depth + 1 {
                failures.push(format!("{what}: more than one undo step"));
            }
            if s.text() != before {
                s.keys("u");
                if s.text() != before {
                    failures.push(format!("{what}: undo did not restore the buffer"));
                }
            }
            if s.mode() != Mode::Normal {
                failures.push(format!("{what}: not back in Normal mode"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Every motion in Visual and Visual Line mode, then a delete and an undo.
#[test]
fn every_motion_in_visual_mode() {
    use writer_vim::Mode;
    let buffer = "One two. Three (four) five\nsec|ond \"line\" here. More\n\n  last one";
    let motions = [
        "h", "l", "j", "k", "w", "W", "b", "B", "e", "E", "ge", "gE", "0", "^", "$", "g_", "g0", "g$", "gg",
        "G", "2G", "{", "}", "(", ")", "fe", "Fe", "te", "Te", "%", "H", "M", "L", "*", "#", "+", "-", "_",
        "<Space>", "<BS>", "iw", "aw", "is", "as", "ip", "ap", "i\"", "a\"", "i(", "a(", "o", "/one<CR>",
    ];
    let mut failures = Vec::new();
    for start in ["v", "V"] {
        for m in motions {
            for op in ["d", "y", "c", ">", "U", "J", "rx", "p"] {
                let mut s = Session::new(buffer);
                s.keys("yiw");
                let before = s.text();
                let depth = s.ed.undo_depth();
                let what = format!("{start}{m}{op}");
                s.keys(&format!("{start}{m}"));
                if !s.mode().is_visual() {
                    failures.push(format!("{what}: left Visual mode after the motion"));
                    continue;
                }
                s.keys(op);
                if let Err(e) = s.check_invariants() {
                    failures.push(format!("{what}: {e}"));
                    continue;
                }
                s.keys("<Esc>");
                if s.mode() != Mode::Normal {
                    failures.push(format!("{what}: not back in Normal mode"));
                }
                if s.ed.undo_depth() > depth + 1 {
                    failures.push(format!("{what}: more than one undo step"));
                }
                if s.text() != before {
                    s.keys("u");
                    if s.text() != before {
                        failures.push(format!("{what}: undo did not restore the buffer"));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
