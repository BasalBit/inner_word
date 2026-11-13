# GPUI notes (gpui 0.2.2 from crates.io)

Verified reference for this repo's exact gpui version. Source of truth:
`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gpui-0.2.2/` (`$G` below),
examples in `$G/examples/`. Do not write GPUI code from memory: other versions differ
(older `cx.spawn(|this, mut cx| ..)`, `ViewContext`, `WindowContext` do **not** exist here).

Markers: **(tested)** = asserted in `crates/writer-ui/tests/gpui_harness.rs` or
`gpui_snippets.rs`; **(probe)** = observed with the real macOS text system in a probe
example that has since been removed (the numbers stand); **(app)** = seen in the running app;
**(source)** = read in the gpui source, not exercised; **UNVERIFIED** = not checked.

Contents: [Setup](#setup) · [Bootstrap](#bootstrap) · [Rendering](#rendering) ·
[Custom elements](#custom-elements) · [Text system](#text-system) · [Text input](#text-input) ·
[Actions and keys](#actions-and-keys) · [Focus](#focus) · [Mouse](#mouse) ·
[State and async](#state-and-async) · [Platform](#platform) · [Testing](#testing) ·
[Visual QA](#visual-qa) · [Theme API](#theme-api-writer-ui)

---

## Setup

```toml
# root Cargo.toml (already there)
[workspace.dependencies]
gpui = { version = "0.2.2", features = ["runtime_shaders"] }

# a GPUI crate's Cargo.toml
[dependencies]
gpui.workspace = true
[dev-dependencies]
gpui = { workspace = true, features = ["test-support"] }   # #[gpui::test], TestAppContext
```

- **`runtime_shaders` is required on this machine.** Without it the gpui build script runs
  `xcrun metal` and fails: `cannot execute tool 'metal' due to missing Metal Toolchain`
  (Xcode 27.2 ships without the offline Metal compiler). With the feature, `shaders.metal`
  is compiled at startup by Metal.framework. No other patch needed (tested: build, tests, app).
- `test-support` as a dev-dependency means `cargo test -p <crate>` builds a second copy of
  gpui (with `leak-detection` etc.). Normal builds are unaffected. Expect one extra gpui
  compile the first time.
- The registry also contains community forks (`gpui-ce`, `gpui-component`, `gpui-fast`…).
  Ignore them; only `gpui-0.2.2`, `gpui-macros-0.2.2` and `gpui_*-0.2.2` are ours.
- `gpui::Result` is `anyhow::Result`; `gpui::anyhow` is re-exported.
- `use gpui::prelude::*;` brings `Styled`, `InteractiveElement`, `StatefulInteractiveElement`,
  `ParentElement`, `FluentBuilder`, `IntoElement`, `Render`, `AppContext`, `VisualContext`…

## Bootstrap

Source: `$G/src/app.rs`, `$G/src/app/context.rs`, `$G/src/window.rs`, `$G/src/platform.rs`
(WindowOptions), example `$G/examples/hello_world.rs`, `window.rs`, `on_window_close_quit.rs`.

```rust
Application::new()                       // ::headless() exists too
    .with_assets(Assets)                 // impl AssetSource (svg/img paths)
    .run(|cx: &mut App| { /* on_finish_launching */ });
// Application::on_reopen(|cx| ..) (dock click), on_open_urls(|urls| ..)
```

Types: `App` (global context), `Context<'a, T>` (derefs to `App`, `cx.entity()`,
`cx.weak_entity()`, `cx.notify()`, `cx.emit(ev)`), `Window` (always passed separately),
`Entity<T>` (strong handle), `WeakEntity<T>`, `WindowHandle<V>`, `AnyWindowHandle`,
`AsyncApp`, `AsyncWindowContext`.

```rust
let entity: Entity<T> = cx.new(|cx: &mut Context<T>| T { .. });      // AppContext::new
entity.read(cx) -> &T;  entity.update(cx, |t, cx| ..) -> R;           // via App/Context
entity.read_with(cx, |t, app| ..); entity.update_in(visual_cx, |t, window, cx| ..)
weak.upgrade() -> Option<Entity<T>>; weak.update(cx, |t, cx| ..) -> anyhow::Result<R>

cx.open_window(options, |window: &mut Window, cx: &mut App| -> Entity<V> { cx.new(..) })
    -> anyhow::Result<WindowHandle<V>>;
handle.update(cx, |view: &mut V, window, cx: &mut Context<V>| ..) -> Result<R>;
any_handle.update(cx, |root: AnyView, window, cx: &mut App| ..) -> Result<R>;
handle.root(cx) -> Result<Entity<V>>;   cx.windows(); cx.active_window();
cx.activate(true);                      // bring app to front (ignoring other apps)
window.activate_window(); window.remove_window(); window.minimize_window(); window.zoom_window();
```

WindowOptions (all fields, `$G/src/platform.rs:1086`): `window_bounds: Option<WindowBounds>`,
`titlebar: Option<TitlebarOptions>`, `focus`, `show`, `kind: WindowKind::{Normal,PopUp,Floating}`,
`is_movable`, `is_resizable`, `is_minimizable`, `display_id`, `window_background`, `app_id`,
`window_min_size: Option<Size<Pixels>>`, `window_decorations` (Wayland), `tabbing_identifier`.

```rust
WindowOptions {
    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1200.), px(800.)), cx))),
    titlebar: Some(TitlebarOptions {
        title: Some("The Writer".into()),     // still used by Mission Control / Window menu
        appears_transparent: true,            // full-size content view AND hides the title text
        traffic_light_position: Some(point(px(14.), px(13.))),
    }),
    window_min_size: Some(size(px(640.), px(480.))),   // NSWindow setContentMinSize
    ..Default::default()
}
```

- `appears_transparent: true` → content draws under the titlebar; reserve ~38px at the
  top and ~78px at the left for the traffic lights (`writer_ui::theme::TITLEBAR_HEIGHT`,
  `TRAFFIC_LIGHT_RESERVED_WIDTH`). `titlebar: None` behaves like transparent (source).
  Window dragging over the transparent titlebar band: native AppKit behaviour, UNVERIFIED
  here; `window.titlebar_double_click()` performs the system zoom/minimize action.
- **Bounds persistence:** read `window.window_bounds() -> WindowBounds`
  (`Windowed/Maximized/Fullscreen(restore_bounds)`) in `cx.observe_window_bounds(window, |this, window, cx| ..)`
  (fires on move and resize); pass it back as `window_bounds` on next launch. `Bounds<Pixels>`
  is in screen points, origin top-left of the main display.
- **This machine has a tiling window manager**: new windows are placed at the right half
  (observed 1489×1672 at x=1509, y=10, 2× scale) regardless of the requested size, and
  `window.resize(..)` is overridden (app).
- Quit and close hooks:
  - `cx.on_app_quit(|cx| async {})` / `Context::on_app_quit(|this, cx| async {})` →
    `Subscription`. **Do the flushing synchronously in the closure body.** The returned
    futures are awaited with `SHUTDOWN_TIMEOUT` = 100ms, after windows are dropped (source,
    `App::shutdown`). Keep the subscription alive (store it or `.detach()`).
  - `window.on_window_should_close(cx, |window, cx| -> bool)`: return `false` to veto.
  - `cx.on_window_closed(|cx| ..)`: after a window is gone. macOS apps do not quit when
    the last window closes unless you do it here (`writer_app::init` does).
  - There is no default cmd-q: bind `KeyBinding::new("cmd-q", Quit, None)` and
    `cx.on_action(|_: &Quit, cx| cx.quit())`.
  - `Context::on_release(|this, cx| ..)` runs when the entity is dropped.

## Rendering

Source: `$G/src/element.rs`, `$G/src/elements/div.rs`, `$G/src/styled.rs`,
`$G/../gpui-macros-0.2.2/src/styles.rs` (generated helpers), `$G/src/util.rs` (FluentBuilder).

- `Render` (views, stateful entities): `fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement`.
  An `Entity<V: Render>` is itself an element child (`.child(self.editor.clone())`).
- `RenderOnce` (components, consumed): `fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement`
  + `#[derive(IntoElement)]` on the struct (tested).
- `IntoElement`: `&str`, `String`, `SharedString` are text elements; `AnyElement` via
  `.into_any_element()`; `Empty`.

Styling quick reference (tailwind-like; unit suffix scale `_0, _0p5, _1, _2 … _96, _full, _auto`
where `_1` = 0.25rem = 4px at the default 16px rem; or pass a value: `.w(px(300.))`,
`.h(relative(0.5))`):

| What | Methods |
|---|---|
| layout | `flex() flex_row() flex_col() flex_1() flex_none() flex_grow() flex_shrink_0() flex_wrap() gap_2() gap(px) items_center() items_start() justify_between() justify_center() grid() block() hidden()` |
| size | `w h size size_full w_full h_full min_w min_h max_w max_h` (`size_4()`, `w(px(700.))`) |
| spacing | `p px py pt pb pl pr m mx my mt …` (`px_3()`, `p(px(4.))`); `px()` method name collides with the `px()` fn only visually |
| position | `relative() absolute() top left right bottom inset_0()` |
| colours | `bg(color)` `text_color(color)` `border_color(color)` `opacity(f32)`; colours: `rgb(0xRRGGBB)`, `rgba(0xRRGGBBAA)`, `hsla(h,s,l,a)` (all 0..1), `Hsla::alpha(a)`, `gpui::white()/black()/red()/transparent_black()` |
| borders | `border_1() border_b_1() border_t_1() border_2() border_dashed()` |
| radii | `rounded_sm() rounded_md() rounded_lg() rounded_full() rounded(px(6.)) rounded_t_md()` |
| shadow | `shadow(vec![BoxShadow { color, offset: point(..), blur_radius, spread_radius }])`, `shadow_sm/md/lg/xl/2xl()`, `shadow_none()` |
| text | `text_size(px) text_xs/sm/base/lg/xl/2xl/3xl() font_family(name) font_weight(FontWeight::BOLD) italic() line_height(px or relative(1.6)) text_align(..) text_center() truncate() text_ellipsis() whitespace_nowrap() line_clamp(n) underline() line_through() text_decoration_*` |
| overflow | `overflow_hidden() overflow_x_hidden()`; scrolling needs `.id(..)`: `overflow_y_scroll() overflow_scroll() track_scroll(&ScrollHandle)` |
| cursor | `cursor_pointer() cursor_text() cursor_default() cursor(CursorStyle::IBeam)` |
| states | `hover(|s| s.bg(..))` `active(|s| ..)` `focus(|s| ..)` `in_focus(|s| ..)` `group("g")` + `group_hover("g", |s| ..)` |

Conditional children (tested): `.when(cond, |el| el.child(..))`, `.when_some(opt, |el, v| ..)`,
`.when_else(..)`, `.map(|el| ..)`, `.children(iter)`, `.child(Option<impl IntoElement>)` works
(Option implements IntoElement).

Ids and stateful interactivity: `div().id("x")` (any `Into<ElementId>`: &str, usize,
`("row", ix_usize)`, `(SharedString, usize)`, `ElementId::named_usize(..)`) returns `Stateful<Div>`, required for
`on_click`, `tooltip`, `hover` *state*, `active`, `overflow_*_scroll`, `on_drag`. Ids must be unique
among siblings under the nearest ancestor with an id.

Long lists (tested render + scroll):
```rust
uniform_list("docs", count, |range: Range<usize>, window, cx| -> Vec<impl IntoElement> { .. })
    .track_scroll(self.handle.clone())        // UniformListScrollHandle::new()
    .h_full();
self.handle.scroll_to_item(ix, ScrollStrategy::Center);   // Top | Center | Bottom; _strict variants always scroll
// variable height:
let state = ListState::new(count, ListAlignment::Top, px(200.) /*overdraw*/);
list(state.clone(), |ix, window, cx| -> AnyElement { .. }).flex_1();
state.reset(count); state.splice(old_range, new_count); state.scroll_to_reveal_item(ix);
state.scroll_to(ListOffset { item_ix, offset_in_item }); state.logical_scroll_top();
```
- `list` is the only built-in consumer of `window.request_autoscroll(bounds)` (called by a
  child during prepaint). A custom editor inside a plain scroll container must scroll itself.

Scrolling a div: `ScrollHandle::new()`; `.id(..).overflow_y_scroll().track_scroll(&handle)`;
`handle.scroll_to_item(child_ix)`, `scroll_to_top_of_item`, `scroll_to_bottom()`,
`set_offset(point)` (offsets are ≤ 0), `offset()`, `max_offset()`, `bounds()`,
`bounds_for_item(ix)`, `top_item()`. `ScrollWheelEvent.delta.pixel_delta(line_height)`.

Floating overlays at an arbitrary point (tested render):
```rust
deferred(                                     // paint after the rest of the tree (on top)
    anchored()                                // positioned in WINDOW coordinates by default
        .position(caret_point)                // Point<Pixels>, window coords
        .anchor(Corner::TopLeft)              // which corner sits at `position`
        .snap_to_window_with_margin(px(8.))   // default fit mode: SwitchAnchor (flip)
        .child(div().occlude().w(px(240.)).child(..)),   // occlude: block mouse below
).with_priority(1)                            // higher = drawn later
```
`AnchoredPositionMode::Local` positions relative to the parent instead. `window.defer_draw(element, offset, priority)`
is the element-level primitive (prepaint only).

Tooltips: `.id(..).tooltip(|window, cx| cx.new(|_| TooltipView).into())` (returns `AnyView`;
tested compile/render). `hoverable_tooltip` keeps it open while hovered.

**Wrapped text in a flex column inside a flex row** (seen in the Agents panel, tested): text
that wraps, or has several lines, inside a `flex_col()` container that is itself the
`flex_1()` child of a flex row is measured at no width, and the row comes out thousands of
pixels tall. Make the growing child a plain block `div()` and space its children with
margins (`crates/writer-app/src/agents_view.rs`, `render_row`).

**A block in a block that is a `flex_none()` item of a flex column is as wide as its
content** (seen in the assistant's side chat, tested in the assembled app): the composer (a
view whose root is `w_full()`) and a notice (a plain block) sat in plain `div()` wrappers
that were items of the panel's column. They came out at their content's width (the composer
at the 240 px a compact editor falls back to when it is offered none, a notice as wide as
its text) while the item beside them was a plain `div().size_full()`, and at the full width
once that item held a `list`. `w_full()` on the wrapper changed nothing. Making the wrapper
a flex column (`.flex().flex_col()`) stretches what is in it (`writer-assistant-ui`,
`panel/render.rs`; the test `the_composer_and_the_notices_fill_the_side_chat_from_the_first_moment`).
The same one level down: the row inside the composer's box (the text and the Send
control) stayed as wide as its content after the box itself filled the panel, so the
control stood in the middle; the box is a flex column too (`composer.rs`).

Three more from the assistant's crates (each read from gpui's source and seen in a test):
an element's own mouse-down listeners run before the listener gpui adds for focus on
click, so `on_any_mouse_down` with `prevent_default` on an element with `track_focus`
keeps the pointer from focusing it; `ListState::splice` on the item at the top of the
scroll resets the offset within that item; and `debug_bounds` are never cleared between
frames, so `cx.debug_bounds(selector)` still answers for an element that is no longer
drawn (ask the view's state to prove that something is gone).

Animation (tested render): `el.with_animation("id", Animation::new(Duration).repeat().with_easing(ease_in_out), |el, delta: f32| el.opacity(delta))`.
Easings: `linear, quadratic, ease_in_out, ease_out_quint(), bounce(f), pulsating_between(a,b)`.
For a caret blink, prefer a timer task + `cx.notify()` (see State).

Other elements: `canvas(prepaint: FnOnce(Bounds, &mut Window, &mut App) -> T, paint: FnOnce(Bounds, T, &mut Window, &mut App))`,
`svg().path("icons/x.svg").size_4().text_color(c)` (tinted monochrome), `img(path_or_uri)`,
`gpui::StyledText` / `InteractiveText` for simple styled text with highlights (source,
`$G/src/elements/text.rs`).

## Custom elements

Source: `$G/src/element.rs:18`, example `$G/examples/input.rs` (TextElement), `painting.rs`.

```rust
impl IntoElement for MyEl { type Element = Self; fn into_element(self) -> Self { self } }
impl Element for MyEl {
    type RequestLayoutState = ...;   // 'static, passed to prepaint and paint
    type PrepaintState = ...;        // 'static, passed to paint
    fn id(&self) -> Option<ElementId>;      // Some(..) => GlobalElementId for element state
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>>; // None is fine
    fn request_layout(&mut self, id: Option<&GlobalElementId>, inspector_id: Option<&InspectorElementId>,
                      window: &mut Window, cx: &mut App) -> (LayoutId, Self::RequestLayoutState);
    fn prepaint(&mut self, id: Option<&GlobalElementId>, inspector_id: Option<&InspectorElementId>,
                bounds: Bounds<Pixels>, request_layout: &mut Self::RequestLayoutState,
                window: &mut Window, cx: &mut App) -> Self::PrepaintState;
    fn paint(&mut self, id: Option<&GlobalElementId>, inspector_id: Option<&InspectorElementId>,
             bounds: Bounds<Pixels>, request_layout: &mut Self::RequestLayoutState,
             prepaint: &mut Self::PrepaintState, window: &mut Window, cx: &mut App);
}
```

Layout:
- `window.request_layout(style: Style, children: impl IntoIterator<Item = LayoutId>, cx) -> LayoutId`.
  `Style::default()` then `style.size.width = relative(1.).into(); style.size.height = px(h).into();`
- Content height that depends on width (wrapped text):
  `window.request_measured_layout(style, |known: Size<Option<Pixels>>, available: Size<AvailableSpace>, window, cx| -> Size<Pixels>)`.
  `AvailableSpace::{Definite(px), MinContent, MaxContent}` (tested: element measured to 300×60).
  Shape inside the measure closure with `window.text_system()`; cache the result in an
  entity/`Rc<RefCell<..>>` keyed by width so prepaint/paint reuse it.
- Bounds in prepaint/paint are absolute window coordinates. `window.viewport_size()` is the
  window content size; `window.content_mask().bounds` is the visible clip (intersect it with
  your bounds to find what is on screen, e.g. inside a scroll container).
- Children: build `AnyElement`s and call `child.request_layout(window, cx)` /
  `child.prepaint_at(origin, window, cx)` / `child.paint(window, cx)` (see `AnyElement`
  in `$G/src/element.rs`; UNVERIFIED in this repo).

Hit testing and mouse (tested):
```rust
// prepaint
let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal); // or BlockMouse, BlockMouseExceptScroll
// paint
window.set_cursor_style(CursorStyle::IBeam, &hitbox);
window.on_mouse_event({ let hitbox = hitbox.clone(); move |ev: &MouseDownEvent, phase, window, cx| {
    if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) { .. }   // window-wide listener
}});
window.on_mouse_event(|ev: &MouseMoveEvent, phase, window, cx| { if ev.dragging() { .. } });
window.on_key_event(|ev: &KeyDownEvent, phase, window, cx| ..);   // focused path only (source)
```
Listeners registered in paint live for one frame (re-register every paint).

Painting (paint phase only):
- `window.paint_quad(fill(bounds, color))`, `outline(bounds, color, BorderStyle)`,
  `quad(bounds, corner_radii, background, border_widths, border_color, border_style)`;
  `PaintQuad` builder: `.corner_radii(..) .border_widths(..) .border_color(..) .background(..)`.
- `window.paint_shadows(bounds, corner_radii, &[BoxShadow])`, `paint_path(Path, color)`
  (`PathBuilder`), `paint_svg`, `paint_image`, `paint_underline(origin, width, &UnderlineStyle)`,
  `paint_strikethrough`.
- Order: later paint calls are on top. `window.paint_layer(bounds, |window| ..)` only batches
  for performance. Floating things: `deferred`/`defer_draw` (prepaint).
- Clip: `window.with_content_mask(Some(ContentMask { bounds }), |window| ..)` (intersects
  with the current mask). Offset children: `with_element_offset`, `with_absolute_element_offset`.
- Text style inherited from ancestors: `window.text_style()` (`.font()`, `.font_size.to_pixels(window.rem_size())`,
  `.color`), `window.line_height()` (tested: `line_height(relative(1.5))` at 20px → 30px).
  `window.with_text_style(Some(refinement), |w| ..)`.
- Element state across frames: give the element an `id` and use
  `window.with_element_state::<S, R>(global_id, |state: Option<S>, window| (r, new_state))`,
  or `window.use_keyed_state(key, cx, |window, cx| S) -> Entity<S>`. Usually simpler: keep
  state in the owning view entity and pass `Entity<View>` into the element.

Repaint: `cx.notify()` on the entity whose `render` must rerun (marks its window dirty);
`window.refresh()` redraws the whole window; `window.request_animation_frame()` schedules the
next frame (continuous animation); `cx.on_next_frame(window, |this, window, cx| ..)`.

## Text system

Source: `$G/src/text_system.rs`, `$G/src/text_system/{line.rs,line_layout.rs,line_wrapper.rs}`,
example `$G/examples/text_layout.rs`, `text_wrapper.rs`.

```rust
pub struct Font { pub family: SharedString, pub features: FontFeatures, pub fallbacks: Option<FontFallbacks>,
                  pub weight: FontWeight, pub style: FontStyle }
font("Charter").bold().italic();   Font { weight: FontWeight(600.), ..f }  // THIN..BLACK consts
FontStyle::{Normal, Italic, Oblique};  FontFeatures::disable_ligatures(); FontFeatures(Arc<Vec<(String, u32)>>)
FontFallbacks::from_fonts(vec!["Georgia".into()])
pub struct TextRun { pub len: usize /*UTF-8 bytes*/, pub font: Font, pub color: Hsla,
                     pub background_color: Option<Hsla>, pub underline: Option<UnderlineStyle>,
                     pub strikethrough: Option<StrikethroughStyle> }
UnderlineStyle { thickness: Pixels, color: Option<Hsla>, wavy: bool }
StrikethroughStyle { thickness: Pixels, color: Option<Hsla> }

// WindowTextSystem (window.text_system(); derefs to TextSystem)
fn shape_line(&self, text: SharedString, font_size: Pixels, runs: &[TextRun], force_width: Option<Pixels>) -> ShapedLine;
fn shape_text(&self, text: SharedString, font_size: Pixels, runs: &[TextRun], wrap_width: Option<Pixels>,
              line_clamp: Option<usize>) -> Result<SmallVec<[WrappedLine; 1]>>;
fn layout_line(&self, text: &str, font_size: Pixels, runs: &[TextRun], force_width: Option<Pixels>) -> Arc<LineLayout>;
// TextSystem (cx.text_system() or window.text_system())
all_font_names() -> Vec<String>; add_fonts(Vec<Cow<'static, [u8]>>) -> Result<()>;
resolve_font(&Font) -> FontId; get_font_for_id(FontId) -> Option<Font>;
ascent/descent/cap_height/x_height(font_id, size) -> Pixels; em_width/em_advance/ch_width(font_id, size) -> Result<Pixels>;
advance(font_id, size, ch) -> Result<Size<Pixels>>; line_wrapper(font, size) -> LineWrapperHandle
```

ShapedLine (derefs to `Arc<LineLayout>`): `width`, `ascent`, `descent`, `font_size`, `runs: Vec<ShapedRun { font_id, glyphs: Vec<ShapedGlyph { id, position, index /*byte*/, is_emoji }> }>`,
`len`, `text`; `x_for_index(byte) -> Pixels`, `index_for_x(x) -> Option<usize>` (None past
the end; the glyph *containing* x), `closest_index_for_x(x) -> usize` (nearest boundary; use
for clicks and up/down), `font_id_for_index`, `paint(origin, line_height, window, cx) -> Result<()>`,
`paint_background(origin, line_height, window, cx)`.

WrappedLine (derefs to `Arc<WrappedLineLayout>`): `text`, `len()`, `width()`, `size(line_height)`,
`wrap_boundaries() -> &[WrapBoundary { run_ix, glyph_ix }]`, `runs()`, `unwrapped_layout`,
`position_for_index(byte, line_height) -> Option<Point>` (row-local origin),
`index_for_position(point, line_height) -> Result<usize, usize>`,
`closest_index_for_position(..)` (`Ok` inside a row, `Err(row start/end)` when left/right of
it), `paint(origin, line_height, TextAlign, Option<Bounds>, window, cx)`, `paint_background(..)`.

Per visual row ranges (tested, probe): row starts are
`line.runs()[b.run_ix].glyphs[b.glyph_ix].index` for each boundary `b`, plus 0 and `len`.
Trailing spaces stay on the row they end ("aaaa bbbb " | "cccc"). `LineWrapper::wrap_line(&[LineFragment::text(s)], width)`
yields `Boundary { ix /*next row start*/, next_indent }` with the same breaks (tested).

Facts (probe on this Mac, Charter 17px):
- **`shape_line` merges runs with different fonts when their decorations are equal.**
  `WindowTextSystem::layout_line` only starts a new font run when colour/underline/strikethrough
  change, so "regular **bold** *italic* `code`" with one colour shaped as a single Charter run
  (probe: 1 run, FontId(0)). `shape_text` resolves fonts per run correctly (probe: 4 runs).
  **For rich text, use `shape_text(.., wrap_width, None)` per block, even for one line.**
- One shaped line has **one font size** (single `font_size` parameter; `Font` has no size).
  Per-block font size is the only option, as CLAUDE.md already says.
- `shape_line` text must not contain `\n`: `debug_assert!` panics in debug builds.
  `shape_text` splits on `\n` into one `WrappedLine` per line, **without** the `\n` bytes in
  `WrappedLine::text`; your `TextRun` lengths must still cover the `\n` bytes (tested). Map
  offsets yourself: line k starts at the sum of previous line lengths + 1 per newline.
- Wrapping: `wrap_width: Some(w)` wraps at word boundaries; a word longer than the width is
  broken mid-word. Rows: probe output for a 159-byte paragraph at 300px gave 5 rows.
- Metrics: Charter 17px ascent 16.66, descent 4.08 (reported negative by `descent()`, positive
  in `LineLayout::descent`), cap 11.55, x-height 8.26. Line box: paint centres glyphs
  vertically inside `line_height` (`padding_top = (line_height - ascent - descent) / 2`, source line.rs).
- Missing family → silently replaced by the fallback stack (`.ZedMono`, `.ZedSans`, Helvetica, …):
  `resolve_font(font("No Such Family"))` → Helvetica (probe). Check `all_font_names()` first.
- Glyph fallback is automatic (CoreText cascade): emoji and CJK in a Charter run come back as
  extra shaped runs with other font ids (probe; `get_font_for_id` returns None for those).
  Emoji glyphs have `is_emoji = true`; glyph `index` values are byte offsets, so a caret
  inside a multi-byte grapheme maps to the next glyph's x. Combining marks share one glyph:
  `x_for_index` of an index inside a cluster returns the cluster end. Move carets by grapheme.
- Ligatures: none produced by Charter for "office fit" (probe). Disable explicitly in code
  fonts with `FontFeatures::disable_ligatures()`.
- Installed families here (probe, 237 total): Charter yes, **Iowan Old Style no**, Georgia
  yes, **SF Mono no** (only inside Terminal.app), Menlo yes, Monaco yes, Palatino,
  Baskerville, Avenir Next, Helvetica Neue yes, New York no. `.SystemUIFont` = SF Pro (UI).
- Cost and caching: 1000 distinct 60-char lines shaped in ~19ms (debug build), the same 1000
  again in ~1ms. Cache key = text + font size + font runs (font id + length) + wrap width;
  colours/decorations are not part of the key. Layouts survive only if used in the current or
  previous frame (`LineLayoutCache::finish_frame` swaps two frame maps). Shaping outside
  paint (e.g. in an action handler) also goes into the current frame map.
- Painting: `ShapedLine::paint` / `WrappedLine::paint` do **not** paint `background_color`;
  call `paint_background` first (source line.rs; tested call).
- Line height for documents: `font_size * 1.6`; pass the same `line_height` to `paint` and to
  `position_for_index`/`closest_index_for_position`.

## Text input

Source: `$G/src/input.rs`, `$G/src/platform.rs:995` (InputHandler docs), mac key path
`$G/src/platform/mac/window.rs:1660` (`handle_key_event`), example `$G/examples/input.rs`.

```rust
pub trait EntityInputHandler: 'static + Sized {      // all ranges are UTF-16 code units
    fn text_for_range(&mut self, range: Range<usize>, adjusted_range: &mut Option<Range<usize>>,
                      window: &mut Window, cx: &mut Context<Self>) -> Option<String>;
    fn selected_text_range(&mut self, ignore_disabled_input: bool, window: &mut Window,
                           cx: &mut Context<Self>) -> Option<UTF16Selection>;  // { range, reversed }
    fn marked_text_range(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<Range<usize>>;
    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>);
    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str,
                             window: &mut Window, cx: &mut Context<Self>);
    fn replace_and_mark_text_in_range(&mut self, range: Option<Range<usize>>, new_text: &str,
                             new_selected_range: Option<Range<usize>>, window: &mut Window, cx: &mut Context<Self>);
    fn bounds_for_range(&mut self, range_utf16: Range<usize>, element_bounds: Bounds<Pixels>,
                        window: &mut Window, cx: &mut Context<Self>) -> Option<Bounds<Pixels>>;
    fn character_index_for_point(&mut self, point: Point<Pixels>, window: &mut Window,
                                 cx: &mut Context<Self>) -> Option<usize>;
}
// in Element::paint, every frame:
window.handle_input(&focus_handle, ElementInputHandler::new(bounds, view_entity.clone()), cx);
```

- `range: None` in replace calls means "the marked range if any, else the selection".
  `new_selected_range` is relative to the inserted text (see example `input.rs`).
- `handle_input` only registers if `focus_handle.is_focused(window)` at paint time; the last
  registration in the frame wins. No registration → typed text is dropped.
- `bounds_for_range` positions the IME candidate window and the emoji picker; return window
  coordinates (element bounds are passed in). `character_index_for_point` is used for
  force-click lookups (source).
- Key path on macOS (source + tested in the test platform):
  1. Not composing and the key has a `key_char` (printable): GPUI dispatch first:
     interceptors → keymap bindings → `on_key_down`/`capture_key_down` listeners (they see
     printable keys too, tested) → observers. If nothing stopped propagation, the event goes
     to NSTextInputContext → `insertText` → `replace_text_in_range`.
  2. Composing (marked text exists) or a non-printing key without ctrl/fn (arrows, escape,
     backspace…): the IME gets it first; if it does not consume it, GPUI dispatch runs.
  3. A bound printable key (e.g. `"/"` in an "Editor" context) never inserts text.
- Dead keys (option-e then e): AppKit calls `setMarkedText` → `replace_and_mark_text_in_range`
  with "´", then `insertText("é")` replacing the marked range (source; simulated in the
  `ime_marked_text_through_input_handler` test). Render marked text underlined.
- Emoji & symbols picker: `window.show_character_palette()` (bind ctrl-cmd-space); the chosen
  character arrives through `replace_text_in_range` (source; UNVERIFIED interactively).
- `apple_press_and_hold_enabled` (InputHandler default true) controls the accent menu on held keys.
- Typed text must not panic on `Entity` leases: the platform calls the handler outside any
  update, but calling your own handler methods from inside `view.update` is fine only through
  `&mut self` (not via the entity).

## Actions and keys

Source: `$G/src/action.rs`, `$G/src/keymap.rs`, `$G/src/keymap/{binding.rs,context.rs}`,
`$G/src/key_dispatch.rs`, `$G/src/window.rs:3730` (`dispatch_key_event`), `$G/src/platform/keystroke.rs`.

```rust
actions!(editor, [Bold, Italic, /// doc comment becomes cx.action_documentation()
                  Undo]);                         // unit structs, names "editor::Bold"
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = editor, no_json)]            // no_json: no serde/schemars needed
struct SetHeading { level: u8 }                   // without no_json: derive serde::Deserialize + schemars::JsonSchema
KeyBinding::new("cmd-alt-1", SetHeading { level: 1 }, Some("Editor"));   // panics on a bad keystroke/predicate
cx.bind_keys([..]);  cx.clear_key_bindings();  cx.key_bindings() -> Rc<RefCell<Keymap>>
div().key_context("Editor")            // or KeyContext: "Editor mode=full vim"
    .track_focus(&focus)               // key events only reach the focused element's ancestors
    .on_action(cx.listener(|this, a: &Bold, window, cx| ..))
    .capture_action(|a: &Bold, window, cx| ..);    // root-to-focus before bubble
cx.on_action(|a: &Quit, cx: &mut App| ..);          // global fallback (after the element tree)
```

Predicates (`Some("..")`): `Editor`, `Editor && mode == full`, `Editor && !vim`, `Workspace > Editor`
(child), `a || b`. A binding with no context matches everywhere (depth = deepest).

Dispatch rules (tested in `dispatch_precedence_and_chords`):
- Matching bindings are ordered by **context depth** (deeper/focused wins), then **later added
  wins** at equal depth. GPUI dispatches the best binding's action from the focused node:
  capture listeners root→focus, then bubble focus→root. The first listener that handles it
  stops dispatch.
- **A binding without a context has the depth of the whole context stack** (`Keymap::binding_enabled`
  returns `contexts.len()`): it ranks like a binding in the focused element's own context,
  not like an outermost one. So it beats every binding of an *outer* context on the same key,
  whatever the order they were added in. Example that went wrong: the comments crate binds
  `cmd-alt-c` without a context; the find bar bound the same key in `"FindBar"`, but the
  keyboard is in a `TextField` inside the bar, one level deeper, so the window-wide binding
  won. Fix: bind at the focused element's level (`"FindBar > TextField"`, same depth as a
  context-free binding) and add it later. To see what a key will run where the keyboard is:
  `cx.key_bindings().borrow().bindings_for_input(&[keystroke], &window.context_stack())`,
  first entry (used by `crates/writer-app/tests/suite/keymap.rs`).
- **Real keyboards report shift plus a non-letter as the shifted character without the shift
  flag** (`parse_keystroke` in `$G/src/platform/mac/events.rs`): `cmd-shift-7` arrives as
  `cmd-&`, `cmd-shift-.` as `cmd->`, `cmd-shift-'` as `cmd-"` on a US layout. A binding
  spelled `cmd-shift-7` only matches scripted and test keystrokes. Bind both, the spelled-out
  one last so `KeyHint` shows it. Letters, enter, tab, arrows and backspace keep the shift
  flag; option plus a key keeps the plain key name with the alt flag (`cmd-alt-]`).
- Action listeners stop propagation by default. `cx.propagate()` inside a handler means "not
  handled": GPUI then tries **the next matching binding** (tested: cmd-k bound to Inner (Editor)
  and Outer (Workspace) → Inner handler propagates → Outer runs). `cx.stop_propagation()` exists for events.
- Bound keystrokes do not reach `on_key_down` nor the input handler; unbound ones reach
  `on_key_down` (capture then bubble) and then the input handler (tested).
- Multi-keystroke bindings `"ctrl-x ctrl-s"`: the first stroke is pending
  (`window.has_pending_keystrokes()`, `pending_input_keystrokes()`, `cx.observe_pending_input(window, ..)`);
  timeout **1 second**, then the pending strokes are replayed as single strokes (tested).
  **A shorter binding for the prefix added *after* the chord wins immediately** and the chord can
  never fire; add prefix bindings before chords (tested).
- `Keystroke::parse("cmd-shift-p")` modifiers: `ctrl alt shift fn cmd|super|win secondary`.
  **There is no `opt`: write `alt`** (`"cmd-opt-s"` fails to parse; `KeyBinding::new` panics).
  Uppercase letter implies shift (`cmd-B` = `cmd-shift-b`). Punctuation works: `cmd-\\`,
  `cmd-'`, `cmd-shift-.`, `cmd--` (minus), `cmd-[`. Key names: `enter escape backspace delete
  tab space up down left right home end pageup pagedown f1..`.
- Display: `keystroke.to_string()` / `KeybindingKeystroke` Display use mac symbols in the order
  `^` (ctrl, a caret, not ⌃) ⌥ ⌘ ⇧ + key (`cmd-alt-s` → "⌥⌘S", tested; source `display_modifiers`).
  macOS menus use ⌃⌥⇧⌘; a shortcuts overlay that must match the menus should format keys itself
  from `keystroke.modifiers()` / `.key()`.
  `unparse()` gives the parseable form ("alt-cmd-2").
  `window.keystroke_text_for(&action) -> String` = highest-precedence binding display, falls back to the action name.

Registry queries for a shortcuts overlay / palette hints (tested):
```rust
let keymap = cx.key_bindings(); let keymap = keymap.borrow();
for b in keymap.bindings() {          // insertion order; later = higher precedence
    b.keystrokes() /* &[KeybindingKeystroke] */; b.action().name(); b.predicate() /* Option<Rc<..>>, Display */;
}
keymap.bindings_for_action(&action)   // compares action VALUES (data-sensitive), last = preferred
window.bindings_for_action(&action)   // NOT the focused element's: see below
window.highest_precedence_binding_for_action(&action);  window.bindings_for_action_in(&action, &focus_handle)
cx.all_action_names(); cx.action_documentation() /* HashMap<&str, &str> from doc comments */
window.available_actions(cx)          // actions with listeners on the focused path
cx.build_action("editor::Bold", None) // by name; fails for no_json actions with fields
```

`Window::bindings_for_action` and `Window::highest_precedence_binding_for_action` (and so
`Window::keystroke_text_for` and writer-ui's `KeyHint::for_action_focused`) answer from
`dispatch_tree.context_stack`, which outside a draw is the build stack left over from the
last frame: the key contexts of the element that was painted last, not of the focused one,
and often empty (then only bindings without a key context are found). To ask for a place,
use `bindings_for_action_in(&action, &focus_handle)` (writer-ui: `KeyHint::for_action_at`,
`for_action_here`). Held by `a_hint_is_asked_in_the_place_it_applies` in
`crates/writer-ui/tests/widgets/rebinding.rs`. `Window::context_stack()` debug-asserts that
a frame was drawn when nothing in the last frame is focused: not from `render`, and not on
the first frame of a window.

Taking the keymap apart and putting it back (the user's own shortcuts; tested in
`crates/writer-app/tests/suite/keybindings.rs`, and every suite of the app runs on a keymap
that was rebuilt this way, because `writer_app::init` ends with it):
- `KeyBinding` is `Clone`, and it can be read completely from outside: `keystrokes()`
  (`&[KeybindingKeystroke]`, each with `unparse()` and `inner()`), `action()` (`boxed_clone()`
  copies it, fields included), `predicate()` (`Option<Rc<KeyBindingContextPredicate>>`, shared,
  not copied), `meta()`, `action_input()`.
- `KeyBinding::load(keys, action, predicate, false, action_input, &DummyKeyboardMapper)` is what
  `KeyBinding::new` calls. A binding rebuilt with it from those parts **is the binding it
  was**: equal keystrokes (`KeybindingKeystroke: PartialEq`), `action().partial_eq`, equal
  predicate, for all 404 bindings of the app. `load` leaves `meta` at `None`: put it back
  with `set_meta` (no binding of this app has one, nor an action input; nothing loads a
  keymap file). On macOS a `KeybindingKeystroke` is only the `Keystroke` (the display key and
  display modifiers are Windows fields), so nothing about key equivalents is lost with
  `use_key_equivalents = false` and the dummy mapper.
- Text round trips: `Keystroke::parse(&key.unparse()) == key` for every key of the keymap
  (`cmd-&`, `cmd-"`, `cmd-\`, `shift-f10`, `space`), and
  `KeyBindingContextPredicate::parse(&predicate.to_string()) == predicate` for every key
  context (Display puts parentheses around every pair: `((LibraryPanel && mode == tree) &&
  !TextField)`, `FindBar > TextField`). `unparse` writes the modifiers in the order `fn ctrl
  alt cmd shift` (`cmd-alt-s` comes back as `alt-cmd-s`): compare parsed keystrokes, not text.
- `cx.clear_key_bindings()` then `cx.bind_keys(the same list in the same order)` gives a
  keymap that resolves every key as before: precedence is context depth, then order, and
  both are kept. Each of the two calls queues a refresh of every window and bumps
  `Keymap::version()`. `cx.key_bindings()` is an `Rc<RefCell<Keymap>>`: drop the borrow before
  `clear_key_bindings` or `bind_keys` (they `borrow_mut`).
- A `Keymap` can be made without an app (`Keymap::new(bindings)`) to ask it things
  (`bindings_for_action`, `bindings_for_input`) before anything is installed.
- **The menu bar keeps the keys it was built with.** `cx.set_menus` reads the keymap once;
  after a change of the keymap, call it again. And the key beside an item is live: AppKit
  runs an enabled item when its key is pressed and GPUI has no binding for that key where
  the keyboard is, so a key taken away from a command keeps running it from the menu bar
  until the menu bar is built again.
- What does not come along: anything that compared against a key written in the code (an
  interceptor, an `on_key_down`) and anything that kept a `KeyHint` it made earlier. Ask the
  keymap when the key arrives or when the hint is drawn (`writer_ui::keymap`,
  `KeyHint::for_action`).

Raw keys (Vim mode):
- `div().on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| ..))` (`ev.keystroke`,
  `ev.is_held`), `capture_key_down`, `on_key_up`, `on_modifiers_changed`. Runs only when no
  binding handled the key. Call `cx.stop_propagation()` to also stop text input.
- `cx.intercept_keystrokes(|ev: &KeystrokeEvent, window, cx| ..) -> Subscription` runs **before**
  keymap matching; `cx.stop_propagation()` swallows the key entirely (no action, no
  text) (tested). Best hook for a modal Vim layer that must see every key.
- `cx.observe_keystrokes(|ev: &KeystrokeEvent, window, cx| ..)` runs after dispatch, with
  `ev.action: Option<Box<dyn Action>>` = the action that ran; not called if propagation was stopped.
- Alternative for Vim: key contexts (`"Editor && vim_mode == normal"`) with plain bindings
  like `"h"`, `"d d"`.

Programmatic dispatch:
- `window.dispatch_action(Box::new(action), cx)`: to the focused node, **deferred** to the end
  of the current effect cycle (tested).
- `focus_handle.dispatch_action(&action, window, cx)`: immediately, from that handle's node.
- `cx.dispatch_action(&action)`: active window's focused node / global listeners (source).
- `window.dispatch_keystroke(Keystroke, cx) -> bool`: full keystroke path including text input
  (used by the automation hook, app).
- `Window::dispatch_event(PlatformInput, cx)` is `pub` but returns a crate-private type and
  **cannot be called from outside gpui** (compile error); no programmatic mouse events in the app.

## Focus

Source: `$G/src/window.rs:180-420`, `$G/src/app/context.rs` (on_focus…), example `$G/examples/tab_stop.rs`.

```rust
let focus = cx.focus_handle();                 // App/Context
impl Focusable for View { fn focus_handle(&self, cx: &App) -> FocusHandle { self.focus.clone() } }
div().track_focus(&focus)                      // must be rendered for focus to "exist"
window.focus(&focus); focus.focus(window); cx.focus_self(window) /* T: Focusable */; window.blur();
focus.is_focused(window); focus.contains_focused(window, cx) /* focus is in a descendant */;
focus.within_focused(window, cx) /* self is inside the focused element */; window.focused(cx) -> Option<FocusHandle>
cx.on_focus(&focus, window, |this, window, cx| ..)     // became focused
cx.on_focus_in(&focus, window, ..)  cx.on_blur(&focus, window, ..)  cx.on_focus_out(&focus, window, |this, ev: FocusOutEvent, w, cx| ..)
cx.on_focus_lost(window, ..)        // window lost all focus
```
- Focus events fire when a frame is drawn **and the window is active** (tested; test windows
  start inactive: call `window.activate_window()` in tests).
- **Focus can sit on an element that is not drawn.** `window.focus(&handle)` only stores the
  id; nothing checks that the handle is rendered, and GPUI does not move the focus when the
  focused element disappears. While the `FocusHandle` is alive, `window.focused(cx)` is
  `Some`, keys are dispatched at the root node and reach no element (`window.context_stack()`
  is empty); once it is dropped, `focused` is `None`, and no redraw is triggered either way.
  Typical cause: a modal remembers the focused handle, the thing behind it closes on blur (a
  popup, a rename field), and the modal "restores" focus to it. Check with
  `root_handle.contains(&focused, window)` (the last **rendered** frame: a just-focused
  element may be missing from it for a frame or two, e.g. a row of a `uniform_list`).
  `Workspace::keep_focus_on_screen` does this after every frame, from a `cx.defer_in` queued
  in `render` (a deferred effect of a draw runs right after that draw).
- Restore after an overlay (tested): on open `let previous = window.focused(cx);` focus the
  overlay; on close `window.focus(&previous)` (fall back to the editor handle).
- Tab stops: `cx.focus_handle().tab_index(1).tab_stop(true)`, `div().tab_index(i)`, `.tab_group()`;
  bind `tab`/`shift-tab` to actions that call `window.focus_next()` / `window.focus_prev()`
  (source + example; there is no built-in tab binding).
- Style by focus: `.focus(|s| ..)` (this element focused), `.in_focus(|s| ..)` (focus within).

## Mouse

Source: `$G/src/interactive.rs`, `$G/src/elements/div.rs`.

- `div().on_mouse_down(MouseButton::Left, |ev: &MouseDownEvent, window, cx| ..)`,
  `on_mouse_up`, `on_mouse_up_out` (released outside: end drags), `on_mouse_down_out`
  (click outside: close menus), `on_any_mouse_down`, `capture_any_mouse_down`,
  `on_mouse_move(|ev: &MouseMoveEvent, ..|)`, `on_scroll_wheel(|ev: &ScrollWheelEvent, ..|)`,
  `on_click(|ev: &ClickEvent, ..|)` (needs `.id`), `on_hover(|hovered: &bool, ..|)`,
  `on_drag(value, |v, offset, window, cx| Entity<W>)` + `on_drop::<T>(..)`.
- `MouseDownEvent { button, position, modifiers, click_count, first_mouse }`: `click_count`
  2 = double, 3 = triple (from NSEvent). `ClickEvent::click_count()`, `.modifiers()`, `.position()`.
- `MouseMoveEvent { position, pressed_button: Option<MouseButton>, modifiers }`, `.dragging()`.
- Drag selection: on mouse down set `selecting = true` + anchor; on move while
  `ev.dragging()` (or `selecting`) extend; end on `on_mouse_up` **and** `on_mouse_up_out`.
  In a custom element, `window.on_mouse_event` listeners are window-wide, so moves outside
  the hitbox still arrive (example `input.rs`).
- ctrl-click arrives as a right click (`MouseButton::Right`) on macOS (source).
- Cursor: `.cursor(CursorStyle::IBeam)` / `window.set_cursor_style(style, &hitbox)`.
  Styles: Arrow, IBeam, PointingHand, ClosedHand, OpenHand, Crosshair, ResizeLeftRight, …

Wheel, pinch and drawing at a scale (from the zoom of the text, 0.6.0):

- **`ScrollWheelEvent` is AppKit's event, unchanged** (`$G/src/platform/mac/events.rs`):
  `delta` is `scrollingDeltaX/Y`, `ScrollDelta::Pixels` for a trackpad and
  `ScrollDelta::Lines` for a wheel (`delta.pixel_delta(line_height)` turns either into
  pixels), `modifiers` are the keys held. GPUI does nothing with shift: where a wheel that
  only goes up and down should go sideways with shift, the element does it (macOS turns
  the wheel of most mice itself and then `delta.x` is set). Two fingers never move
  straight, so an element that scrolls both ways gets a little of the other direction
  with every event: drop the small part (`writer-editor`, `mouse.rs`, `sideways_intent`).
  `cmd` with the wheel is `event.modifiers.platform` (source + tested with
  `simulate_event`).
- **There is no magnify (pinch) event in gpui 0.2.2**, and none for rotate or smart
  magnify: `PlatformInput` has keys, mouse buttons, moves, scroll and file drops only
  (source). AppKit hands an app every event it is about to dispatch through
  `+[NSEvent addLocalMonitorForEventsMatchingMask:handler:]`; a monitor for
  `NSEventMaskMagnify` (`1 << 30`) that returns the event sees pinches and nothing else
  (`writer-app`, `pinch.rs`: the block is made with the `block` crate, the version gpui
  itself depends on; it only reads `magnification`, `locationInWindow` and the window
  number into a channel, because GPUI must not be entered from inside AppKit's dispatch).
  Installing it works in a scripted run (app); **what a real pinch does was not seen**
  (nothing can pinch but fingers).
- **Text can be painted at another size than it was shaped at**:
  `window.paint_glyph(origin, font_id, glyph_id, font_size, color)` (and `paint_emoji`)
  take any size, so a row shaped at 17 px is drawn at 150 % by painting each glyph of
  `ShapedRun::glyphs` at its `position` times 1.5 with `font_size` times 1.5
  (`writer-editor`, `paint.rs`). Nothing is shaped again and the line breaks cannot move;
  seen in the app at 50 % to 400 % (app). `ShapedLine::paint` itself has no scale.
- **How large a display is**: `window.display(cx)` gives the `PlatformDisplay` (`id()`,
  `bounds()` in logical pixels); `u32::from(display.id())` is the `CGDirectDisplayID`, and
  `CGDisplayScreenSize(id)` (CoreGraphics, a plain C call) gives the glass in millimetres,
  zero when the display does not say. Pixels to the inch over 72 is the scale at which a
  point is a real point (`writer-app`, `zoom.rs`: 127.5 and 177 % on this Mac's built-in
  display at its default scaling, 151 and 210 % at "More Space"). The test platform's
  display has no size.

## State and async

Source: `$G/src/app/context.rs`, `$G/src/app/async_context.rs`, `$G/src/executor.rs`, `$G/src/global.rs`.

```rust
cx.notify();                                                    // re-render observers / window
cx.observe(&entity, |this, entity, cx| ..) -> Subscription      // on notify
impl EventEmitter<MyEvent> for Model {}   cx.emit(MyEvent::Edited);
cx.subscribe(&entity, |this: &mut Me, entity, ev: &MyEvent, cx| ..) -> Subscription  // annotate `this` (type inference fails otherwise)
cx.subscribe_in(&entity, window, |this, entity, ev, window, cx| ..); cx.observe_in(..)
struct Settings { .. } impl Global for Settings {}
cx.set_global(Settings{..}); cx.global::<Settings>(); cx.try_global::<S>(); cx.global_mut::<S>();
cx.update_global::<Settings, _>(|s, cx| ..); cx.observe_global::<Settings>(|this, cx| ..)
cx.default_global::<S>()  // S: Default
```
- Subscriptions unsubscribe on drop: store them (`_subscriptions: Vec<Subscription>`) or `.detach()`.
- Global observers are notified once per effect flush (several `set_global` in one update →
  one callback, tested).

Async (closure signatures of this version use async closures):
```rust
// Context<T>::spawn: AsyncFnOnce(WeakEntity<T>, &mut AsyncApp) -> R
let task: Task<()> = cx.spawn(async move |this, cx| {
    let n = cx.background_spawn(async move { heavy() }).await;     // Send future on the thread pool
    this.update(cx, |this, cx| { this.n = n; cx.notify(); }).ok(); // Result: entity may be gone
});
cx.spawn_in(window, async move |this, cx: &mut AsyncWindowContext| { this.update_in(cx, |this, window, cx| ..) });
// App::spawn: AsyncFnOnce(&mut AsyncApp);  Window::spawn(cx, async move |cx: &mut AsyncWindowContext| ..)
cx.background_executor().timer(Duration::from_millis(500)).await;
async_app.update(|cx: &mut App| ..) -> Result<R>;  async_window_cx.update(|window, cx| ..) -> Result<R>
task.detach(); task.detach_and_log_err(cx);          // dropping a Task CANCELS it
Task::ready(value)
```
- Debounce (tested): store `self.pending: Option<Task<()>>`; each edit replaces it with a task
  that awaits a timer then saves; replacing drops (cancels) the previous one.
- Caret blink: a looping `cx.spawn` that awaits a 500ms timer and calls `this.update(cx, |_, cx| cx.notify())`,
  breaking when the update errors; reset it on every edit/caret move so the caret is solid while typing.
- `cx.defer(|cx| ..)`, `cx.defer_in(window, |this, window, cx| ..)`, `window.defer(cx, |window, cx| ..)`:
  run after the current effect cycle.
- **"cannot update X while it is already being updated"** (and "cannot read …"): an entity is
  leased while its `update`/`render`/listener runs; touching the same entity again through
  its `Entity` handle (read or update) panics. Happens with: calling `self_entity.update(..)`
  inside its own handler, `entity.read(cx)` in its own `render`, dispatching an action to the
  same view from inside `cx.defer_in` of that view (tested). Fix: use `&mut self` and the
  given `cx`; or hop out with `window.defer(cx, ..)` / `cx.defer(..)`; in elements painted by
  the view, `entity.update` is fine (render has returned).

## Platform

Source: `$G/src/app.rs`, `$G/src/platform.rs`, `$G/src/platform/app_menu.rs`,
`$G/src/platform/mac/platform.rs`, examples `set_menus.rs`, `window_positioning.rs`.

- Clipboard (tested on the test platform): `cx.write_to_clipboard(ClipboardItem::new_string(s))`,
  `ClipboardItem::new_string_with_metadata(text, metadata_string)`,
  `new_string_with_json_metadata(text, serde_value)`; `cx.read_from_clipboard() -> Option<ClipboardItem>`;
  `item.text()`, `item.metadata() -> Option<&String>` (only when there is exactly one string entry),
  `item.entries()` → `ClipboardEntry::String(ClipboardString)` / `Image`, `ClipboardString::metadata_json::<T>()`.
  On macOS the metadata is stored under private pasteboard types `zed-metadata` +
  `zed-text-hash`; it is returned only if the plain text is unchanged (hash match), so other
  apps see plain text only (source). No HTML/RTF writing in 0.2.2: rich paste between apps
  needs your own pasteboard code (UNVERIFIED).
- URLs/files: `cx.open_url("https://..")` (tested via `cx.opened_url()`), `cx.reveal_path(&path)`,
  `cx.open_with_system(&path)`, `cx.add_recent_document(&path)`.
- Menus: `cx.set_menus(vec![Menu { name: "Edit".into(), items: vec![MenuItem::action("Bold", Bold),
  MenuItem::separator(), MenuItem::submenu(Menu{..}), MenuItem::os_submenu("Services", SystemMenuType::Services),
  MenuItem::os_action("Copy", Copy, OsAction::Copy)] }])` (tested compile + set).
  `OsAction::{Cut, Copy, Paste, SelectAll, Undo, Redo}` wires the item to the standard
  AppKit selector so system text fields work. The first menu is the app menu. Shortcuts shown
  in menus come from the keymap; menu items are enabled when `cx.is_action_available(&action)`.
- Prompts (tested on the test platform):
  `window.prompt(PromptLevel::{Info,Warning,Critical}, msg, Some(detail), &["Delete", "Cancel"], cx) -> oneshot::Receiver<usize>`
  (index of the clicked answer; "Cancel"/"Ok" strings map to PromptButton kinds). Native
  NSAlert on macOS. `cx.set_prompt_builder(..)` replaces it with a GPUI-rendered prompt.
- Panels: `cx.prompt_for_new_path(&Path /*directory*/, Option<&str> /*suggested name*/) -> oneshot::Receiver<Result<Option<PathBuf>>>`
  (tested); `cx.prompt_for_paths(PathPromptOptions { files, directories, multiple, prompt: Option<SharedString> })
  -> oneshot::Receiver<Result<Option<Vec<PathBuf>>>>` (**`unimplemented!()` on the test
  platform**, compile-checked only). Await the receiver in `cx.spawn`.
- Appearance: `cx.window_appearance()`, `window.appearance()` → `WindowAppearance::{Light, VibrantLight, Dark, VibrantDark}`;
  observe with `window.observe_window_appearance(|window, cx| ..)` or
  `cx.observe_window_appearance(window, |this, window, cx| ..)` (app: system is Dark on this
  Mac and `System` mode resolved to Night). The test platform reports Light.
- Assets and icons: `impl AssetSource for Assets { fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>>; fn list(&self, path: &str) -> Result<Vec<SharedString>> }`,
  `Application::new().with_assets(Assets)`, then `svg().path("icons/check.svg")` /
  `img("images/x.png")`. Embed files with `include_bytes!` (or rust-embed, not a dependency yet).
- Extra fonts: `cx.text_system().add_fonts(vec![Cow::Borrowed(include_bytes!("..").as_slice())])` (compile-checked), then use the family name.
- Displays: `cx.displays()`, `cx.primary_display()`, `window.display(cx)`, `window.scale_factor()` (2 here).

## Printing

What GPUI has and has not for laying a document out off screen and putting it on paper.
The code: `crates/writer-editor/src/print.rs` (the print pass), `crates/writer-print` (the
PDF writer, the PDF reader, the print operation). Everything here was run on this Mac
unless it says "read".

- **There is no text system without a window.** `WindowTextSystem::new` is `pub(crate)`
  (`$G/src/text_system.rs:340`); the only way to one is `window.text_system()`, which is
  an `Arc<WindowTextSystem>` that can be cloned and kept. Any window will do. The print
  pass keeps the clone in a layout environment of its own and shapes with it outside any
  frame. What it shapes goes into GPUI's line cache of the current frame
  (`LineLayoutCache`, two frame maps, swapped in `finish_frame`): a pass over a book in
  one go holds every shaped line until the window draws twice, a pass in slices with
  frames in between holds a slice.
- **A `FontId` goes back to a `Font` only halfway.** `TextSystem::get_font_for_id`
  (`$G/src/text_system.rs:134-142`, public) looks through the fonts that were asked for
  by name: it answers for those, and has no answer for a font CoreText put in as a
  fallback (an emoji, a CJK character in a Charter run), which is entered by PostScript
  name in a private table (`$G/src/platform/mac/text_system.rs:304-320`). No public call
  gives the native font of a `FontId`. So the print pass never reads a glyph's `FontId`:
  it takes the `Font` of every run from `LayoutStyle` (what the layout asked for), and the
  PDF writer finds the face again and lets CoreText find the fallbacks again when it
  draws.
- **Glyph ids are CoreText's and are not reused.** `ShapedGlyph::id` is the `CGGlyph` of
  the `CTRun` (`text_system.rs:510-537`), but without the font it is of no use, and in
  tests every id is 1 or 2. What the pass takes from a shaped glyph is its byte offset and
  its x; the PDF writer makes one `CTLine` of the row's text and draws each of its glyphs
  where the cluster starts in the layout.
- **The face of a `gpui::Font`, step by step** (mirrored in `writer-print/src/fonts.rs`):
  the family by name through font-kit (`SystemSource::select_family_by_name`;
  `.SystemUIFont` is asked for as `.AppleSystemUIFont`), every face loaded and given the
  cascade list of `Font::fallbacks` (one descriptor per family, `open_type.rs:30-69`; the
  system's own fallbacks follow by themselves), a face without an `m` left out,
  `find_best_match` on weight and style. A family that is not installed goes down
  `TextSystem::fallback_font_stack` in the *regular* face: bold text in a missing family
  is Helvetica Regular on screen, and on paper. Never make a font from a PostScript name:
  CoreText answers `.SFNS-Regular` with Times New Roman.
  Checked with the editor's demo (`print:<path>`): Charter in four faces, Helvetica Neue,
  Menlo, the interface font and the emoji and CJK fallbacks came out at the layout's
  widths in every row (0 rows off of 37; 0 of 35,178 in a generated book).
- **Where a single shaped line has its baseline.** `ShapedLine::paint(origin,
  line_height, ..)` centres the line in its box with the layout's own numbers
  (`$G/src/text_system/line.rs`): the baseline is at `origin.y + (line_height - ascent -
  descent) / 2 + ascent`. A label that is printed (a list number, a page number) gets its
  baseline from the same expression over the shaped line's `ascent` and `descent`.
- **A y far down a long document is coarse.** Content space is `f32` pixels from the top;
  at page 400 a y is in the hundreds of thousands and single precision has a thirty-second
  of a point to offer, at page 1,500 an eighth. The screen does not show it. Paper would:
  what is rounded to half points (a rule, an underline) can land half a point off on one
  page and not on the next. The pass therefore works every page out on its own sheet
  (`Placed::on_sheet`: a block's place from the sheet's top edge, from the placement's
  own page-local numbers), and a test holds 400 identical pages equal to the first to the
  last bit.
- **Sheets are started from a foreground task, outside an update.** AppKit calls the
  window's delegate while a sheet comes up, and those calls re-enter GPUI: started inside
  an `update` that is "already borrowed". GPUI does the same with its own sheets
  (`beginSheetModalForWindow:` in `$G/src/platform/mac/window.rs:1197-1206`, the save
  panel in `platform.rs:768-771`; read). `writer_print::run_sheet` is to be called that
  way: make the PDF inside the update, then `cx.spawn(..)` and call it from the task. Its
  callback runs inside AppKit and may only send on a channel. (The sheet itself has not
  been opened by anything but the design's Swift probes: no test and no script opens a
  print panel.)
- **The print operation runs from Rust without a panel.** `writer_print::run_save_job`
  (PDFKit's `printOperationForPrintInfo:scalingMode:autoRotate:`, the job disposition
  "save", both panels off, `runOperation`) wrote a two-page PDF with both pages at the
  paper's size, their text, and the first word where it was within a thousandth of a
  point (`cargo test -p writer-print --test save_job`). It wants the main thread of the
  process and an `NSApplication` (`[NSApplication sharedApplication]` is enough; no run
  loop has to turn), which is why that test is a program of its own (`harness = false`).
  Leave `NSPrintInfo`'s centering on and the scaling off: only then is the page on the
  paper at its real size. The selector is `scalingMode:`, not `scaleMode:`.
- **Small things that bit there.** `[NSString instancesRespondToSelector:
  @selector(initWithBytes:length:encoding:)]` is NO (a class cluster: the abstract class
  does not implement what its placeholder does), so a check of selectors must not list
  it; strings and the file URL go to AppKit as the `CFString` and `CFURL` they are. A PDF
  context that is closed without a page writes one blank Letter page. A JPEG's own data
  goes into the PDF unchanged, also with `kCGImageSourceShouldCache` false (tested: a
  stretch of the file's bytes is found in the PDF, once however often the picture is
  drawn). `CGColorCreateGenericRGB` (what `core_graphics::color::CGColor::rgb` calls) is
  not sRGB; `CGColorCreateSRGB` is.
- **Reading a PDF's text back** (`writer_print::PdfFile`, PDFKit's `PDFPage.string`). The
  text of such a PDF is what CoreGraphics maps the drawn glyphs back to. A ligature comes
  back as its own character (U+FB03 for "ffi"), 日 from a CJK fallback font as the Kangxi
  radical of the same shape (U+2F47): compare after NFKC, or avoid them in a test.
  Marked content with `ActualText` (`CGPDFContextBeginTag`) does not change what PDFKit
  reads (tried: it made it worse, an emoji came back on a line of its own). PDFKit's box
  for a word (`selectionForRange:`, `boundsForPage:`) starts at the origin of the first
  glyph and is as high as the row (the ascent and the descent of its font; higher in a
  row with an emoji), whatever the size of the word in it: with one font in the row the
  baseline is the box's bottom less the font's descent.
- **The fake text system and a PDF.** In tests every glyph is 0.6 em wide (an emoji, two
  UTF-16 units, 1.2 em). Real glyphs at those places make PDFKit see spaces inside words
  ("Fi xed pi t ch"), unless the font is that wide: **Courier New is** (1229 of 2048
  units). Documents whose pages go through the PDF writer in a test of the editor are set
  in Courier New, and their text is compared with white space removed. The row widths of
  the fake are 0.6 em per character too, so `Report::rows_off` is 0 for Courier New rows
  and counts the others. Two more things are the fake's and not the Mac's: a shaped
  line's `descent` is negative there (the font's sign; the Mac's line layout has it
  positive), so a label's baseline, centred with it, is lower than its row's in a test and
  on it with real fonts; and the theme names the first family of each font stack
  (`SF Mono`, which this Mac does not have as a family) because the test platform lists
  no installed fonts, so the PDF writer reports it missing and sets code in Helvetica.

## Images

From images in the editor (`writer-editor`, `src/images.rs`, `src/images/cache.rs`,
`src/element.rs`). "tested" is the editor's headless tests on the test atlas, "seen" is the
editor's demo in a real window (`cargo run -p writer-editor --example demo`).

- **Painting.** `window.paint_image(bounds, corner_radii, Arc<RenderImage>, frame_index,
  grayscale) -> Result<()>` (`$G/src/window.rs:3129-3175`), paint phase only. It puts the
  frame into the sprite atlas the first time it is painted (keyed by the `RenderImage`'s
  own id and the frame index), floors the origin and ceils the size to device pixels, and
  draws inside the current content mask. Later paint calls are on top of it like on top of
  anything, so a quad painted after it tints it (a translucent selection colour over a
  picture: tested, seen). Call it with `.ok()`: a frame the atlas cannot take is an `Err`
  from the allocation, not a panic.
- **`RenderImage` is BGRA with straight alpha** (`$G/src/assets.rs:42-68`):
  `RenderImage::new(frames: impl Into<SmallVec<[image::Frame; 1]>>)`, each frame an
  `image::Frame` over an `RgbaImage` whose bytes were swapped to blue, green, red, alpha.
  The alpha is not multiplied in (GPUI's own decoder only swaps the two channels,
  `$G/src/platform.rs`, `Image::to_image_data`; tested: a half transparent pixel decodes
  to `[b, g, r, 128]` and is what the frame hands the atlas). Every `RenderImage` gets a
  new id (a static counter), so two of them never share a texture; keep the `Arc` for as
  long as the picture is on screen. `image` 0.25 and `smallvec` 1 are the versions gpui
  itself builds with: naming them in a crate's manifest compiles nothing new.
- **The atlas: 16384 pixels a side at most, and no mip maps.** A texture larger than
  `MAX_ATLAS_SIZE` cannot be allocated (`$G/src/platform/mac/metal_atlas.rs:117-131`), and
  the sprite shader samples with `mag_filter::linear, min_filter::linear` and nothing else
  (`$G/src/platform/mac/shaders.metal:703-706`). A picture drawn at less than half its
  size therefore looks rough: scale it on the CPU first. The editor decodes a picture at
  its natural size halved as often as it can be while it stays at least as wide as the
  device pixels it is drawn on (`bounds.size.width * window.scale_factor()`), and never
  over 8192 pixels a side (`images::cache::level_for`); seen sharp at 100 % and 200 %.
- **A texture stays in the atlas until it is dropped.** `window.drop_image(Arc<RenderImage>)
  -> Result<()>` (`$G/src/window.rs:3198-3209`) removes the tiles of every frame; it does
  not need a draw phase. `cx.drop_image(image, Some(window))` (`$G/src/app.rs:2071-2081`)
  does the same in every window. Dropping the `Arc` alone frees the pixels and leaves the
  texture: a cache that evicts must hand what it evicts to a window (the editor does it at
  the start of its prepaint, and in `cx.on_release_in(window, ..)` for an editor that goes
  away; the count is tested).
- **GPUI's own image path is not for a custom element with many large pictures.** `img()`,
  `ImageSource`, `Image::use_render_image` and `window.use_asset` decode at natural size
  and keep every decoded image in the app until `remove_asset` is called: no sizes, no
  budget (source).
- **A custom element cannot paint an image half transparent.** `paint_image` has no opacity
  argument. It draws with the opacity of the element that is being painted
  (`$G/src/window.rs:3160`, `element_opacity()`), and only GPUI's own elements can set
  that: `with_element_opacity` is `pub(crate)` (`:2433`). What dims a picture in a custom
  element is a quad of the colour of the surface under it at 50 % alpha, painted over it
  (the editor's picture while it is dragged; seen in Paper and Night).
- **The clipboard is text or one picture, never both.** Reading
  (`$G/src/platform/mac/platform.rs:1102-1137`): when the pasteboard has
  `public.utf8-plain-text` the string is the answer and no image is looked for; otherwise
  PNG, JPEG, WebP, GIF, SVG, BMP, TIFF are tried in that order and the first found comes
  back as `ClipboardEntry::Image(gpui::Image { format, bytes, .. })`. So a screenshot and
  "Copy" in Preview arrive as pictures, while a file copied in Finder (its name) and "Copy
  Image" in a browser (its address) arrive as their text: pictures of that kind need the
  app's own read of `NSPasteboard`. An empty string on the pasteboard reads as an item
  with an empty text, not as nothing (source). Writing (`:1027-1090`): an item with one
  entry is written as that string (with its metadata) or as that image
  (`ClipboardItem::new_image(&gpui::Image::from_bytes(ImageFormat::Png, bytes))`); with
  several entries the strings are joined and images are left out. `gpui::Image::id()` is a
  hash of the bytes: the way to recognise a picture one has written oneself when it comes
  back. The test platform keeps the item as it was written, so both directions are
  testable headlessly (tested).
- **Drops from outside are files, and they reach an element as a drag of
  `ExternalPaths`.** The window registers for `NSFilenamesPboardType` only
  (`$G/src/platform/mac/window.rs:670-674`): files from Finder arrive, a picture dragged
  out of a browser or Photos does not. GPUI turns the drag into its own
  (`$G/src/window.rs:3622-3661`): on `FileDropEvent::Entered` the app's active drag becomes
  an `ExternalPaths` and a mouse move with the left button is dispatched, `Pending` is
  another such move, `Submit` a mouse up, and `Exited` drops the drag and is itself
  dispatched as a mouse event. An element takes part through a div:
  `.on_drag_move::<ExternalPaths>(|event: &DragMoveEvent<ExternalPaths>, window, cx| ..)`
  (every move while such a drag is active, inside or outside the div, in the capture
  phase; `event.event.position`), `.on_drop::<ExternalPaths>(|paths: &ExternalPaths,
  window, cx| ..)` (`paths.paths() -> &[PathBuf]`; the pointer is
  `window.mouse_position()`), and `window.on_mouse_event(|event: &FileDropEvent, ..|)` in a
  custom element's paint to learn that the drag left the window. The moves of such a drag
  reach window-wide mouse listeners too, as moves with the left button held: a listener
  that follows its own drags must know it has none. `ExternalPaths` cannot be built
  outside gpui (its field is `pub(crate)`), so a headless test calls what the two div
  listeners call; **the listeners themselves were compiled and not run** (nothing can
  drag from Finder but a hand).
- **The open panel has no file type filter.** `PathPromptOptions` is `files`,
  `directories`, `multiple`, `prompt` and nothing else (`$G/src/platform.rs:1330-1339`):
  any file can be chosen, and whoever asked has to say what is wrong with one that is not
  a picture.
- **gpui 0.2.2 has no accessibility interface.** Nothing in its source implements
  `NSAccessibility` or names it (searched): no part of a GPUI window reaches VoiceOver, so
  neither does the description of an image. The description serves where it can (a DOCX's
  alternative text, Markdown's alt text, agents, the placeholder).
- **Decode off the main thread, and repaint when it is there**: `cx.spawn(async move
  |this, cx| { let picture = cx.background_spawn(async move { decode(..) }).await;
  this.update(cx, |this, cx| { ..; cx.notify() }) })`. `AsyncApp::background_spawn` needs
  `gpui::AppContext` in scope. `image::load_from_memory` gives the first frame of a GIF;
  `DynamicImage::thumbnail_exact` scales down by whole blocks, which is what halving
  wants. On the test executor the decode runs inside `cx.run_until_parked()`, and a dirty
  window is drawn again in the same call (tested).
- **A pointer in a scripted run.** `Window::dispatch_event` cannot be called from outside
  gpui (see "Actions and keys"), so a scripted run cannot press a button. What it can do
  is call what the element's mouse listeners call: the editor has four doc-hidden methods
  for that (`Editor::pointer_press`, `pointer_drag_to`, `pointer_release`,
  `pointer_drags_files`), and the demo's script steps over them (`press:handle:3;
  drag:40,10; screenshot:..; release`). A pointer that is put outside the viewport that
  way scrolls the view for as long as it stays there, as a real one does.

## Testing

Source: `$G/src/app/test_context.rs`, `$G/src/platform/test/`, `$G/../gpui-macros-0.2.2/src/test.rs`.
Examples in this repo: `crates/writer-ui/tests/gpui_harness.rs` (text box with
EntityInputHandler, dispatch rules, chords, intercept, focus, clipboard, timers, theme) and
`crates/writer-ui/tests/gpui_snippets.rs` (lists, overlays, custom element hitbox, prompts, menus).

```rust
#[gpui::test]                                   // sync or async fn; params: &mut TestAppContext (any number),
fn typing(cx: &mut TestAppContext) {            // BackgroundExecutor, StdRng (needs `rand` dep)
    cx.update(|cx| cx.bind_keys([KeyBinding::new("backspace", Backspace, Some("TextBox"))]));
    let (view, cx) = cx.add_window_view(|window, cx| {           // shadows cx: &mut VisualTestContext
        let v = TextBox::new(cx); window.focus(&v.focus_handle); v
    });
    cx.simulate_input("Hello wörld");           // one keystroke per char → bindings, key_down, input handler
    cx.simulate_keystrokes("backspace cmd-b");  // space-separated Keystroke::parse syntax
    view.read_with(cx, |v, _| assert_eq!(v.text, "Hello wör"));
}
```
- `TestAppContext`: `update(|cx: &mut App| ..)`, `read`, `new`, `add_window(|w, cx| V) -> WindowHandle<V>`,
  `add_window_view`, `add_empty_window() -> &mut VisualTestContext`, `run_until_parked()`,
  `executor().advance_clock(Duration)` (timers; tested), `dispatch_action(window, action)`,
  `simulate_keystrokes(window, ..)`, `write_to_clipboard`/`read_from_clipboard`,
  `simulate_prompt_answer("Delete")`, `has_pending_prompt()`, `simulate_new_path_selection(|dir| Some(path))`,
  `opened_url()`, `simulate_window_resize`, `set_global`, `update_global`, `spawn`, `to_async()`.
- `VisualTestContext` (derefs to TestAppContext): `update(|window, cx| ..)`, `simulate_input`,
  `simulate_keystrokes`, `dispatch_action`, `simulate_click(point, modifiers)`, `simulate_mouse_down/up/move`,
  `simulate_event(any InputEvent e.g. ScrollWheelEvent)`, `simulate_modifiers_change`,
  `simulate_resize(size)` (tested), `debug_bounds("selector")` (needs `.debug_selector(|| "selector".into())`
  on a div; tested), `draw(origin, space, |window, cx| element)` (draw an element directly), `deactivate_window()`,
  `simulate_close()`, `window_title()`. `entity.update_in(cx, |v, window, cx| ..)` works with it.
- **Text system in tests is `NoopTextSystem`**: no real fonts (`all_font_names()` = the 10
  fallback names + `.SystemUIFont`), every glyph advances `0.6 × font_size` (2 UTF-16 units =
  emoji = 1.2×), ascent/descent fixed. Shaping, wrapping and hit-testing APIs work and are
  deterministic (tested: width of "hello world" at 10px = 66px), but **not real metrics**.
  For real metrics run something under `Application::new()` (see the probe example).
- Windows draw in tests at the end of each effect flush when dirty (no frame clock);
  `simulate_*` and `update` calls draw. Test windows start **inactive** (focus events need
  `window.activate_window()`), appearance Light, size = the test display.
- `simulate_input` also sends empty `""` keystrokes around the text (it splits on "");
  `on_key_down` sees them (tested).
- Tests run on normal test threads; no main thread or GUI session needed. `cargo test -p writer-ui`
  takes ~0.01s per test.
- **A real thread can wake a task of the test's main thread** (tested,
  `crates/writer-app/tests/suite/live.rs`): the waker of a foreground task may be called from
  any thread (the test dispatcher's queue is behind a mutex), but the task only runs when the
  test lets it: `run_until_parked()`. A test that waits for work another thread started (a
  socket client that blocks until the app answers) therefore loops: `run_until_parked()`,
  look whether the thread is done (a channel's `try_recv`), sleep a millisecond or two; never
  `join()` first, which would wait for an answer nobody is there to give. Timers of the app
  (`cx.background_executor().timer(..)`, `.now()`) are on the test clock and need
  `advance_clock` in that loop; the other thread's own waits are real time.
- **What a view emitted inside `window.update(cx, ..)` from an async task has been delivered
  when `update` returns** (read from `$G/src/app.rs`: `update_window_id` runs inside
  `App::update`, and `finish_update` flushes the effects when the outermost update ends,
  before the task goes on; not tested by itself, the tests of that file only see the state
  afterwards). So an answer sent right after the `update` is sent after every subscriber has
  heard, and no key press can come between the closure and the delivery.

## Visual QA

Works on this machine (verified): the app runs a script from `THE_WRITER_AUTOMATION`
(`crates/writer-app/src/automation.rs`) and captures **only its own window** with
`screencapture -x -o -l <CGWindowID>` (the window number comes from the NSView behind the
GPUI window). The terminal already has the Screen Recording permission, so captures contain
the real window content. GPUI 0.2.2 cannot render a window to an image itself.

Script steps (`;` or newline separated, `#` comment lines; `THE_WRITER_AUTOMATION=@file` reads a file):

| step | effect |
|---|---|
| `wait:<ms>` | sleep |
| `keys:<keystrokes>` | GPUI keystrokes, space separated (`keys:cmd-shift-p escape`); logs handled / not handled. The key `;` is written `\;` (`keys:cmd-\;`), because `;` separates steps |
| `type:<text>` | one keystroke per character (bindings first, then the focused input handler). Escapes `\;` `\n` `\t` `\s` `\\`. **The argument is trimmed: write a leading or trailing space as `\s`** |
| `action:<name>` | dispatch a registered action by name (`action:app::Quit`) |
| `resize:<w>x<h>` | resize window content (overridden by the tiling window manager here) |
| `theme:<light\|dark\|system>` | switch theme mode |
| `screenshot:<path>` | draw a fresh frame, wait 250ms, capture this window to PNG (Retina 2×) |
| `info` | print window bounds, viewport, scale, appearance, active, focused, theme |
| `menu:<name>` | opens a menu of the workspace at its control: `title`, `save`, `statistics`, `page`, `limit` |
| `choose:<label>` | chooses that item of the open context menu, as a click on it would |
| `menukey:<title>` | prints the key the native menu bar has beside the item with that title, read from AppKit's own `NSMenu` (`menukey:Bold` → `cmd-b`, `none` for an item without a key) |
| `bar[:<button>]` | shows the format bar over the selection, as a mouse selection does; `bar:bold`, `bar:extra-0`, `bar:more` press a button of it |
| `shortcut:<action>[@<place>]=<keys>` | gives a command its keys without the list of keyboard shortcuts: `shortcut:editor::ToggleBold=cmd-alt-b`; several keys with a bar between them, `none`, `default`; `@In the find bar` for a command that is listed in several places |
| `shortcuts:import:<path>`, `shortcuts:press:<label>`, `shortcuts:reset`, `shortcuts:undo` | the keyboard shortcuts: read a shortcuts file and show the preview in the list (the open panel cannot be driven), press a control of the open list or a choice of its question by its label, put every key back without asking, undo the last import or reset |
| `fake:hold`, `fake:advance:<n>`, `fake:release` | the fake engine of the assistant (`THE_WRITER_ASSISTANT_FAKE`): hold its events back, let the next `n` through (an answer half written, with its Stop control), let everything through again |
| `state` | one line about the workspace: document, save state, focus (editor, library, comments, modal, tool), the open context menu, Vim mode and command line, open tool and find count, the format bar, comment threads, spelling (`on language=<id\|auto> (app\|document) [detected=<id>] marks=N pending=N`), status bar |
| `book:<blocks>` | creates and opens a generated document; prints what opening cost and how long it took until the text on screen was checked for spelling |
| `savecost[:<rounds>]`, `limitcost[:<rounds>]` | time a synchronous save part by part, and the character-limit count |
| `keyscost[:<rounds>]` | times a change of a keyboard shortcut (the key of Bold, there and back, as the list makes it: keymap installed, menu bar built again, change stored), the frame after it, and the parts of installing the keymap alone. Release build of 0.5.0: 6 to 14 ms for a change; of the install, `clear_key_bindings` and `bind_keys` of 452 bindings take under 0.1 ms, building the bindings about 1 ms, `set_menus` 1 to 5 ms |
| `typecost:<text>`, `keycost:<keystrokes>` | press the keys at a writer's pace (one every 80 ms) and time each: dispatch, the frame after it, and how long the main thread kept a 4 ms timer waiting in between; ends with the spell checker's slices |
| `stall:<ms>` | watches the main thread for that long without pressing anything (the background pass of the spell checker, events posted from outside): timer waits, frame costs, slices |
| `dump` | the first 400 characters of the text in the editor |
| `dump:<path>` | the whole text in the editor, written to a file |
| `log:<text>` | print |
| `quit` | quit through `cx.quit()` (runs on_app_quit hooks) |

No mouse steps: `Window::dispatch_event` cannot be called from outside gpui (see Actions).

Environment: `THE_WRITER_DATA_DIR` (always a temp dir), `THE_WRITER_SEED_DEMO=1` (demo
folders and documents in an empty library; without it a fresh library gets the welcome
document), `THE_WRITER_EXPORT_DIR=<dir>` (file exports are written there under the suggested
name; the native save panel cannot be driven), `THE_WRITER_AUTOMATION_FOREGROUND=1`
(activate the app; **do not use it while someone is at the machine**, their typing lands in
the window), `THE_WRITER_AGENT_HOME=<dir>` (the Agents panel of a scripted run sees that
directory as the whole machine: agents are found by what is in it, the skill is installed
into it; without the variable the panel lists nothing), `THE_WRITER_AGENTS_HINT=1` (the
one-time toast about agents, which a scripted run otherwise leaves out). While the Agents
panel is open the `state` line ends with `agents=[tool=... found=... skills=[...] mcp=N
rows=N selected=... report=[...]]`.

The assistant in a scripted run: none, unless `THE_WRITER_ASSISTANT_FAKE=ready` (or
`signed-out`, `missing`) gives it the fake engine, which starts no program and answers from
`THE_WRITER_ASSISTANT_FAKE_REPLIES=<file>` (answers separated by a line of `====`, leading
lines `!tool <name> <words>` and `!error <kind>`; `crates/writer-app/src/assistant/fake.rs`).
With an engine the `state` line has `assistant=[shown=... engine=[...] tools=[<the command of
the library's MCP server>] quick=[open side=... selection="..." messages=N answering=...
model=... effort=... replace=... notices=[...] draft="..." last="..."] side=[document="..."
messages=N answering=... last=done|streaming|stopped|failed model=... effort=... attached=N
notices=[...] focus=composer|...]] right_panel=...`. The picker of a chat is a context menu:
`keys:cmd-shift-m`, then `state` lists its items under `menu=[...]` and `choose:<label>`
takes one. `THE_WRITER_ASSISTANT_REAL=1` gives the run the real Claude Code: a script that
only looks sends nothing, **every message it sends spends the user's allowance**. How the
few real messages of 0.4.0 were watched: a small pass-through named `claude`, first on the
`PATH` of the run (the engine looks through the `PATH` before the installers' folders),
that starts the real program with the same arguments and environment and writes down both
directions of the stream; `system/init` in what comes back names the tools, MCP servers,
skills and commands the process has. Start such a run from a shell and it inherits that
shell's environment: the engine takes the markers of an enclosing Claude Code session out
(`claude/args.rs`, `REMOVED_ENV`).

Changing a key in a scripted run: `keys:cmd-/` opens the list, `type:bold` filters it,
`keys:enter` makes it wait for a key, the next `keys:` step is the key (it reaches the list
through `App::intercept_keystrokes`, which `window.dispatch_keystroke` runs like a real key
press does; no binding runs and nothing is typed), `keys:enter` takes the marked choice.
The `state` line has `keys=[changed=N]` and, while the list is open, `list=[mode=... rows=N
changed=N selected="..." keys=[...] default=[...] question="..." says="..." choices=[*Use
⌥⌘B | Press Another Key | Cancel]]`. A key a script presses is never released
(`dispatch_keystroke` sends no key-up), and the list takes the same key pressed again for a
held one: the `keys:` step tells the open list that the key was let go
(`KeymapView::key_released`); a headless test sends a `KeyUpEvent` after each key.
The native menu bar exists in a scripted run although the app is never activated
(`cx.set_menus` sets `NSApplication.mainMenu` either way), and `menukey:<title>` reads an
item's `keyEquivalent` and `keyEquivalentModifierMask` from it: after
`shortcut:editor::ToggleBold=cmd-alt-b` the item "Bold" has `cmd-alt-b`, after `=none` it
has none, after `shortcut:editor::InsertPageBreak=f6` "Page Break" (which shows no key by
default, its key has return in it) has `f6` (seen in a run of the debug build).
`action:workspace::ExportShortcuts` writes "The Writer Shortcuts.json" into
`THE_WRITER_EXPORT_DIR`. What neither a script nor a test can drive: the open panel
(`cx.prompt_for_paths` is `unimplemented!()` on the test platform, so a test that reaches
it panics; the app names the file beforehand, `workspace::choose_import_file_beforehand`),
and the system dialog of "Reset All Keyboard Shortcuts…" in a scripted run (a test answers
it with `simulate_prompt_answer`).

Real key and pointer events for a window in the background (used for 0.5.0 on the list of
keyboard shortcuts; the tool is `scratchpad/v050/tool/poster.swift`, first written for the
mouse in 0.2.0): `CGEvent.postToPid(pid)` delivers a key or mouse event to one process, and
the app's background window gets it through AppKit (`performKeyEquivalent:`, `keyDown:`,
`keyUp:`, hover, clicks), which a `keys:` step does not go through. Seen that way: a click
on the key of a row starts the waiting, the next real key is taken (`cmd-alt-b`, `cmd-w`,
`cmd-,`, `cmd-i`, `f5`, which arrives as plain `f5`), a held key (auto-repeat events, then
a key-up) starts the waiting once and is recorded once, a click on a choice takes it, the
controls of a row that is not selected come up under the pointer and work, a right-click
opens the row menu. **What it cannot show: the keys of the application menu.** A background
app has no active window, `App::is_action_available` then only knows actions with app-wide
handlers, so of the menu bar's items only Quit, Hide and Hide Others are enabled, and
AppKit runs an enabled item from the menu bar before the window sees the key. A posted
`cmd-q` quit the app while the list waited for a key; `cmd-h` and `cmd-alt-h` ran too.
**Do not post `cmd-h`, `cmd-alt-h`, `cmd-m` or `cmd-q` to the app: Hide Others hides every
other application of the person at the Mac** (undone with `NSRunningApplication.unhide()`
for each hidden app). In an active window AppKit asks the key window first, where GPUI's
interceptor takes the key (read from `handle_key_event` in gpui's mac `window.rs`); that
needs a person at the keyboard.

**`.line_clamp(n).text_ellipsis()` with `n` above 1 does not keep a text inside its box.**
Read from gpui 0.2.2 (`elements/text.rs`, `text_system/line_layout.rs`): the text is cut,
with the ellipsis, at `n` times the available width as if every line were full, and the
wrapping stops after `n - 1` breaks, leaving whatever remains on the last line. A long word
that goes to the next line whole (a path: `/` is a place to break, `-` and `.` are not)
leaves the line before it half empty, so the last line is longer than the box and is
clipped at its edge without an ellipsis (seen in a capture of 0.5.0: the toast after an
export of the shortcuts). `line_clamp(1)` is exact. For more lines, wrap with GPUI
(`window.text_system().shape_text(text, size, runs, Some(width), None)`), count
`wrap_boundaries()`, and when there are too many cut the text at the start of the last
line that may be drawn (`unwrapped_layout.runs[b.run_ix].glyphs[b.glyph_ix].index` is its
byte index), shorten that line with an ellipsis until what is shaped fits, and paint the
`WrappedLine`: `writer_ui::toast` does this in a small element of its own (`Message`,
measured with `window.request_measured_layout`, which is handed the width the text really
gets). A measure function is asked several times, also without a width ("how wide would
you like to be"): once a layout for a width exists, answer that question with it, as
GPUI's own text does, or the last answer overwrites the layout that is painted.

Two layout facts from the list of keyboard shortcuts: `.truncate()` drew no ellipsis in a
flex row there, `.line_clamp(1).text_ellipsis()` does (seen in captures). And a toast with
a button, shown while a modal that fills most of the window's height is open, lies over the
modal's lower part (toasts sit 44 px above the bottom edge and take clicks): the list
leaves room for one toast under itself and the app shows one toast about the shortcuts at a
time. `WindowHandle<V>::update` leases the root view, so `window.dispatch_keystroke` inside
it panics when the key's action updates that view: dispatch through the `AnyWindowHandle`.

A library the `writer` tool made is a library like any other: build one with the tool
(`THE_WRITER_DATA_DIR=$QA/data target/debug/writer create ...`), then start the app on the
same directory to look at it. Not both at once: while the app runs it has the lock and the
tool only reads.

Ready-to-run (from the repo root; `timeout` is not installed, so wait in a loop):

```bash
cargo build -p writer-app
QA=$(mktemp -d)
THE_WRITER_DATA_DIR=$QA/data \
THE_WRITER_AUTOMATION="wait:1200; info; theme:light; screenshot:$QA/paper.png; theme:dark; screenshot:$QA/night.png; quit" \
./target/debug/the-writer & PID=$!
for i in $(seq 1 30); do sleep 1; kill -0 $PID 2>/dev/null || break; done; kill $PID 2>/dev/null
sips -Z 1400 $QA/paper.png --out $QA/paper_small.png     # downscale before opening with Read
sips --cropOffset 100 200 -c 400 1600 $QA/paper.png --out $QA/crop.png  # crop: y x then h w, in pixels;
                                                                         # offset 0 0 is ignored (centre crop), use 1 1
echo $QA                                                   # open the PNGs with the Read tool
```

Useful lines: open a document `keys:cmd-p; type:chap 2; keys:enter`; a tool
`keys:cmd-.` / `cmd-k` / `cmd-t` / `cmd-f`, then `type:` filters it; a comment
`keys:shift-alt-right cmd-alt-m; type:Why?; keys:enter`; Vim `keys:cmd-alt-v; type:ggdw`,
a command line `type:\:w; keys:enter`; the library `keys:cmd-0 n; type:Name; keys:enter`.

Notes:
- stderr lines start with `[automation]`; a failing step is reported and the script continues.
- `cargo test -p writer-app` rebuilds `target/debug/the-writer` with gpui's test features.
  Run `cargo build -p writer-app` before a scripted run, or copy the binary first.
- A scripted run stays in the background (no `cx.activate`). **A window that is completely
  covered by other windows gets no frames**: macOS reports it occluded and GPUI stops its
  display link (`window_did_change_occlusion_state` in `$G/src/platform/mac/window.rs`). The
  app keeps handling keys, but nothing is drawn: captures are stale, and everything a frame
  settles lags (the registered input handler, focus events, layout). The automation
  therefore draws a frame itself after every key and before every capture by sending the
  view `displayLayer:` (`writer_app::screenshot::force_frame`; it is the path AppKit uses for
  a synchronous frame, and it must be called outside any GPUI update). With that, a covered
  window behaves and captures like a visible one (app).
- **Look at every capture.** In one run of 0.4.0 two captures in a row came back as the
  same stale picture (byte for byte, of a state several steps earlier), although a frame is
  forced before each; the ones before and after were right. The `state` line of the same
  moment was right too, so it is the picture that was old, not the app.
- A background window is **inactive**: focus events do not fire (see Focus), so behaviour
  that hangs on blur (a popup tool closing when a modal opens over it) does not happen in a
  scripted run. Check those paths headlessly, where the test window is activated.
- Scripts must not write the system clipboard (copy, cut, "Copy as Markdown", Vim's `"+`
  register): it is the user's clipboard.
- The window is placed by the tiling window manager at ~1489×1672 points (2978×3344 px PNG).
- **A stall of almost exactly one second while frames are forced is not the spell
  checker.** With real scroll events posted to the background window and a frame forced
  every 16 ms (`stall:`), one frame or timer wait of 1005 to 1015 ms showed up about once a
  run, with spell checking off as well as on (seen 4 times in 5 runs, release build). The
  cause was not tracked down; the length points at Metal's `nextDrawable`, which gives up
  after one second when the window server hands no drawable back, as it may for a window
  it is not showing. Not seen without posted scroll events.
- Measure in a release build, and run a second time with spell checking off (palette
  "Check spelling while typing" first): this Mac has other work, and single waits of 20 to
  40 ms turn up in both runs.
- If a capture ever comes back black/wallpaper-only or `screencapture` fails with a permission
  error, the Screen Recording permission was revoked for the launching terminal. Do not try to
  change system settings; fall back to headless tests and ask the user.

## Theme API (writer-ui)

`crates/writer-ui/src/theme/{mod.rs,palette.rs,tokens.rs}`.

```rust
use writer_ui::theme::{self, ActiveTheme, ThemeMode, HighlightColor};
theme::init(ThemeMode::System, cx);                 // once (writer_app::init does it); resolves font stacks
theme::observe_system_appearance(window, cx).detach();  // per window, so System follows macOS
theme::set_theme_mode(ThemeMode::Dark, cx);         // applies + cx.refresh_windows()
theme::theme_mode(cx) -> ThemeMode;  cx.observe_global::<theme::GlobalTheme>(..)
let t = cx.theme();                                 // &Theme, on App and Context<T>
t.colors.canvas / sidebar / elevated / desk / page / hover
t.colors.ink / muted / faint / on_accent; hairline / border_strong
t.colors.accent / accent_hover / accent_subtle; selection / selection_inactive (translucent) / caret
t.highlight(HighlightColor::Yellow); t.colors.comment / comment_active / comment_underline
t.colors.find_match / find_match_current (translucent); danger(_subtle) / success(_subtle); overlay_backdrop
t.colors.spelling                                   // the wavy underline of a misspelled word (a quiet red)
t.shadow_overlay() / shadow_popover() / shadow_page() -> Vec<BoxShadow>
t.ui_font() / document_font() / mono_font() -> Font;  t.fonts.{ui, document, mono}: SharedString
theme::{UI_FONT_FAMILY, UI_TEXT_XS..UI_TEXT_TITLE, DOCUMENT_TEXT_SIZE (17px), DOCUMENT_LINE_HEIGHT_RATIO (1.6),
        DOCUMENT_COLUMN_WIDTH (700px), HEADING_SCALE, document_line_height(size), SPACE_1..SPACE_16, space(n),
        RADIUS_SM/MD/LG/PAGE, TITLEBAR_HEIGHT, TRAFFIC_LIGHT_*}
```
Resolved on this Mac: document = Charter, mono = Menlo (SF Mono is not installed), UI = `.SystemUIFont`.
Contrast is enforced by unit tests (ink ≥ 12:1 on all surfaces, muted ≥ 4.5:1, ink ≥ 7:1 on
every highlight, accent ≥ 3:1 on canvas).
