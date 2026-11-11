//! Registers and put.

use crate::harness::*;
use writer_vim::{RegisterContent, VimOptions};

#[test]
fn named_registers() {
    run(&[
        c("|foo bar", "\"ayiww\"ap", "foo bfo|oar").regs(&[('a', "foo"), ('"', "foo")]),
        // A named yank leaves register 0 alone.
        c("|foo bar", "yiww\"ayiw", "foo |bar").regs(&[('0', "foo"), ('a', "bar"), ('"', "bar")]),
        c("|foo bar", "\"ayiww\"Ayiw", "foo |bar").regs(&[('a', "foobar"), ('"', "foobar")]),
        // Appending lines to characters makes the register linewise.
        c("|foo\nbar", "\"ayiwj\"Ayy", "foo\n|bar").regs(&[('a', "foo\nbar\n")]),
        c("|foo\nbar", "\"ayyj\"Ayiw", "foo\n|bar").regs(&[('a', "foo\nbar\n")]),
        c("|foo\nbar", "\"zdd\"zp", "bar\n|foo").regs(&[('z', "foo\n"), ('1', "<unset>")]),
        c("|foo bar", "\"adw\"bdw\"aP\"bp", "foo ba|r"),
        c("|foo", "\"Ayiw", "|foo").regs(&[('a', "foo")]),
        c("|foo", "\"!x", "|oo").bell(true),
        c("|foo", "\".yy", "|foo")
            .bell(true)
            .regs(&[('"', "<unset>")]),
        c("|ab", "\"a\"byl", "|ab").regs(&[('a', "<unset>"), ('b', "a")]),
    ]);
}

#[test]
fn yank_and_delete_registers() {
    run(&[
        c("|foo bar baz", "yiwwdiw", "foo | baz").regs(&[('0', "foo"), ('"', "bar"), ('-', "bar")]),
        c("|foo bar", "yiwwdiw\"0P", "foofo|o "),
        c("|a\nb\nc\nd", "dddddd", "|d").regs(&[
            ('1', "c\n"),
            ('2', "b\n"),
            ('3', "a\n"),
            ('"', "c\n"),
        ]),
        // A small delete does not shift the numbered registers.
        c("|foo\nbar", "ddx", "|ar").regs(&[('1', "foo\n"), ('-', "b"), ('"', "b")]),
        // Some motions always use register 1.
        c("|a b (c d) e", "f(d%", "a b | e").regs(&[('1', "(c d)"), ('-', "(c d)")]),
        c("|One. Two.", "d)", "|Two.").regs(&[('1', "One. ")]),
        c("|a b", "dw", "|b").regs(&[('1', "<unset>"), ('-', "a ")]),
        // A delete over several lines is not small.
        c("a|b\ncd", "vjd", "|a").regs(&[('1', "b\ncd"), ('-', "<unset>")]),
        c("|foo bar", "yiww\"_diwP", "foofo|o ").regs(&[('"', "foo"), ('-', "<unset>")]),
        c("|foo bar", "\"_dd", "|").regs(&[('"', "<unset>"), ('1', "<unset>")]),
        c(
            "|a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk",
            "dddddddddddddddddddd",
            "|k",
        )
        .regs(&[('1', "j\n"), ('9', "b\n")]),
        c("|foo bar", "cwx<Esc>", "|x bar").regs(&[('"', "foo"), ('-', "foo")]),
    ]);
}

#[test]
fn clipboard_registers() {
    run(&[
        c("|foo bar", "\"+yiw", "|foo bar")
            .expect_clip("foo")
            .regs(&[('"', "foo"), ('0', "<unset>")]),
        c("|foo bar", "\"*yiw", "|foo bar").expect_clip("foo"),
        c("|x", "\"+p", "xcli|p").clip("clip"),
        c("|x", "\"*P", "cli|px").clip("clip"),
        c("|x", "\"+p", "x\n|line").clip("line\n"),
        c("|x", "\"+p", "x\n|one\ntwo").clip("one\ntwo\n"),
        c("|x", "\"+p", "x|one\ntwo").clip("one\ntwo"),
        c("|foo", "\"+yy", "|foo").expect_clip("foo\n"),
        c("|foo bar", "\"+dw", "|bar").expect_clip("foo "),
        c("|x", "\"+p", "|x").message("E353"),
    ]);
}

#[test]
fn unnamed_register_can_mirror_the_clipboard() {
    let options = VimOptions {
        unnamed_clipboard: true,
        ..VimOptions::default()
    };
    let mut s = Session::with("|foo bar", 1000, 10, options.clone());
    s.keys("yiw");
    assert_eq!(
        s.host
            .clipboard
            .as_ref()
            .map(|c| c.to_plain_text())
            .as_deref(),
        Some("foo")
    );
    s.host.clipboard = Some(RegisterContent::from_plain_text("ext"));
    s.keys("$p");
    assert_eq!(s.buffer(), "foo barex|t");
    s.keys("dd");
    assert_eq!(
        s.host
            .clipboard
            .as_ref()
            .map(|c| c.to_plain_text())
            .as_deref(),
        Some("foo barext\n")
    );
    // A named register stays away from the clipboard.
    let mut s = Session::with("|foo bar", 1000, 10, options);
    s.keys("\"ayiw");
    assert!(s.host.clipboard.is_none());
}

#[test]
fn read_only_registers() {
    run(&[
        c("|x", "iabc<Esc>\".p", "abcab|cx"),
        c("|x y", ":noh<CR>\":p", "xno|h y"),
        c("|foo bar", "/bar<CR>\"/P", "foo ba|rbar"),
        c("|x", "\":p", "|x").message("E353"),
    ]);
}

#[test]
fn put_characters() {
    run(&[
        c("|ab", "ylp", "a|ab"),
        c("a|b", "ylP", "a|bb"),
        c("|ab", "yl3p", "aaa|ab"),
        c("|ab cd", "yiwgp", "aab|b cd"),
        c("|ab", "ylgP", "a|ab"),
        c("ab|c", "ylp", "abc|c"),
        c("|", "ix<Esc>ylp", "x|x"),
        c("|x", "p", "|x").message("E353"),
        c("|añb", "lylp", "añ|ñb"),
        // Text over several blocks: the caret stays at its start.
        c("a|b\ncd\nxy", "vjy$p", "ab|b\ncd\ncd\nxy"),
        c("a|b\ncd\nxy", "vjyGP", "ab\ncd\n|b\ncdxy"),
        c("|ab\n---\ncd", "yljp", "ab\n---\n|a\ncd"),
    ]);
}

#[test]
fn put_lines() {
    run(&[
        c("|a\nb", "yyp", "a\n|a\nb"),
        c("a\n|b", "yyP", "a\n|b\nb"),
        c("|a", "yy2p", "a\n|a\na"),
        c("|a\nb", "yygp", "a\na\n|b"),
        c("|a", "yygp", "a\n|a"),
        c("|a\nb", "yyjgP", "a\na\n|b"),
        c("|a\nb\nc", "2yyGp", "a\nb\nc\n|a\nb"),
        c("|a\n  b", "jyyggP", "  |b\na\n  b"),
        c("|a\nb\nc", "yy3p", "a\n|a\na\na\nb\nc").message("3 more lines"),
        c("a|b↵cd\nx", "yyp", "ab↵cd\n|ab↵cd\nx"),
    ]);
}
