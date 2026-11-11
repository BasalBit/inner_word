//! Sequences as people type them, several commands in a row.

use crate::harness::*;

#[test]
fn editing_sessions() {
    run(&[
        c(
            "|The quick brown fox",
            "wcwslow<Esc>wdwibig <Esc>",
            "The slow big| fox",
        ),
        c("|one\ntwo\nthree", "ddjp", "two\nthree\n|one"),
        c("|item", "yypA 2<Esc>", "item\nitem |2"),
        c("|a\nb", "Jx", "a|b"),
        c(
            "|cat\ncat dog\ncats",
            ":%s/cat/dog/g<CR>",
            "dog\ndog dog\n|dogs",
        ),
        c("|abc", "xxxuu<C-r>", "|c"),
        c("call(|a, b) now", "ci(x<Esc>", "call(|x) now"),
        c("|one. two. three.", "dt..", "|. three."),
        c("|one, two", "ct,three<Esc>", "thre|e, two"),
        c("|foo bar", "eas<Esc>", "foo|s bar"),
        c("|a\tb", "w", "a\t|b"),
        c("|a\nb\nc", "VjyGp", "a\nb\nc\n|a\nb"),
        c("a\n|b\nc", "ggVGd", "|"),
        c("|a\nb", "ggVG>", "{1}|a\n{1}b"),
        c("|foo foobar foo", ":%s/\\<foo\\>/x/g<CR>", "|x foobar x"),
        c("|foo bar\n\nbaz", "gUap", "|FOO BAR\n\nbaz"),
        c("|one two three", "wdwP", "one two| three"),
        c("|foo bar", "0D", "|"),
        c("  |foo bar", "^Cx<Esc>", "  |x"),
        c("|foo", "gg=G", "|foo").bell(true),
        c("|a b c", "wywPb", "a |b b c"),
        c(
            "|x",
            "ione<CR>two<CR>three<Esc>ggjddkP",
            "|two\none\nthreex",
        ),
    ]);
}

#[test]
fn writing_prose() {
    run(&[
        // A new paragraph after a heading, typed with the editor's rules.
        c("|", "i# Chapter<CR>It began.<Esc>", "# Chapter\nIt began|.").rules(),
        c(
            "# |Chapter",
            "oFirst.<Esc>oSecond.<Esc>",
            "# Chapter\nFirst.\nSecond|.",
        ),
        // Fixing a word in the middle of a sentence.
        c(
            "It was a |drak night.",
            "cwdark<Esc>",
            "It was a dar|k night.",
        ),
        c("It was a d|rak night.", "xp", "It was a da|rk night."),
        // Moving a sentence to the end of its paragraph.
        c("|One. Two. Three.", "das$a <Esc>p", "Two. Three. One.| "),
        // Splitting and joining paragraphs.
        c("One. |Two.", "i<CR><Esc>", "One. \n|Two."),
        c("One.\n|Two.", "kJ", "One.| Two."),
        // A list: new items, reordering, nesting.
        c(
            "- |milk",
            "oeggs<Esc>obread<Esc>",
            "- milk\n- eggs\n- brea|d",
        ),
        c(
            "- milk\n- |eggs\n- bread",
            "ddp",
            "- milk\n- bread\n- |eggs",
        ),
        c(
            "- milk\n- |eggs\n- bread",
            ">>",
            "- milk\n{1}- |eggs\n- bread",
        ),
        c(
            "- milk\n- |eggs\n- bread",
            "Vj>",
            "- milk\n{1}- |eggs\n{1}- bread",
        ),
        // Emphasis survives corrections.
        c("a **bolt|** move", "hrd", "a **bol|d** move"),
        c("a **bold|** move", "bcwbrave<Esc>", "a **brav|e** move"),
        // Quoted speech.
        c(
            "She said \u{201C}no|,\u{201D} and left.",
            "ci\"yes<Esc>",
            "She said \u{201C}ye|s\u{201D} and left.",
        ),
        c(
            "She said \u{201C}n|o,\u{201D} and left.",
            "da\"",
            "She said |and left.",
        ),
        // Searching for a phrase and changing it everywhere.
        c(
            "|the cat sat. The cat ran.",
            ":s/the cat/a dog/g<CR>",
            "|a dog sat. a dog ran.",
        ),
        c(
            "|the cat sat. The cat ran.",
            ":s/the cat/a dog/gI<CR>",
            "|a dog sat. The cat ran.",
        ),
        c(
            "|the cat sat. The cat ran.",
            "/cat<CR>cwdog<Esc>n.",
            "the dog sat. The do|g ran.",
        ),
    ]);
}
