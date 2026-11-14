//! Headless tests for the shared widgets: TextField editing, Picker,
//! ModalLayer focus restore, ConfirmDialog, toasts, key hints.

use std::{cell::Cell, cell::RefCell, rc::Rc, time::Duration};

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, FocusHandle, Focusable, KeyBinding,
    Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, SharedString, TestAppContext,
    VisualTestContext, Window, actions, div, point, prelude::*, px,
};
use writer_ui::{
    ConfirmDialog, ConfirmDialogEvent, KeyHint, ModalLayer, Picker, PickerDelegate, PickerEntry,
    PickerItem, TextField, TextFieldEvent, Toast, ToastLayer,
    fuzzy::{StringMatch, match_strings},
    theme::{self, ThemeMode},
};

#[path = "widgets/context_menu.rs"]
mod context_menu;
#[path = "widgets/mouse.rs"]
mod mouse;
#[path = "widgets/rebinding.rs"]
mod rebinding;

fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        theme::init(ThemeMode::Light, cx);
        writer_ui::init(cx);
    });
}

// ---------------------------------------------------------------------------
// TextField
// ---------------------------------------------------------------------------

fn field_window<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
) -> (
    Entity<TextField>,
    &'a mut VisualTestContext,
    Rc<RefCell<Vec<TextFieldEvent>>>,
) {
    init(cx);
    let text = text.to_string();
    let (field, cx) = cx.add_window_view(move |window, cx| {
        TextField::new(window, cx)
            .with_placeholder("Title")
            .with_text(text)
    });
    let events = Rc::new(RefCell::new(Vec::new()));
    cx.update(|window, cx| {
        window.activate_window();
        field.read(cx).focus(window);
        let events = events.clone();
        cx.subscribe(&field, move |_, event: &TextFieldEvent, _| {
            events.borrow_mut().push(*event)
        })
        .detach();
    });
    cx.run_until_parked();
    (field, cx, events)
}

fn text(field: &Entity<TextField>, cx: &mut VisualTestContext) -> String {
    field.read_with(cx, |field, _| field.text().to_string())
}

fn selection(field: &Entity<TextField>, cx: &mut VisualTestContext) -> std::ops::Range<usize> {
    field.read_with(cx, |field, _| field.selection())
}

/// Longer than the field's blink interval (530 ms).
const PAST_A_BLINK: Duration = Duration::from_millis(600);

/// How often the field asks to be drawn again in `duration`.
fn notifies_during(
    field: &Entity<TextField>,
    duration: Duration,
    cx: &mut VisualTestContext,
) -> usize {
    let count = Rc::new(Cell::new(0usize));
    let subscription = cx.update(|_, cx| {
        let count = count.clone();
        cx.observe(field, move |_, _| count.set(count.get() + 1))
    });
    cx.executor().advance_clock(duration);
    cx.run_until_parked();
    drop(subscription);
    count.get()
}

#[gpui::test]
fn text_field_blinks_in_the_active_window(cx: &mut TestAppContext) {
    let (field, cx, _) = field_window(cx, "Hello");
    assert!(field.read_with(cx, |field, _| field.is_blinking()));
    assert!(notifies_during(&field, PAST_A_BLINK, cx) >= 1);
    assert!(notifies_during(&field, PAST_A_BLINK * 4, cx) >= 4);
}

#[gpui::test]
fn text_field_does_not_blink_in_an_inactive_window(cx: &mut TestAppContext) {
    let (field, cx, _) = field_window(cx, "Hello");
    cx.deactivate_window();
    cx.run_until_parked();
    // Still focused in its window, but the window is in the background.
    assert!(cx.update(|window, cx| field.read(cx).is_focused(window)));
    assert!(!field.read_with(cx, |field, _| field.is_blinking()));
    assert_eq!(notifies_during(&field, PAST_A_BLINK * 10, cx), 0);

    // Something that would restart the blink (an edit, a move) does not
    // start it while the window is inactive.
    field.update(cx, |field, cx| field.set_text("Hello there", cx));
    cx.run_until_parked();
    assert!(!field.read_with(cx, |field, _| field.is_blinking()));
    assert_eq!(notifies_during(&field, PAST_A_BLINK * 10, cx), 0);

    // The window comes back: the blink starts again.
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    assert!(field.read_with(cx, |field, _| field.is_blinking()));
    assert!(notifies_during(&field, PAST_A_BLINK * 4, cx) >= 4);
}

#[gpui::test]
fn text_field_does_not_start_blinking_when_focused_in_an_inactive_window(
    cx: &mut TestAppContext,
) {
    let (field, cx, _) = field_window(cx, "Hello");
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    assert!(!field.read_with(cx, |field, _| field.is_blinking()));
    cx.deactivate_window();
    cx.update(|window, cx| field.read(cx).focus(window));
    cx.run_until_parked();
    assert!(cx.update(|window, cx| field.read(cx).is_focused(window)));
    assert!(!field.read_with(cx, |field, _| field.is_blinking()));
    assert_eq!(notifies_during(&field, PAST_A_BLINK * 10, cx), 0);
}

#[gpui::test]
fn text_field_typing_and_deleting(cx: &mut TestAppContext) {
    let (field, cx, events) = field_window(cx, "");
    cx.simulate_input("Hello wörld");
    assert_eq!(text(&field, cx), "Hello wörld");
    assert!(events.borrow().contains(&TextFieldEvent::Changed));
    assert!(events.borrow().contains(&TextFieldEvent::Focused));

    cx.simulate_keystrokes("backspace backspace");
    assert_eq!(text(&field, cx), "Hello wör");
    cx.simulate_keystrokes("left left delete");
    assert_eq!(text(&field, cx), "Hello wr");
    cx.simulate_input("ö");
    assert_eq!(text(&field, cx), "Hello wör");

    // Graphemes: an emoji flag is deleted as one unit.
    cx.simulate_keystrokes("cmd-right");
    field.update(cx, |field, cx| field.insert(" 🇩🇪", cx));
    cx.simulate_keystrokes("backspace");
    assert_eq!(text(&field, cx), "Hello wör ");

    // ctrl-h / ctrl-d, emacs style
    cx.simulate_keystrokes("ctrl-h ctrl-a ctrl-d");
    assert_eq!(text(&field, cx), "ello wör");
}

#[gpui::test]
fn text_field_word_and_line_movement(cx: &mut TestAppContext) {
    let (field, cx, _) = field_window(cx, "one two, three");
    // caret at end
    cx.simulate_keystrokes("alt-left");
    assert_eq!(selection(&field, cx), 9..9);
    cx.simulate_keystrokes("alt-left alt-left");
    assert_eq!(selection(&field, cx), 0..0);
    cx.simulate_keystrokes("alt-right");
    assert_eq!(selection(&field, cx), 3..3);
    cx.simulate_keystrokes("alt-shift-right");
    assert_eq!(selection(&field, cx), 3..7);
    cx.simulate_keystrokes("shift-left");
    assert_eq!(selection(&field, cx), 3..6);
    // collapsing a selection with an arrow goes to its edge
    cx.simulate_keystrokes("right");
    assert_eq!(selection(&field, cx), 6..6);
    cx.simulate_keystrokes("cmd-shift-left");
    assert_eq!(selection(&field, cx), 0..6);
    field.read_with(cx, |field, _| assert_eq!(field.cursor(), 0));
    cx.simulate_keystrokes("cmd-right");
    assert_eq!(selection(&field, cx), 14..14);
    cx.simulate_keystrokes("cmd-left");
    assert_eq!(selection(&field, cx), 0..0);
    cx.simulate_keystrokes("ctrl-e");
    assert_eq!(selection(&field, cx), 14..14);
    cx.simulate_keystrokes("shift-home");
    assert_eq!(selection(&field, cx), 0..14);
    // up / down go to the line ends in a plain field
    cx.simulate_keystrokes("down");
    assert_eq!(selection(&field, cx), 14..14);
    cx.simulate_keystrokes("up");
    assert_eq!(selection(&field, cx), 0..0);

    // word and line deletion
    cx.simulate_keystrokes("cmd-right alt-backspace");
    assert_eq!(text(&field, cx), "one two, ");
    cx.simulate_keystrokes("alt-left alt-left alt-delete");
    assert_eq!(text(&field, cx), " two, ");
    cx.simulate_keystrokes("alt-delete");
    assert_eq!(text(&field, cx), ", ");
    cx.simulate_keystrokes("cmd-right");
    cx.simulate_input("end");
    cx.simulate_keystrokes("left left cmd-backspace");
    assert_eq!(text(&field, cx), "nd");
    cx.simulate_keystrokes("ctrl-k");
    assert_eq!(text(&field, cx), "");
}

#[gpui::test]
fn text_field_select_all_and_clipboard(cx: &mut TestAppContext) {
    let (field, cx, _) = field_window(cx, "draft title");
    cx.simulate_keystrokes("cmd-a cmd-c");
    assert_eq!(selection(&field, cx), 0..11);
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("draft title")
    );
    cx.simulate_input("New");
    assert_eq!(text(&field, cx), "New");
    cx.simulate_keystrokes("cmd-v");
    assert_eq!(text(&field, cx), "Newdraft title");
    cx.simulate_keystrokes("alt-shift-left cmd-x");
    assert_eq!(text(&field, cx), "Newdraft ");
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("title")
    );
    // Multi-line clipboard text becomes one line.
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("a\nb\r\nc".into()));
    cx.simulate_keystrokes("cmd-v");
    assert_eq!(text(&field, cx), "Newdraft a b c");
    // Copy with an empty selection keeps the clipboard.
    cx.simulate_keystrokes("cmd-c");
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("a\nb\r\nc")
    );
}

#[gpui::test]
fn text_field_undo_redo(cx: &mut TestAppContext) {
    let (field, cx, _) = field_window(cx, "");
    cx.simulate_input("hello world");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&field, cx), "hello");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&field, cx), "");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&field, cx), "");
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(&field, cx), "hello");
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(&field, cx), "hello world");

    // Moving the caret starts a new undo step; deletions coalesce.
    cx.simulate_keystrokes("left left left left left");
    cx.simulate_input("big ");
    cx.simulate_keystrokes("cmd-right backspace backspace");
    assert_eq!(text(&field, cx), "hello big wor");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&field, cx), "hello big world");
    // "big " is two steps: the word, then the space that starts a new one.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&field, cx), "hello bigworld");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&field, cx), "hello world");
    // Undo restores the caret from before the edit.
    assert_eq!(selection(&field, cx), 6..6);
    // A new edit clears redo.
    cx.simulate_keystrokes("cmd-right");
    cx.simulate_input("!");
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(&field, cx), "hello world!");
    // set_text resets the history
    field.update(cx, |field, cx| field.set_text("fresh", cx));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&field, cx), "fresh");
}

#[gpui::test]
fn text_field_ime_marked_text(cx: &mut TestAppContext) {
    let (field, cx, _) = field_window(cx, "caf");
    // Dead key: option-e shows a marked accent, the next key commits "é".
    field.update_in(cx, |field, window, cx| {
        use gpui::EntityInputHandler;
        field.replace_and_mark_text_in_range(None, "´", Some(1..1), window, cx);
        assert_eq!(field.marked_range(), Some(3..5));
        assert_eq!(field.text(), "caf´");
        assert_eq!(
            field.marked_text_range(window, cx),
            Some(3..4),
            "marked range is reported in UTF-16"
        );
        field.replace_text_in_range(None, "é", window, cx);
        assert_eq!(field.text(), "café");
        assert_eq!(field.marked_range(), None);
        assert_eq!(field.selection(), 5..5);
    });
    // Multi-step composition (Japanese kana): each update replaces the marked text.
    field.update_in(cx, |field, window, cx| {
        use gpui::EntityInputHandler;
        field.replace_and_mark_text_in_range(None, " k", None, window, cx);
        field.replace_and_mark_text_in_range(None, " か", None, window, cx);
        assert_eq!(field.text(), "café か");
        field.replace_and_mark_text_in_range(None, " 家", Some(0..2), window, cx);
        assert_eq!(field.selection(), 5..9);
        field.replace_text_in_range(None, " 家", window, cx);
        assert_eq!(field.text(), "café 家");
        // UTF-16 queries from the platform
        let mut adjusted = None;
        assert_eq!(
            field
                .text_for_range(5..6, &mut adjusted, window, cx)
                .as_deref(),
            Some("家")
        );
        assert_eq!(
            field.selected_text_range(false, window, cx).unwrap().range,
            6..6
        );
    });
    // The whole composition is one undo step.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&field, cx), "café");
    // Typed text through the platform handler (simulated IME path).
    cx.simulate_input("!");
    assert_eq!(text(&field, cx), "café!");
}

#[gpui::test]
fn text_field_events(cx: &mut TestAppContext) {
    let (field, cx, events) = field_window(cx, "x");
    events.borrow_mut().clear();
    cx.simulate_keystrokes("enter escape");
    assert_eq!(
        *events.borrow(),
        vec![TextFieldEvent::Submitted, TextFieldEvent::Cancelled]
    );
    // set_text does not emit Changed
    events.borrow_mut().clear();
    field.update(cx, |field, cx| field.set_text("y", cx));
    assert!(events.borrow().is_empty());
    // blur
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    assert_eq!(*events.borrow(), vec![TextFieldEvent::Blurred]);
}

#[gpui::test]
fn text_field_mouse_selection(cx: &mut TestAppContext) {
    let (field, cx, _) = field_window(cx, "alpha beta gamma");
    let bounds = field
        .read_with(cx, |field, _| field.last_painted_bounds())
        .expect("painted");
    // Test text system: every char advances 0.6 × 13px = 7.8px.
    let x_of = |ix: f32| bounds.left() + px(7.8 * ix);
    let y = bounds.center().y;

    cx.simulate_click(point(x_of(2.), y), Modifiers::default());
    assert_eq!(selection(&field, cx), 2..2);

    // shift-click extends
    cx.simulate_click(point(x_of(8.), y), Modifiers::shift());
    assert_eq!(selection(&field, cx), 2..8);

    // drag
    cx.simulate_mouse_down(point(x_of(1.), y), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(point(x_of(4.), y), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point(x_of(4.), y), MouseButton::Left, Modifiers::default());
    assert_eq!(selection(&field, cx), 1..4);

    // double-click selects a word
    let position = point(x_of(7.5), y);
    cx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position,
        modifiers: Modifiers::default(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position,
        modifiers: Modifiers::default(),
        click_count: 2,
    });
    assert_eq!(selection(&field, cx), 6..10);
}

#[gpui::test]
fn text_field_scrolls_horizontally(cx: &mut TestAppContext) {
    init(cx);
    let (root, cx) = cx.add_window_view(|window, cx| {
        let field = cx.new(|cx| TextField::new(window, cx));
        field.read(cx).focus(window);
        Narrow { field }
    });
    cx.run_until_parked();
    let field = root.read_with(cx, |root, _| root.field.clone());
    cx.simulate_input(&"word ".repeat(20));
    let (scroll, bounds) = field.read_with(cx, |field, _| {
        (field.scroll_offset(), field.last_painted_bounds().unwrap())
    });
    assert!(bounds.size.width < px(200.));
    // 100 chars × 7.8px = 780px of text: scrolled so the caret is visible.
    assert!(scroll > px(500.), "scroll {scroll:?}");
    cx.simulate_keystrokes("cmd-left");
    assert_eq!(
        field.read_with(cx, |field, _| field.scroll_offset()),
        px(0.)
    );
}

struct Narrow {
    field: Entity<TextField>,
}

impl Render for Narrow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(200.)).child(self.field.clone())
    }
}

// ---------------------------------------------------------------------------
// Picker
// ---------------------------------------------------------------------------

struct Names {
    names: Vec<SharedString>,
    with_headers: bool,
    matches: Vec<StringMatch>,
    confirmed: Rc<RefCell<Vec<(String, bool)>>>,
    dismissed: Rc<Cell<usize>>,
    entry_calls: Rc<Cell<usize>>,
}

impl Names {
    fn new(names: &[&str]) -> Self {
        Self {
            names: names
                .iter()
                .map(|n| SharedString::from(n.to_string()))
                .collect(),
            with_headers: false,
            matches: Vec::new(),
            confirmed: Rc::default(),
            dismissed: Rc::default(),
            entry_calls: Rc::default(),
        }
    }

    /// With headers: entry 0 is "Recent", entry 3 is "All" (fixed layout,
    /// only for the unfiltered list).
    fn header_at(&self, ix: usize) -> Option<&'static str> {
        if !self.with_headers {
            return None;
        }
        match ix {
            0 => Some("Recent"),
            3 => Some("All"),
            _ => None,
        }
    }

    fn match_ix(&self, ix: usize) -> usize {
        if self.with_headers {
            ix - if ix > 3 { 2 } else { 1 }
        } else {
            ix
        }
    }
}

impl PickerDelegate for Names {
    fn update_matches(&mut self, query: &str, _: &mut Window, _: &mut Context<Picker<Self>>) {
        self.matches = match_strings(query, self.names.iter().map(|n| n.as_ref()), usize::MAX);
    }

    fn entry_count(&self) -> usize {
        self.matches.len() + if self.with_headers { 2 } else { 0 }
    }

    fn entry(&self, ix: usize, _: &App) -> PickerEntry {
        self.entry_calls.set(self.entry_calls.get() + 1);
        if let Some(header) = self.header_at(ix) {
            return PickerEntry::Header(header.into());
        }
        let m = &self.matches[self.match_ix(ix)];
        PickerItem::new(self.names[m.candidate_index].clone())
            .positions(m.positions.clone())
            .into()
    }

    fn is_selectable(&self, ix: usize, _: &App) -> bool {
        self.header_at(ix).is_none()
    }

    fn confirm(
        &mut self,
        ix: usize,
        secondary: bool,
        _: &mut Window,
        _: &mut Context<Picker<Self>>,
    ) -> bool {
        let name = self.names[self.matches[self.match_ix(ix)].candidate_index].to_string();
        self.confirmed.borrow_mut().push((name, secondary));
        true
    }

    fn dismissed(&mut self, _: &mut Window, _: &mut Context<Picker<Self>>) {
        self.dismissed.set(self.dismissed.get() + 1);
    }

    fn empty_text(&self, query: &str) -> SharedString {
        format!("No documents match “{query}”").into()
    }
}

fn picker_window(
    cx: &mut TestAppContext,
    delegate: Names,
) -> (
    Entity<Picker<Names>>,
    &mut VisualTestContext,
    Rc<Cell<usize>>,
) {
    init(cx);
    let (picker, cx) = cx.add_window_view(move |window, cx| Picker::modal(delegate, window, cx));
    let dismiss_events = Rc::new(Cell::new(0));
    cx.update(|window, cx| {
        window.activate_window();
        window.focus(&picker.focus_handle(cx));
        let dismiss_events = dismiss_events.clone();
        cx.subscribe(&picker, move |_, _: &DismissEvent, _| {
            dismiss_events.set(dismiss_events.get() + 1)
        })
        .detach();
    });
    cx.run_until_parked();
    (picker, cx, dismiss_events)
}

const DOCS: &[&str] = &[
    "Chapter one",
    "Chapter two",
    "Notes on style",
    "Draft: opening",
    "Letter to the editor",
    "Outline",
];

fn selected_label(picker: &Entity<Picker<Names>>, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| {
        let picker = picker.read(cx);
        picker
            .selected_index()
            .map(|ix| match picker.delegate.entry(ix, cx) {
                PickerEntry::Item(item) => item.label.to_string(),
                PickerEntry::Header(h) => format!("header {h}"),
            })
    })
}

#[gpui::test]
fn picker_filters_and_navigates(cx: &mut TestAppContext) {
    let (picker, cx, dismiss_events) = picker_window(cx, Names::new(DOCS));
    assert_eq!(selected_label(&picker, cx).as_deref(), Some("Chapter one"));

    cx.simulate_keystrokes("down down");
    assert_eq!(
        selected_label(&picker, cx).as_deref(),
        Some("Notes on style")
    );
    cx.simulate_keystrokes("ctrl-p");
    assert_eq!(selected_label(&picker, cx).as_deref(), Some("Chapter two"));
    cx.simulate_keystrokes("ctrl-n ctrl-n");
    assert_eq!(
        selected_label(&picker, cx).as_deref(),
        Some("Draft: opening")
    );
    cx.simulate_keystrokes("end");
    assert_eq!(selected_label(&picker, cx).as_deref(), Some("Outline"));
    cx.simulate_keystrokes("down"); // no wrap
    assert_eq!(selected_label(&picker, cx).as_deref(), Some("Outline"));
    cx.simulate_keystrokes("home");
    assert_eq!(selected_label(&picker, cx).as_deref(), Some("Chapter one"));
    cx.simulate_keystrokes("pagedown");
    assert_eq!(selected_label(&picker, cx).as_deref(), Some("Outline"));
    cx.simulate_keystrokes("pageup");
    assert_eq!(selected_label(&picker, cx).as_deref(), Some("Chapter one"));

    // Typing filters; the best match is selected.
    cx.simulate_input("ed");
    picker.read_with(cx, |picker, cx| {
        assert_eq!(picker.query(cx), "ed");
        assert_eq!(picker.delegate.entry_count(), 1);
    });
    assert_eq!(
        selected_label(&picker, cx).as_deref(),
        Some("Letter to the editor")
    );

    // The query field still edits normally (left/right are not list keys).
    cx.simulate_keystrokes("left backspace");
    picker.read_with(cx, |picker, cx| assert_eq!(picker.query(cx), "d"));

    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("ch");
    cx.simulate_keystrokes("down cmd-enter");
    picker.read_with(cx, |picker, _| {
        assert_eq!(
            *picker.delegate.confirmed.borrow(),
            vec![("Chapter two".to_string(), true)]
        );
    });
    assert_eq!(dismiss_events.get(), 1);
}

#[gpui::test]
fn picker_confirm_and_dismiss(cx: &mut TestAppContext) {
    let (picker, cx, dismiss_events) = picker_window(cx, Names::new(DOCS));
    cx.simulate_input("outl");
    cx.simulate_keystrokes("enter");
    picker.read_with(cx, |picker, _| {
        assert_eq!(
            *picker.delegate.confirmed.borrow(),
            vec![("Outline".to_string(), false)]
        );
        assert_eq!(picker.delegate.dismissed.get(), 1);
    });
    assert_eq!(dismiss_events.get(), 1);
}

#[gpui::test]
fn picker_escape_dismisses(cx: &mut TestAppContext) {
    let (picker, cx, dismiss_events) = picker_window(cx, Names::new(DOCS));
    cx.simulate_keystrokes("escape");
    picker.read_with(cx, |picker, _| {
        assert!(picker.delegate.confirmed.borrow().is_empty());
        assert_eq!(picker.delegate.dismissed.get(), 1);
    });
    assert_eq!(dismiss_events.get(), 1);
}

#[gpui::test]
fn picker_headers_are_skipped(cx: &mut TestAppContext) {
    let mut names = Names::new(DOCS);
    names.with_headers = true;
    let (picker, cx, _) = picker_window(cx, names);
    // Entry 0 is a header: the first item is selected.
    picker.read_with(cx, |picker, _| assert_eq!(picker.selected_index(), Some(1)));
    cx.simulate_keystrokes("down");
    picker.read_with(cx, |picker, _| assert_eq!(picker.selected_index(), Some(2)));
    cx.simulate_keystrokes("down"); // skips the header at 3
    picker.read_with(cx, |picker, _| assert_eq!(picker.selected_index(), Some(4)));
    cx.simulate_keystrokes("up");
    picker.read_with(cx, |picker, _| assert_eq!(picker.selected_index(), Some(2)));
    cx.simulate_keystrokes("home up");
    picker.read_with(cx, |picker, _| assert_eq!(picker.selected_index(), Some(1)));
}

#[gpui::test]
fn picker_empty_state(cx: &mut TestAppContext) {
    let (picker, cx, _) = picker_window(cx, Names::new(DOCS));
    cx.simulate_input("zzz");
    picker.read_with(cx, |picker, cx| {
        assert_eq!(picker.delegate.entry_count(), 0);
        assert_eq!(picker.selected_index(), None);
        assert_eq!(
            picker.delegate.empty_text(&picker.query(cx)),
            "No documents match “zzz”"
        );
    });
    // enter with nothing selected is a no-op
    cx.simulate_keystrokes("enter");
    picker.read_with(cx, |picker, _| {
        assert!(picker.delegate.confirmed.borrow().is_empty())
    });
}

#[gpui::test]
fn picker_large_list_is_virtualized(cx: &mut TestAppContext) {
    let names: Vec<String> = (0..5000).map(|i| format!("Document {i:04}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (picker, cx, _) = picker_window(cx, Names::new(&refs));
    let calls = picker.read_with(cx, |picker, _| picker.delegate.entry_calls.clone());
    calls.set(0);
    cx.simulate_keystrokes("end");
    picker.read_with(cx, |picker, _| {
        assert_eq!(picker.selected_index(), Some(4999))
    });
    // Only the visible rows (12) plus navigation lookups were built.
    assert!(calls.get() < 100, "entry() called {} times", calls.get());
    cx.simulate_keystrokes("pageup");
    picker.read_with(cx, |picker, _| {
        assert_eq!(picker.selected_index(), Some(4988))
    });
    cx.simulate_input("4321");
    assert_eq!(
        selected_label(&picker, cx).as_deref(),
        Some("Document 4321")
    );
}

#[gpui::test]
fn picker_mouse_hover_and_click(cx: &mut TestAppContext) {
    let (picker, cx, dismiss_events) = picker_window(cx, Names::new(DOCS));
    // Rows are 32px tall in the modal picker; find the list via the field.
    let field = picker.read_with(cx, |picker, _| picker.query_field().unwrap().clone());
    let field_bounds = field
        .read_with(cx, |field, _| field.last_painted_bounds())
        .unwrap();
    // query row (48px) + divider (1px) + list padding (4px); field is centred in the row.
    let row_top = field_bounds.center().y + px(24. + 1. + 4.);
    let third_row = point(field_bounds.left() + px(40.), row_top + px(32. * 2. + 16.));
    cx.simulate_mouse_move(third_row, None, Modifiers::default());
    assert_eq!(
        selected_label(&picker, cx).as_deref(),
        Some("Notes on style")
    );
    cx.simulate_click(third_row, Modifiers::default());
    picker.read_with(cx, |picker, _| {
        assert_eq!(
            *picker.delegate.confirmed.borrow(),
            vec![("Notes on style".to_string(), false)]
        );
    });
    assert_eq!(dismiss_events.get(), 1);
    // The click did not move focus out of the query field.
    cx.update(|window, cx| assert!(field.read(cx).is_focused(window)));
}

#[gpui::test]
fn picker_external_query(cx: &mut TestAppContext) {
    init(cx);
    let (picker, cx) = cx
        .add_window_view(|window, cx| Picker::popup(Names::new(DOCS), window, cx).external_query());
    cx.run_until_parked();
    picker.update_in(cx, |picker, window, cx| {
        assert!(picker.query_field().is_none());
        picker.set_query("chap", window, cx);
        assert_eq!(picker.delegate.entry_count(), 2);
        picker.select_next(window, cx);
        picker.confirm(false, window, cx);
        assert_eq!(
            *picker.delegate.confirmed.borrow(),
            vec![("Chapter two".to_string(), false)]
        );
    });
}

// ---------------------------------------------------------------------------
// ModalLayer and ConfirmDialog
// ---------------------------------------------------------------------------

actions!(widgets_test, [OpenPalette]);

struct Root {
    editor_focus: FocusHandle,
    layer: Entity<ModalLayer>,
    toasts: Entity<ToastLayer>,
}

impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .child(
                div()
                    .key_context("Editor")
                    .track_focus(&self.editor_focus)
                    .size_full(),
            )
            .child(self.layer.clone())
            .child(self.toasts.clone())
    }
}

fn root_window(cx: &mut TestAppContext) -> (Entity<Root>, &mut VisualTestContext) {
    init(cx);
    let (root, cx) = cx.add_window_view(|window, cx| {
        let root = Root {
            editor_focus: cx.focus_handle(),
            layer: cx.new(|_| ModalLayer::new()),
            toasts: cx.new(|_| ToastLayer::new()),
        };
        window.focus(&root.editor_focus);
        root
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (root, cx)
}

fn editor_focused(root: &Entity<Root>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| root.read(cx).editor_focus.is_focused(window))
}

#[gpui::test]
fn modal_layer_moves_and_restores_focus(cx: &mut TestAppContext) {
    let (root, cx) = root_window(cx);
    let layer = root.read_with(cx, |root, _| root.layer.clone());
    assert!(editor_focused(&root, cx));

    // Show a picker: focus moves into its query field.
    cx.update(|window, cx| {
        layer.update(cx, |layer, cx| {
            let picker = cx.new(|cx| Picker::modal(Names::new(DOCS), window, cx));
            layer.show(picker, window, cx);
        })
    });
    cx.run_until_parked();
    assert!(!editor_focused(&root, cx));
    let picker = layer
        .read_with(cx, |layer, _| layer.active::<Picker<Names>>())
        .unwrap();
    cx.simulate_input("not");
    picker.read_with(cx, |picker, cx| assert_eq!(picker.query(cx), "not"));

    // Escape closes it and focus goes back to the editor.
    cx.simulate_keystrokes("escape");
    layer.read_with(cx, |layer, _| assert!(!layer.has_modal()));
    assert!(editor_focused(&root, cx));

    // Confirming closes it too.
    cx.update(|window, cx| {
        layer.update(cx, |layer, cx| {
            layer.toggle(window, cx, |window, cx| {
                Picker::modal(Names::new(DOCS), window, cx)
            });
        })
    });
    cx.run_until_parked();
    assert!(layer.read_with(cx, |layer, _| layer.is_open::<Picker<Names>>()));
    cx.simulate_keystrokes("enter");
    layer.read_with(cx, |layer, _| assert!(!layer.has_modal()));
    assert!(editor_focused(&root, cx));

    // Programmatic dismiss and toggle.
    cx.update(|window, cx| {
        layer.update(cx, |layer, cx| {
            layer.toggle(window, cx, |window, cx| {
                Picker::modal(Names::new(DOCS), window, cx)
            });
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        layer.update(cx, |layer, cx| {
            layer.toggle(window, cx, |window, cx| {
                Picker::modal(Names::new(DOCS), window, cx)
            });
        })
    });
    cx.run_until_parked();
    layer.read_with(cx, |layer, _| assert!(!layer.has_modal()));
    assert!(editor_focused(&root, cx));
}

#[gpui::test]
fn modal_layer_escape_fallback_for_any_view(cx: &mut TestAppContext) {
    struct Plain {
        focus: FocusHandle,
    }
    impl gpui::EventEmitter<DismissEvent> for Plain {}
    impl Focusable for Plain {
        fn focus_handle(&self, _: &App) -> FocusHandle {
            self.focus.clone()
        }
    }
    impl Render for Plain {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().track_focus(&self.focus).size(px(100.))
        }
    }

    let (root, cx) = root_window(cx);
    let layer = root.read_with(cx, |root, _| root.layer.clone());
    cx.update(|window, cx| {
        layer.update(cx, |layer, cx| {
            let plain = cx.new(|cx| Plain {
                focus: cx.focus_handle(),
            });
            layer.show(plain, window, cx);
        })
    });
    cx.run_until_parked();
    assert!(!editor_focused(&root, cx));
    cx.simulate_keystrokes("escape");
    layer.read_with(cx, |layer, _| assert!(!layer.has_modal()));
    assert!(editor_focused(&root, cx));
}

#[gpui::test]
fn confirm_dialog_keys(cx: &mut TestAppContext) {
    let (root, cx) = root_window(cx);
    let layer = root.read_with(cx, |root, _| root.layer.clone());
    let log: Rc<RefCell<Vec<String>>> = Rc::default();

    let open = |cx: &mut VisualTestContext, log: Rc<RefCell<Vec<String>>>, danger: bool| {
        let layer = layer.clone();
        cx.update(move |window, cx| {
            let confirmed = log.clone();
            let dialog = cx.new(|cx| {
                let dialog = if danger {
                    ConfirmDialog::new("Delete “Outline” forever?", "This cannot be undone.", cx)
                        .confirm_label("Delete forever")
                        .danger()
                } else {
                    ConfirmDialog::new("Restore this version?", "The current text is kept.", cx)
                        .confirm_label("Restore")
                };
                dialog.on_confirm(move |_, _| confirmed.borrow_mut().push("confirm".into()))
            });
            let events = log.clone();
            cx.subscribe(&dialog, move |_, event: &ConfirmDialogEvent, _| {
                events.borrow_mut().push(format!("{event:?}"))
            })
            .detach();
            ConfirmDialog::show(dialog, &layer, window, cx);
        });
        cx.run_until_parked();
    };

    // enter confirms (focus starts on the confirm button)
    open(cx, log.clone(), false);
    cx.simulate_keystrokes("enter");
    assert_eq!(*log.borrow(), vec!["Confirmed", "confirm"]);
    assert!(editor_focused(&root, cx));
    layer.read_with(cx, |layer, _| assert!(!layer.has_modal()));

    // tab to cancel, then enter cancels
    log.borrow_mut().clear();
    open(cx, log.clone(), false);
    cx.simulate_keystrokes("tab enter");
    assert_eq!(*log.borrow(), vec!["Cancelled"]);
    assert!(editor_focused(&root, cx));

    // tab twice comes back to confirm; space presses it
    log.borrow_mut().clear();
    open(cx, log.clone(), false);
    cx.simulate_keystrokes("tab shift-tab space");
    assert_eq!(*log.borrow(), vec!["Confirmed", "confirm"]);

    // escape cancels
    log.borrow_mut().clear();
    open(cx, log.clone(), false);
    cx.simulate_keystrokes("escape");
    assert_eq!(*log.borrow(), vec!["Cancelled"]);
    assert!(editor_focused(&root, cx));

    // cmd-enter confirms from either button
    log.borrow_mut().clear();
    open(cx, log.clone(), false);
    cx.simulate_keystrokes("tab cmd-enter");
    assert_eq!(*log.borrow(), vec!["Confirmed", "confirm"]);
}

#[gpui::test]
fn a_destructive_dialog_starts_on_cancel(cx: &mut TestAppContext) {
    let (root, cx) = root_window(cx);
    let layer = root.read_with(cx, |root, _| root.layer.clone());
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let open = |cx: &mut VisualTestContext| {
        let layer = layer.clone();
        let log = log.clone();
        cx.update(move |window, cx| {
            let confirmed = log.clone();
            let dialog = cx.new(|cx| {
                ConfirmDialog::new("Delete “Outline” forever?", "This cannot be undone.", cx)
                    .confirm_label("Delete forever")
                    .danger()
                    .on_confirm(move |_, _| confirmed.borrow_mut().push("confirm".into()))
            });
            let events = log.clone();
            cx.subscribe(&dialog, move |_, event: &ConfirmDialogEvent, _| {
                events.borrow_mut().push(format!("{event:?}"))
            })
            .detach();
            ConfirmDialog::show(dialog, &layer, window, cx);
        });
        cx.run_until_parked();
    };

    // enter does the safe thing
    open(cx);
    cx.simulate_keystrokes("enter");
    assert_eq!(*log.borrow(), vec!["Cancelled"]);
    assert!(editor_focused(&root, cx));

    // the destructive button has its own key
    log.borrow_mut().clear();
    open(cx);
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(*log.borrow(), vec!["Confirmed", "confirm"]);
    cx.update(|_, cx| {
        assert_eq!(
            KeyHint::for_action(&writer_ui::confirm::ConfirmAnyway, cx).map(|hint| hint.text()),
            Some("⌘↩".to_string())
        );
    });

    // moving to it and pressing it works as in any dialog
    log.borrow_mut().clear();
    open(cx);
    cx.simulate_keystrokes("tab enter");
    assert_eq!(*log.borrow(), vec!["Confirmed", "confirm"]);
    log.borrow_mut().clear();
    open(cx);
    cx.simulate_keystrokes("right space");
    assert_eq!(*log.borrow(), vec!["Confirmed", "confirm"]);

    log.borrow_mut().clear();
    open(cx);
    cx.simulate_keystrokes("escape");
    assert_eq!(*log.borrow(), vec!["Cancelled"]);
}

// ---------------------------------------------------------------------------
// Toasts and key hints
// ---------------------------------------------------------------------------

#[gpui::test]
fn toasts_stack_and_expire(cx: &mut TestAppContext) {
    let (root, cx) = root_window(cx);
    let toasts = root.read_with(cx, |root, _| root.toasts.clone());
    cx.update(|_, cx| {
        ToastLayer::set_global(&toasts, cx);
        toasts.update(cx, |layer, cx| {
            layer.push(Toast::new("Exported to ~/Essay.md").success(), cx);
            layer.push(
                Toast::new("Short one").duration(Duration::from_millis(500)),
                cx,
            );
        });
        writer_ui::toast::show_toast(Toast::new("Export failed").error(), cx).unwrap();
    });
    cx.run_until_parked();
    assert_eq!(toasts.read_with(cx, |layer, _| layer.toasts().count()), 3);
    // Focus is untouched.
    assert!(editor_focused(&root, cx));

    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    let left: Vec<String> = toasts.read_with(cx, |layer, _| {
        layer.toasts().map(|t| t.message.to_string()).collect()
    });
    assert_eq!(left, vec!["Exported to ~/Essay.md", "Export failed"]);

    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert_eq!(toasts.read_with(cx, |layer, _| layer.toasts().count()), 0);

    // At most MAX_VISIBLE at once.
    cx.update(|_, cx| {
        toasts.update(cx, |layer, cx| {
            for i in 0..10 {
                layer.push(Toast::new(format!("toast {i}")), cx);
            }
        })
    });
    let shown: Vec<String> = toasts.read_with(cx, |layer, _| {
        layer.toasts().map(|t| t.message.to_string()).collect()
    });
    assert_eq!(shown, vec!["toast 6", "toast 7", "toast 8", "toast 9"]);
}

/// No toast draws its message past its own edge, whatever the message is
/// made of: it wraps onto at most `MESSAGE_LINES` lines and ends in an
/// ellipsis when there is more. (0.5.0 drew "Exported the keyboard
/// shortcuts to /private/tmp/…" on two lines, the second one running out
/// of the toast: a path is one long word, and GPUI's own line clamp leaves
/// the rest of the text on the last line.)
#[gpui::test]
fn no_toast_draws_its_message_past_its_edge(cx: &mut TestAppContext) {
    use writer_ui::toast::{MAX_WIDTH, MESSAGE_LINES};
    actions!(widgets_test, [UndoIt]);

    let (root, cx) = root_window(cx);
    let toasts = root.read_with(cx, |root, _| root.toasts.clone());
    let path = "/private/tmp/claude-501/-Users-someone-Development-the-writer/\
                32da77ec-6880-4cdd-a335-5be772628529/scratchpad/shots-export/14/\
                The Writer Shortcuts.json";
    let messages: Vec<(String, bool)> = vec![
        // The message of the capture: a long path after a few words.
        (format!("Exported the keyboard shortcuts to {path}"), true),
        // One word and nothing else, far longer than three lines.
        ("x".repeat(600), true),
        // Many short words.
        ("word ".repeat(200).trim_end().to_string(), true),
        // Words of another script, and line breaks in the message.
        (
            format!(
                "Könnte nicht geschrieben werden:\n{}",
                "ünïcödé ".repeat(80)
            ),
            true,
        ),
        // What fits is drawn whole: on one line, on two, on three.
        ("Exported to ~/Essay.md".to_string(), false),
        (
            "Exported the keyboard shortcuts to ~/Documents/The Writer Shortcuts.json".to_string(),
            false,
        ),
        (
            "Could not write ~/Documents/Books/The Harbour/Exports/The Writer Shortcuts.json: \
             the folder is read-only, choose another one"
                .to_string(),
            false,
        ),
    ];
    for with_button in [false, true] {
        for (message, cut) in &messages {
            let id = cx.update(|_, cx| {
                toasts.update(cx, |layer, cx| {
                    layer.clear(cx);
                    let toast = Toast::new(message.clone()).success();
                    let toast = if with_button {
                        toast.action("Undo", UndoIt)
                    } else {
                        toast
                    };
                    layer.push(toast, cx)
                })
            });
            cx.run_until_parked();
            let drawn = toasts
                .read_with(cx, |layer, _| layer.drawn_message(id))
                .unwrap_or_else(|| panic!("the toast was drawn: {message}"));
            let about = format!("{message:?} with_button={with_button}: {drawn:?}");
            // Every line is inside the room the message had.
            assert!(!drawn.lines.is_empty(), "{about}");
            assert!(drawn.lines.len() <= MESSAGE_LINES, "{about}");
            for line in &drawn.lines {
                assert!(*line <= drawn.width + px(0.5), "{about}");
            }
            assert!(drawn.width < MAX_WIDTH, "{about}");
            // Cut only when it has to be, and then with an ellipsis.
            let whole = message.replace('\n', " ");
            assert_eq!(drawn.is_cut(), *cut, "{about}");
            if *cut {
                assert_eq!(drawn.lines.len(), MESSAGE_LINES, "{about}");
                let shown = drawn.text.trim_end_matches('\u{2026}');
                assert!(whole.starts_with(shown), "{about}");
                // What is drawn is most of what there was room for.
                let last = *drawn.lines.last().unwrap();
                assert!(last > drawn.width * 0.6, "{about}");
            } else {
                assert_eq!(drawn.text, whole, "{about}");
            }
            // The toast itself is as tall as its lines and no taller, and
            // the message lies inside it.
            let toast_bounds = cx
                .debug_bounds(Box::leak(
                    format!("toast-{}", toast_number(&format!("{id:?}"))).into_boxed_str(),
                ))
                .expect("the toast is on screen");
            assert!(toast_bounds.size.width <= MAX_WIDTH, "{about}");
            // (18 px a line, or the 22 px of the button, with 9 px above
            // and below and the border.)
            let lines = drawn.lines.len() as f32;
            assert!(
                toast_bounds.size.height <= (px(18.) * lines).max(px(22.)) + px(20.),
                "{about}: {toast_bounds:?}"
            );
        }
    }
}

/// The number in the debug form of a toast's id (`ToastId(3)`).
/// `toast_bounds` is where each visible toast was drawn in the last frame,
/// in window coordinates, oldest first, and empty without toasts.
#[gpui::test]
fn toast_bounds_are_where_the_toasts_were_drawn(cx: &mut TestAppContext) {
    let (root, cx) = root_window(cx);
    let toasts = root.read_with(cx, |root, _| root.toasts.clone());
    assert!(toasts.read_with(cx, |layer, _| layer.toast_bounds()).is_empty());

    let ids = cx.update(|_, cx| {
        toasts.update(cx, |layer, cx| {
            vec![
                layer.push(Toast::new("Exported to ~/Essay.md").success(), cx),
                layer.push(
                    Toast::new("Short one").duration(Duration::from_millis(500)),
                    cx,
                ),
            ]
        })
    });
    cx.run_until_parked();
    let drawn = |cx: &mut VisualTestContext, id: &writer_ui::toast::ToastId| {
        let selector = format!("toast-{}", toast_number(&format!("{id:?}")));
        cx.debug_bounds(Box::leak(selector.into_boxed_str()))
            .expect("the toast is on screen")
    };
    let expected: Vec<_> = ids.iter().map(|id| drawn(cx, id)).collect();
    let bounds = toasts.read_with(cx, |layer, _| layer.toast_bounds());
    assert_eq!(bounds, expected);
    // oldest first, stacked downwards
    assert!(bounds[0].origin.y < bounds[1].origin.y);
    assert!(bounds.iter().all(|b| b.size.width > px(0.) && b.size.height > px(0.)));

    // one expires: one left, where it is drawn now
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    let bounds = toasts.read_with(cx, |layer, _| layer.toast_bounds());
    assert_eq!(bounds, vec![drawn(cx, &ids[0])]);

    // none: empty
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert!(toasts.read_with(cx, |layer, _| layer.toast_bounds()).is_empty());
}

fn toast_number(debug: &str) -> String {
    debug.chars().filter(char::is_ascii_digit).collect()
}

#[gpui::test]
fn key_hints_come_from_the_keymap(cx: &mut TestAppContext) {
    let (root, cx) = root_window(cx);
    cx.update(|_, cx| {
        cx.bind_keys([
            KeyBinding::new("cmd-k", OpenPalette, Some("Workspace")),
            KeyBinding::new("cmd-shift-p", OpenPalette, Some("Editor")),
        ])
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        // keymap-wide: the last added binding
        assert_eq!(KeyHint::for_action(&OpenPalette, cx).unwrap().text(), "⇧⌘P");
        // from the editor's focus: only bindings valid in its contexts
        let focus = root.read(cx).editor_focus.clone();
        assert_eq!(
            KeyHint::for_action_in(&OpenPalette, &focus, window)
                .unwrap()
                .text(),
            "⇧⌘P"
        );
        assert_eq!(
            KeyHint::for_action_in(&writer_ui::text_field::SelectAll, &focus, window),
            None
        );
        assert_eq!(
            KeyHint::for_action(&writer_ui::text_field::Undo, cx)
                .unwrap()
                .text(),
            "⌘Z"
        );
    });
}
