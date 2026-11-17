//! Dot-repeat.

use crate::harness::*;

#[test]
fn simple_changes() {
    run(&[
        c("|abcdef", "x..", "|def"),
        c("|abcdefgh", "2x.", "|efgh"),
        c("|a b c d", "dw.", "|c d"),
        // A count replaces the old one and sticks.
        c("|a b c d e", "dw2.", "|d e"),
        c("|a b c d e f", "dw2..", "|f"),
        c("|a b c d e f", "2dw.", "|e f"),
        c("|a\nb\nc\nd", "dd.", "|c\nd"),
        c("|a\nb\nc", "\"add.", "|c").regs(&[('a', "b\n")]),
        c("|a.b.c.d", "df..", "|c.d"),
        c("|x a b a b a", "d/a<CR>.", "|a b a"),
        c("|ab", "ylp.", "aa|ab"),
        c("|a\nb\nc", "J.", "a b| c"),
        c("|a", ">>.", "{2}|a"),
        c("|abc", "rxl.", "x|xc"),
        c("|abc", "~.", "AB|c"),
        c("|foo bar", "gUiww.", "FOO |BAR"),
        c("foo |bar baz qux", "daw.", "foo |qux"),
        c("a|bc def", "D0.", "|"),
        c("|a", ".", "|a").bell(true),
        // Yanks and motions are not repeated.
        c("|a b c", "dwyw.", "|c"),
        c("|a b c d", "dww.", "b |d"),
        c("|abc", "xu.", "|bc"),
    ]);
}

#[test]
fn inserts() {
    run(&[
        c("|foo bar baz", "cwX<Esc>w.", "X |X baz"),
        c("|foo bar baz", "ciwX<Esc>ww.", "X bar |X"),
        c("|a\nb", "A!<Esc>j.", "a!\nb|!"),
        c("a|b\ncd", "IX<Esc>j.", "Xab\n|Xcd"),
        c("|abc", "ix<Esc>ll.", "xa|xbc"),
        c("|abc", "ax<Esc>l.", "axb|xc"),
        c("|a\nb", "ox<Esc>j.", "a\nx\nb\n|x"),
        c("a\n|b", "Ox<Esc>.", "a\n|x\nx\nb"),
        c("- |a", "ob<Esc>.", "- a\n- b\n- |b"),
        c("|a", "3ix<Esc>.", "xxxx|xxa"),
        c("|a", "3ix<Esc>2.", "xxx|xxa"),
        c("|abcd", "sX<Esc>l.", "X|Xcd"),
        c("|abcdef", "RXY<Esc>l.", "XYX|Yef"),
        c("|a\nb\nc", "ccx<Esc>j.", "x\n|x\nc"),
        c("|a b\nc d", "Cx<Esc>j.", "x\n|x"),
        // Enter, backspace and ctrl-w are part of what is repeated.
        c("|x", "iab<BS>c<CR>d<Esc>G.", "ac\nac\n|ddx"),
        c("|x", "ifoo bar<C-w>baz<Esc>0.", "foo ba|zfoo bazx"),
        c("|x", "ia<S-CR>b<Esc>0.", "a↵a↵|bbx"),
        // Moving the caret while typing starts the record again.
        c("|abc", "ix<Right>y<Esc>0.", "|yxaybc"),
        c("|x\ny", "I- <Esc>j.", "- x\n- |y").rules(),
        c("|x y", "ywA <C-r>\"<Esc>.", "x y x  x| "),
        c("|x", "i<Esc>.", "|x"),
        c("|abc", "ix<Esc>u.", "|xabc"),
    ]);
}

#[test]
fn visual_changes() {
    run(&[
        c("|abcdef", "vld.", "|ef"),
        // Near the end of the line: as much as there is.
        c("|abcdef", "vlldl.", "|d"),
        c("|a\nb\nc\nd", "Vjd.", "|"),
        c("|a\nb", "V>.", "{2}|a\nb"),
        c("|abcd", "vlU0ll.", "AB|CD"),
        c("|a\nb\nc\nd\ne", "VjJj.", "a b\nc| d\ne"),
        c("|abcd efgh", "vlcX<Esc>w.", "Xcd |Xgh"),
        c("|abcdef", "vlrxll.", "xx|xxef"),
    ]);
}
