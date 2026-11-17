//! Dividers and page breaks: blocks without text.

use crate::harness::*;

#[test]
fn moving_over_them() {
    run(&[
        c("a\n|---\nb", "l", "a\n|---\nb").bell(true),
        c("a\n|---\nb", "h", "a\n|---\nb").bell(true),
        c("a\n|---\nb", "$", "a\n|---\nb"),
        c("a\n|---\nb", "0", "a\n|---\nb"),
        c("a\n|---\nb", "^", "a\n|---\nb"),
        c("|a\n---\nb", "jj", "a\n---\n|b"),
        c("a\n---\n|b", "kk", "|a\n---\nb"),
        c("|a\n===\nb", "w", "a\n|===\nb"),
        c("a\n===\n|b", "b", "a\n|===\nb"),
        c("|a\n---\nb", "e", "a\n---\n|b"),
        c("a\n|---\nb", "w", "a\n---\n|b"),
        c("a\n|---\nb", "%", "a\n|---\nb").bell(true),
        c("a\n|---\nb", "fx", "a\n|---\nb").bell(true),
        c("|a\n---\nb", "G", "a\n---\n|b"),
        c("|a\n---", "G", "a\n|---"),
        c("a\n|---", "gg", "|a\n---"),
        c("|---", "j", "|---").bell(true),
        c("|---\n---", "<Space>", "---\n|---"),
    ]);
}

#[test]
fn changing_them() {
    run(&[
        c("a\n|---\nb", "x", "a\n|---\nb").bell(false),
        c("a\n|---\nb", "dd", "a\n|b").regs(&[('"', "---\n")]),
        c("a\n|---\nb", "yyp", "a\n---\n|---\nb"),
        c("a\n|===\nb", "ddp", "a\nb\n|==="),
        c("a\n|---\nb", "ccX<Esc>", "a\n|X\nb"),
        c("a\n|---\nb", "SX<Esc>", "a\n|X\nb"),
        c("|---", "ccX<Esc>", "|X"),
        // Typing on a divider starts a paragraph after it.
        c("a\n|---\nb", "iX<Esc>", "a\n---\n|X\nb"),
        c("a\n|---\nb", "aX<Esc>", "a\n---\n|X\nb"),
        c("a\n|---\nb", "AX<Esc>", "a\n---\n|X\nb"),
        c("a\n|---\nb", "IX<Esc>", "a\n---\n|X\nb"),
        c("|---", "oX<Esc>", "---\n|X"),
        c("|---", "OX<Esc>", "|X\n---"),
        c("|---", "RX<Esc>", "---\n|X"),
        c("a\n|---", "cwX<Esc>", "a\n---\n|X"),
        // Joining treats it as an empty line: it goes.
        c("|---\nb", "J", "|b"),
        c("|a\n---\nb", "J", "|a\nb"),
        c("|a\n---\nb", "3J", "a| b"),
        c("|---", "rx", "|---").bell(true),
        c("|---", "~", "|---").bell(true),
        c("|---", ">>", "|---"),
        c("|---", "gUU", "|---"),
        c("|---\nb", "dw", "|b"),
        c("|---", "D", "|---"),
        c("|---", "diw", "|---"),
        c("|---", "dd", "|"),
        c("|---\n---", "dj", "|"),
        c("a\n|---\nb", "vd", "a\n|b"),
        c("a\n|---\nb", "Vd", "a\n|b"),
        c("|a\n---\nb", "vjjd", "|"),
        c("a\n|---\nb", "dip", "a\n|b"),
        c("|a\n---\nb", "dG", "|"),
    ]);
}

#[test]
fn putting_around_them() {
    run(&[
        c("|x\n---", "yljp", "x\n---\n|x"),
        c("|x\n---", "yljP", "x\n---\n|x"),
        c("|a\n---", "yyjp", "a\n---\n|a"),
        c("|a\n---", "yyjP", "a\n|a\n---"),
        c("a\n|---\nb", "yyjVp", "a\n---\n|---"),
        c("|a b\n---", "yiwjvp", "a b\n|a"),
        c("|---\nab", "yyjlvp", "---\na\n|---\n"),
    ]);
}

#[test]
fn nearly_empty_documents() {
    run(&[
        c("|", "dd", "|"),
        c("|", "x", "|").bell(false),
        c("|", "dw", "|"),
        c("|", "J", "|").bell(true),
        c("|", "yyp", "\n|"),
        c("|", "p", "|").message("E353"),
        c("|", "G", "|"),
        c("|", "gg", "|"),
        c("|", "w", "|").bell(true),
        c("|", "b", "|").bell(true),
        c("|", "}", "|").bell(true),
        c("|", "v", "«|").visual(),
        c("|", "vd", "|"),
        c("|", "Vd", "|"),
        c("|", ">>", "{1}|"),
        c("|", "ccx<Esc>", "|x"),
        c("|", "ox<Esc>", "\n|x"),
        c("|", "~", "|").bell(true),
        c("|", "*", "|").message("E348"),
        c("|", ":s/a/b/<CR>", "|").message("E486"),
        c("|", ":%d<CR>", "|"),
        c("|", "dip", "|"),
        c("|", "dap", "|"),
        c("|", "gUU", "|"),
        c("|", "D", "|"),
        c("|", "C<Esc>", "|"),
        c("|", "iab<Esc>dd", "|"),
    ]);
}
