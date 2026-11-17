//! The public surface: consumed keys, effects, attaching and syncing.

use crate::harness::*;
use innerword_core::{Position, Selection};
use innerword_vim::{CaretShape, Effect, Key, KeyCode, Mode, RegisterContent, Vim, VimOptions};

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
    let doc = innerword_core::Document::from_blocks(vec![innerword_core::Block::paragraph("abc")]);
    let mut ed = innerword_core::EditorState::new(doc);
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
fn an_edit_from_outside_does_not_start_visual_mode() {
    // The editor's own undo (cmd-z, the Edit menu) brings back the
    // selection the undone step was made on. From Normal mode that is not
    // a Visual selection: the next `d` would delete it.
    let mut s = Session::new("alpha |beta gamma");
    s.ed.set_selection(Selection::new(Position::new(0, 6), Position::new(0, 10)));
    s.vim.sync(&mut s.ed, &mut s.host);
    assert_eq!(s.mode(), Mode::Visual);
    s.ed.toggle_bold();
    s.vim.sync(&mut s.ed, &mut s.host);
    // An edit made on a Visual selection keeps it.
    assert_eq!(s.mode(), Mode::Visual);
    s.keys("<Esc>");
    assert_eq!(s.mode(), Mode::Normal);
    assert!(s.ed.undo());
    assert!(
        !s.ed.selection().is_collapsed(),
        "the undo restores the selection"
    );
    let o = s.vim.sync(&mut s.ed, &mut s.host);
    assert_eq!(s.mode(), Mode::Normal);
    assert!(!o.effects.contains(&Effect::ModeChanged(Mode::Visual)));
    assert_eq!(s.buffer(), "alpha |beta gamma");
    // Without a sync the engine catches up at the next key the same way.
    assert!(s.ed.redo());
    s.keys("x");
    assert_eq!(s.mode(), Mode::Normal);
    assert_eq!(s.text(), "alpha **eta** gamma");
    // A selection made without an edit (a drag, shift-arrows) still
    // starts Visual mode.
    s.ed.set_selection(Selection::new(Position::new(0, 0), Position::new(0, 2)));
    s.vim.sync(&mut s.ed, &mut s.host);
    assert_eq!(s.mode(), Mode::Visual);
}

/// A change somebody else made to the document (an agent's, taken over by
/// the app while the document is open) is not the writer's doing: what
/// they are in the middle of typing stays, where a `sync` drops it.
#[test]
fn a_change_made_elsewhere_keeps_a_half_typed_command_and_the_command_line() {
    // What an agent's change looks like to the engine: the text changed
    // before the caret, and the caret is on the letter it was on.
    fn changed_before_the_caret(s: &mut Session) {
        let caret = s.ed.selection().head;
        s.ed.set_caret(Position::new(0, 0));
        s.ed.insert_text("So ");
        s.ed.set_caret(Position::new(caret.block, caret.offset + 3));
    }

    // An operator waiting for its motion.
    let mut s = Session::new("alpha |beta gamma");
    s.keys("d");
    assert_eq!(s.mode(), Mode::OperatorPending);
    changed_before_the_caret(&mut s);
    let o = s.vim.changed_elsewhere(&mut s.ed, &mut s.host);
    assert_eq!(s.mode(), Mode::OperatorPending);
    assert!(!o.effects.contains(&Effect::PendingKeys(String::new())));
    // The `sync` a host makes for the same change finds nothing to do.
    assert!(s.vim.sync(&mut s.ed, &mut s.host).effects.is_empty());
    assert_eq!(s.mode(), Mode::OperatorPending);
    s.keys("w");
    assert_eq!(s.text(), "So alpha gamma");
    assert_eq!(s.mode(), Mode::Normal);

    // A count, and a key waiting for its argument.
    let mut s = Session::new("alpha |beta gamma");
    s.keys("2");
    changed_before_the_caret(&mut s);
    s.vim.changed_elsewhere(&mut s.ed, &mut s.host);
    s.keys("x");
    assert_eq!(s.text(), "So alpha ta gamma");
    s.keys("r");
    changed_before_the_caret(&mut s);
    s.vim.changed_elsewhere(&mut s.ed, &mut s.host);
    s.keys("X");
    assert_eq!(s.text(), "So So alpha Xa gamma");

    // A search that is being typed: the rest of it is still the search.
    let mut s = Session::new("|alpha beta gamma");
    s.keys("/ga");
    assert_eq!(s.mode(), Mode::CommandLine);
    changed_before_the_caret(&mut s);
    let o = s.vim.changed_elsewhere(&mut s.ed, &mut s.host);
    assert_eq!(s.mode(), Mode::CommandLine);
    assert!(!o.effects.contains(&Effect::CommandLine(None)));
    s.keys("mma<CR>");
    assert_eq!(s.mode(), Mode::Normal);
    assert_eq!(s.buffer(), "So alpha beta |gamma");

    // Nothing changed: the call is a no-op.
    let mut s = Session::new("alpha |beta gamma");
    s.keys("2d");
    assert!(
        s.vim
            .changed_elsewhere(&mut s.ed, &mut s.host)
            .effects
            .is_empty()
    );
    s.keys("w");
    assert_eq!(s.text(), "alpha ");

    // In Normal mode the caret is put on a character, as after a `sync`.
    // (The letter it was on is gone: the caret stands after the last one.)
    let mut s = Session::new("abc|d");
    s.ed.set_selection(Selection::new(Position::new(0, 3), Position::new(0, 4)));
    s.ed.delete_selection();
    s.ed.set_caret(Position::new(0, 3));
    assert_eq!(s.text(), "abc");
    s.vim.changed_elsewhere(&mut s.ed, &mut s.host);
    assert_eq!(s.mode(), Mode::Normal);
    assert_eq!(s.buffer(), "ab|c");

    // The same change taken for the writer's own doing drops the command:
    // that is `sync`, and it stays what it was.
    let mut s = Session::new("alpha |beta gamma");
    s.keys("d");
    changed_before_the_caret(&mut s);
    s.vim.sync(&mut s.ed, &mut s.host);
    assert_eq!(s.mode(), Mode::Normal);
    s.keys("w");
    assert_eq!(s.text(), "So alpha beta gamma");
    let mut s = Session::new("|alpha beta gamma");
    s.keys("/ga");
    changed_before_the_caret(&mut s);
    s.vim.sync(&mut s.ed, &mut s.host);
    assert_eq!(s.mode(), Mode::Normal);
}

#[test]
fn syncing_after_the_host_moved_things() {
    // Normal mode: the caret is put on a character.
    let mut s = Session::new("|abc");
    s.ed.set_caret(Position::new(0, 3));
    s.vim.sync(&mut s.ed, &mut s.host);
    assert_eq!(s.buffer(), "ab|c");
    // Nothing changed outside: the call is a no-op, also for a half-typed
    // command.
    s.keys("2d");
    let o = s.vim.sync(&mut s.ed, &mut s.host);
    assert!(o.effects.is_empty());
    assert_eq!(s.vim.pending_keys(), "2d");
    // A change from outside drops the command.
    s.ed.set_caret(Position::new(0, 0));
    let o = s.vim.sync(&mut s.ed, &mut s.host);
    assert!(o.effects.contains(&Effect::ModeChanged(Mode::Normal)));
    assert_eq!(s.vim.pending_keys(), "");
    // So it does an open command line.
    s.keys(":abc");
    s.vim.sync(&mut s.ed, &mut s.host);
    assert_eq!(s.mode(), Mode::CommandLine);
    s.ed.set_caret(Position::new(0, 1));
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
    // The caret steps onto the last character, as in Vim.
    assert_eq!(s.ed.selection().head, Position::new(0, 2));
    s.keys("0");
    assert_eq!(s.mode(), Mode::Insert);
    assert!(!s.vim.insert_pending());
}

#[test]
fn sync_can_be_called_on_every_change_report() {
    // A host that calls sync after every key, as it would on every
    // "selection changed" or "edited" event.
    fn run_with_sync(buffer: &str, keys: &str) -> Session {
        let mut s = Session::new(buffer);
        for k in Key::parse_sequence(keys) {
            s.key(k);
            let o = s.vim.sync(&mut s.ed, &mut s.host);
            s.effects.extend(o.effects);
            // Twice: one change can be reported as two events.
            s.vim.sync(&mut s.ed, &mut s.host);
        }
        s
    }
    // Typing, enter and backspace stay part of what dot repeats.
    let s = run_with_sync("|x", "iab<BS>c<CR>d<Esc>G.");
    assert_eq!(s.buffer(), "ac\nac\n|ddx");
    let s = run_with_sync("|x", "3iab<Esc>");
    assert_eq!(s.buffer(), "ababa|bx");
    // Visual mode on an empty line survives (its selection can be empty).
    let s = run_with_sync("a\n|", "vk");
    assert_eq!(s.mode(), Mode::Visual);
    let s = run_with_sync("a\n|\nb", "Vjd");
    assert_eq!(s.buffer(), "|a");
    // Pending commands and the command line are not disturbed.
    let s = run_with_sync("|a b c", "2dw");
    assert_eq!(s.buffer(), "|c");
    let s = run_with_sync("|foo bar", "/bar<CR>");
    assert_eq!(s.buffer(), "foo |bar");
    let s = run_with_sync("|a\nb\nc", ":2,3d<CR>");
    assert_eq!(s.buffer(), "|a");
    let s = run_with_sync("|abc", "A<C-o>hX<Esc>");
    assert_eq!(s.buffer(), "a|Xbc");
    let s = run_with_sync("|abc", "RXY<BS>Z<Esc>");
    assert_eq!(s.buffer(), "X|Zc");
}

#[test]
fn the_engine_catches_up_without_sync() {
    // The host moved the caret after the end of a line and did not say so.
    let mut s = Session::new("|abc def");
    s.ed.set_caret(Position::new(0, 7));
    s.keys("x");
    assert_eq!(s.buffer(), "abc d|e");
    // A selection made behind the engine becomes Visual mode at the next key.
    let mut s = Session::new("|abc def");
    s.ed.set_selection(Selection::new(Position::new(0, 0), Position::new(0, 3)));
    s.keys("d");
    assert_eq!(s.buffer(), "| def");
    // A pending operator does not survive a click.
    let mut s = Session::new("|abc def");
    s.keys("d");
    s.ed.set_caret(Position::new(0, 4));
    s.keys("w");
    assert_eq!(s.buffer(), "abc de|f");
    assert_eq!(s.text(), "abc def");
}

#[test]
fn typed_text_may_be_announced_before_it_is_inserted() {
    let mut s = Session::new("|x");
    s.keys("i");
    for ch in ["a", "b"] {
        let k = Key::parse_sequence(ch)[0];
        assert!(!s.vim.handle_key(k, &mut s.ed, &mut s.host).consumed);
        // Announce, then insert, then the editor reports the change.
        s.vim.text_inserted(ch, &mut s.ed, &mut s.host);
        s.ed.insert_text(ch);
        s.vim.sync(&mut s.ed, &mut s.host);
    }
    s.keys("<Esc>");
    assert_eq!(s.buffer(), "a|bx");
    s.keys("$.");
    assert_eq!(s.buffer(), "aba|bx");
    // The same in Replace mode: the overwritten character goes first.
    let mut s = Session::new("|xyz");
    s.keys("R");
    let k = Key::char('a');
    s.vim.handle_key(k, &mut s.ed, &mut s.host);
    s.vim.text_inserted("a", &mut s.ed, &mut s.host);
    s.ed.insert_text("a");
    s.vim.sync(&mut s.ed, &mut s.host);
    s.keys("<Esc>");
    assert_eq!(s.buffer(), "|ayz");
}

#[test]
fn a_read_only_document_cannot_be_changed() {
    let options = VimOptions {
        read_only: true,
        ..VimOptions::default()
    };
    let buffer = "one two\nth|ree (four)\n\nfive";
    let changing = [
        "x",
        "X",
        "dd",
        "dw",
        "D",
        "cw",
        "cc",
        "C",
        "s",
        "S",
        "rz",
        "R",
        "~",
        "g~w",
        "gUU",
        "guiw",
        "J",
        "gJ",
        ">>",
        "<<",
        "p",
        "P",
        "gp",
        "ix",
        "ax",
        "Ix",
        "Ax",
        "ox",
        "Ox",
        "gi",
        "gI",
        ".",
        "u",
        "<C-r>",
        "&",
        "g&",
        "vd",
        "vc",
        "vJ",
        "vp",
        "vrx",
        "vU",
        "V>",
        "Vd",
        ":d<CR>",
        ":s/e/x/<CR>",
        ":%j<CR>",
        ":><CR>",
        ":u<CR>",
        ":red<CR>",
        "yyp",
        "\"ap",
        "di(",
        "ciw",
        "d/five<CR>",
        "yiwviwp",
    ];
    for keys in changing {
        let mut s = Session::with(buffer, 1000, 10, options.clone());
        let before = s.text();
        s.clear_effects();
        s.keys(keys);
        assert_eq!(s.text(), before, "{keys} changed a read-only document");
        assert!(!s.mode().is_insert(), "{keys} entered {:?}", s.mode());
        assert_eq!(s.ed.undo_depth(), 0, "{keys} left an undo step");
        s.keys("<Esc>");
        assert!(s.check_invariants().is_ok());
    }
    // Everything else works.
    let mut s = Session::with(buffer, 1000, 10, options.clone());
    s.keys("ggwyiw");
    assert_eq!(s.reg('"'), "two");
    s.keys("/four<CR>");
    assert_eq!(s.buffer(), "one two\nthree (|four)\n\nfive");
    s.keys("vi(y:1<CR>");
    assert_eq!(s.reg('"'), "four");
    assert_eq!(s.buffer(), "|one two\nthree (four)\n\nfive");
    s.keys("Vjy:s/o//gn<CR>");
    assert!(s.messages().iter().any(|m| m.contains("2 matches")));
    assert!(s.messages().iter().all(|m| !m.contains("E21")));
    s.clear_effects();
    s.keys("x");
    assert!(s.messages().iter().any(|m| m.starts_with("E21")));
    assert_eq!(s.bells(), 1);
    s.keys(":w<CR>");
    assert_eq!(s.requests().len(), 1);
}

#[test]
fn a_pending_command_can_be_cancelled_from_outside() {
    // An operator with a count and a register.
    let mut s = session("|one two three", "\"a2d");
    assert_eq!(s.mode(), Mode::OperatorPending);
    assert_eq!(s.vim.pending_keys(), "\"a2d");
    let outcome = s.vim.cancel_pending_command();
    assert!(outcome.effects.contains(&Effect::ModeChanged(Mode::Normal)));
    assert!(
        outcome
            .effects
            .contains(&Effect::PendingKeys(String::new()))
    );
    assert!(!outcome.rang_bell());
    assert_eq!(s.vim.pending_keys(), "");
    assert_eq!(s.vim.pending_register(), None);
    // The next key is a command of its own.
    s.keys("w");
    assert_eq!(s.buffer(), "one |two three");

    // A key waiting for its argument.
    for keys in ["f", "r", "g", "z", "m", "'", "3", "di"] {
        let mut s = session("|one two three", keys);
        assert_ne!(s.vim.pending_keys(), "", "{keys}");
        s.vim.cancel_pending_command();
        assert_eq!(s.vim.pending_keys(), "", "{keys}");
        s.keys("x");
        assert_eq!(s.buffer(), "|ne two three", "{keys}");
    }

    // Visual and Insert mode stay; so does an open command line.
    let mut s = session("|one two", "v\"a");
    s.vim.cancel_pending_command();
    assert_eq!(s.mode(), Mode::Visual);
    s.keys("d");
    assert_eq!(s.vim.register('a'), None);
    let mut s = session("|one two", "i<C-r>");
    s.vim.cancel_pending_command();
    assert_eq!(s.mode(), Mode::Insert);
    s.keys("a<Esc>");
    assert_eq!(s.buffer(), "|aone two");
    let mut s = session("|one two", ":s/one");
    let outcome = s.vim.cancel_pending_command();
    assert!(outcome.effects.is_empty());
    assert_eq!(s.mode(), Mode::CommandLine);
    s.keys("/1<CR>");
    assert_eq!(s.buffer(), "|1 two");

    // Nothing pending: nothing reported.
    let mut s = session("|one", "");
    assert!(s.vim.cancel_pending_command().effects.is_empty());
}
