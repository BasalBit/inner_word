//! Compile-checked snippets for `docs/gpui-notes.md` (gpui 0.2.2).
//!
//! Every snippet in the notes is a trimmed copy of code in this file or in
//! `gpui_harness.rs`. Most functions also run under `#[gpui::test]`, which
//! draws the elements in a headless window (no Metal, fake text metrics).

#![allow(dead_code)]

use std::{borrow::Cow, cell::Cell, path::PathBuf, rc::Rc, time::Duration};

use gpui::{
    Action, Animation, AnimationExt, AnyElement, App, AssetSource, Bounds, BoxShadow,
    ClipboardItem, ContentMask, Context, Corner, CursorStyle, DispatchPhase, Element, ElementId,
    Entity, EventEmitter, FocusHandle, Font, FontFeatures, FontWeight, Global, GlobalElementId,
    Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, KeyBinding, LayoutId,
    ListAlignment, ListState, Menu, MenuItem, MouseDownEvent, MouseMoveEvent, OsAction,
    PathPromptOptions, Pixels, PromptLevel, Render, RenderOnce, ScrollHandle, ScrollStrategy,
    ScrollWheelEvent, SharedString, StrikethroughStyle, Style, Subscription, Task, TestAppContext,
    TextRun, TitlebarOptions, UnderlineStyle, UniformListScrollHandle, Window, WindowBounds,
    WindowOptions, actions, anchored, bounce, deferred, div, ease_in_out, fill, font, hsla, list,
    point, prelude::*, px, quad, relative, rgb, size, svg, uniform_list,
};

// ---------------------------------------------------------------------------
// Bootstrap
// ---------------------------------------------------------------------------

/// Window options for a macOS app window with a transparent titlebar.
fn main_window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1200.), px(800.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("The Writer".into()),
            appears_transparent: true, // also hides the title text
            traffic_light_position: Some(point(px(14.), px(13.))),
        }),
        window_min_size: Some(size(px(640.), px(480.))),
        ..Default::default()
    }
}

struct Shell {
    focus: FocusHandle,
    saved_bounds: Option<WindowBounds>,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            // Window bounds persistence: read on resize/move.
            cx.observe_window_bounds(window, |this, window, _| {
                this.saved_bounds = Some(window.window_bounds());
            }),
            // Flush before quit: do the work synchronously in the closure body;
            // the returned future only gets 100ms (SHUTDOWN_TIMEOUT).
            cx.on_app_quit(|this, _cx| {
                let _bounds = this.saved_bounds; // write to disk here
                async {}
            }),
            cx.observe_window_appearance(window, |_, window, cx| {
                writer_ui::theme::sync_system_appearance(window.appearance(), cx);
            }),
        ];
        // Closing the window: return false to keep it open (e.g. to flush first).
        window.on_window_should_close(cx, |_window, _cx| true);
        Self {
            focus: cx.focus_handle(),
            saved_bounds: None,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus).size_full()
    }
}

#[gpui::test]
fn bootstrap_window_and_entities(cx: &mut TestAppContext) {
    cx.update(|cx| writer_ui::theme::init(writer_ui::ThemeMode::System, cx));
    let window = cx.update(|cx| {
        cx.open_window(main_window_options(cx), |window, cx| {
            cx.new(|cx| Shell::new(window, cx))
        })
        .unwrap()
    });
    // WindowHandle<V>::update gives (&mut V, &mut Window, &mut Context<V>).
    window
        .update(cx, |shell, window, cx| {
            window.focus(&shell.focus);
            cx.notify();
        })
        .unwrap();
    let shell: Entity<Shell> = window.root(cx).unwrap();
    let weak = shell.downgrade();
    assert!(weak.upgrade().is_some());
    cx.update(|cx| {
        assert_eq!(cx.windows().len(), 1);
        // WeakEntity::update returns gpui::Result (entity may be gone).
        weak.update(cx, |shell, _| shell.saved_bounds.take())
            .unwrap();
    });
}

// ---------------------------------------------------------------------------
// Rendering: RenderOnce components, styling, conditional children
// ---------------------------------------------------------------------------

#[derive(IntoElement)]
struct KeyHint {
    keys: SharedString,
    color: Hsla,
}

impl RenderOnce for KeyHint {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .px_1()
            .rounded(px(4.))
            .border_1()
            .border_color(self.color)
            .text_xs()
            .child(self.keys)
    }
}

fn styled_row(selected: bool, hint: Option<&str>) -> impl IntoElement {
    let ink: Hsla = rgb(0x1f1c18).into();
    div()
        .id("row") // makes the div stateful: on_click, hover state, tooltip, scroll
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .h(px(28.))
        .w_full()
        .px_3()
        .rounded_md()
        .text_sm()
        .text_color(ink)
        .font_family("Charter")
        .font_weight(FontWeight::MEDIUM)
        .line_height(px(18.))
        .overflow_hidden()
        .text_ellipsis()
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|style| style.bg(hsla(0., 0., 0., 0.05)))
        .when(selected, |row| row.bg(rgb(0xF4E5DF)))
        .when_some(hint, |row, hint| {
            row.child(KeyHint {
                keys: hint.to_string().into(),
                color: ink,
            })
        })
        .children(["left"].into_iter().map(|label| div().child(label)))
        .on_click(|event, _window, _cx| {
            let _ = event.click_count();
        })
        .tooltip(|_window, cx| cx.new(|_| TooltipView("Open")).into())
}

struct TooltipView(&'static str);

impl Render for TooltipView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().p_1().child(self.0)
    }
}

fn panel_with_shadow() -> impl IntoElement {
    div()
        .absolute()
        .top(px(40.))
        .left(px(0.))
        .w(px(480.))
        .max_h(px(400.))
        .bg(gpui::white())
        .border_1()
        .border_color(rgb(0xE4DFD6))
        .rounded_lg()
        .shadow(vec![BoxShadow {
            color: hsla(0., 0., 0., 0.12),
            offset: point(px(0.), px(8.)),
            blur_radius: px(24.),
            spread_radius: px(-4.),
        }])
        .shadow_lg()
}

// ---------------------------------------------------------------------------
// Long lists, scrolling, overlays, animation
// ---------------------------------------------------------------------------

struct Lists {
    uniform_scroll: UniformListScrollHandle,
    list_state: ListState,
    scroll: ScrollHandle,
    menu_at: Option<gpui::Point<Pixels>>,
}

impl Lists {
    fn new() -> Self {
        Self {
            uniform_scroll: UniformListScrollHandle::new(),
            // Variable-height rows; overdraw = extra pixels rendered off screen.
            list_state: ListState::new(1_000, ListAlignment::Top, px(200.)),
            scroll: ScrollHandle::new(),
            menu_at: Some(point(px(120.), px(80.))),
        }
    }

    fn reveal(&mut self, ix: usize) {
        self.uniform_scroll
            .scroll_to_item(ix, ScrollStrategy::Center);
        self.list_state.scroll_to_reveal_item(ix);
        self.scroll.scroll_to_item(ix); // child index of the tracked div
    }
}

impl Render for Lists {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .relative()
            .flex()
            .child(
                // Same-height rows: only the visible range is rendered.
                uniform_list("documents", 10_000, |range, _window, _cx| {
                    range
                        .map(|ix| div().h(px(24.)).child(format!("Row {ix}")))
                        .collect()
                })
                .track_scroll(self.uniform_scroll.clone())
                .h_full()
                .flex_1(),
            )
            .child(
                list(self.list_state.clone(), |ix, _window, _cx| {
                    div()
                        .h(px(20. + (ix % 3) as f32 * 10.))
                        .child(format!("Item {ix}"))
                        .into_any_element()
                })
                .flex_1(),
            )
            .child(
                div()
                    .id("scroller") // overflow scrolling needs an id
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .h(px(200.))
                    .children((0..50).map(|ix| div().h(px(20.)).child(format!("{ix}"))))
                    .on_scroll_wheel(cx.listener(|_, _event: &ScrollWheelEvent, _, _| {})),
            )
            .when_some(self.menu_at, |root, position| {
                // A floating menu at an arbitrary window point (e.g. the caret):
                // deferred = painted after (on top of) everything else;
                // anchored = positioned in window coordinates, flipped to fit.
                root.child(
                    deferred(
                        anchored()
                            .position(position)
                            .anchor(Corner::TopLeft)
                            .snap_to_window_with_margin(px(8.))
                            .child(
                                div()
                                    .id("menu")
                                    .occlude() // block mouse events to what is below
                                    .w(px(240.))
                                    .bg(gpui::white())
                                    .child("Menu"),
                            ),
                    )
                    .with_priority(1),
                )
            })
            .child(
                div().size(px(8.)).bg(gpui::red()).with_animation(
                    "caret-pulse",
                    Animation::new(Duration::from_millis(1000))
                        .repeat()
                        .with_easing(bounce(ease_in_out)),
                    |el, delta| el.opacity(delta),
                ),
            )
    }
}

#[gpui::test]
fn lists_overlays_and_animation_render(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, _| Lists::new());
    view.update(cx, |lists, cx| {
        lists.reveal(500);
        cx.notify();
    });
    cx.run_until_parked();
    view.read_with(cx, |lists, _| {
        // uniform_list scrolled: offset is negative y.
        assert!(lists.uniform_scroll.0.borrow().base_handle.offset().y < px(0.));
    });
}

// ---------------------------------------------------------------------------
// Custom element: layout to content height, hitbox, mouse, clipping, layers
// ---------------------------------------------------------------------------

/// Lays out to a height computed from its content (here: `rows` lines).
struct Blocks {
    rows: usize,
    line_height: Pixels,
    clicked: Rc<Cell<Option<gpui::Point<Pixels>>>>,
}

impl IntoElement for Blocks {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Blocks {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some("blocks".into())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        // Fixed height known up front:
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (self.line_height * self.rows as f32).into();
        let _ = window.request_layout(style.clone(), [], cx);
        // Or measured from the available width (wrap text, then report height):
        let line_height = self.line_height;
        let rows = self.rows;
        let mut measured = Style::default();
        measured.size.width = relative(1.).into();
        let id = window.request_measured_layout(measured, move |known, available, _window, _cx| {
            let width = known.width.unwrap_or(match available.width {
                gpui::AvailableSpace::Definite(w) => w,
                _ => px(700.),
            });
            size(width, line_height * rows as f32)
        });
        (id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _cx: &mut App,
    ) -> Hitbox {
        // Hitboxes are inserted in prepaint and queried in paint/event handlers.
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        _cx: &mut App,
    ) {
        window.set_cursor_style(CursorStyle::IBeam, hitbox);
        let viewport = window.viewport_size(); // window content size
        let _visible = bounds.intersect(&window.content_mask().bounds);
        let _ = viewport;

        // Paint only the part inside `clip`.
        let clip = Bounds::new(bounds.origin, size(bounds.size.width, self.line_height));
        window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
            window.paint_quad(fill(bounds, rgb(0xFFF2B3)));
        });
        // Later paints are on top of earlier ones; paint_layer batches a
        // non-overlapping group (performance only, not z-order).
        window.paint_layer(bounds, |window| {
            window.paint_quad(quad(
                Bounds::new(bounds.origin, size(px(2.), self.line_height)),
                px(0.),
                rgb(0xB0503A),
                px(0.),
                gpui::transparent_black(),
                Default::default(),
            ));
        });

        let hitbox = hitbox.clone();
        let clicked = self.clicked.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, _cx| {
            if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                clicked.set(Some(event.position - hitbox.bounds.origin));
            }
        });
        window.on_mouse_event(move |_event: &MouseMoveEvent, _phase, _window, _cx| {
            // drag selection: track while `event.pressed_button == Some(Left)`
        });
    }
}

struct CustomHost {
    clicked: Rc<Cell<Option<gpui::Point<Pixels>>>>,
}

impl Render for CustomHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(
                div()
                    .debug_selector(|| "blocks".into())
                    .w(px(300.))
                    .child(Blocks {
                        rows: 3,
                        line_height: px(20.),
                        clicked: self.clicked.clone(),
                    }),
            )
    }
}

#[gpui::test]
fn custom_element_layout_and_hitbox(cx: &mut TestAppContext) {
    let clicked = Rc::new(Cell::new(None));
    let shared = clicked.clone();
    let (_view, cx) = cx.add_window_view(move |_, _| CustomHost { clicked: shared });
    let bounds = cx.debug_bounds("blocks").unwrap();
    assert_eq!(bounds.size, size(px(300.), px(60.)));
    cx.simulate_click(bounds.origin + point(px(10.), px(25.)), Default::default());
    assert_eq!(clicked.get(), Some(point(px(10.), px(25.))));
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

fn text_runs(document_font: Font) -> Vec<TextRun> {
    let ink: Hsla = rgb(0x1f1c18).into();
    let bold = Font {
        weight: FontWeight::BOLD,
        ..document_font.clone()
    };
    let mono = Font {
        features: FontFeatures::disable_ligatures(),
        ..font("Menlo")
    };
    vec![
        TextRun {
            len: "Plain ".len(),
            font: document_font.clone(),
            color: ink,
            background_color: None,
            underline: None,
            strikethrough: None,
        },
        TextRun {
            len: "bold ".len(),
            font: bold,
            color: ink,
            background_color: Some(rgb(0xF6E6A0).into()), // highlight
            underline: Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(ink),
                wavy: false,
            }),
            strikethrough: Some(StrikethroughStyle {
                thickness: px(1.),
                color: Some(ink),
            }),
        },
        TextRun {
            len: "code".len(),
            font: mono,
            color: ink,
            background_color: None,
            underline: None,
            strikethrough: None,
        },
    ]
}

#[gpui::test]
fn text_paint_in_element(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    cx.draw(point(px(0.), px(0.)), size(px(400.), px(200.)), |_, _| {
        gpui::canvas(
            |_bounds, _window, _cx| (),
            |bounds, (), window, cx| {
                let text = "Plain bold code";
                let runs = text_runs(font("Charter"));
                // Prefer shape_text for mixed fonts (see notes: shape_line
                // merges runs whose decorations are equal).
                let lines = window
                    .text_system()
                    .shape_text(text.into(), px(17.), &runs, Some(bounds.size.width), None)
                    .unwrap();
                let line_height = px(17. * 1.6);
                let mut origin = bounds.origin;
                for line in &lines {
                    // Backgrounds are not painted by paint(): call paint_background first.
                    line.paint_background(
                        origin,
                        line_height,
                        gpui::TextAlign::Left,
                        None,
                        window,
                        cx,
                    )
                    .unwrap();
                    line.paint(origin, line_height, gpui::TextAlign::Left, None, window, cx)
                        .unwrap();
                    origin.y += line.size(line_height).height;
                }
            },
        )
        .size_full()
    });
}

// ---------------------------------------------------------------------------
// Actions with data, programmatic dispatch
// ---------------------------------------------------------------------------

actions!(snippets, [Save, OpenPalette]);

/// An action carrying data. `no_json` skips the serde/schemars requirement
/// (such actions cannot be built by name from JSON).
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = snippets, no_json)]
struct SetHeading {
    level: u8,
}

struct Dispatcher {
    focus: FocusHandle,
    last: Option<u8>,
    saves: usize,
}

impl Render for Dispatcher {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Editor")
            .track_focus(&self.focus)
            .on_action(
                cx.listener(|this, action: &SetHeading, _, _| this.last = Some(action.level)),
            )
            .on_action(cx.listener(|this, _: &Save, _, _| this.saves += 1))
    }
}

#[gpui::test]
fn actions_with_data_and_dispatch(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.bind_keys([
            KeyBinding::new("cmd-alt-1", SetHeading { level: 1 }, Some("Editor")),
            KeyBinding::new("cmd-alt-2", SetHeading { level: 2 }, Some("Editor")),
            KeyBinding::new("cmd-s", Save, None),
        ])
    });
    let (view, cx) = cx.add_window_view(|window, cx| {
        let focus = cx.focus_handle();
        window.focus(&focus);
        Dispatcher {
            focus,
            last: None,
            saves: 0,
        }
    });
    cx.simulate_keystrokes("cmd-alt-2");
    view.read_with(cx, |v, _| assert_eq!(v.last, Some(2)));

    // From code: dispatch to the focused element (deferred to end of effects).
    view.update_in(cx, |_, window, cx| {
        window.dispatch_action(Box::new(SetHeading { level: 1 }), cx)
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert_eq!(v.last, Some(1)));

    // Dispatch on a specific focus handle's node (immediate). Not from inside
    // `view.update(..)` / `cx.defer_in(..)` of the same view: the handler
    // updates the view again and panics with "cannot update Dispatcher while
    // it is already being updated". `window.defer` runs with no view borrowed.
    view.update_in(cx, |v, window, cx| {
        let focus = v.focus.clone();
        window.defer(cx, move |window, cx| {
            focus.dispatch_action(&Save, window, cx)
        });
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert_eq!(v.saves, 1));

    // Which bindings exist for an action value (data-sensitive).
    cx.update(|_, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let keys: Vec<String> = keymap
            .bindings_for_action(&SetHeading { level: 2 })
            .map(|b| {
                b.keystrokes()
                    .iter()
                    .map(|k| k.unparse())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        assert_eq!(keys, vec!["alt-cmd-2"]);
        assert!(cx.all_action_names().contains(&"snippets::Save"));
        // Build by name (only for actions that support JSON / unit actions).
        assert!(cx.build_action("snippets::Save", None).is_ok());
        assert!(cx.build_action("snippets::SetHeading", None).is_err());
    });
}

// ---------------------------------------------------------------------------
// State: observe, subscribe, events, globals
// ---------------------------------------------------------------------------

struct Document {
    words: usize,
}

enum DocumentEvent {
    Edited,
}

impl EventEmitter<DocumentEvent> for Document {}

struct WordCount {
    seen_edits: usize,
    seen_notifies: usize,
    _subs: Vec<Subscription>,
}

#[derive(Default)]
struct Settings {
    vim_mode: bool,
}

impl Global for Settings {}

#[gpui::test]
fn observe_subscribe_global(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_global(Settings::default()));
    let doc = cx.new(|_| Document { words: 0 });
    let counter = cx.new(|cx| WordCount {
        seen_edits: 0,
        seen_notifies: 0,
        _subs: vec![
            cx.subscribe(
                &doc,
                |this: &mut WordCount, _doc, event: &DocumentEvent, _cx| match event {
                    DocumentEvent::Edited => this.seen_edits += 1,
                },
            ),
            cx.observe(&doc, |this, _doc, _cx| this.seen_notifies += 1),
            cx.observe_global::<Settings>(|_, cx| {
                let _vim = cx.global::<Settings>().vim_mode;
            }),
        ],
    });
    doc.update(cx, |doc, cx| {
        doc.words += 1;
        cx.emit(DocumentEvent::Edited);
        cx.notify();
    });
    cx.update(|cx| cx.update_global::<Settings, _>(|settings, _| settings.vim_mode = true));
    counter.read_with(cx, |c, _| {
        assert_eq!(c.seen_edits, 1);
        assert_eq!(c.seen_notifies, 1);
    });
}

/// Background work: `cx.background_spawn` (Send future, thread pool), then
/// back on the main thread through the WeakEntity.
fn load_in_background(cx: &mut Context<Document>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        let words = cx
            .background_spawn(async move { "some long text".split_whitespace().count() })
            .await;
        this.update(cx, |doc, cx| {
            doc.words = words;
            cx.notify();
        })
        .ok();
    })
}

/// Caret blink: a looping task held by the view (dropping it stops it).
fn blink(cx: &mut Context<Document>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        }
    })
}

#[gpui::test]
fn background_task_round_trip(cx: &mut TestAppContext) {
    let doc = cx.new(|_| Document { words: 0 });
    let task = doc.update(cx, |_, cx| load_in_background(cx));
    cx.run_until_parked();
    drop(task);
    doc.read_with(cx, |doc, _| assert_eq!(doc.words, 3));
}

// ---------------------------------------------------------------------------
// Platform: prompts, panels, clipboard, URLs, menus, assets, fonts
// ---------------------------------------------------------------------------

#[gpui::test]
fn prompts_and_panels(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let answer = cx.update(|window, cx| {
        window.prompt(
            PromptLevel::Warning,
            "Delete “Chapter 3”?",
            Some("It moves to the trash."),
            &["Delete", "Cancel"],
            cx,
        )
    });
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Delete");
    assert_eq!(cx.executor().block(answer).unwrap(), 0);

    let path =
        cx.update(|_, cx| cx.prompt_for_new_path(&PathBuf::from("/tmp"), Some("Chapter 3.md")));
    cx.simulate_new_path_selection(|_dir| Some(PathBuf::from("/tmp/out.md")));
    assert_eq!(
        cx.executor().block(path).unwrap().unwrap(),
        Some(PathBuf::from("/tmp/out.md"))
    );

    // prompt_for_paths is `unimplemented!()` on the test platform: do not call
    // it under #[gpui::test]; see `open_panel` below for the real-app call.

    cx.update(|_, cx| cx.open_url("https://example.com"));
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com"));

    cx.update(|_, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(
            "text".into(),
            r#"{"spans":[]}"#.into(),
        ));
        let item = cx.read_from_clipboard().unwrap();
        assert_eq!(item.metadata().map(String::as_str), Some(r#"{"spans":[]}"#));
    });
}

/// Open panel (real app only; panics on the test platform).
fn open_panel(cx: &mut App) -> Task<Option<Vec<PathBuf>>> {
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: true,
        prompt: Some("Import".into()),
    });
    cx.spawn(async move |_cx| paths.await.ok()?.ok()?)
}

actions!(
    menu_snippets,
    [Undo, Redo, Cut, Copy, Paste, SelectAll, Quit]
);

fn app_menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "The Writer".into(),
            items: vec![MenuItem::action("Quit The Writer", Quit)],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Undo", Undo, OsAction::Undo),
                MenuItem::os_action("Redo", Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", Cut, OsAction::Cut),
                MenuItem::os_action("Copy", Copy, OsAction::Copy),
                MenuItem::os_action("Paste", Paste, OsAction::Paste),
                MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
            ],
        },
    ]
}

#[gpui::test]
fn menus_compile_and_set(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_menus(app_menus()));
}

/// Assets: embed files and hand them to `Application::new().with_assets(..)`.
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(match path {
            "icons/check.svg" => Some(Cow::Borrowed(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M3 8l3 3 7-7" stroke="black" fill="none"/></svg>"#,
            )),
            _ => None,
        })
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(vec!["icons/check.svg".into()])
    }
}

fn icon() -> impl IntoElement {
    // svg() is tinted with text_color; size it explicitly.
    svg()
        .path("icons/check.svg")
        .size_4()
        .text_color(rgb(0xB0503A))
}

fn register_fonts(cx: &mut App, bytes: &'static [u8]) -> gpui::Result<()> {
    cx.text_system().add_fonts(vec![Cow::Borrowed(bytes)])
}

fn everything_else(_: AnyElement) {}
