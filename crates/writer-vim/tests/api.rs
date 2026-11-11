//! The public surface: consumed keys, effects, attaching and syncing.

use crate::harness::*;
use writer_core::{Position, Selection};
use writer_vim::{CaretShape, Effect, Key, KeyCode, Mode, RegisterContent, Vim, VimOptions};

fn consumed(s: &mut Session, key: &str) -> bool {
    let k = Key::parse_sequence(key)[0];
    s.vim.handle_key(k, &mut s.ed, &mut s.host).consumed
}

#[test]
fn which_keys_are_consumed() {
    // Normal mode: every character, also one that means nothing.
    let mut s = Session::new("|abc def");
    assert!(consumed(&mut s, "x"));
    assert!(consumed(&mut s, "Q"));
    assert!(consumed(&mut s, "é"));
    assert!(consumed(&mut s, "<Left>"));
    assert!(consumed(&mut s, "<CR>"));
    assert!(consumed(&mut s, "<Tab>"));
    // Not: option chords, shifted arrows, unknown control chords, a lone
    // escape.
    assert!(!consumed(&mut s, "<A-x>"));
    assert!(!consumed(&mut s, "<S-Left>"));
    assert!(!consumed(&mut s, "<C-a>"));
    assert!(!consumed(&mut s, "<C-k>"));
    assert!(!consumed(&mut s, "<Esc>"));
    assert_eq!(s.text(), "bc def");
    // Escape is consumed when it cancels something.
    assert!(consumed(&mut s, "d"));
    assert!(consumed(&mut s, "<Esc>"));
    assert!(consumed(&mut s, "2"));
    assert!(consumed(&mut s, "<C-[>"));
    // A key that is not the engine's cancels a pending command, consumed.
    assert!(consumed(&mut s, "d"));
    assert!(consumed(&mut s, "<A-x>"));
    assert_eq!(s.mode(), Mode::Normal);

    // Insert mode: only what Vim owns.
    let mut s = Session::new("|abc");
    s.keys("i");
    for k in [
        "a", "Z", " ", "<CR>", "<BS>", "<Del>", "<Tab>", "<Left>", "<Up>", "<C-a>", "<C-e>",
        "<C-d>", "<C-k>", "<C-n>", "<A-BS>", "<S-CR>",
    ] {
        assert!(!consumed(&mut s, k), "{k} must be left to the host");
    }
    for k in [
        "<C-w>", "<C-u>", "<C-h>", "<C-r>", "a", "<C-o>", "l", "<Esc>",
    ] {
        assert!(consumed(&mut s, k), "{k} must be consumed");
    }
    assert_eq!(s.mode(), Mode::Normal);

    // Replace mode also owns backspace.
    let mut s = Session::new("|abc");
    s.keys("R");
    assert!(!consumed(&mut s, "x"));
    assert!(consumed(&mut s, "<BS>"));

    // Visual mode and the command line: everything typed.
    let mut s = Session::new("|abc");
    s.keys("v");
    assert!(consumed(&mut s, "l"));
    assert!(consumed(&mut s, "Q"));
    assert!(consumed(&mut s, "<Esc>"));
    s.keys(":");
    for k in ["a", "<Left>", "<Tab>", "<C-a>", "<Up>", "<Esc>"] {
        assert!(consumed(&mut s, k), "{k} must be consumed");
    }
}

#[test]
fn mode_effects() {
    let mut s = Session::new("|abc");
    s.clear_effects();
    s.keys("i<Esc>v<Esc>V<Esc>R<Esc>d<Esc>:<Esc>/<Esc>");
    let modes: Vec<Mode> = s
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::ModeChanged(m) => Some(*m),
            _ => None,
        })
        .collect();
    use Mode::*;
    assert_eq!(
        modes,
        vec![
            Insert,
            Normal,
            Visual,
            Normal,
            VisualLine,
            Normal,
            Replace,
            Normal,
            OperatorPending,
            Normal,
            CommandLine,
            Normal,
            CommandLine,
            Normal
        ]
    );
    assert!(Insert.is_insert() && Replace.is_insert() && !Normal.is_insert());
    assert!(Visual.is_visual() && VisualLine.is_visual() && !Insert.is_visual());
    assert_eq!(VisualLine.label(), "VISUAL LINE");
    assert_eq!(Normal.label(), "NORMAL");
}

#[test]
fn caret_shapes() {
    let mut s = Session::new("|abc");
    assert_eq!(s.vim.caret_shape(), CaretShape::Block);
    s.keys("d");
    assert_eq!(s.vim.caret_shape(), CaretShape::Underline);
    s.keys("<Esc>r");
    assert_eq!(s.vim.caret_shape(), CaretShape::Underline);
    s.keys("<Esc>i");
    assert_eq!(s.vim.caret_shape(), CaretShape::Bar);
    s.keys("<Esc>R");
    assert_eq!(s.vim.caret_shape(), CaretShape::Underline);
    s.keys("<Esc>v");
    assert_eq!(s.vim.caret_shape(), CaretShape::Block);
    s.keys("<Esc>:");
    assert_eq!(s.vim.caret_shape(), CaretShape::Bar);
}

#[test]
fn pending_keys_are_shown() {
    let mut s = Session::new("|a b c d e f g h");
    let shown = |s: &mut Session, keys: &str| {
        s.clear_effects();
        s.keys(keys);
        s.vim.pending_keys().to_string()
    };
    assert_eq!(shown(&mut s, "2"), "2");
    assert_eq!(shown(&mut s, "d"), "2d");
    assert!(s.effects.contains(&Effect::PendingKeys("2d".into())));
    assert_eq!(shown(&mut s, "3"), "2d3");
    assert_eq!(shown(&mut s, "w"), "");
    assert!(s.effects.contains(&Effect::PendingKeys(String::new())));
    assert_eq!(shown(&mut s, "\""), "\"");
    assert_eq!(shown(&mut s, "a"), "\"a");
    assert_eq!(s.vim.pending_register(), Some('a'));
    assert_eq!(shown(&mut s, "y"), "\"ay");
    assert_eq!(shown(&mut s, "i"), "\"ayi");
    assert_eq!(shown(&mut s, "w"), "");
    assert_eq!(s.vim.pending_register(), None);
    assert_eq!(shown(&mut s, "g"), "g");
    assert_eq!(shown(&mut s, "<Esc>"), "");
    assert_eq!(shown(&mut s, "f"), "f");
    assert_eq!(shown(&mut s, " "), "");
    assert_eq!(shown(&mut s, "z"), "z");
    assert_eq!(shown(&mut s, "q"), "");
    assert_eq!(s.bells(), 1);
}

#[test]
fn attaching_reports_everything_and_fixes_the_caret() {
    let doc = writer_core::Document::from_blocks(vec![writer_core::Block::paragraph("abc")]);
    let mut ed = writer_core::EditorState::new(doc);
    let mut host = MockHost::new(80, 10);
    let mut vim = Vim::new();
    // A caret after the last character, as an ordinary editor leaves it.
    ed.set_caret(Position::new(0, 3));
    let o = vim.attach(&mut ed, &mut host);
    assert_eq!(ed.selection(), Selection::caret(Position::new(0, 2)));
    assert!(o.effects.contains(&Effect::ModeChanged(Mode::Normal)));
    assert!(o.effects.contains(&Effect::CommandLine(None)));
    assert!(o.effects.contains(&Effect::PendingKeys(String::new())));
    assert!(o.effects.contains(&Effect::SearchHighlight(None)));
    // With a selection the engine starts in Visual mode.
    ed.set_selection(Selection::new(Position::new(0, 0), Position::new(0, 2)));
    let o = vim.attach(&mut ed, &mut host);
    assert!(o.effects.contains(&Effect::ModeChanged(Mode::Visual)));
    assert_eq!(vim.caret(&ed), Position::new(0, 1));
}

#[test]
fn detaching_closes_what_is_open() {
    // In Insert mode the undo group of the session is closed.
    let mut s = Session::new("|abc");
    s.keys("ix");
    let depth = s.ed.undo_depth();
    let o = s.vim.detach(&mut s.ed, &mut s.host);
    assert!(o.effects.contains(&Effect::ModeChanged(Mode::Normal)));
    s.ed.set_caret(Position::new(0, 0));
    s.ed.insert_text("1");
    s.ed.set_caret(Position::new(0, 3));
    s.ed.insert_text("2");
    assert_eq!(s.ed.undo_depth(), depth + 2);
    // In Visual mode the selection collapses onto the cursor.
    let mut s = Session::new("|abc");
    s.keys("vl");
    s.vim.detach(&mut s.ed, &mut s.host);
    assert_eq!(s.ed.selection(), Selection::caret(Position::new(0, 1)));
    assert_eq!(s.mode(), Mode::Normal);
    // Search highlights go away.
    let mut s = Session::new("|abc abc");
    s.keys("/abc<CR>");
    assert!(s.vim.search_highlight().is_some());
    let o = s.vim.detach(&mut s.ed, &mut s.host);
    assert!(o.effects.contains(&Effect::SearchHighlight(None)));
    // Registers survive a detach and attach.
    s.keys("yiw");
    s.vim.detach(&mut s.ed, &mut s.host);
    s.vim.attach(&mut s.ed, &mut s.host);
    s.keys("P");
    assert_eq!(s.text(), "abc abcabc");
}

#[test]
fn syncing_after_the_host_moved_things() {
    // Normal mode: the caret is put on a character.
    let mut s = Session::new("|abc");
    s.ed.set_caret(Position::new(0, 3));
    s.vim.sync(&mut s.ed, &mut s.host);
    assert_eq!(s.buffer(), "ab|c");
    // A half-typed command is dropped.
    s.keys("2d");
    let o = s.vim.sync(&mut s.ed, &mut s.host);
    assert!(o.effects.contains(&Effect::ModeChanged(Mode::Normal)));
    assert_eq!(s.vim.pending_keys(), "");
    // So is an open command line.
    s.keys(":abc");
    s.vim.sync(&mut s.ed, &mut s.host);
    assert_eq!(s.mode(), Mode::Normal);
    // Insert mode: a click starts a new piece of typing for dot-repeat.
    let mut s = Session::new("|abc");
    s.keys("ix");
    s.ed.set_caret(Position::new(0, 4));
    s.vim.sync(&mut s.ed, &mut s.host);
    assert_eq!(s.mode(), Mode::Insert);
    s.keys("y<Esc>0.");
    assert_eq!(s.buffer(), "|yxabcy");
}

#[test]
fn registers_through_the_api() {
    let mut s = Session::new("|abc");
    s.vim
        .set_register('a', RegisterContent::from_plain_text("XY"), &mut s.host);
    s.keys("\"ap");
    assert_eq!(s.buffer(), "aX|Ybc");
    s.vim
        .set_register('b', RegisterContent::from_plain_text("line\n"), &mut s.host);
    s.keys("\"bp");
    assert_eq!(s.buffer(), "aXYbc\n|line");
    assert_eq!(s.vim.register('b').map(|r| r.linewise), Some(true));
    assert!(s.vim.register('q').is_none());
    // The clipboard registers go to the host.
    s.vim
        .set_register('+', RegisterContent::from_plain_text("clip"), &mut s.host);
    assert_eq!(
        s.host.clipboard.as_ref().map(|c| c.text()).as_deref(),
        Some("clip")
    );
    s.keys("iabc def<Esc>");
    assert_eq!(s.vim.last_inserted_text(), "abc def");
}

#[test]
fn options() {
    // Without incremental search the caret waits for enter.
    let options = VimOptions {
        incremental_search: false,
        ..VimOptions::default()
    };
    let mut s = Session::with("|foo bar", 1000, 10, options);
    s.keys("/bar");
    assert_eq!(s.buffer(), "|foo bar");
    s.keys("<CR>");
    assert_eq!(s.buffer(), "foo |bar");
    // Without search highlights none are reported.
    let options = VimOptions {
        highlight_search: false,
        ..VimOptions::default()
    };
    let mut s = Session::with("|foo bar", 1000, 10, options);
    s.clear_effects();
    s.keys("/bar<CR>");
    assert!(s.vim.search_highlight().is_none());
    assert!(
        !s.effects
            .iter()
            .any(|e| matches!(e, Effect::SearchHighlight(Some(_))))
    );
    assert!(s.vim.search_pattern().is_some());
    // Options can change later.
    let mut o = s.vim.options().clone();
    o.shift_width = 2;
    s.vim.set_options(o);
    let mut s2 = Session::new("~ |a");
    let mut o = s2.vim.options().clone();
    o.shift_width = 2;
    s2.vim.set_options(o);
    s2.keys(">>");
    assert_eq!(s2.buffer(), "~   |a");
}

#[test]
fn key_sequences_and_outcomes() {
    let mut s = Session::new("|foo bar");
    let o = s
        .vim
        .handle_keys(&Key::parse_sequence("dw"), &mut s.ed, &mut s.host);
    assert!(o.consumed);
    assert_eq!(s.buffer(), "|bar");
    let o = s
        .vim
        .handle_keys(&Key::parse_sequence("fz"), &mut s.ed, &mut s.host);
    assert!(o.consumed && o.rang_bell() && o.error().is_none());
    let o = s
        .vim
        .handle_keys(&Key::parse_sequence("/zzz<CR>"), &mut s.ed, &mut s.host);
    assert_eq!(o.error(), Some("E486: Pattern not found: zzz"));
    // Text typed in Insert mode is the host's: handle_keys drops it.
    let o = s
        .vim
        .handle_keys(&Key::parse_sequence("ix<Esc>"), &mut s.ed, &mut s.host);
    assert!(!o.consumed);
    assert_eq!(s.text(), "bar");
    assert_eq!(Key::named(KeyCode::Enter), Key::ENTER);
}

#[test]
fn the_ctrl_o_state_is_visible() {
    let mut s = Session::new("|abc");
    s.keys("A<C-o>");
    assert_eq!(s.mode(), Mode::Normal);
    assert!(s.vim.insert_pending());
    // The caret may still sit after the last character.
    assert_eq!(s.ed.selection().head, Position::new(0, 3));
    s.keys("0");
    assert_eq!(s.mode(), Mode::Insert);
    assert!(!s.vim.insert_pending());
}
