//! Motions as caret movements.

use crate::harness::*;

#[test]
fn left_right() {
    run(&[
        c("ab|c", "h", "a|bc"),
        c("|abc", "h", "|abc").bell(true),
        c("a|bc", "l", "ab|c"),
        c("ab|c", "l", "ab|c").bell(true),
        c("|abcdef", "3l", "abc|def"),
        c("abcde|f", "10h", "|abcdef"),
        c("|abc", "10l", "ab|c"),
        // h and l stay on their line.
        c("ab\n|cd", "h", "ab\n|cd").bell(true),
        c("a|b\ncd", "l", "a|b\ncd").bell(true),
        c("ab↵|cd", "h", "ab↵|cd").bell(true),
        // Space and backspace wrap.
        c("a|b\ncd", "<Space>", "ab\n|cd"),
        c("ab\n|cd", "<BS>", "a|b\ncd"),
        c("|abc", "2<Space>", "ab|c"),
        c("a|b↵cd", "<Space>", "ab↵|cd"),
        c("ab\n\n|cd", "<BS>", "ab\n|\ncd"),
        // Arrow keys.
        c("a|bc", "<Left>", "|abc"),
        c("a|bc", "<Right>", "ab|c"),
        c("a|bc", "<C-h>", "|abc"),
        // An empty line has one place.
        c("|", "l", "|").bell(true),
        c("|", "h", "|").bell(true),
    ]);
}

#[test]
fn left_right_multibyte() {
    run(&[
        c("|añb", "l", "a|ñb"),
        c("a|ñb", "l", "añ|b"),
        c("añ|b", "h", "a|ñb"),
        // A combining accent travels with its letter.
        c("|e\u{301}x", "l", "e\u{301}|x"),
        c("e\u{301}|x", "h", "|e\u{301}x"),
        // A ZWJ family is one character.
        c("|👩‍👩‍👧x", "l", "👩‍👩‍👧|x"),
        c("a👩‍👩‍👧|x", "h", "a|👩‍👩‍👧x"),
        c("|日本語", "2l", "日本|語"),
        c("|🇦🇱🇩🇪x", "l", "🇦🇱|🇩🇪x"),
    ]);
}

#[test]
fn line_positions() {
    run(&[
        c("  fo|o bar  ", "0", "|  foo bar  "),
        c("  fo|o bar  ", "^", "  |foo bar  "),
        c("  fo|o bar  ", "$", "  foo bar | "),
        c("  fo|o bar  ", "g_", "  foo ba|r  "),
        c("|abcdef", "3|", "ab|cdef"),
        c("abc|def", "|", "|abcdef"),
        c("|abc", "9|", "ab|c"),
        c("  fo|o", "<Home>", "|  foo"),
        c("|foo", "<End>", "fo|o"),
        // Hard lines inside a block.
        c("ab↵c|d↵ef", "0", "ab↵|cd↵ef"),
        c("ab↵|cd↵ef", "$", "ab↵c|d↵ef"),
        c("ab↵  c|d↵ef", "^", "ab↵  |cd↵ef"),
        // A count on $ goes down.
        c("|ab\ncd", "2$", "ab\nc|d"),
        c("|ab↵cd↵ef", "3$", "ab↵cd↵e|f"),
        c("|ab\ncd", "2g_", "ab\nc|d"),
        // Blank and empty lines.
        c("|   ", "^", "  | "),
        c("|", "$", "|"),
        c("|", "0", "|"),
        c("a|ñ", "$", "a|ñ"),
        c("|añ", "$", "a|ñ"),
    ]);
}

#[test]
fn words_forward() {
    run(&[
        c("|foo bar", "w", "foo |bar"),
        c("|foo.bar", "w", "foo|.bar"),
        c("foo|.bar", "w", "foo.|bar"),
        c("|foo.bar baz", "W", "foo.bar |baz"),
        c("|foo bar baz", "2w", "foo bar |baz"),
        c("|foo   bar", "w", "foo   |bar"),
        c("fo|o\nbar", "w", "foo\n|bar"),
        c("fo|o\n  bar", "w", "foo\n  |bar"),
        // An empty line is a word.
        c("fo|o\n\nbar", "w", "foo\n|\nbar"),
        c("foo\n|\nbar", "w", "foo\n\n|bar"),
        c("fo|o\n---\nbar", "w", "foo\n|---\nbar"),
        // The last word: to the end, then nowhere.
        c("foo |bar", "w", "foo ba|r"),
        c("foo ba|r", "w", "foo ba|r").bell(true),
        c("fo|o↵bar", "w", "foo↵|bar"),
        c("|foo, bar", "w", "foo|, bar"),
        c("foo|, bar", "w", "foo, |bar"),
        c("|foo, bar", "W", "foo, |bar"),
        c("|a b c d e", "4w", "a b c d |e"),
        c("|a b c", "9w", "a b |c"),
    ]);
}

#[test]
fn words_backward() {
    run(&[
        c("foo |bar", "b", "|foo bar"),
        c("foo ba|r", "b", "foo |bar"),
        c("foo\n|bar", "b", "|foo\nbar"),
        c("foo\n\n|bar", "b", "foo\n|\nbar"),
        c("|foo", "b", "|foo").bell(true),
        c("foo.b|ar", "2b", "foo|.bar"),
        c("foo.bar ba|z", "B", "foo.bar |baz"),
        c("foo.bar ba|z", "2B", "|foo.bar baz"),
        c("foo  |  bar", "b", "|foo    bar"),
        c("a b c d |e", "3b", "a |b c d e"),
        c("foo↵|bar", "b", "|foo↵bar"),
    ]);
}

#[test]
fn word_ends() {
    run(&[
        c("|foo bar", "e", "fo|o bar"),
        c("fo|o bar", "e", "foo ba|r"),
        c("|foo bar baz", "3e", "foo bar ba|z"),
        c("fo|o\nbar", "e", "foo\nba|r"),
        // e does not stop at empty lines.
        c("fo|o\n\nbar", "e", "foo\n\nba|r"),
        c("|foo.bar baz", "E", "foo.ba|r baz"),
        c("|foo.bar", "e", "fo|o.bar"),
        c("fo|o.bar", "e", "foo|.bar"),
        c("foo ba|r", "e", "foo ba|r").bell(true),
        c("foo ba|r", "ge", "fo|o bar"),
        c("foo |bar", "ge", "fo|o bar"),
        c("foo\n|bar", "ge", "fo|o\nbar"),
        c("foo.bar ba|z", "gE", "foo.ba|r baz"),
        c("foo.ba|r", "ge", "foo|.bar"),
        c("a b c |d", "2ge", "a |b c d"),
        c("fo|o", "ge", "|foo"),
    ]);
}

#[test]
fn words_in_prose_and_other_scripts() {
    run(&[
        // An apostrophe inside a word belongs to it.
        c("|don't stop", "w", "don't |stop"),
        c("|don\u{2019}t stop", "e", "don\u{2019}|t stop"),
        c("it's |fine", "b", "|it's fine"),
        // Vim's own rule is available.
        c("|don't stop", "w", "don|'t stop").strict_words(),
        // A quoted word is not glued to its quotes.
        c("|'quoted' x", "w", "'|quoted' x"),
        c("|naïve café", "w", "naïve |café"),
        c("|naïve café", "e", "naïv|e café"),
        c("|日本語 text", "w", "日本語 |text"),
        // Scripts are separate words, as in Vim.
        c("|漢字かな", "w", "漢字|かな"),
        c("|hi 👍 there", "w", "hi |👍 there"),
        c("hi |👍 there", "w", "hi 👍 |there"),
        c("|Привет мир", "w", "Привет |мир"),
        c("Привет ми|р", "b", "Привет |мир"),
        c("|snake_case x", "w", "snake_case |x"),
    ]);
}

#[test]
fn find_character() {
    run(&[
        c("|hello world", "fo", "hell|o world"),
        c("|hello world", "2fo", "hello w|orld"),
        c("|hello world", "to", "hel|lo world"),
        c("hello wor|ld", "Fo", "hello w|orld"),
        c("hello wor|ld", "To", "hello wo|rld"),
        c("hello wor|ld", "2Fo", "hell|o world"),
        c("|a.b.c.d", "f.;", "a.b|.c.d"),
        c("|a.b.c.d", "f.;;", "a.b.c|.d"),
        c("|a.b.c.d", "f.;,", "a|.b.c.d"),
        c("|ab.cd.e", "t.", "a|b.cd.e"),
        c("|ab.cd.e", "t.;", "ab.c|d.e"),
        c("|ab.cd.e", "t.;,", "ab.|cd.e"),
        c("|a.b.c.d.e", "f.2;", "a.b.c|.d.e"),
        c("|hello", "fz", "|hello").bell(true),
        c("|ab\ncd", "fc", "|ab\ncd").bell(true),
        c("|ab↵cd", "fc", "|ab↵cd").bell(true),
        c("|añb", "fñ", "a|ñb"),
        c("|a b c", "f ", "a| b c"),
        c("|hello", ";", "|hello").bell(true),
        c("|a.b.c", "3f.", "|a.b.c").bell(true),
    ]);
}

#[test]
fn matching_brackets() {
    run(&[
        c("|(a [b] c)", "%", "(a [b] c|)"),
        c("(a [b] c|)", "%", "|(a [b] c)"),
        c("(a |[b] c)", "%", "(a [b|] c)"),
        c("|foo (bar)", "%", "foo (bar|)"),
        c("|foo", "%", "|foo").bell(true),
        c("|((a)(b))", "%", "((a)(b)|)"),
        c("(|(a)(b))", "%", "((a|)(b))"),
        c("|{a\nb}", "%", "{a\nb|}"),
        c("{a\nb|}", "%", "|{a\nb}"),
        c("|(unclosed", "%", "|(unclosed").bell(true),
        // With a count: that percentage of the document.
        c("|a\nb\nc\nd", "50%", "a\n|b\nc\nd"),
        c("|a\nb\nc\nd", "100%", "a\nb\nc\n|d"),
        c("a\nb\nc\n|d", "1%", "|a\nb\nc\nd"),
    ]);
}

#[test]
fn document_lines() {
    run(&[
        c("a\nb\n|c", "gg", "|a\nb\nc"),
        c("|a\nb\nc", "G", "a\nb\n|c"),
        c("|a\nb\nc", "2G", "a\n|b\nc"),
        c("a\nb\n|c", "2gg", "a\n|b\nc"),
        c("|a\nb\nc", "9G", "a\nb\n|c"),
        c("|a\n  b", "G", "a\n  |b"),
        c("  a\n|b", "gg", "  |a\nb"),
        c("|a\n  b\nc", "+", "a\n  |b\nc"),
        c("|a\n  b\nc", "<CR>", "a\n  |b\nc"),
        c("|a\nb\nc", "2+", "a\nb\n|c"),
        c("a\n  b\n|c", "-", "a\n  |b\nc"),
        c("  a|b", "_", "  |ab"),
        c("|a\n  b", "2_", "a\n  |b"),
        c("a\n|b", "+", "a\n|b").bell(true),
        c("|a\nb", "-", "|a\nb").bell(true),
        // A block with soft breaks is one line for these.
        c("|a↵b\nc", "+", "a↵b\n|c"),
        c("a↵b\n|c", "-", "|a↵b\nc"),
    ]);
}

#[test]
fn paragraphs() {
    run(&[
        c("|a\nb\n\nc\nd", "}", "a\nb\n|\nc\nd"),
        c("|a\nb\n\nc\nd", "}}", "a\nb\n\nc\n|d"),
        c("|a\nb\n\nc\nd", "2}", "a\nb\n\nc\n|d"),
        c("a\nb\n\nc\n|d", "{", "a\nb\n|\nc\nd"),
        c("a\nb\n\nc\n|d", "{{", "|a\nb\n\nc\nd"),
        c("a\n|b\n\nc", "{", "|a\nb\n\nc"),
        // A divider is a boundary too.
        c("|a\n---\nb", "}", "a\n|---\nb"),
        c("a\n---\n|b", "{", "a\n|---\nb"),
        c("a\n|\n\nb\n\nc", "}", "a\n\n\nb\n|\nc"),
        c("|abc", "}", "ab|c"),
        c("ab|c", "}", "ab|c").bell(true),
        c("|abc", "{", "|abc").bell(true),
    ]);
}

#[test]
fn sentences() {
    run(&[
        c("|One. Two. Three.", ")", "One. |Two. Three."),
        c("|One. Two. Three.", "2)", "One. Two. |Three."),
        c("One. Two. |Three.", "(", "One. |Two. Three."),
        c("One. T|wo. Three.", "(", "One. |Two. Three."),
        c("One. |Two.\nThree.", ")", "One. Two.\n|Three."),
        c("One.\n|Two.", "(", "|One.\nTwo."),
        c("|Really? Yes! Fine.", "))", "Really? Yes! |Fine."),
        c(
            "|He said \u{201C}no.\u{201D} Then left.",
            ")",
            "He said \u{201C}no.\u{201D} |Then left.",
        ),
        c("|One.\n\nTwo.", ")", "One.\n|\nTwo."),
        c("One. Tw|o", ")", "One. Tw|o").bell(true),
        c("|One. Two.", "(", "|One. Two.").bell(true),
        c("|e.g. this is one", ")", "e.g. |this is one"),
    ]);
}

#[test]
fn rows() {
    run(&[
        // One block of ten characters wraps into two rows of five.
        c("ab|cdefghij", "j", "abcdefg|hij").width(5),
        c("abcdefg|hij", "k", "ab|cdefghij").width(5),
        c("ab|cdefghij", "gj", "abcdefg|hij").width(5),
        c("abcdefg|hij", "gk", "ab|cdefghij").width(5),
        // A short row clamps; the goal column comes back.
        c("abc|defg\nhijkl", "j", "abcdef|g\nhijkl").width(5),
        c("abc|defg\nhijkl", "jj", "abcdefg\nhij|kl").width(5),
        c("ab|c\n\nxyz", "jj", "abc\n\nxy|z"),
        c("ab|c\n\nxyz", "j", "abc\n|\nxyz"),
        // $ makes vertical moves stick to row ends.
        c("ab|c\nde\nfghij", "$jj", "abc\nde\nfghi|j"),
        c("abcd|e\nfg\nhi", "$kjj", "abcde\nfg\nh|i"),
        c("|a\nb\nc\nd", "2j", "a\nb\n|c\nd"),
        c("|a\nb\nc\nd", "9j", "a\nb\nc\n|d"),
        c("a\nb\nc\n|d", "2k", "a\n|b\nc\nd"),
        c("|a\nb", "k", "|a\nb").bell(true),
        c("a\n|b", "j", "a\n|b").bell(true),
        c("|a\nb", "<Down>", "a\n|b"),
        c("a\n|b", "<Up>", "|a\nb"),
        c("|a\nb", "<C-n>", "a\n|b"),
        c("a\n|b", "<C-p>", "|a\nb"),
        c("|ab\n---\ncd", "j", "ab\n|---\ncd"),
        c("a|b\n---\ncd", "jj", "ab\n---\nc|d"),
        // Soft breaks end rows.
        c("a|b↵cd\nef", "j", "ab↵c|d\nef"),
        c("a|b↵cd\nef", "jj", "ab↵cd\ne|f"),
        // A horizontal move resets the goal column.
        c("abc|d\nxy\nabcd", "jhj", "abcd\nxy\n|abcd"),
        // A failed move does not.
        c("abc|d\nx\nabcd", "jhj", "abcd\nx\nabc|d"),
    ]);
}

#[test]
fn row_positions() {
    run(&[
        c("abcdefg|hij", "g0", "abcde|fghij").width(5),
        c("a|bcdefghij", "g$", "abcd|efghij").width(5),
        c("abcdefg|hij", "g$", "abcdefghi|j").width(5),
        c("  abcdefg|hij", "g^", "  abc|defghij").width(5),
        c("a|b", "g$", "a|b"),
        // g$ also makes j and k stick to row ends.
        c("a|bcdefghij", "g$j", "abcdefghi|j").width(5),
    ]);
}

#[test]
fn viewport_positions() {
    run(&[
        c("a\nb\n|c\nd\ne", "H", "|a\nb\nc\nd\ne").height(3),
        c("|a\nb\nc\nd\ne", "L", "a\nb\n|c\nd\ne").height(3),
        c("|a\nb\nc\nd\ne", "M", "a\n|b\nc\nd\ne").height(3),
        c("|a\nb\nc\nd\ne", "2H", "a\n|b\nc\nd\ne").height(3),
        c("|a\nb\nc\nd\ne", "2L", "a\n|b\nc\nd\ne").height(3),
        c("|a\nb\nc\nd\ne", "9H", "a\nb\n|c\nd\ne").height(3),
        c("|a\n  b\nc", "M", "a\n  |b\nc").height(3),
    ]);
}

#[test]
fn marks() {
    run(&[
        c("a|bc\ndef", "majG'a", "|abc\ndef"),
        c("a|bc\ndef", "maj`a", "a|bc\ndef"),
        c("|a\nb\nc", "G''", "|a\nb\nc"),
        c("a\n|b\nc", "G``", "a\n|b\nc"),
        c("|a", "'z", "|a").message("E20"),
        c("|a", "`z", "|a").message("E20"),
        // A mark follows its block when lines are added above.
        c("a\n|b", "mbggOnew<Esc>'b", "new\na\n|b"),
        c("a\nb|c", "mbggOnew<Esc>`b", "new\na\nb|c"),
        // A mark in a deleted block is gone.
        c("a\n|b\nc", "mbddgg'b", "|a\nc").message("E20"),
        // Upper-case marks work inside the document.
        c("a|b\ncd", "mAG`A", "a|b\ncd"),
    ]);
}

#[test]
fn jump_list() {
    run(&[
        c("|a\nb\nc", "G<C-o>", "|a\nb\nc"),
        c("|a\nb\nc", "G<C-o><C-i>", "a\nb\n|c"),
        c("|a\nb\nc", "G<C-o><Tab>", "a\nb\n|c"),
        c("|a\nb\nc\nd", "2GG<C-o><C-o>", "|a\nb\nc\nd"),
        c("|a\nb\nc\nd", "2GG2<C-o>", "|a\nb\nc\nd"),
        c("|a\nb", "<C-o>", "|a\nb").bell(true),
        c("|a\nb", "<C-i>", "|a\nb").bell(true),
        c("|a (b\nc) d", "%<C-o>", "|a (b\nc) d"),
    ]);
}
