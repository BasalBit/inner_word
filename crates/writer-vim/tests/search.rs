//! Searching with `/`, `?`, `n`, `N`, `*` and `#`.

use crate::harness::*;
use writer_vim::{CommandLine, Effect, Mode};

#[test]
fn slash_and_question() {
    run(&[
        c("|foo bar foo", "/foo<CR>", "foo bar |foo"),
        c("|foo bar foo baz foo", "/foo<CR>n", "foo bar foo baz |foo"),
        c("|foo bar foo baz foo", "/foo<CR>nN", "foo bar |foo baz foo"),
        c("|foo bar foo", "/foo<CR>n", "|foo bar foo").message("search hit BOTTOM"),
        c("foo bar |foo", "?foo<CR>", "|foo bar foo"),
        c("foo foo |foo", "?foo<CR>n", "|foo foo foo"),
        c("foo foo |foo", "?foo<CR>nN", "foo |foo foo"),
        c("|foo bar", "?bar<CR>", "foo |bar").message("search hit TOP"),
        c("|a\nb foo\nc", "/foo<CR>", "a\nb |foo\nc"),
        c("a\nb foo\n|c", "/foo<CR>", "a\nb |foo\nc").message("search hit BOTTOM"),
        c("|a b a b a b", "2/b<CR>", "a b a |b a b"),
        c("|a b a b a b", "/b<CR>2n", "a b a b a |b"),
        c("|foo", "/zzz<CR>", "|foo").message("E486"),
        c("|foo", "n", "|foo").message("E35"),
        c("|foo", "/<CR>", "|foo").message("E35"),
        // An empty pattern is the last one.
        c("|a b a b a", "/b<CR>/<CR>", "a b a |b a"),
        c("|a b a b a", "/b<CR>?<CR>", "a b a |b a").message("search hit TOP"),
        c("|añb ñ", "/ñ<CR>", "a|ñb ñ"),
        c("|ab↵cd", "/b\\nc<CR>", "a|b↵cd"),
    ]);
}

#[test]
fn case_rules() {
    run(&[
        // Lower case finds any case.
        c("|x Foo foo", "/foo<CR>", "x |Foo foo"),
        // An upper-case letter makes it exact.
        c("|x foo Foo", "/Foo<CR>", "x foo |Foo"),
        c("|x foo Foo", "/Foo\\c<CR>", "x |foo Foo"),
        c("|x Foo foo", "/\\Cfoo<CR>", "x Foo |foo"),
        c("|x ÉCOLE école", "/école<CR>", "x |ÉCOLE école"),
    ]);
}

#[test]
fn literal_patterns() {
    run(&[
        c("|a/b c a/b", "/a\\/b<CR>", "a/b c |a/b"),
        // No regular expressions: a dot is a dot.
        c("|axb a.b", "/a.b<CR>", "axb |a.b"),
        c("|a*b ab a*b", "/a*b<CR>", "a*b ab |a*b"),
        c("|x \\ y \\", "/\\\\<CR>", "x |\\ y \\"),
        c("|cat scatter cat", "/\\<cat\\><CR>", "cat scatter |cat"),
        c("|foo? yes foo?", "/foo?<CR>", "foo? yes |foo?"),
        c("foo? yes |foo?", "?foo\\?<CR>", "|foo? yes foo?"),
    ]);
}

#[test]
fn word_under_the_caret() {
    run(&[
        c("|foo bar foo", "*", "foo bar |foo"),
        c("foo bar |foo", "#", "|foo bar foo"),
        // Whole words only.
        c("|foo foobar foo", "*", "foo foobar |foo"),
        c("|foo foobar", "g*", "foo |foobar"),
        c("foobar |foo", "g#", "|foobar foo"),
        c("| foo x foo", "*", " foo x |foo"),
        c("|a b a b a", "*n", "a b a b |a"),
        c("a b a b |a", "#n", "|a b a b a"),
        c("|a a a a", "2*", "a a |a a"),
        // A keyword later on the line wins over the punctuation under
        // the caret.
        c("|-- x --", "*", "-- |x --").message("search hit BOTTOM"),
        c("|-- ++ --", "*", "-- ++ |--"),
        c("|foo x", "*", "|foo x").message("search hit BOTTOM"),
        c("|   ", "*", "|   ").message("E348"),
        c("fo|o bar\nfoo", "*", "foo bar\n|foo"),
        c("|don\u{2019}t x don\u{2019}t", "*", "don\u{2019}t x |don\u{2019}t"),
        c("|Foo foo Foo", "*", "Foo foo |Foo"),
        c("|foo bar foo", "*N", "|foo bar foo"),
        c("fo|o bar foo", "#", "foo bar |foo").message("search hit TOP"),
        c("foo bar fo|o", "#", "|foo bar foo"),
    ]);
}

#[test]
fn as_operator_motion() {
    run(&[
        c("|foo bar baz", "d/baz<CR>", "|baz"),
        c("|foo bar baz", "c/bar<CR>X <Esc>", "X| bar baz"),
        c("|foo bar baz", "y/bar<CR>", "|foo bar baz").regs(&[('"', "foo ")]),
        c("|a b a b", "/b<CR>dn", "a |b"),
        c("foo bar |baz", "d?foo<CR>", "|baz"),
        c("|foo bar baz", "v/baz<CR>d", "|az"),
        c("|foo bar baz", "d/zzz<CR>", "|foo bar baz").message("E486"),
        c("|foo bar baz", "d/bar<Esc>x", "|oo bar baz"),
        c("|foo\nbar\nbaz", "d/baz<CR>", "|baz"),
        // From inside a line the lines in between go, the break before
        // the match stays.
        c("x |foo\nbar\nbaz", "d/baz<CR>", "x| \nbaz"),
        c("|a b c", "d*", "|a b c").message("search hit BOTTOM"),
    ]);
}

#[test]
fn command_line_editing() {
    run(&[
        c("|foo bar", "/br<Left>a<CR>", "foo |bar"),
        c("|foo bar", "/zzz<C-u>bar<CR>", "foo |bar"),
        c("|foo bar", "/zzz qqq<C-w><C-w>bar<CR>", "foo |bar"),
        c("|foo bar", "/barx<BS><CR>", "foo |bar"),
        c("|foo bar", "/xbar<Home><Del><CR>", "foo |bar"),
        c("|foo bar", "/ar<C-b>b<C-e><CR>", "foo |bar"),
        c("|foo bar", "/bar<Esc>", "|foo bar"),
        c("|foo bar", "/bar<C-c>", "|foo bar"),
        // Backspace on an empty line closes it.
        c("|foo", "/<BS>x", "|oo"),
        // History.
        c("|a b c b", "/b<CR>/<Up><CR>", "a b c |b"),
        c("|a b c b a", "/b<CR>/a<CR>/<Up><Up><CR>", "a |b c b a").message("search hit BOTTOM"),
        c("|a b c b a", "/b<CR>/c<CR>/<Up><Up><Down><CR>", "a b |c b a").message("search hit BOTTOM"),
        c("|a b", "/zz<Up><Down><BS><BS>b<CR>", "a |b"),
        c("|foo bar foo", "/<C-r><C-w><CR>", "foo bar |foo"),
        c("|foo bar foo", "yiwwh/<C-r>\"<CR>", "foo bar |foo"),
    ]);
}

#[test]
fn incremental_search_previews_and_restores() {
    let mut s = Session::new("|foo bar baz");
    s.clear_effects();
    s.keys("/ba");
    assert_eq!(s.mode(), Mode::CommandLine);
    assert_eq!(s.buffer(), "foo |bar baz");
    assert_eq!(
        s.vim.command_line(),
        Some(CommandLine {
            prefix: '/',
            text: "ba".into(),
            cursor: 2
        })
    );
    s.keys("z");
    assert_eq!(s.buffer(), "foo bar |baz");
    // No match: back at the start.
    s.keys("q");
    assert_eq!(s.buffer(), "|foo bar baz");
    s.keys("<Esc>");
    assert_eq!(s.mode(), Mode::Normal);
    assert_eq!(s.buffer(), "|foo bar baz");
    assert_eq!(s.vim.command_line(), None);
    // The cancelled pattern did not replace the last one.
    assert!(s.vim.search_pattern().is_none());
}

#[test]
fn highlight_effects() {
    let mut s = Session::new("|foo bar foo");
    s.clear_effects();
    s.keys("/foo<CR>");
    let shown: Vec<Option<String>> = s
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::SearchHighlight(p) => Some(p.as_ref().map(|p| p.text.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(shown.last(), Some(&Some("foo".to_string())));
    let pat = s.vim.search_highlight().cloned().expect("highlight on");
    assert_eq!(pat.matches_in_block(s.ed.document(), 0), vec![0..3, 8..11]);
    s.clear_effects();
    s.keys(":noh<CR>");
    assert!(s.effects.contains(&Effect::SearchHighlight(None)));
    assert!(s.vim.search_highlight().is_none());
    assert!(s.vim.search_pattern().is_some());
    // The next search command turns them on again.
    s.clear_effects();
    s.keys("n");
    assert!(matches!(
        s.effects.iter().find(|e| matches!(e, Effect::SearchHighlight(_))),
        Some(Effect::SearchHighlight(Some(_)))
    ));
    s.keys("<C-l>");
    assert!(s.vim.search_highlight().is_none());
}
