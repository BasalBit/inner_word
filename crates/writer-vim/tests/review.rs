//! Cases from the review against a real Vim: behaviour that was wrong or
//! missing, checked here on rich text, soft line breaks and dividers, which
//! the corpus of plain paragraphs cannot reach.

use writer_vim::Mode;

use crate::harness::{c, run};

#[test]
fn bracket_objects_look_through_the_document() {
    run(&[
        // Outside any pair: the next pair, in a later block too.
        c("x|y\n(a b)", "di(", "xy\n(|)"),
        c("|xy\n\n# Title (x)", "ci(y<Esc>", "xy\n\n# Title (|y)"),
        // A pair that spans blocks.
        c("(a\nb|c\nd)", "di(", "(|)"),
        c("(a\nb|c\nd)", "da(", "|"),
        // Formatting around the pair stays.
        c("a **(b|c)** d", "di(", "a **(|)** d"),
        // A count takes the pair further out, and fails without one.
        c("(a (b|c) d)", "d2i(", "(|)"),
        c("(b|c)", "d2i(", "(b|c)").bell(true),
        // A close bracket on the way hides the next open one, as in Vim.
        c("|a) (b) (c)", "di(", "|a) (b) (c)").bell(true),
        c("|a (b) (c)", "di(", "a (|) (c)"),
    ]);
}

#[test]
fn quote_objects_follow_vim() {
    run(&[
        // Not on a quote: from the nearest quote before the caret.
        c("\"a\" b|c \"d\"", "di\"", "\"a\"|\"d\""),
        // On a quote: pairs are counted from the start of the line.
        c("\"a\" bc |\"d\"", "di\"", "\"a\" bc \"|\""),
        // Before any quote: the first string on the line.
        c("|x \"a\" b", "di\"", "x \"|\" b"),
        // Visual mode on an empty string takes the quotes.
        c("a |\"\" b", "vi\"d", "a | b"),
        // An escaped quote does not end the string.
        c("\"a\\\"|b\" c", "di\"", "\"|\" c"),
    ]);
}

#[test]
fn paragraph_objects() {
    run(&[
        // A divider is a paragraph of its own, not a blank line.
        c("a|a\nbb\n---\ncc", "dap", "|---\ncc"),
        c("aa\n|---\n---\ncc", "dip", "aa\n|cc"),
        // A block of nothing but blanks separates paragraphs like an empty
        // one.
        c("a|a\n  \nbb", "yip", "|aa\n  \nbb").regs(&[('"', "aa\n")]),
        c("a|a\n  \nbb", "dap", "|bb"),
        // No blank lines after: the ones before go along.
        c("aa\n\nb|b", "dap", "|aa"),
        // Counts: fail when the document ends first.
        c("a|a\n\nbb", "d3ap", "a|a\n\nbb").bell(true),
        c("a|a\n\nbb\n\ncc", "d2ap", "|cc"),
        // Extending a selection keeps its mode.
        c("|aa\n\nbb\n\ncc", "vjipd", "|b\n\ncc"),
    ]);
}

#[test]
fn word_objects_cross_lines_as_in_vim() {
    run(&[
        // The last word of a line: no white space to take along.
        c("a|a\nbb", "daw", "|\nbb"),
        c("aa |bb\ncc", "daw", "a|a\ncc"),
        // On an empty line `aw` takes the line break and the next word.
        c("aa\n|\nbb cc", "daw", "aa\n| cc"),
        // A count that runs past the end fails and leaves the caret where
        // the search stopped.
        c("a|a bb", "d5aw", "aa b|b").bell(true),
    ]);
}

#[test]
fn counts_that_reach_past_the_end() {
    run(&[
        c("a\n|b", "3dd", "a\n|b").bell(true),
        c("|a\nb", "3dd", "|"),
        c("a\n|b", "2yy", "a\n|b")
            .bell(true)
            .regs(&[('"', "<unset>")]),
        c("|a\n\nb", "2}", "a\n\n|b"),
        c("|a\n\nb", "3}", "|a\n\nb").bell(true),
        c("a\n\n|b", "2{", "|a\n\nb"),
        c("a\n\n|b", "3{", "a\n\n|b").bell(true),
        c("|ab↵cd↵ef", "2$", "ab↵c|d↵ef"),
        c("|ab↵cd↵ef", "9$", "ab↵cd↵e|f"),
        c("ab↵cd↵|ef", "2$", "ab↵cd↵|ef").bell(true),
        c("|One. Two.", "5)", "|One. Two.").bell(true),
        // `3J` on the last line: nothing to join, the caret goes home.
        c("a\nb|c", "3J", "a\n|bc").bell(false),
        c("a\nb|c", "J", "a\nb|c").bell(true),
    ]);
}

#[test]
fn carets_after_linewise_operators() {
    run(&[
        // A yank leaves the caret alone going down, and moves it up with
        // the motion.
        c("ab\nc|d", "yk", "a|b\ncd").regs(&[('"', "ab\ncd\n")]),
        c("a|b\ncd", "yj", "a|b\ncd"),
        c("  a|b", "yy", "  a|b"),
        c("  a|b", "y_", "  a|b"),
        // Case operators on lines go to the first non-blank.
        c("  a|b", "gUU", "  |AB"),
        c("ab\nc|d", "gUk", "A|B\nCD"),
        // A linewise Visual selection: column one, unless the caret is on
        // a line above the start.
        c("a|b\ncd", "Vjy", "|ab\ncd"),
        c("ab\nc|d", "Vky", "a|b\ncd"),
    ]);
}

#[test]
fn undo_and_redo_carets() {
    run(&[
        c("ab\nc|d\nef", "dku", "a|b\ncd\nef"),
        c("ab\nc|d\nef", "dju", "ab\nc|d\nef"),
        c("  a|b\ncd", "ddu", "  |ab\ncd"),
        c("ab|c", "gUiwu", "|abc"),
        c("a b|c", "gUbu", "a |bc"),
        // A change of several lines: Vim comes back on the second.
        c("|a\nb\nc", "2ccX<Esc>u", "a\n|b\nc"),
        // Redo returns to the same place, by position.
        c("ab\nc|d", "yyPu<C-r>", "ab\nc|d\ncd"),
        // Lines removed at the end: the last line that is left.
        c("ab\n  c|d", "ddu<C-r>", "|ab"),
        // `:s` comes back to the start of the first changed line.
        c("ab\nc|b", ":%s/b/x/<CR>u", "|ab\ncb"),
        // Text of a session up to a cursor key is an undo step of its own.
        c("|ab", "ix<Right>y<Esc>u", "xa|b"),
    ]);
}

#[test]
fn register_rules() {
    run(&[
        // A named register also fills register 1, not the small one.
        c("|foo bar", "\"adw", "|bar").regs(&[('a', "foo "), ('1', "foo "), ('-', "<unset>")]),
        // Appending: the unnamed register shows what was deleted.
        c("|ab cd", "\"ayw\"Adw", "|cd").regs(&[('a', "ab ab "), ('"', "ab ")]),
        // An empty document: `dd` deletes nothing and yanks nothing.
        c("|", "dd", "|").regs(&[('"', "<unset>")]),
        // Nothing to delete is not an error, and `.` repeats the command.
        c("|\nab", "xj.", "\n|b").bell(false),
        // `cw` on an empty line is `cc`.
        c("a\n|\nb", "cwX<Esc>", "a\n|X\nb").regs(&[('"', "\n")]),
        // `D` with a count from the first column takes the lines.
        c("|a\nb\nc", "2D", "|c").regs(&[('"', "a\nb\n")]),
        c("a|a\nb\nc", "2D", "|a\nc"),
    ]);
}

#[test]
fn visual_mode_details() {
    run(&[
        // Backspace deletes, and fills register 1 as well.
        c("a|bc", "v<BS>", "a|c").regs(&[('1', "b"), ('-', "b")]),
        // `l` can take the line break.
        c("a|b\ncd", "vld", "a|cd"),
        // Lines put over characters split the line; a heading stays one.
        c("# |Head\nabc", "yyjlvp", "# Head\na\n# |Head\nc"),
        // An empty register: the selection is deleted.
        c("a|bc", "vp", "a|c").regs(&[('"', "b")]),
        // `gv` after the lines were deleted takes the line that moved up.
        c("a\n|b\nc", "Vdgvd", "|a"),
        c("a\n|b", "Vdgv", "|a").bell(true),
        // `gv` after a put selects what was put.
        c("|ab cd", "ywwviwpgvy", "ab |ab ").regs(&[('"', "ab ")]),
        // `gv` after an operator was started drops the operator.
        c("|ab cd", "vly$dgvd", "| cd"),
        // `'>` of a linewise selection is the end of its last line.
        c("a|bc\ndef", "Vj<Esc>gg`>", "abc\nde|f"),
        // A failing Visual-mode `J` leaves the caret at the selection.
        c("ab\nc|d", "VJ", "ab\n|cd").bell(true),
    ]);
}

#[test]
fn jumps_and_the_context_mark() {
    run(&[
        // Before the first jump `''` goes to the top.
        c("a\nb\n  |c", "''", "|a\nb\n  c"),
        c("|a\nb\nc", "G<C-o>", "|a\nb\nc"),
        // Going back sets the mark: `''` returns.
        c("|a\nb\nc", "G<C-o>''", "a\nb\n|c"),
        // Two places in one block are two entries until the list is used.
        c("|foo bar foo", "*<C-o>", "|foo bar foo"),
        // A jump that ends where it started does not move the mark.
        c("a\n|b", "ggG1G2G''", "|a\nb"),
    ]);
}

#[test]
fn insert_mode_keys() {
    run(&[
        // ctrl-u and ctrl-w stop where typing started, then go on.
        c("ab |cd", "ixy<C-u>", "ab |cd").insert(),
        c("ab |cd", "ixy<C-u><C-u>", "|cd").insert(),
        c("foo|bar", "ixy<C-w>", "foo|bar").insert(),
        c("foo|bar", "ixy<C-w><C-w>", "|bar").insert(),
        // Replace mode: they move back and restore, they do not delete.
        c("one |two", "R<C-w>", "|one two").mode(Mode::Replace),
        c("|abc", "Rxy<C-w>", "|abc").mode(Mode::Replace),
        // ctrl-o at the end of a line.
        c("|abc def", "A<C-o>D<Esc>", "abc d|e"),
        c("|abc", "A<C-o>~X<Esc>", "abC|X"),
        c("|abc", "i<C-o>$X<Esc>", "abc|X"),
        // What repeats after ctrl-o is a plain insert of what came after.
        c("|ab cd", "ix<C-o>wy<Esc>0.", "|yxab ycd"),
        // ... and the session so far when nothing came after.
        c("|ab cd", "ix<C-o>w<Esc>0.", "|xxab cd"),
        // A cursor key that does not move is not an interruption.
        c("|ab", "3Ax<Right><Esc>", "abxx|x"),
        // A backspace that deletes nothing is not repeated.
        c("|ab\ncd", "i<BS>x<Esc>j$.", "xab\nc|xd"),
        // Counts open lines even with nothing typed.
        c("|a", "3o<Esc>", "a\n\n\n|"),
    ]);
}

#[test]
fn sentences_and_empty_blocks() {
    run(&[
        c("|One. Two.\n\n\nThree.", "))", "One. Two.\n|\n\nThree."),
        c("|One. Two.\n\n\nThree.", ")))", "One. Two.\n\n\n|Three."),
        c("One. Two.\n\n\n|Three.", "(", "One. Two.\n\n|\nThree."),
        c("One. Two.\n\n\n|Three.", "((", "One. |Two.\n\n\nThree."),
        // A divider is an empty line here too.
        c("|One.\n---\nTwo.", "))", "One.\n---\n|Two."),
        // On an empty line the object is the empty lines up to the text.
        c("A.\n|\n\nB.", "das", "A.\n|B."),
        c("A.\n|\n", "das", "A.\n|\n").bell(true),
        // White space before the first sentence belongs to it.
        c("|  One. Two.", ")", "  One. |Two."),
    ]);
}

#[test]
fn motions_with_operators() {
    run(&[
        // The last word of a line ends with the line, blanks included.
        c("|ab  \ncd", "dw", "|\ncd"),
        c("|ab  \ncd", "2dw", "|"),
        // `^` on a line of blanks is its last character.
        c("| a\n   ", "j^", " a\n  | "),
        // Backspace as a motion: `d` takes the line break, other operators
        // the last character of the line before.
        c("ab\n|cd", "d<BS>", "ab|cd"),
        c("ab\n|cd", "y<BS>", "a|b\ncd").regs(&[('"', "b")]),
        c("ab↵|cd", "d<BS>", "ab|cd"),
        // `|` never goes past the last character, also for an operator.
        c("a|bc", "d9|", "a|c"),
        // Empty motions are not errors.
        c("|abc", "dh", "|abc").bell(false),
        c("|", "cwx<Esc>", "|x"),
        // `$` on a range that ends in an empty line stays characterwise
        // (from the first column the delete takes the lines, as `2dd`).
        c("a|b\n\ncd", "d2$", "|a\ncd"),
        c("|ab\n\ncd", "d2$", "|cd"),
    ]);
}

#[test]
fn ex_commands_with_counts() {
    run(&[
        c("|a\nb\nc", ":d 2<CR>", "|c").regs(&[('"', "a\nb\n")]),
        c("a\n|b\nc\nd", ":1d 2<CR>", "|c\nd"),
        c("|a\nb\nc", ":y 2<CR>", "|a\nb\nc").regs(&[('"', "a\nb\n")]),
        c("|a\nb\nc\nd", ":j 3<CR>", "|a b c\nd"),
        c("|a\n b", ":j!<CR>", "|a b"),
        c("a\n|b", ":j<CR>", "a\n|b").bell(true),
        c("|a\nb", ":d x<CR>", "|b").regs(&[('x', "a\n")]),
    ]);
}
