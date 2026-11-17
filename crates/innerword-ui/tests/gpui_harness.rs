//! Headless GPUI tests on gpui 0.2.2 (`test-support` feature).
//!
//! These prove the test harness works on this machine and pin down API
//! behaviour that `docs/gpui-notes.md` relies on. Every claim in the notes
//! marked "(tested)" has a test here.

use std::{cell::RefCell, ops::Range, rc::Rc, time::Duration};

use gpui::{
    App, Bounds, ClipboardItem, Context, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, InspectorElementId, IntoElement,
    KeyBinding, KeyDownEvent, Keystroke, LayoutId, MouseButton, MouseDownEvent, Pixels, Render,
    ShapedLine, SharedString, Style, Task, TestAppContext, TextRun, UTF16Selection, Window,
    actions, div, font, point, prelude::*, px, relative, size,
};
use innerword_ui::theme::{self, ActiveTheme, Appearance, GlobalTheme, ThemeMode};

actions!(
    harness,
    [
        /// Delete the character before the caret.
        Backspace,
        SelectAll,
        ToggleBold,
        Outer,
        Inner,
        Chord,
        ChordPrefixOnly,
    ]
);

// ---------------------------------------------------------------------------
// A minimal text box: focusable view + custom element + EntityInputHandler.
// ---------------------------------------------------------------------------

struct TextBox {
    focus_handle: FocusHandle,
    text: String,
    selection: Range<usize>,
    marked: Option<Range<usize>>,
    bold_toggles: usize,
    unhandled_keys: Vec<String>,
    mouse_downs: Vec<usize>,
    last_layout: Option<ShapedLine>,
}

impl TextBox {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            text: String::new(),
            selection: 0..0,
            marked: None,
            bold_toggles: 0,
            unhandled_keys: Vec::new(),
            mouse_downs: Vec::new(),
            last_layout: None,
        }
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() && self.selection.start > 0 {
            let prev = self.text[..self.selection.start]
                .char_indices()
                .last()
                .map(|(ix, _)| ix)
                .unwrap_or(0);
            self.selection = prev..self.selection.end;
        }
        self.text.replace_range(self.selection.clone(), "");
        self.selection = self.selection.start..self.selection.start;
        cx.notify();
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = 0..self.text.len();
        cx.notify();
    }

    fn toggle_bold(&mut self, _: &ToggleBold, _: &mut Window, cx: &mut Context<Self>) {
        self.bold_toggles += 1;
        cx.notify();
    }

    fn utf16_to_utf8(&self, utf16: usize) -> usize {
        let mut units = 0;
        for (ix, ch) in self.text.char_indices() {
            if units >= utf16 {
                return ix;
            }
            units += ch.len_utf16();
        }
        self.text.len()
    }

    fn utf8_to_utf16(&self, utf8: usize) -> usize {
        self.text[..utf8].chars().map(char::len_utf16).sum()
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.utf16_to_utf8(range.start)..self.utf16_to_utf8(range.end)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.utf8_to_utf16(range.start)..self.utf8_to_utf16(range.end)
    }
}

impl Focusable for TextBox {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for TextBox {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range);
        adjusted_range.replace(self.range_to_utf16(&range));
        Some(self.text[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selection),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|range| self.range_from_utf16(&range))
            .or(self.marked.clone())
            .unwrap_or(self.selection.clone());
        self.text.replace_range(range.clone(), text);
        let caret = range.start + text.len();
        self.selection = caret..caret;
        self.marked = None;
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        new_selected_range: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|range| self.range_from_utf16(&range))
            .or(self.marked.clone())
            .unwrap_or(self.selection.clone());
        self.text.replace_range(range.clone(), text);
        self.marked = (!text.is_empty()).then(|| range.start..range.start + text.len());
        self.selection = new_selected_range
            .map(|sel| self.range_from_utf16(&sel))
            .map(|sel| range.start + sel.start..range.start + sel.end)
            .unwrap_or(range.start + text.len()..range.start + text.len());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range);
        Some(Bounds::from_corners(
            point(
                element_bounds.left() + layout.x_for_index(range.start),
                element_bounds.top(),
            ),
            point(
                element_bounds.left() + layout.x_for_index(range.end),
                element_bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let layout = self.last_layout.as_ref()?;
        Some(self.utf8_to_utf16(layout.closest_index_for_x(point.x)))
    }
}

struct TextBoxElement {
    view: Entity<TextBox>,
}

impl IntoElement for TextBoxElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for TextBoxElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<ShapedLine>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Option<ShapedLine> {
        let text: SharedString = self.view.read(cx).text.clone().into();
        let style = window.text_style();
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        Some(
            window
                .text_system()
                .shape_line(text, font_size, &[run], None),
        )
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        line: &mut Option<ShapedLine>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.view.read(cx).focus_handle.clone();
        // Typed text reaches the view only through this registration, and only
        // while `focus_handle` is focused at paint time.
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        let line = line.take().unwrap();
        line.paint(bounds.origin, window.line_height(), window, cx)
            .unwrap();
        self.view
            .update(cx, |view, _| view.last_layout = Some(line));
    }
}

impl Render for TextBox {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("TextBox")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::toggle_bold))
            // Runs only for keystrokes that no binding handled.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, _| {
                this.unhandled_keys.push(event.keystroke.unparse());
            }))
            .size_full()
            .child(
                div()
                    .id("text-box-line")
                    .debug_selector(|| "text-box-line".into())
                    .w(px(400.))
                    .h(px(40.))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, _| {
                            this.mouse_downs.push(event.click_count);
                        }),
                    )
                    .child(TextBoxElement { view: cx.entity() }),
            )
    }
}

fn bind_text_box_keys(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.bind_keys([
            KeyBinding::new("backspace", Backspace, Some("TextBox")),
            KeyBinding::new("cmd-a", SelectAll, Some("TextBox")),
            KeyBinding::new("cmd-b", ToggleBold, Some("TextBox")),
        ])
    });
}

/// The proof test: a window, a focused view, keystrokes and text input.
#[gpui::test]
fn typing_and_keystrokes_reach_focused_view(cx: &mut TestAppContext) {
    bind_text_box_keys(cx);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let text_box = TextBox::new(cx);
        window.focus(&text_box.focus_handle);
        text_box
    });

    // simulate_input sends one keystroke per character; unbound printable
    // keys fall through to the input handler registered in paint.
    cx.simulate_input("Hello wörld");
    view.read_with(cx, |view, _| {
        assert_eq!(view.text, "Hello wörld");
        assert_eq!(view.selection, view.text.len()..view.text.len());
    });

    // Bound keys become actions and never reach the input handler.
    cx.simulate_keystrokes("backspace backspace cmd-b");
    view.read_with(cx, |view, _| {
        assert_eq!(view.text, "Hello wör");
        assert_eq!(view.bold_toggles, 1);
    });

    // Typing over a selection replaces it.
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Hi");
    view.read_with(cx, |view, _| assert_eq!(view.text, "Hi"));

    // on_key_down sees every keystroke that no binding handled, printable
    // ones included (before the input handler gets the text). simulate_input
    // also sends empty "" keystrokes around the text (it splits on "").
    view.read_with(cx, |view, _| {
        let printable: Vec<&str> = view
            .unhandled_keys
            .iter()
            .map(String::as_str)
            .filter(|k| !k.is_empty())
            .collect();
        assert_eq!(&printable[..3], &["shift-h", "e", "l"]);
        assert!(
            !view
                .unhandled_keys
                .iter()
                .any(|k| k == "cmd-b" || k == "backspace")
        );
    });
    view.update(cx, |view, _| view.unhandled_keys.clear());
    cx.simulate_keystrokes("escape cmd-b left");
    view.read_with(cx, |view, _| {
        assert_eq!(view.unhandled_keys, vec!["escape", "left"]);
        assert_eq!(view.bold_toggles, 2);
    });

    // Mouse: debug_selector bounds + simulate_click.
    let bounds = cx
        .debug_bounds("text-box-line")
        .expect("element was painted");
    cx.simulate_click(bounds.center(), Default::default());
    view.read_with(cx, |view, _| assert_eq!(view.mouse_downs, vec![1]));
}

#[gpui::test]
fn ime_marked_text_through_input_handler(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|window, cx| {
        let text_box = TextBox::new(cx);
        window.focus(&text_box.focus_handle);
        text_box
    });
    cx.simulate_input("a");
    // Simulate what macOS does for a dead key: mark, then commit.
    view.update_in(cx, |view, window, cx| {
        view.replace_and_mark_text_in_range(None, "´", Some(1..1), window, cx);
        assert_eq!(view.marked, Some(1..3));
        view.replace_text_in_range(None, "é", window, cx);
    });
    view.read_with(cx, |view, _| {
        assert_eq!(view.text, "aé");
        assert_eq!(view.marked, None);
    });
}

// ---------------------------------------------------------------------------
// Key dispatch precedence, multi-keystroke bindings, interception.
// ---------------------------------------------------------------------------

struct Nested {
    outer_focus: FocusHandle,
    inner_focus: FocusHandle,
    log: Rc<RefCell<Vec<&'static str>>>,
}

impl Render for Nested {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let log = self.log.clone();
        let log2 = self.log.clone();
        let log3 = self.log.clone();
        let log4 = self.log.clone();
        let log5 = self.log.clone();
        div()
            .key_context("Workspace")
            .track_focus(&self.outer_focus)
            .on_action(move |_: &Outer, _, _| log.borrow_mut().push("outer"))
            .on_action(move |_: &Chord, _, _| log4.borrow_mut().push("chord"))
            .capture_action(move |_: &Inner, _, _| log5.borrow_mut().push("capture inner"))
            .child(
                div()
                    .key_context("Editor mode=full")
                    .track_focus(&self.inner_focus)
                    .on_action(move |_: &Inner, _, cx| {
                        log2.borrow_mut().push("inner");
                        // Handlers stop propagation by default; opt back in:
                        cx.propagate();
                    })
                    .on_action(move |_: &Outer, _, _| log3.borrow_mut().push("outer from editor")),
            )
    }
}

#[gpui::test]
fn dispatch_precedence_and_chords(cx: &mut TestAppContext) {
    let log = Rc::new(RefCell::new(Vec::new()));
    cx.update(|cx| {
        cx.bind_keys([
            // Same keystroke in two contexts: the deeper context wins.
            KeyBinding::new("cmd-k", Outer, Some("Workspace")),
            KeyBinding::new("cmd-k", Inner, Some("Editor")),
            // Same keystroke and same depth: the binding added later wins.
            KeyBinding::new("cmd-j", Inner, Some("Editor")),
            KeyBinding::new("cmd-j", Outer, Some("Editor && mode == full")),
            // A two-keystroke chord. A single-keystroke binding for its prefix
            // only waits for the chord if it was added BEFORE the chord; a
            // prefix binding added later wins immediately (no pending state).
            KeyBinding::new("ctrl-x", ChordPrefixOnly, None),
            KeyBinding::new("ctrl-x ctrl-s", Chord, None),
        ])
    });
    let log_for_view = log.clone();
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let nested = Nested {
            outer_focus: cx.focus_handle(),
            inner_focus: cx.focus_handle(),
            log: log_for_view,
        };
        window.focus(&nested.inner_focus);
        nested
    });

    cx.simulate_keystrokes("cmd-k");
    assert_eq!(
        log.borrow().as_slice(),
        ["capture inner", "inner", "outer from editor"],
        "deeper context wins; capture listeners run root-to-focus first; cx.propagate() \
         in a handler means 'not handled': GPUI then dispatches the next matching \
         binding (cmd-k => Outer in Workspace)"
    );
    log.borrow_mut().clear();

    cx.simulate_keystrokes("cmd-j");
    assert_eq!(log.borrow().as_slice(), ["outer from editor"]);
    log.borrow_mut().clear();

    // The chord: the first keystroke is pending (ChordPrefixOnly also matches,
    // but GPUI waits because a longer, later-added binding is possible).
    cx.simulate_keystrokes("ctrl-x");
    cx.update(|window, _| assert!(window.has_pending_keystrokes()));
    cx.simulate_keystrokes("ctrl-s");
    assert_eq!(log.borrow().as_slice(), ["chord"]);
    log.borrow_mut().clear();

    // Pending input times out after 1 second and is then replayed alone:
    // ctrl-x resolves to ChordPrefixOnly (no handler here, so nothing logs).
    cx.simulate_keystrokes("ctrl-x");
    cx.executor().advance_clock(Duration::from_millis(1100));
    cx.run_until_parked();
    cx.update(|window, _| assert!(!window.has_pending_keystrokes()));
    assert!(log.borrow().is_empty());

    // Registry queries used by a shortcuts overlay.
    cx.update(|window, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let all: Vec<String> = keymap
            .bindings()
            .map(|binding| {
                let keys: Vec<String> = binding.keystrokes().iter().map(|k| k.unparse()).collect();
                format!(
                    "{} => {} in {}",
                    keys.join(" "),
                    binding.action().name(),
                    binding
                        .predicate()
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "*".into())
                )
            })
            .collect();
        assert!(all.contains(&"cmd-k => harness::Outer in Workspace".to_string()));
        assert!(all.contains(&"ctrl-x ctrl-s => harness::Chord in *".to_string()));

        let for_inner: Vec<String> = keymap
            .bindings_for_action(&Inner)
            .map(|b| b.keystrokes()[0].to_string())
            .collect();
        // Display uses macOS symbols.
        assert_eq!(for_inner, vec!["⌘K", "⌘J"]);

        // Window-level query honours the focused context stack.
        let shown = window.keystroke_text_for(&Outer);
        assert_eq!(shown, "⌘J");
        let _ = view;

        // Doc comments on actions! become action documentation.
        assert_eq!(
            cx.action_documentation().get("harness::Backspace").copied(),
            Some("Delete the character before the caret.")
        );
    });
}

#[gpui::test]
fn keystroke_parsing(_cx: &mut TestAppContext) {
    // "opt" is not a modifier name in GPUI; use "alt".
    assert!(Keystroke::parse("cmd-opt-s").is_err());
    let ks = Keystroke::parse("cmd-alt-s").unwrap();
    assert!(ks.modifiers.platform && ks.modifiers.alt);
    assert_eq!(ks.to_string(), "⌥⌘S");
    // Uppercase letter means shift.
    let ks = Keystroke::parse("cmd-B").unwrap();
    assert!(ks.modifiers.shift);
    assert_eq!(ks.key, "b");
    // Punctuation keys: "cmd-shift-." and "cmd-\" parse; "cmd--" is minus.
    assert_eq!(Keystroke::parse("cmd-shift-.").unwrap().key, ".");
    assert_eq!(Keystroke::parse("cmd-\\").unwrap().key, "\\");
    assert_eq!(Keystroke::parse("cmd--").unwrap().key, "-");
    assert_eq!(Keystroke::parse("cmd-'").unwrap().key, "'");
}

struct Interceptor {
    focus: FocusHandle,
    actions: usize,
}

impl Render for Interceptor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Outer, _, _| this.actions += 1))
    }
}

#[gpui::test]
fn intercept_and_observe_keystrokes(cx: &mut TestAppContext) {
    cx.update(|cx| cx.bind_keys([KeyBinding::new("j", Outer, None)]));
    let intercepted = Rc::new(RefCell::new(Vec::<String>::new()));
    let observed = Rc::new(RefCell::new(Vec::<String>::new()));
    let (view, cx) = cx.add_window_view(|window, cx| {
        let focus = cx.focus_handle();
        window.focus(&focus);
        Interceptor { focus, actions: 0 }
    });
    let intercept_log = intercepted.clone();
    let observe_log = observed.clone();
    let _subs = cx.update(|_, cx| {
        (
            // Runs before the keymap is consulted. stop_propagation swallows
            // the keystroke (no action, no key_down, no text input).
            cx.intercept_keystrokes(move |event, _, cx| {
                intercept_log.borrow_mut().push(event.keystroke.unparse());
                if event.keystroke.key == "k" {
                    cx.stop_propagation();
                }
            }),
            // Runs after dispatch; `action` tells which action (if any) ran.
            cx.observe_keystrokes(move |event, _, _| {
                observe_log.borrow_mut().push(format!(
                    "{}:{}",
                    event.keystroke.unparse(),
                    event.action.as_ref().map(|a| a.name()).unwrap_or("-")
                ));
            }),
        )
    });
    cx.simulate_keystrokes("j k");
    assert_eq!(intercepted.borrow().as_slice(), ["j", "k"]);
    assert_eq!(observed.borrow().as_slice(), ["j:harness::Outer"]);
    view.read_with(cx, |view, _| assert_eq!(view.actions, 1));
}

// ---------------------------------------------------------------------------
// Text system in tests: fake metrics.
// ---------------------------------------------------------------------------

#[gpui::test]
fn test_text_system_uses_fake_metrics(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    cx.update(|window, _| {
        let run = |len| TextRun {
            len,
            font: font("Charter"),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let text = "hello world";
        let line = window
            .text_system()
            .shape_line(text.into(), px(10.), &[run(text.len())], None);
        // NoopTextSystem: every char advances 0.6 * font_size.
        assert_eq!(line.width, px(66.));
        assert_eq!(line.x_for_index(6), px(36.));
        assert_eq!(line.closest_index_for_x(px(37.)), 6);
        assert_eq!(line.index_for_x(px(100.)), None);

        // shape_text splits on '\n' (runs must cover the '\n' bytes) and
        // wraps each line at wrap_width.
        let text = "aaaa bbbb cccc\ndd";
        let lines = window
            .text_system()
            .shape_text(
                text.into(),
                px(10.),
                &[run(text.len())],
                Some(px(60.)),
                None,
            )
            .unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text.as_ref(), "aaaa bbbb cccc");
        assert_eq!(lines[1].text.as_ref(), "dd");
        // Visual rows of the first line: the boundary is the byte index where
        // the next row starts.
        let starts: Vec<usize> = lines[0]
            .wrap_boundaries()
            .iter()
            .map(|b| lines[0].runs()[b.run_ix].glyphs[b.glyph_ix].index)
            .collect();
        // Rows: "aaaa bbbb " (60px, the trailing space stays on the row) and "cccc".
        assert_eq!(starts, vec![10]);
        assert_eq!(lines[0].size(px(20.)).height, px(40.));
        assert_eq!(
            lines[0].position_for_index(7, px(20.)),
            Some(point(px(42.), px(0.)))
        );
        assert_eq!(
            lines[0].position_for_index(12, px(20.)),
            Some(point(px(12.), px(20.)))
        );
        // Ok(ix) inside the row, Err(ix) when the point is past the row end.
        assert_eq!(
            lines[0].closest_index_for_position(point(px(13.), px(25.)), px(20.)),
            Ok(12)
        );
        assert_eq!(
            lines[0].closest_index_for_position(point(px(50.), px(25.)), px(20.)),
            Err(14)
        );
        // LineWrapper (what the Text element uses): Boundary.ix is the byte
        // index where the next row starts, same as the wrap boundaries above.
        let mut wrapper = window.text_system().line_wrapper(font("Charter"), px(10.));
        let rows: Vec<usize> = wrapper
            .wrap_line(&[gpui::LineFragment::text("aaaa bbbb cccc")], px(60.))
            .map(|boundary| boundary.ix)
            .collect();
        assert_eq!(rows, vec![10]);
        // The test text system reports no installed families.
        let names = window.text_system().all_font_names();
        assert!(!names.contains(&"Charter".to_string()));
        assert!(names.contains(&".SystemUIFont".to_string()));
    });
}

// ---------------------------------------------------------------------------
// Async: debounce with timers, advance_clock.
// ---------------------------------------------------------------------------

struct Saver {
    saves: usize,
    pending: Option<Task<()>>,
}

impl Saver {
    fn schedule(&mut self, cx: &mut Context<Self>) {
        // Replacing the task drops (cancels) the previous one: a debounce.
        self.pending = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            this.update(cx, |this, _| this.saves += 1).ok();
        }));
    }
}

#[gpui::test]
fn debounce_with_timer(cx: &mut TestAppContext) {
    let saver = cx.new(|_| Saver {
        saves: 0,
        pending: None,
    });
    for _ in 0..3 {
        saver.update(cx, |saver, cx| saver.schedule(cx));
        cx.executor().advance_clock(Duration::from_millis(200));
    }
    assert_eq!(saver.read_with(cx, |s, _| s.saves), 0);
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    assert_eq!(saver.read_with(cx, |s, _| s.saves), 1);
}

// ---------------------------------------------------------------------------
// Focus events and restoring focus after an overlay.
// ---------------------------------------------------------------------------

struct FocusProbe {
    editor: FocusHandle,
    overlay: FocusHandle,
    events: Vec<&'static str>,
    _subs: Vec<gpui::Subscription>,
}

impl Render for FocusProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .child(div().track_focus(&self.editor))
            .child(div().track_focus(&self.overlay))
    }
}

#[gpui::test]
fn focus_events_and_restore(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|window, cx| {
        let editor = cx.focus_handle();
        let overlay = cx.focus_handle();
        let subs = vec![
            cx.on_focus(&editor, window, |this: &mut FocusProbe, _, _| {
                this.events.push("editor focus")
            }),
            cx.on_blur(&editor, window, |this: &mut FocusProbe, _, _| {
                this.events.push("editor blur")
            }),
        ];
        FocusProbe {
            editor,
            overlay,
            events: Vec::new(),
            _subs: subs,
        }
    });
    // Focus events are computed when a frame is drawn, and only for an active
    // window; test windows start inactive.
    cx.update(|window, _| window.activate_window());
    view.update_in(cx, |view, window, _| window.focus(&view.editor));
    cx.run_until_parked();
    // Opening an overlay: remember the previously focused handle.
    let previous = view.update_in(cx, |view, window, cx| {
        let previous = window.focused(cx);
        window.focus(&view.overlay);
        previous
    });
    cx.run_until_parked();
    // Closing it: restore.
    view.update_in(cx, |view, window, _| {
        assert!(view.overlay.is_focused(window));
        window.focus(&previous.unwrap());
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.events,
            vec!["editor focus", "editor blur", "editor focus"]
        );
    });
}

// ---------------------------------------------------------------------------
// Clipboard with metadata, theme global.
// ---------------------------------------------------------------------------

#[gpui::test]
fn clipboard_metadata_round_trip(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(
            "plain text".into(),
            vec!["bold", "0..5"],
        ))
    });
    let item = cx.update(|cx| cx.read_from_clipboard()).unwrap();
    assert_eq!(item.text().as_deref(), Some("plain text"));
    let gpui::ClipboardEntry::String(entry) = &item.entries()[0] else {
        panic!("expected a string entry")
    };
    assert_eq!(
        entry.metadata_json::<Vec<String>>(),
        Some(vec!["bold".to_string(), "0..5".to_string()])
    );
}

#[gpui::test]
fn theme_global_and_mode_switch(cx: &mut TestAppContext) {
    cx.update(|cx| theme::init(ThemeMode::System, cx));
    // The test platform reports a light appearance.
    cx.update(|cx| {
        assert_eq!(cx.theme().appearance(), Appearance::Light);
        assert_eq!(cx.theme().name.as_ref(), "Paper");
        // No real fonts in tests: the stack falls back to its first entry.
        assert_eq!(cx.theme().fonts.document.as_ref(), "Charter");
    });
    let changes = Rc::new(RefCell::new(0));
    let counter = changes.clone();
    let _sub =
        cx.update(|cx| cx.observe_global::<GlobalTheme>(move |_| *counter.borrow_mut() += 1));
    cx.update(|cx| theme::set_theme_mode(ThemeMode::Dark, cx));
    cx.update(|cx| {
        assert!(cx.theme().is_dark());
        assert_eq!(theme::theme_mode(cx), ThemeMode::Dark);
        // System appearance changes are ignored outside System mode.
        theme::sync_system_appearance(gpui::WindowAppearance::Light, cx);
        assert!(cx.theme().is_dark());
        theme::set_theme_mode(ThemeMode::System, cx);
        theme::sync_system_appearance(gpui::WindowAppearance::VibrantDark, cx);
        assert_eq!(cx.theme().name.as_ref(), "Night");
    });
    // Global observers are notified once per effect flush: the three changes
    // in the second `cx.update` coalesce into one notification.
    assert_eq!(*changes.borrow(), 2);
}

struct LineHeightProbe(Rc<RefCell<Option<Pixels>>>);

impl Render for LineHeightProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let out = self.0.clone();
        div()
            .text_size(px(20.))
            .line_height(relative(1.5))
            .child(gpui::canvas(
                move |_, window, _| *out.borrow_mut() = Some(window.line_height()),
                |_, _, _, _| {},
            ))
    }
}

#[gpui::test]
fn line_height_is_inherited_text_style(cx: &mut TestAppContext) {
    let seen = Rc::new(RefCell::new(None));
    let probe = seen.clone();
    let (_view, cx) = cx.add_window_view(move |_, _| LineHeightProbe(probe));
    cx.run_until_parked();
    // window.line_height() = the inherited text style's line height resolved
    // against its font size (relative(1.5) of 20px).
    assert_eq!(*seen.borrow(), Some(px(30.)));
}

/// A window with an explicit size, for layout-dependent tests.
#[gpui::test]
fn window_resize_in_tests(cx: &mut TestAppContext) {
    let (_view, cx) = cx.add_window_view(|_, cx| TextBox::new(cx));
    cx.simulate_resize(size(px(800.), px(600.)));
    cx.update(|window, _| assert_eq!(window.viewport_size(), size(px(800.), px(600.))));
}
