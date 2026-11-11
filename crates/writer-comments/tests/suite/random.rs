//! A long random walk over the keyboard. Whatever the order of keys, the
//! panel and the editor must agree, focus must stay on something that is
//! on screen, and the document must stay valid.

use gpui::TestAppContext;

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
    let (f, cx) = open(sample(), cx);
    // The app's own "focus editor" key, to leave the panel at random.
    cx.update(|window, cx| {
        let _ = window;
        cx.bind_keys([gpui::KeyBinding::new(
            "cmd-1",
            writer_comments::FocusEditor,
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
        let mut trail: Vec<&str> = Vec::new();
        for step in 0..500 {
            let key = KEYS[random.next(KEYS.len())];
            trail.push(key);
            cx.simulate_keystrokes(key);

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
    assert!(saw_list && saw_composer, "the walk never entered the panel");
    assert!(saw_delete_question && saw_reply && saw_resolved);
    assert!(most_threads >= 3, "only {most_threads} threads at once");
}
