//! `:` commands.

use crate::harness::*;
use writer_vim::{CommandLine, Effect, HostRequest, Mode};

#[test]
fn host_requests() {
    let req = |keys: &str| session("|a\nb\nc", keys).requests();
    assert_eq!(req(":w<CR>"), vec![HostRequest::Save]);
    assert_eq!(req(":write<CR>"), vec![HostRequest::Save]);
    assert_eq!(req(":wa<CR>"), vec![HostRequest::Save]);
    assert_eq!(req(":q<CR>"), vec![HostRequest::Close { force: false }]);
    assert_eq!(req(":q!<CR>"), vec![HostRequest::Close { force: true }]);
    assert_eq!(req(":qa<CR>"), vec![HostRequest::Close { force: false }]);
    assert_eq!(req(":wq<CR>"), vec![HostRequest::SaveAndClose]);
    assert_eq!(req(":x<CR>"), vec![HostRequest::SaveAndClose]);
    assert_eq!(req("ZZ"), vec![HostRequest::SaveAndClose]);
    assert_eq!(req("ZQ"), vec![HostRequest::Close { force: true }]);
    // Anything else is the application's.
    assert_eq!(
        req(":export md<CR>"),
        vec![HostRequest::Command {
            name: "export".into(),
            args: "md".into(),
            bang: false,
            range: None
        }]
    );
    assert_eq!(
        req(":2,3fmt now<CR>"),
        vec![HostRequest::Command {
            name: "fmt".into(),
            args: "now".into(),
            bang: false,
            range: Some(1..3)
        }]
    );
    assert_eq!(
        req(":%sort!<CR>"),
        vec![HostRequest::Command {
            name: "sort".into(),
            args: String::new(),
            bang: true,
            range: Some(0..3)
        }]
    );
    assert_eq!(
        req(":w notes.md<CR>"),
        vec![HostRequest::Command {
            name: "write".into(),
            args: "notes.md".into(),
            bang: false,
            range: None
        }]
    );
    // After every command the engine is back in Normal mode.
    assert_eq!(session("|a", ":w<CR>").mode(), Mode::Normal);
    assert_eq!(session("|a", ":<CR>").requests(), vec![]);
}

#[test]
fn going_to_lines() {
    run(&[
        c("|a\nb\nc", ":2<CR>", "a\n|b\nc"),
        c("|a\nb\nc", ":$<CR>", "a\nb\n|c"),
        c("a\nb\n|c", ":1<CR>", "|a\nb\nc"),
        c("a\n|b\nc", ":+1<CR>", "a\nb\n|c"),
        c("a\n|b\nc", ":-1<CR>", "|a\nb\nc"),
        c("|a\nb\nc", ":99<CR>", "a\nb\n|c"),
        c("a\n|b\nc", ":0<CR>", "|a\nb\nc"),
        c("|a\n  b", ":2<CR>", "a\n  |b"),
        c("|a\nb\nc\nd", ":.+2<CR>", "a\nb\n|c\nd"),
        c("a\n|b\nc", "ma:1<CR>:'a<CR>", "a\n|b\nc"),
        c("|a\nb\nc", ":3<CR><C-o>", "|a\nb\nc"),
        c("|a", ":'q<CR>", "|a").message("E20"),
    ]);
}

#[test]
fn deleting_and_yanking_lines() {
    run(&[
        c("a\n|b\nc", ":d<CR>", "a\n|c").regs(&[('"', "b\n")]),
        c("|a\nb\nc\nd", ":2,3d<CR>", "a\n|d"),
        c("a\n|b", ":%d<CR>", "|"),
        c("|a\nb", ":d x<CR>", "|b").regs(&[('x', "a\n")]),
        c("a\n|b\nc\nd", ":.,$d<CR>", "|a"),
        c("|a\nb\nc\nd", "majmb:'a,'bd<CR>", "|c\nd"),
        c("|a\nb\nc\nd", ":3,2d<CR>", "a\n|d"),
        c("|a\nb", ":%y<CR>", "|a\nb").regs(&[('"', "a\nb\n")]),
        c("|a\nb", ":2y z<CR>", "|a\nb").regs(&[('z', "b\n")]),
        c("|a\nb\nc\nd", "3:d<CR>", "|d"),
        c("|a\nb\nc", "Vj:d<CR>", "|c"),
        c("# |H\nb", ":d<CR>p", "b\n# |H"),
    ]);
}

#[test]
fn substitute() {
    run(&[
        c("|foo foo", ":s/foo/bar/<CR>", "|bar foo").message("1 substitution on 1 line"),
        c("|foo foo", ":s/foo/bar/g<CR>", "|bar bar"),
        c(
            "|foo\nx foo\nfoo foo",
            ":%s/foo/bar/g<CR>",
            "bar\nx bar\n|bar bar",
        )
        .message("4 substitutions on 3 lines"),
        c(
            "|foo\nx foo\nfoo foo",
            ":%s/foo/bar/<CR>",
            "bar\nx bar\n|bar foo",
        ),
        // Lower case finds any case, like a search; i and I decide.
        c("|Foo foo", ":s/foo/x/g<CR>", "|x x"),
        c("|Foo foo", ":s/foo/x/gI<CR>", "|Foo x"),
        c("|Foo foo", ":s/Foo/x/g<CR>", "|x foo"),
        c("|Foo foo", ":s/Foo/x/gi<CR>", "|x x"),
        c("|foo", ":s/foo/[&]/<CR>", "|[foo]"),
        c("|foo", ":s/foo/\\&/<CR>", "|&"),
        c("|foo", ":s/foo/<\\0>/<CR>", "|<foo>"),
        c("|foo bar", ":s/foo //<CR>", "|bar"),
        c("|foo bar", ":s/foo /<CR>", "|bar"),
        c("|a/b", ":s#a/b#x#<CR>", "|x"),
        c("|a/b", ":s/a\\/b/x/<CR>", "|x"),
        c("|foo\nfoo\nfoo", ":2,3s/foo/x/<CR>", "foo\nx\n|x"),
        c("|foo", ":s/zzz/x/<CR>", "|foo").message("E486"),
        c("|foo foo", ":s/foo//gn<CR>", "|foo foo").message("2 matches on 1 line"),
        c("|a,b", ":s/,/\\r/<CR>", "|a↵b"),
        c("|a.b axb", ":s/a.b/Z/g<CR>", "|Z axb"),
        c("|añb", ":s/ñ/nn/<CR>", "|annb"),
        c("|foo\nfoo\nfoo", "Vj:s/foo/x/<CR>", "x\n|x\nfoo"),
        c("a\n|b\n---\nb", ":%s/b/x/<CR>", "a\nx\n---\n|x"),
    ]);
}

#[test]
fn substitute_again() {
    run(&[
        // An empty pattern is the last search.
        c("|foo bar foo", "/bar<CR>:s//X/<CR>", "|foo X foo"),
        c("|a a\na a", ":s/a/b/<CR>j&", "b a\n|b a"),
        c("|a a\na a", ":s/a/b/<CR>j:s<CR>", "b a\n|b a"),
        c("|a a\na a", ":s/a/b/g<CR>j:&&<CR>", "b b\n|b b"),
        c("|a a\na a", ":s/a/b/g<CR>j:&<CR>", "b b\n|b a"),
        c("|a a\na a", ":s/a/b/g<CR>g&", "b b\n|b b"),
        c("|a", "&", "|a").message("E35"),
        // A substitution sets the search pattern.
        c("|a b a b", ":s/b/B/<CR>n", "a |B a b"),
    ]);
}

#[test]
fn other_commands() {
    run(&[
        // The cursor goes to the first non-blank of the joined line.
        c("|a\nb\nc", ":%j<CR>", "|a b c"),
        c("|a\nb", ":j<CR>", "|a b"),
        c("|a\nb\nc", ":1,2j<CR>", "|a b\nc"),
        c("|a\nb", ":%><CR>", "{1}|a\n{1}b"),
        c("{2}|a", ":<<<CR>", "|a"),
        c("|a\nb\nc", ":2><CR>", "a\n{1}|b\nc"),
        c("|abc", "x:u<CR>", "|abc"),
        c("|abc", "xu:red<CR>", "|bc"),
        c("|abc", "x:undo<CR>:redo<CR>", "|bc"),
        c("|a", ":!ls<CR>", "|a").message("E492"),
        c("|a", ":s/a/b/c<CR>", "|a").message("not supported"),
        c("|a", ":s/a/b/z<CR>", "|a").message("E488"),
        c("|a", ":d !<CR>", "|a").message("E488"),
        c("|a b", ":  s/a/x/  <CR>", "|x b"),
        c("|a\nb", ":2<Esc>x", "|\nb"),
        c("|a\nb", ":<BS>x", "|\nb"),
    ]);
}

#[test]
fn command_line_keys() {
    run(&[
        c("|foo bar", "yiw:s/<C-r>\"/X/<CR>", "|X bar"),
        c("|a\nb\nc", ":3<CR>gg:<Up><CR>", "a\nb\n|c"),
        c("|a\nb\nc", ":2<CR>:3<CR>gg:<Up><Up><CR>", "a\n|b\nc"),
        c("|a\nb\nc", ":2<CR>gg:<C-p><CR>", "a\n|b\nc"),
        c("|a\nb", ":xd<Home><Del><End><CR>", "|b"),
        c("|a\nb", ":d2<Left><BS><Right><BS>2d<CR>", "|a"),
        c("|a\nb\nc", ":2<C-m>", "a\n|b\nc"),
    ]);
}

#[test]
fn command_line_view() {
    let mut s = Session::new("|a\nb");
    s.clear_effects();
    s.keys(":");
    assert_eq!(s.mode(), Mode::CommandLine);
    assert!(s.effects.contains(&Effect::ModeChanged(Mode::CommandLine)));
    assert!(s.effects.contains(&Effect::CommandLine(Some(CommandLine {
        prefix: ':',
        text: String::new(),
        cursor: 0
    }))));
    s.clear_effects();
    s.keys("2x<Left>");
    assert_eq!(
        s.effects.last(),
        Some(&Effect::CommandLine(Some(CommandLine {
            prefix: ':',
            text: "2x".into(),
            cursor: 1
        })))
    );
    // Text that does not come as keys (an input method).
    let o = s.vim.command_line_input("ñ", &mut s.ed, &mut s.host);
    assert!(o.consumed);
    assert_eq!(s.vim.command_line().unwrap().text, "2ñx");
    s.clear_effects();
    s.keys("<Esc>");
    assert!(s.effects.contains(&Effect::CommandLine(None)));
    assert!(s.effects.contains(&Effect::ModeChanged(Mode::Normal)));
    // Outside the command line the call does nothing.
    let o = s.vim.command_line_input("x", &mut s.ed, &mut s.host);
    assert!(!o.consumed);
    // In Visual mode the range is filled in.
    let mut s = Session::new("|a\nb");
    s.keys("V:");
    assert_eq!(s.vim.command_line().unwrap().text, "'<,'>");
}

#[test]
fn run_directly() {
    let mut s = Session::new("|a\nb\nc");
    let o = s.vim.execute("3", &mut s.ed, &mut s.host);
    assert!(o.consumed);
    assert_eq!(s.buffer(), "a\nb\n|c");
    s.vim.execute("%s/b/x/", &mut s.ed, &mut s.host);
    assert_eq!(s.text(), "a\nx\nc");
    let o = s.vim.execute("export pdf", &mut s.ed, &mut s.host);
    assert!(o.effects.contains(&Effect::Request(HostRequest::Command {
        name: "export".into(),
        args: "pdf".into(),
        bang: false,
        range: None
    })));
    // From Insert mode the session ends first.
    s.keys("ix");
    s.vim.execute("1", &mut s.ed, &mut s.host);
    assert_eq!(s.mode(), Mode::Normal);
    s.keys("u");
    assert_eq!(s.text(), "a\nx\nc");
}
