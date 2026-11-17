//! Code blocks: one block, many lines. Linewise commands work on its
//! lines as long as they stay inside it.

use crate::harness::*;

#[test]
fn lines_inside_a_code_block() {
    run(&[
        c("~ a↵|b↵c", "dd", "~ a↵|c").regs(&[('"', "~ b\n")]),
        c("~ a↵|b", "dd", "~ |a"),
        c("~ |a↵b", "dd", "~ |b"),
        c("~ a↵|b↵c↵d", "2dd", "~ a↵|d"),
        c("~ |a↵b↵c", "dj", "~ |c"),
        c("~ a↵b↵|c", "dk", "~ |a"),
        c("~ a↵|b", "yyp", "~ a↵b↵|b"),
        c("~ a↵|b", "yyP", "~ a↵|b↵b"),
        c("~ |a", "yy2p", "~ a↵|a↵a"),
        c("~ a↵  |b↵c", "ccX<Esc>", "~ a↵  |X↵c"),
        c("~ a↵  |b↵c", "SX<Esc>", "~ a↵  |X↵c"),
        c("~ a↵|b↵c", "Vd", "~ a↵|c"),
        c("~ |a↵b↵c", "Vjd", "~ |c"),
        c("~ a↵|b↵c", "Vy", "~ a↵|b↵c").regs(&[('"', "~ b\n")]),
        c("~ a↵|b↵c", "gUU", "~ a↵|B↵c"),
        c("~ a↵|b↵c", "VU", "~ a↵|B↵c"),
        c("~ |a↵  b", "J", "~ a| b"),
        c("~ |a↵b↵c", "3J", "~ a b| c"),
        c("~ |a↵  b", "+", "~ a↵  |b"),
        c("~ a↵|b", "-", "~ |a↵b"),
        c("~ a↵|b↵c", "ddp", "~ a↵c↵|b"),
        c("~ |a↵b", "yyjVp", "~ a↵|a"),
    ]);
}

#[test]
fn indenting_code() {
    run(&[
        c("~ a↵|b", ">>", "~ a↵    |b"),
        c("~ a↵    |b", "<<", "~ a↵|b"),
        c("~ a↵  |b", "<<", "~ a↵|b"),
        c("~ |a↵b↵c", ">j", "~     |a↵    b↵c"),
        c("~ |a↵b↵c", "2>>", "~     |a↵    b↵c"),
        c("~ |a↵↵c", ">2j", "~     |a↵↵    c"),
        c("~ |a↵b", "Vj>", "~     |a↵    b"),
        c("~ x↵        |a", "V2<", "~ x↵|a"),
        c("~ x↵\t|a", "<<", "~ x↵|a"),
        c("~ |a\nx", ">>", "~     |a\nx"),
        // A range that leaves the block indents blocks.
        c("~ |a\nx", ">j", "{1}~ |a\n{1}x"),
    ]);
}

#[test]
fn opening_lines_in_code() {
    run(&[
        c("~ a↵  |b↵c", "oX<Esc>", "~ a↵  b↵  |X↵c"),
        c("~ a↵  |b", "OX<Esc>", "~ a↵  |X↵  b"),
        c("~ |a", "ob<Esc>", "~ a↵|b"),
        c("~ |a", "Ob<Esc>", "~ |b↵a"),
        c("~ |a", "2ob<Esc>", "~ a↵b↵|b"),
        c("~ |a", "ob<Esc>.", "~ a↵b↵|b"),
    ]);
}

#[test]
fn leaving_the_block() {
    run(&[
        // A range that leaves the code block takes whole blocks.
        c("~ a↵|b\nx\ny", "dj", "|y"),
        c("x\n~ |a↵b", "dk", "|"),
        c("~ |a↵b\nx", "Vjd", "|x"),
        c("~ |a↵b\nx", "dG", "|"),
        c("~ a↵|b", "dgg", "|"),
        c("~ |a\nx", "dd", "|x"),
        // A copied code line pasted outside becomes a code block.
        c("~ a↵|b\nx", "yyjp", "~ a↵b\nx\n~ |b"),
        // Prose lines go after the block, not into it.
        c("|x\n~ a↵b", "yyjp", "x\n~ a↵b\n|x"),
        c("|x\n~ a↵b", "yyjP", "x\n|x\n~ a↵b"),
        // Line numbers are blocks.
        c("|x\n~ a↵b", "G", "x\n~ a↵|b"),
        c("|x\n~ a↵b", "2G", "x\n~ |a↵b"),
        c("~ a↵|b\nx", ":d<CR>", "|x"),
        c("~ a↵|b\nx", "ggdd", "~ |b\nx"),
    ]);
}

#[test]
fn text_in_code() {
    run(&[
        c("~ fo|o bar↵x", "dw", "~ fo|bar↵x"),
        c("~ foo |bar↵x", "D", "~ foo| ↵x"),
        c("~ |a↵b", "x", "~ |↵b"),
        c("~ a|b↵cd", "vjd", "~ |a"),
        c("~ a↵|b", "A;<Esc>", "~ a↵b|;"),
        c("~ |f(a,↵  b)", "%", "~ f(a,↵  b|)"),
        c("~ f(a,↵  |b)", "di(", "~ f(|)"),
        c("~ a↵|b↵c", "j", "~ a↵b↵|c"),
        c("~ |ab↵cd", "$j", "~ ab↵c|d"),
    ]);
}
