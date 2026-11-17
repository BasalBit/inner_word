//! A long random walk over the keyboard, and one over keyboard and mouse.
//! Whatever the order of keys, clicks and right-clicks, the panel and the
//! editor must agree, focus must stay on something that is on screen, and
//! the document must stay valid.

use gpui::{Modifiers, MouseButton, TestAppContext, point, px};
use innerword_ui::ContextMenu;

use super::support::{Focus, open, sample};

const KEYS: &[&str] = &[
    "cmd-alt-m",
    "cmd-alt-]",
    "cmd-alt-[",
    "cmd-alt-c",
    "cmd-2",
    "cmd-1",
    "j",
    "k",
    "x",
    "s",
    "r",
    "e",
    "h",
    "l",
    "a",
    "enter",
    "shift-enter",
    "cmd-enter",
    "escape",
    "cmd-backspace",
    "backspace",
    "cmd-z",
    "cmd-shift-z",
    "left",
    "right",
    "up",
    "down",
    "home",
    "end",
    "shift-right",
    "alt-shift-right",
    "alt-right",
    "cmd-a",
    "tab",
    "shift-tab",
    "ctrl-enter",
    "cmd-c",
];

/// A small deterministic generator (xorshift), so a failure can be replayed
/// from its seed.
struct Random(u64);

impl Random {
    fn next(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as usize
    }
}

#[gpui::test]
fn random_keys_keep_panel_and_editor_consistent(cx: &mut TestAppContext) {
    walk(false, cx);
}

/// The same walk with the pointer in it: every fourth step or so is a
/// move, a click or a right-click somewhere in the window (mostly in the
/// panel), so buttons, cards, replies and menu rows are pressed in every
/// state the keys lead to.
#[gpui::test]
fn random_keys_and_clicks_keep_panel_and_editor_consistent(cx: &mut TestAppContext) {
    walk(true, cx);
}

fn walk(with_pointer: bool, cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let viewport = cx.update(|window, _| window.viewport_size());
    let mut saw_menu = false;
    let mut saw_hover = false;
    // The app's own "focus editor" key, to leave the panel at random.
    cx.update(|window, cx| {
        let _ = window;
        cx.bind_keys([gpui::KeyBinding::new(
            "cmd-1",
            innerword_comments::FocusEditor,
            None,
        )]);
    });

    // What the walk reached, to be sure it is not a walk through nothing.
    let mut saw_list = false;
    let mut saw_composer = false;
    let mut saw_delete_question = false;
    let mut saw_reply = false;
    let mut saw_resolved = false;
    let mut most_threads = 0;

    for seed in 1..=6u64 {
        let mut random = Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut trail: Vec<String> = Vec::new();
        for step in 0..500 {
            if with_pointer && random.next(4) == 0 {
                // Mostly in the panel (the right 320 px), sometimes in
                // the text.
                let x = if random.next(5) == 0 {
                    random.next(f32::from(viewport.width) as usize) as f32
                } else {
                    f32::from(viewport.width) - 320. + random.next(320) as f32
                };
                let y = random.next(420) as f32;
                let at = point(px(x), px(y));
                match random.next(4) {
                    0 => {
                        trail.push(format!("move {x},{y}"));
                        cx.simulate_mouse_move(at, None, Modifiers::none());
                    }
                    1 => {
                        trail.push(format!("right-click {x},{y}"));
                        cx.simulate_mouse_move(at, None, Modifiers::none());
                        cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
                        cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
                    }
                    _ => {
                        trail.push(format!("click {x},{y}"));
                        cx.simulate_mouse_move(at, None, Modifiers::none());
                        cx.simulate_click(at, Modifiers::none());
                    }
                }
                cx.run_until_parked();
            } else {
                let key = KEYS[random.next(KEYS.len())];
                trail.push(key.to_string());
                cx.simulate_keystrokes(key);
            }
            let menu_open = cx.update(|window, cx| ContextMenu::is_open(window, cx));
            saw_menu |= menu_open;

            let context = || {
                let recent = &trail[trail.len().saturating_sub(12)..];
                format!("seed {seed}, step {step}, last keys {recent:?}")
            };
            let focus = f.focus(cx);
            assert_ne!(focus, Focus::Elsewhere, "focus was lost: {}", context());
            saw_list |= focus == Focus::List;
            saw_composer |= focus == Focus::Composer;
            f.panel.read_with(cx, |panel, _| {
                saw_delete_question |= panel.pending_delete().is_some();
                saw_resolved |= panel.resolved_count() > 0;
                saw_reply |= panel
                    .threads()
                    .iter()
                    .any(|entry| !entry.thread.replies.is_empty());
                most_threads = most_threads.max(panel.threads().len());
            });

            let active = f.panel.read_with(cx, |panel, _| panel.active_thread());
            let in_editor = f.editor.read_with(cx, |editor, _| editor.active_comment());
            assert_eq!(active, in_editor, "active thread differs: {}", context());

            f.panel.read_with(cx, |panel, cx| {
                let listed: Vec<_> = panel.listed_threads().map(|entry| entry.id).collect();
                saw_hover |= panel.hovered_thread().is_some();
                // A menu of the panel is about what is marked in it.
                if let Some(target) = panel.menu_target() {
                    assert!(menu_open, "a menu target without a menu: {}", context());
                    if let Some(thread) = target.thread() {
                        assert_eq!(panel.active_thread(), Some(thread), "{}", context());
                    }
                    if let innerword_comments::CommentMenuTarget::Reply(_, reply) = target {
                        assert_eq!(panel.selected_reply(), Some(reply), "{}", context());
                    }
                }
                if let Some(reply) = panel.selected_reply() {
                    let thread = panel.active_thread().and_then(|id| panel.thread(id));
                    assert!(
                        thread.is_some_and(|entry| entry.thread.reply(reply).is_some()),
                        "a selected reply that is not in the active thread: {}",
                        context()
                    );
                }
                if let Some(active) = panel.active_thread() {
                    assert!(listed.contains(&active), "active not listed: {}", context());
                }
                if !panel.is_visible() {
                    assert!(panel.active_thread().is_none(), "{}", context());
                    assert!(!panel.is_composing(), "{}", context());
                }
                if let Some(target) = panel.pending_delete() {
                    assert!(listed.contains(&target.thread()), "{}", context());
                }
                if let Some(thread) = panel.draft().and_then(|draft| draft.thread()) {
                    assert!(listed.contains(&thread), "{}", context());
                }
                if !panel.is_composing() {
                    assert!(panel.composer().read(cx).is_empty(), "{}", context());
                }
                // The target of a new comment is a place in the document
                // as it is now, whatever was typed or deleted meanwhile.
                if let Some(innerword_comments::DraftKind::New { range }) = panel.draft() {
                    let document = panel.editor().read(cx).document();
                    assert!(
                        document.is_valid_position(range.start)
                            && document.is_valid_position(range.end)
                            && range.start <= range.end,
                        "draft target {range:?}: {}",
                        context()
                    );
                    assert_eq!(panel.draft_target_lost(), range.is_empty());
                }
                assert!(panel.kept_drafts() <= 16, "{}", context());
                match focus {
                    Focus::Composer => assert!(panel.is_composing(), "{}", context()),
                    Focus::List => assert!(panel.is_open(), "{}", context()),
                    _ => {}
                }
                // The panel's snapshot is the document's threads.
                let document = panel.editor().read(cx).document();
                assert_eq!(
                    panel.threads().len(),
                    document.comment_count(),
                    "{}",
                    context()
                );
                assert_eq!(
                    panel.is_visible(),
                    panel.editor().read(cx).config().comments_visible,
                    "{}",
                    context()
                );
                if let Err(error) = document.check_invariants() {
                    panic!("document invariant broken ({error}): {}", context());
                }
            });
        }
    }
    assert!(saw_menu, "the walk never opened a menu");
    assert!(
        !with_pointer || saw_hover,
        "the pointer never was on a card"
    );
    assert!(saw_list && saw_composer, "the walk never entered the panel");
    assert!(saw_delete_question && saw_reply && saw_resolved);
    assert!(most_threads >= 3, "only {most_threads} threads at once");
}
