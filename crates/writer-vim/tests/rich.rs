//! Rich text survives every operation.

use crate::harness::*;

#[test]
fn inline_formatting() {
    run(&[
        c("**bo|ld** x", "yiw$p", "**bold** x**bol|d**").regs(&[('"', "**bold**")]),
        c("a **b|c** d", "diwP", "a **b|c** d"),
        c("**fo|o** bar", "cwX<Esc>", "**fo|X** bar"),
        c("**f|oo** bar", "x", "**f|o** bar"),
        c("__a |b c__", "dw", "__a |c__"),
        c("**fo|o** bar", "gUiw", "**|FOO** bar"),
        c("f**o|o** bar", "g~iw", "|F**OO** bar"),
        c("**a|bc**", "rx", "**a|xc**"),
        c("**|ab**c", "3~", "**AB**|C"),
        c("**|abc**", "vlrx", "**|xxc**"),
        c("**|a**\n__b__", "J", "**a| **__b__"),
        c("a **|b** c", "ylP", "a **|bb** c"),
        c("**|foo** bar", ":s/foo/baz/<CR>", "**|baz** bar"),
        c("**|foo** bar", ":s/foo/quux/<CR>", "**|quux** bar"),
        c("f**o**|o", ":s/foo/bar/<CR>", "|b**a**r"),
        c("x **|ab** y", "vly$p", "x **ab** y**a|b**"),
        // A change takes the formatting of what it replaces.
        c("a **|bold** move", "cwbrave<Esc>", "a **brav|e** move"),
        c("a **b|old** move", "ciwbrave<Esc>", "a **brav|e** move"),
        c("**|bold** move", "cwbrave<Esc>", "**brav|e** move"),
        c("a __**|x**__ b", "sy<Esc>", "a **__|y**__ b"),
        c("**|all bold**", "ccnew<Esc>", "**ne|w**"),
        c("**|all bold**", "Cnew<Esc>", "**ne|w**"),
        c("plain **|bold**", "cbx<Esc>", "|x**bold**"),
        c("**|bold** plain", "wcwx<Esc>", "**bold** |x"),
        // Not once the caret has moved on.
        c("a **|bold** move", "cw<Esc>ix<Esc>", "a|x  move"),
        c("a **|bold** move", "cwbrave<Esc>w.", "a **brave** brav|e"),
        // Replace mode keeps the formatting of what it overwrites.
        c("a **|bold** move", "Rcalm<Esc>", "a **cal|m** move"),
        c("**|ab**cd", "Rwxyz<Esc>", "**wx**y|z"),
        c("**|ab**cd", "Rwx<BS><BS><Esc>", "**|ab**cd"),
        c("**|ab**cd", "Rwx<Esc>l.", "**wx**w|x"),
        // Mixed formatting inside one yank.
        c("|a**b**__c__", "y$$p", "a**b**__c__a**b**__|c__"),
        c("**stra|ße** x", "gUiw", "**|STRASSE** x"),
        c("a**ß**|b", "gUiw", "|A**SS**B"),
    ]);
}

#[test]
fn block_kinds_travel_with_lines() {
    run(&[
        c("# |Title\nbody", "ddp", "body\n# |Title").regs(&[('"', "# Title\n")]),
        c("- |a\n- b", "ddp", "- b\n- |a"),
        c("[x] |a\nb", "ddp", "b\n[x] |a"),
        c("{2}|a\nb", "ddp", "b\n{2}|a"),
        c("> |q\np", "yyjp", "> q\np\n> |q"),
        c("## |H\np", "yyjP", "## H\n## |H\np"),
        c("- |a\n1. b", "yj", "- |a\n1. b").regs(&[('"', "- a\n1. b\n")]),
        c("# |Head\npara", "Vjd", "|").regs(&[('"', "# Head\npara\n")]),
        c("# **|Bold** head\nx", "ddp", "x\n# **|Bold** head"),
        c("1. |a\n1. b\n1. c", "ddjp", "1. b\n1. c\n1. |a"),
        c("# |A\nb\n# C", ":1d<CR>:$<CR>p", "b\n# C\n# |A"),
    ]);
}

#[test]
fn text_takes_the_kind_of_where_it_lands() {
    run(&[
        // A piece of a heading pasted into a paragraph is paragraph text.
        c("# |Head\npara", "yiwjP", "# Head\nHea|dpara"),
        c("# |Head\npara", "vjy", "# |Head\npara").regs(&[('"', "# Head\np")]),
        // A change keeps the block it is made in.
        c("# |Title", "ccNew<Esc>", "# Ne|w"),
        c("[x] |done", "Sredo<Esc>", "[x] red|o"),
        c("- |a b", "cwx<Esc>", "- |x b"),
        // Joining keeps the first block.
        c("# |A\nb", "J", "# A| b"),
        c("|a\n# B", "J", "a| B"),
        // A merge by a characterwise delete keeps the first block too.
        c("# A|b\ncd", "vjd", "# |A"),
        c("- on|e\ntwo", "d/w<CR>", "- on|wo"),
    ]);
}

#[test]
fn indent_levels() {
    run(&[
        c("- |a\n- b", ">>", "{1}- |a\n- b"),
        c("{1}- |a", "<<", "- |a"),
        c("- |a\n- b\n- c", "Vj>", "{1}- |a\n{1}- b\n- c"),
        c("- |a\n- b\n- c", "Vj2>", "{2}- |a\n{2}- b\n- c"),
        c("{3}|a", "3<<", "{2}|a"),
        c("{3}|a\n{3}b", "V<.", "{1}|a\n{3}b"),
        c("- |a\n\n- b", ">2j", "{1}- |a\n{1}\n{1}- b"),
        c("|a\n\nb\n", ":%><CR>", "{1}|a\n{1}\n{1}b\n{1}"),
    ]);
}
