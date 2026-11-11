//! Text objects after operators.

use crate::harness::*;

#[test]
fn words() {
    run(&[
        c("foo b|ar baz", "diw", "foo | baz").regs(&[('"', "bar")]),
        c("foo b|ar baz", "daw", "foo |baz").regs(&[('"', "bar ")]),
        // The last word takes the space before it.
        c("foo ba|r", "daw", "fo|o"),
        c("foo b|ar baz", "ciwX<Esc>", "foo |X baz"),
        c("foo b|ar baz", "yiw", "foo |bar baz").regs(&[('"', "bar")]),
        c("foo b|ar baz", "yaw", "foo |bar baz").regs(&[('"', "bar ")]),
        c("a f.o|o b", "diW", "a | b"),
        c("a f.o|o b", "daW", "a |b"),
        c("a f.o|o b", "diw", "a f.| b"),
        c("foo|  bar", "diw", "foo|bar"),
        c("foo|  bar", "daw", "fo|o"),
        c("foo|, bar", "diw", "foo| bar"),
        c("|", "diw", "|"),
        c("|", "daw", "|").bell(true),
        c("fo|o bar baz", "d2aw", "|baz"),
        c("fo|o bar baz", "d3iw", "| baz"),
        c("  fo|o", "daw", " | "),
        c("it\u{2019}s f|ine", "daw", "it\u{2019}|s"),
        c("d|on\u{2019}t stop", "diw", "| stop"),
        c("|naïve café", "daw", "|café"),
        c("foo↵b|ar↵baz", "diw", "foo↵|↵baz"),
        c("foo b|ar baz", "gUiw", "foo |BAR baz"),
    ]);
}

#[test]
fn sentences() {
    run(&[
        c("One. T|wo. Three.", "dis", "One. | Three."),
        c("One. T|wo. Three.", "das", "One. |Three."),
        // The last sentence takes the space before it.
        c("One. Two. Thr|ee.", "das", "One. Two|."),
        c("One. T|wo. Three.", "cisX<Esc>", "One. |X Three."),
        c("One. T|wo. Three.", "yis", "One. |Two. Three.").regs(&[('"', "Two.")]),
        c("One. T|wo. Three.", "yas", "One. |Two. Three.").regs(&[('"', "Two. ")]),
        c("|One. Two. Three.", "d2as", "|Three."),
        c("Really? Y|es! Fine.", "dis", "Really? | Fine."),
        c("|   ", "dis", "|   ").bell(true),
        c("One.|  Two.", "dis", "One.|Two."),
    ]);
}

#[test]
fn paragraphs() {
    run(&[
        c("a\n|b\n\nc", "dip", "|\nc").regs(&[('"', "a\nb\n")]),
        c("a\n|b\n\nc", "dap", "|c"),
        // The last paragraph takes the empty blocks before it.
        c("a\n\n|b", "dap", "|a"),
        c("a\n|b\n\nc", "yip", "|a\nb\n\nc").regs(&[('"', "a\nb\n")]),
        c("a\n|b\n\nc", "cipX<Esc>", "|X\n\nc"),
        c("a\n|\n\nb", "dip", "a\n|b"),
        c("a\n|\n\nb\nc\n\nd", "dap", "a\n|\nd"),
        c("|a\n\nb\n\nc", "d2ap", "|c"),
        c("a\n|b\n\nc", ">ip", "{1}|a\n{1}b\n\nc"),
        c("a\n|b\n---\nc", "dap", "|---\nc"),
        c("a\n|---\nb", "dip", "a\n|b"),
        c("|only", "dap", "|"),
        c("# H\n|text\n\nmore", "yap", "# |H\ntext\n\nmore").regs(&[('"', "# H\ntext\n\n")]),
    ]);
}

#[test]
fn quotes() {
    run(&[
        c("say \"h|i there\" now", "di\"", "say \"|\" now").regs(&[('"', "hi there")]),
        c("say \"h|i there\" now", "da\"", "say |now"),
        c("say \"h|i\" now", "ci\"X<Esc>", "say \"|X\" now"),
        // From before the string on the same line.
        c("|say \"hi\" now", "di\"", "say \"|\" now"),
        c("it 'w|as' so", "di'", "it '|' so"),
        c("a `c|ode` b", "di`", "a `|` b"),
        c("a `c|ode` b", "da`", "a |b"),
        c("say \"hi\" a|nd \"yo\"", "di\"", "say \"hi\" and \"|\""),
        c("say |\"hi\" now", "di\"", "say \"|\" now"),
        c("say \"hi|\" now", "di\"", "say \"|\" now"),
        c("|foo", "di\"", "|foo").bell(true),
        c("x \"a\\\"|b\" y", "di\"", "x \"|\" y"),
        // The typographic quotes that smart punctuation types.
        c("say \u{201C}h|i there\u{201D} now", "di\"", "say \u{201C}|\u{201D} now"),
        c("say \u{201C}h|i there\u{201D} now", "da\"", "say |now"),
        c("it \u{2018}w|as\u{2019} so", "di'", "it \u{2018}|\u{2019} so"),
        c("\u{2018}it\u{2019}s f|ine\u{2019} x", "di'", "\u{2018}|\u{2019} x"),
        c("end \"la|st\"", "da\"", "en|d"),
        c("say \"h|i there\" now", "yi\"", "say \"|hi there\" now").regs(&[('"', "hi there")]),
    ]);
}

#[test]
fn brackets() {
    run(&[
        c("f(a, |b)", "di(", "f(|)").regs(&[('"', "a, b")]),
        c("f(a, |b)", "da(", "|f"),
        c("f(a, |b)", "dib", "f(|)"),
        c("f(a, |b)", "di)", "f(|)"),
        c("f(a, |b)", "ci(X<Esc>", "f(|X)"),
        c("f(|)", "ci(X<Esc>", "f(|X)"),
        c("x[1|2]", "di[", "x[|]"),
        c("x[1|2]", "da]", "|x"),
        c("{a|b}", "di{", "{|}"),
        c("{a|b}", "diB", "{|}"),
        c("{a|b} c", "daB", "| c"),
        c("<a|b>", "di<", "<|>"),
        c("<a|b>", "da>", "|"),
        c("((a|))", "2di(", "(|)"),
        c("(a (b|) c)", "di(", "(a (|) c)"),
        c("(a (b|) c)", "2di(", "(|)"),
        c("f|(a)", "di(", "f(|)"),
        c("f(a|)", "di(", "f(|)"),
        c("|f(a)", "di(", "|f(a)").bell(true),
        c("(a|", "di(", "(|a").bell(true),
        c("f(a, |b)", "yi(", "f(|a, b)").regs(&[('"', "a, b")]),
        c("~ fn x() {↵  bo|dy↵}", "di{", "~ fn x() {|}"),
        c("f(a, |b)", "gUi(", "f(|A, B)"),
    ]);
}
