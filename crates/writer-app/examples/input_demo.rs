//! Visual QA fixture: a window with one focused text line and the automation
//! hook, to check that typed text, bindings and screenshots work end to end
//! in the real app (real fonts, real IME path, real window server).
//!
//! ```sh
//! THE_WRITER_AUTOMATION='wait:600; type:Hello wörld; keys:cmd-b; screenshot:/tmp/demo.png; quit' \
//!     cargo run -p writer-app --example input_demo
//! ```

use std::ops::Range;

use gpui::{
    App, Application, Bounds, Context, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, GlobalElementId, InspectorElementId, KeyBinding, LayoutId,
    Pixels, ShapedLine, SharedString, Style, TextRun, UTF16Selection, Window, actions, div, fill,
    point, prelude::*, px, relative, size,
};
use writer_app::{automation, window::main_window_options};
use writer_ui::theme::{self, ActiveTheme};

actions!(input_demo, [Backspace, ToggleBold]);

struct Line {
    focus: FocusHandle,
    text: String,
    marked: Option<Range<usize>>,
    bold: bool,
    layout: Option<ShapedLine>,
}

impl Line {
    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        self.text.pop();
        cx.notify();
    }

    fn toggle_bold(&mut self, _: &ToggleBold, _: &mut Window, cx: &mut Context<Self>) {
        self.bold = !self.bold;
        cx.notify();
    }

    fn to_utf8(&self, utf16: usize) -> usize {
        let mut units = 0;
        for (ix, ch) in self.text.char_indices() {
            if units >= utf16 {
                return ix;
            }
            units += ch.len_utf16();
        }
        self.text.len()
    }

    fn to_utf16(&self, utf8: usize) -> usize {
        self.text[..utf8].chars().map(char::len_utf16).sum()
    }
}

impl EntityInputHandler for Line {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.to_utf8(range.start)..self.to_utf8(range.end);
        adjusted.replace(self.to_utf16(range.start)..self.to_utf16(range.end));
        Some(self.text[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self.to_utf16(self.text.len());
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|r| self.to_utf16(r.start)..self.to_utf16(r.end))
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
            .map(|r| self.to_utf8(r.start)..self.to_utf8(r.end))
            .or(self.marked.take())
            .unwrap_or(self.text.len()..self.text.len());
        self.text.replace_range(range, text);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|r| self.to_utf8(r.start)..self.to_utf8(r.end))
            .or(self.marked.clone())
            .unwrap_or(self.text.len()..self.text.len());
        self.text.replace_range(range.clone(), text);
        self.marked = (!text.is_empty()).then(|| range.start..range.start + text.len());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let start = layout.x_for_index(self.to_utf8(range.start));
        let end = layout.x_for_index(self.to_utf8(range.end));
        Some(Bounds::from_corners(
            point(bounds.left() + start, bounds.top()),
            point(bounds.left() + end, bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        Some(self.to_utf16(layout.closest_index_for_x(point.x)))
    }
}

struct LineElement(Entity<Line>);

impl IntoElement for LineElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for LineElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

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
        style.size.height = px(40.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let theme = cx.theme().clone();
        let line = self.0.read(cx);
        let text: SharedString = line.text.clone().into();
        let mut font = theme.document_font();
        if line.bold {
            font = font.bold();
        }
        let run = TextRun {
            len: text.len(),
            font,
            color: theme.colors.ink,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped = window.text_system().shape_line(text, px(24.), &[run], None);
        let focus = line.focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.0.clone()), cx);
        shaped.paint(bounds.origin, px(40.), window, cx).unwrap();
        let caret_x = bounds.left() + shaped.width;
        window.paint_quad(fill(
            Bounds::new(point(caret_x, bounds.top() + px(6.)), size(px(2.), px(28.))),
            theme.colors.caret,
        ));
        self.0.update(cx, |line, _| line.layout = Some(shaped));
    }
}

impl Render for Line {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        div()
            .key_context("Line")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::toggle_bold))
            .size_full()
            .bg(colors.canvas)
            .pt(theme::TITLEBAR_HEIGHT + theme::SPACE_8)
            .px(theme::SPACE_16)
            .child(
                div()
                    .border_b_1()
                    .border_color(colors.hairline)
                    .child(LineElement(cx.entity())),
            )
    }
}

fn main() {
    let script = automation::Script::from_env().expect("valid THE_WRITER_AUTOMATION");
    Application::new().run(move |cx: &mut App| {
        writer_app::init(cx);
        cx.bind_keys([
            KeyBinding::new("backspace", Backspace, Some("Line")),
            KeyBinding::new("cmd-b", ToggleBold, Some("Line")),
        ]);
        let options = main_window_options(None, cx);
        let window = cx
            .open_window(options, |window, cx| {
                theme::observe_system_appearance(window, cx).detach();
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus);
                    Line {
                        focus,
                        text: String::new(),
                        marked: None,
                        bold: false,
                        layout: None,
                    }
                })
            })
            .expect("open window");
        cx.activate(true);
        if let Some(script) = script {
            automation::run(script, window.into(), cx);
        }
    });
}
