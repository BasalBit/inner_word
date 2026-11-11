//! Insert and Replace mode.

use crate::harness::*;
use writer_vim::Mode;

#[test]
fn entering() {
    run(&[
        c("ab|c", "iX<Esc>", "ab|Xc"),
        c("ab|c", "aX<Esc>", "abc|X"),
        c("  ab|c", "IX<Esc>", "  |Xabc"),
        c("|abc", "AX<Esc>", "abc|X"),
        c("  ab|c", "gIX<Esc>", "|X  abc"),
        c("|", "aX<Esc>", "|X"),
        c("|", "iX<Esc>", "|X"),
        c("ab|c", "i", "ab|c").insert(),
        c("ab|c", "a", "abc|").insert(),
        c("ab|c", "A", "abc|").insert(),
        // Escape steps back onto a character.
        c("ab|c", "i<Esc>", "a|bc"),
        c("|abc", "i<Esc>", "|abc"),
        c("ab|c", "a<Esc>", "ab|c"),
        // Hard lines inside a block.
        c("a|b↵cd", "AX<Esc>", "ab|X↵cd"),
        c("ab↵  c|d", "IX<Esc>", "ab↵  |Xcd"),
        c("ab|c\ndef", "iX<Esc>jgiY<Esc>", "abX|Yc\ndef"),
        c("a|ñb", "aX<Esc>", "añ|Xb"),
    ]);
}

#[test]
fn opening_lines() {
    run(&[
        c("|a\nb", "oX<Esc>", "a\n|X\nb"),
        c("a\n|b", "OX<Esc>", "a\n|X\nb"),
        c("|a", "o", "a\n|").insert(),
        // Lists and quotes continue, as with Enter.
        c("- |one", "otwo<Esc>", "- one\n- tw|o"),
        c("1. |one", "otwo<Esc>", "1. one\n1. tw|o"),
        c("[x] |done", "onext<Esc>", "[x] done\n[ ] nex|t"),
        c("> |q", "ox<Esc>", "> q\n> |x"),
        c("{1}- |a", "ob<Esc>", "{1}- a\n{1}- |b"),
        c("- |one", "Ozero<Esc>", "- zer|o\n- one"),
        // A heading is followed by a paragraph.
        c("# |T", "obody<Esc>", "# T\nbod|y"),
        c("# |T", "Ox<Esc>", "|x\n# T"),
        // o works from anywhere in a block, also one with soft breaks.
        c("a|b↵cd\nx", "oY<Esc>", "ab↵cd\n|Y\nx"),
        c("- |", "ox<Esc>", "- \n- |x"),
    ]);
}

#[test]
fn counts() {
    run(&[
        c("|x", "3ia<Esc>", "aa|ax"),
        c("|x", "2Aab<Esc>", "xaba|b"),
        c("|x", "3oy<Esc>", "x\ny\ny\n|y"),
        c("|x", "2Oy<Esc>", "y\n|y\nx"),
        c("|x", "3ihello <Esc>", "hello hello hello| x"),
        c("a|b", "2aX<Esc>", "abX|X"),
        c("|x", "3i<Esc>", "|x"),
        c("- |a", "2ob<Esc>", "- a\n- b\n- |b"),
        c("|x", "2ia<CR>b<Esc>", "a\nba\n|bx"),
    ]);
}

#[test]
fn typing() {
    run(&[
        c("|", "ia<CR>b<Esc>", "a\n|b"),
        c("|", "iabc<BS><BS>x<Esc>", "a|x"),
        c("|", "ia<S-CR>b<Esc>", "a↵|b"),
        c("a|bc", "i<Del><Esc>", "|ac"),
        c("|", "ifoo bar<C-w>x<Esc>", "foo |x"),
        c("|", "ifoo   <C-w>x<Esc>", "|x"),
        c("|", "ifoo.bar<C-w><Esc>", "foo|."),
        c("|", "ifoo bar<C-u>x<Esc>", "|x"),
        c("|", "iab<C-h>c<Esc>", "a|c"),
        c("ab\n|cd", "i<C-w><Esc>", "a|bcd"),
        c("ab\n|cd", "i<C-u><Esc>", "a|bcd"),
        c("|", "inaïve <C-w>x<Esc>", "|x"),
        c("|", "idon't<C-w>x<Esc>", "|x"),
        // Typing inside formatted text keeps the formatting.
        c("**bo|ld**", "ix<Esc>", "**bo|xld**"),
        c("**bold**| x", "iy<Esc>", "**bold|y** x"),
        // The editor's own rules still run.
        c("|", "i# Title<Esc>", "# Titl|e").rules(),
        c("|", "i\"hi\"<Esc>", "\u{201C}hi|\u{201D}").rules(),
        c("|", "i- one<CR>two<Esc>", "- one\n- tw|o").rules(),
        c("|", "ia -- b<Esc>", "a \u{2014} |b").rules(),
        c("|x", "i<C-c>", "|x"),
        c("|x", "iab<C-[>", "a|bx"),
    ]);
}

#[test]
fn one_normal_command() {
    run(&[
        c("ab|c", "i<C-o>0X<Esc>", "|Xabc"),
        c("|foo bar", "i<C-o>dwX<Esc>", "|Xbar"),
        // After the last character the caret keeps its place.
        // ctrl-o at the end of a line steps onto the last character first.
        c("|abc", "A<C-o>hX<Esc>", "a|Xbc"),
        c("|abc", "A<C-o>zzX<Esc>", "abc|X"),
        c("|abc", "A<C-o>0", "|abc").insert(),
        c("|abc def", "i<C-o>2lX<Esc>", "ab|Xc def"),
        c("|abc", "i<C-o><Esc>X<Esc>", "|Xabc"),
        c("|abc", "i<C-o>AX<Esc>", "abc|X"),
        c("|a b", "i<C-o>dw<C-o>ix<Esc>", "|xb"),
        c("|abc", "ix<C-o>u", "|abc").insert(),
    ]);
}

#[test]
fn register_paste() {
    run(&[
        c("|foo bar", "ywA <C-r>\"<Esc>", "foo bar foo| "),
        c("|foo bar", "\"ayiwA <C-r>a!<Esc>", "foo bar foo|!"),
        c("|a\nb", "yyjA<C-r>\"x<Esc>", "a\nba\n|x"),
        c("|x", "i<C-r>z<Esc>", "|x").message("E353"),
        c("|x", "i<C-r><Esc>y<Esc>", "|yx"),
        c("**bo|ld** x", "yiw$a <C-r>\"<Esc>", "**bold** x **bol|d**"),
        c("|x", "A<C-r>+<Esc>", "xcli|p").clip("clip"),
    ]);
}

#[test]
fn replace_mode() {
    run(&[
        c("|abc", "RXY<Esc>", "X|Yc"),
        c("a|b", "RXYZ<Esc>", "aXY|Z"),
        c("|abc", "RXY<BS><Esc>", "|Xbc"),
        c("|abc", "RXY<BS><BS><Esc>", "|abc"),
        c("a|bc", "R<BS><Esc>", "|abc"),
        c("|abcdef", "2RXY<Esc>", "XYX|Yef"),
        c("|añb", "RXY<Esc>", "X|Yb"),
        c("|abc", "R", "|abc").mode(Mode::Replace),
        c("|ab\ncd", "RXYZ<Esc>", "XY|Z\ncd"),
        c("|abc", "RX<C-h><Esc>", "|abc"),
        c("|abc", "Rñ<Esc>", "|ñbc"),
        c("|abcd", "RX<CR>Y<Esc>", "X\n|Ycd"),
    ]);
}

#[test]
fn committed_text_counts_as_typing() {
    // An input method commits several characters at once.
    let mut s = Session::new("|x");
    s.keys("i").commit("仮名").keys("<Esc>");
    assert_eq!(s.buffer(), "仮|名x");
    s.keys("$.");
    assert_eq!(s.buffer(), "仮名仮|名x");
    assert_eq!(s.vim.last_inserted_text(), "仮名");
    // Three times.
    let mut s = Session::new("|x");
    s.keys("3i").commit("ab").keys("<Esc>");
    assert_eq!(s.buffer(), "ababa|bx");
}

#[test]
fn text_inserted_outside_insert_mode_is_ignored() {
    let mut s = Session::new("|x");
    let o = s.vim.text_inserted("zzz", &mut s.ed, &mut s.host);
    assert!(o.effects.is_empty());
    s.keys(".");
    assert_eq!(s.buffer(), "|x");
    assert_eq!(s.mode(), Mode::Normal);
}
