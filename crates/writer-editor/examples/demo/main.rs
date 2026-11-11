//! Editor demo: one document with every block kind and inline attribute, in
//! a real window, for visual QA of the editor crate.
//!
//! ```bash
//! cargo run -p writer-editor --example demo
//! ```
//!
//! Besides the editor's own keys (see `writer_editor::actions`), the demo
//! binds a few switches so every seam can be exercised from the keyboard
//! and from an automation script:
//!
//! | Keys | Effect |
//! |---|---|
//! | `cmd-alt-t` | Paper / Night |
//! | `cmd-alt-p` | flow view / page view |
//! | `cmd-alt-c` | show / hide comment tints |
//! | `cmd-alt-]` | make the next comment thread the active one |
//! | `cmd-alt-r` | read-only on / off |
//! | `cmd-alt-k` | caret style: bar, block, underline |
//! | `cmd-alt-h` | highlight layer on every "the" / clear it |
//! | `cmd-alt-u` | popup at the caret / hide it |
//! | `cmd-alt-i` | intercept keys: `x` is swallowed while on |
//! | `cmd-1` / `cmd-2` | focus the editor / the compact composer |
//! | `cmd-q` | quit |
//!
//! The line under the editor shows the statistics, the selection, the page
//! and the last event; the compact editor below it is a comment composer
//! (enter submits, shift-enter breaks the line).
//!
//! Visual QA (see `docs/gpui-notes.md`): build, then run the binary with
//! `THE_WRITER_AUTOMATION`, e.g.
//!
//! ```bash
//! cargo build -p writer-editor --example demo
//! QA=$(mktemp -d)
//! THE_WRITER_AUTOMATION="wait:800; theme:light; screenshot:$QA/paper.png; keys:cmd-down; type:Hello; screenshot:$QA/typed.png; quit" \
//!   ./target/debug/examples/demo
//! ```
//!
//! Steps: `wait:<ms>`, `keys:<keystrokes>`, `type:<text>`,
//! `theme:<light|dark|system>`, `screenshot:<path>`, `log:<text>`, `quit`.

mod automation;

use gpui::{
    App, Application, Bounds, Context, Entity, FocusHandle, Focusable, KeyBinding, SharedString,
    Subscription, Window, WindowBounds, WindowOptions, actions, div, prelude::*, px, size,
};
use writer_core::{
    Alignment, Block, BlockKind, Document, EditorState, Highlight, InlineAttrs, InlineStyle,
    Position, Rgb, SearchOptions, TextRange,
};
use writer_editor::{
    CaretStyle, CompactOptions, Editor, EditorConfig, EditorEvent, HighlightStyle,
    InterceptPriority, KeyDisposition, PopupAnchor, PopupPlacement,
};
use writer_ui::theme::{self, ActiveTheme, ThemeMode};

actions!(
    demo,
    [
        ToggleTheme,
        TogglePageView,
        ToggleComments,
        NextComment,
        ToggleReadOnly,
        CycleCaret,
        ToggleHighlights,
        TogglePopup,
        ToggleIntercept,
        FocusEditor,
        FocusComposer,
        Quit,
    ]
);

/// A document that contains every block kind and every inline attribute.
fn sample_document() -> Document {
    let plain = InlineAttrs::default;
    let style = |style: InlineStyle| InlineAttrs::default().with(style);
    let blocks = vec![
        Block::with_text(BlockKind::heading(1), "The Writer editor demo"),
        Block::new(BlockKind::Paragraph)
            .with_span("Inline attributes: ", plain())
            .with_span("bold", style(InlineStyle::Bold))
            .with_span(", ", plain())
            .with_span("italic", style(InlineStyle::Italic))
            .with_span(", ", plain())
            .with_span(
                "bold italic",
                style(InlineStyle::Bold).with(InlineStyle::Italic),
            )
            .with_span(", ", plain())
            .with_span("underline", style(InlineStyle::Underline))
            .with_span(", ", plain())
            .with_span("strikethrough", style(InlineStyle::Strikethrough))
            .with_span(", ", plain())
            .with_span("inline code", style(InlineStyle::Code))
            .with_span(", ", plain())
            .with_span(
                "highlight",
                InlineAttrs {
                    highlight: Some(Highlight::Yellow),
                    ..plain()
                },
            )
            .with_span(", ", plain())
            .with_span(
                "green highlight",
                InlineAttrs {
                    highlight: Some(Highlight::Green),
                    ..plain()
                },
            )
            .with_span(", ", plain())
            .with_span(
                "coloured text",
                InlineAttrs {
                    color: Some(Rgb::new(0x2f, 0x6f, 0xb0)),
                    ..plain()
                },
            )
            .with_span(", ", plain())
            .with_span(
                "another font",
                InlineAttrs {
                    font: Some("Helvetica Neue".into()),
                    ..plain()
                },
            )
            .with_span(", and ", plain())
            .with_span(
                "a link",
                InlineAttrs {
                    link: Some("https://example.com".into()),
                    ..plain()
                },
            )
            .with_span(".", plain()),
        Block::paragraph(
            "A long paragraph to show wrapping. The quick brown fox jumps over the lazy dog, \
             and then the dog, being lazy, does nothing at all about it, which is exactly what \
             the fox was counting on when it planned the jump in the first place. This sentence \
             carries a comment anchor.",
        ),
        Block::with_text(BlockKind::heading(2), "Heading 2"),
        Block::with_text(BlockKind::heading(3), "Heading 3"),
        Block::with_text(BlockKind::heading(4), "Heading 4"),
        Block::with_text(BlockKind::heading(5), "Heading 5"),
        Block::with_text(BlockKind::heading(6), "Heading 6"),
        Block::with_text(BlockKind::BulletItem, "A bullet item"),
        Block::with_text(BlockKind::BulletItem, "A nested bullet item").with_indent(1),
        Block::with_text(BlockKind::NumberedItem, "First numbered item"),
        Block::with_text(BlockKind::NumberedItem, "Second numbered item"),
        Block::with_text(BlockKind::NumberedItem, "A nested numbered item").with_indent(1),
        Block::with_text(BlockKind::todo(), "An open to-do"),
        Block::with_text(BlockKind::TodoItem { checked: true }, "A finished to-do"),
        Block::with_text(
            BlockKind::Quote,
            "A quote: the chrome is quiet so the text is the loudest thing on screen.",
        ),
        Block::with_text(
            BlockKind::code(),
            "fn main() {\n    println!(\"a code block is one block with several lines\");\n}",
        ),
        Block::new(BlockKind::Divider),
        Block::paragraph("Centred paragraph.").with_alignment(Alignment::Center),
        Block::paragraph("Right-aligned paragraph.").with_alignment(Alignment::Right),
        Block::paragraph(
            "Justified paragraph: enough words are needed here for the text to wrap over more \
             than one row, otherwise there is nothing to justify at all.",
        )
        .with_alignment(Alignment::Justify),
        Block::paragraph("A paragraph with a larger block text size.").with_text_size(Some(24.)),
        Block::paragraph("A soft line break follows\nand this is the second line of the block."),
        Block::new(BlockKind::PageBreak),
        Block::paragraph("Text after the page break. Unicode: naïve café, 日本語, emoji 👩‍👩‍👧."),
    ];
    // Comment anchors only exist together with their threads, so they are
    // added through the engine.
    let mut state = EditorState::new(Document::from_blocks(blocks));
    for (query, body) in [
        ("This sentence carries a comment anchor.", "Tighten this."),
        ("the lazy dog", "Which dog?"),
    ] {
        if let Some(range) = state
            .document()
            .find_all(query, SearchOptions::default())
            .first()
            .copied()
        {
            state.select_range(range);
            state.add_comment(body, "Demo", 0);
        }
    }
    state.set_caret(Position::ZERO);
    state.into_document()
}

/// The popup shown by `cmd-alt-u`: any view works.
struct PopupCard;

impl Render for PopupCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .occlude()
            .w(px(220.))
            .p_2()
            .rounded(theme::RADIUS_LG)
            .border_1()
            .border_color(theme.colors.hairline)
            .bg(theme.colors.elevated)
            .shadow(theme.shadow_popover())
            .font_family(theme.fonts.ui.clone())
            .text_size(theme::UI_TEXT_BASE)
            .text_color(theme.colors.ink)
            .child("A popup anchored at the caret")
            .child(
                div()
                    .text_color(theme.colors.muted)
                    .child("It follows the caret and scrolling."),
            )
    }
}

struct Demo {
    focus_handle: FocusHandle,
    editor: Entity<Editor>,
    composer: Entity<Editor>,
    popup: Entity<PopupCard>,
    interceptor: Option<Subscription>,
    last_event: SharedString,
    submitted: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl Demo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            Editor::new(
                sample_document(),
                EditorConfig::full().placeholder("Start writing"),
                window,
                cx,
            )
        });
        let composer = cx.new(|cx| {
            Editor::new(
                Document::new(),
                EditorConfig::compact(CompactOptions::default().max_height(px(96.)))
                    .placeholder("Add a comment (enter submits, shift-enter for a new line)"),
                window,
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe(&editor, |this: &mut Self, _, event: &EditorEvent, cx| {
                this.last_event = match event {
                    EditorEvent::Edited { change } => {
                        format!("Edited (revision {})", change.revision).into()
                    }
                    other => format!("{other:?}").into(),
                };
                cx.notify();
            }),
            cx.subscribe(&composer, |this: &mut Self, composer, event, cx| {
                if matches!(event, EditorEvent::Submitted) {
                    let text = composer.read(cx).plain_text();
                    if !text.trim().is_empty() {
                        this.submitted.push(text);
                    }
                    composer.update(cx, |composer, cx| composer.clear(cx));
                }
                cx.notify();
            }),
        ];
        Self {
            focus_handle: cx.focus_handle(),
            editor,
            composer,
            popup: cx.new(|_| PopupCard),
            interceptor: None,
            last_event: "none".into(),
            submitted: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    fn toggle_theme(&mut self, _: &ToggleTheme, _: &mut Window, cx: &mut Context<Self>) {
        let mode = if cx.theme().is_dark() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        theme::set_theme_mode(mode, cx);
    }

    fn toggle_page_view(&mut self, _: &TogglePageView, _: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.toggle_page_view(cx));
    }

    fn toggle_comments(&mut self, _: &ToggleComments, _: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            let visible = editor.config().comments_visible;
            editor.set_comments_visible(!visible, cx);
        });
    }

    fn next_comment(&mut self, _: &NextComment, _: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            let from = editor.selection().head;
            if let Some((id, range)) = editor.document().next_comment(from, true) {
                editor.set_active_comment(Some(id), cx);
                editor.edit(|state| state.set_caret(range.start), cx);
            }
        });
    }

    fn toggle_read_only(&mut self, _: &ToggleReadOnly, _: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            let read_only = editor.is_read_only();
            editor.set_read_only(!read_only, cx);
        });
    }

    fn cycle_caret(&mut self, _: &CycleCaret, _: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            let next = match editor.config().caret_style {
                CaretStyle::Bar => CaretStyle::Block,
                CaretStyle::Block => CaretStyle::Underline,
                CaretStyle::Underline => CaretStyle::Bar,
            };
            editor.set_caret_style(next, cx);
        });
    }

    fn toggle_highlights(&mut self, _: &ToggleHighlights, _: &mut Window, cx: &mut Context<Self>) {
        let colors = cx.theme().colors;
        self.editor.update(cx, |editor, cx| {
            if !editor.highlights("find").is_empty() {
                editor.clear_highlights("find", cx);
                editor.clear_highlights("find-current", cx);
                return;
            }
            let matches: Vec<TextRange> =
                editor.document().find_all("the", SearchOptions::default());
            let current = matches.first().copied();
            editor.set_highlights(
                "find",
                matches,
                HighlightStyle::background(colors.find_match),
                cx,
            );
            editor.set_highlights(
                "find-current",
                current.into_iter().collect(),
                HighlightStyle::background(colors.find_match_current).underline(colors.accent),
                cx,
            );
        });
    }

    fn toggle_popup(&mut self, _: &TogglePopup, _: &mut Window, cx: &mut Context<Self>) {
        let popup = self.popup.clone();
        self.editor.update(cx, |editor, cx| {
            if editor.has_popup() {
                editor.hide_popup(cx);
            } else {
                editor.show_popup(popup.into(), PopupAnchor::Caret, PopupPlacement::Below, cx);
            }
        });
    }

    fn toggle_intercept(&mut self, _: &ToggleIntercept, _: &mut Window, cx: &mut Context<Self>) {
        if self.interceptor.take().is_none() {
            self.interceptor = Some(self.editor.read(cx).intercept_keys(
                InterceptPriority::DEFAULT,
                |keystroke, _, _| {
                    if keystroke.key == "x" && !keystroke.modifiers.modified() {
                        KeyDisposition::Consumed
                    } else {
                        KeyDisposition::Pass
                    }
                },
            ));
        }
        cx.notify();
    }

    fn focus_editor(&mut self, _: &FocusEditor, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.read(cx).focus(window);
    }

    fn focus_composer(&mut self, _: &FocusComposer, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.read(cx).focus(window);
    }

    fn status(&self, cx: &App) -> String {
        let editor = self.editor.read(cx);
        let stats = editor.stats();
        let selection = editor.selection();
        let mut status = format!(
            "{} words, {} characters, {} min  |  caret {}:{}",
            stats.words,
            stats.characters,
            stats.reading_minutes,
            selection.head.block,
            selection.head.offset,
        );
        if let Some(selected) = editor.selection_stats() {
            status.push_str(&format!("  |  {} words selected", selected.words));
        }
        if let (Some(page), Some(pages)) = (editor.caret_page(), editor.page_count()) {
            status.push_str(&format!("  |  page {page} of {pages}"));
        }
        if editor.is_read_only() {
            status.push_str("  |  read-only");
        }
        if self.interceptor.is_some() {
            status.push_str("  |  intercepting x");
        }
        status.push_str(&format!("  |  last event: {}", self.last_event));
        status
    }
}

impl Focusable for Demo {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Demo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let status = self.status(cx);
        let submitted = self.submitted.last().cloned();
        div()
            .key_context("Demo")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::toggle_theme))
            .on_action(cx.listener(Self::toggle_page_view))
            .on_action(cx.listener(Self::toggle_comments))
            .on_action(cx.listener(Self::next_comment))
            .on_action(cx.listener(Self::toggle_read_only))
            .on_action(cx.listener(Self::cycle_caret))
            .on_action(cx.listener(Self::toggle_highlights))
            .on_action(cx.listener(Self::toggle_popup))
            .on_action(cx.listener(Self::toggle_intercept))
            .on_action(cx.listener(Self::focus_editor))
            .on_action(cx.listener(Self::focus_composer))
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.canvas)
            .font_family(theme.fonts.ui.clone())
            .text_size(theme::UI_TEXT_SM)
            .text_color(colors.muted)
            // Titlebar band (the window content draws under the titlebar).
            .child(
                div()
                    .h(theme::TITLEBAR_HEIGHT)
                    .flex_none()
                    .bg(colors.sidebar)
                    .border_b_1()
                    .border_color(colors.hairline),
            )
            // The document editor fills the rest.
            .child(div().flex_1().min_h_0().child(self.editor.clone()))
            // Status line.
            .child(
                div()
                    .flex_none()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(colors.hairline)
                    .child(status),
            )
            // Compact composer.
            .child(
                div()
                    .flex_none()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(colors.hairline)
                    .bg(colors.sidebar)
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .w(px(420.))
                            .px_2()
                            .py_1()
                            .rounded(theme::RADIUS_MD)
                            .border_1()
                            .border_color(colors.border_strong)
                            .bg(colors.canvas)
                            .child(self.composer.clone()),
                    )
                    .children(submitted.map(|text| format!("Last submitted: {text}"))),
            )
    }
}

fn main() {
    Application::new()
        .with_assets(writer_ui::assets::Assets)
        .run(|cx: &mut App| {
            theme::init(ThemeMode::System, cx);
            cx.bind_keys([
                KeyBinding::new("cmd-alt-t", ToggleTheme, None),
                KeyBinding::new("cmd-alt-p", TogglePageView, None),
                KeyBinding::new("cmd-alt-c", ToggleComments, None),
                KeyBinding::new("cmd-alt-]", NextComment, None),
                KeyBinding::new("cmd-alt-r", ToggleReadOnly, None),
                KeyBinding::new("cmd-alt-k", CycleCaret, None),
                KeyBinding::new("cmd-alt-h", ToggleHighlights, None),
                KeyBinding::new("cmd-alt-u", TogglePopup, None),
                KeyBinding::new("cmd-alt-i", ToggleIntercept, None),
                KeyBinding::new("cmd-1", FocusEditor, None),
                KeyBinding::new("cmd-2", FocusComposer, None),
                KeyBinding::new("cmd-q", Quit, None),
            ]);
            writer_ui::init(cx);
            writer_editor::init(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let bounds = Bounds::centered(None, size(px(1100.), px(900.)), cx);
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        titlebar: Some(gpui::TitlebarOptions {
                            title: Some("writer-editor demo".into()),
                            appears_transparent: true,
                            traffic_light_position: Some(gpui::point(
                                theme::TRAFFIC_LIGHT_X,
                                theme::TRAFFIC_LIGHT_Y,
                            )),
                        }),
                        ..Default::default()
                    },
                    |window, cx| {
                        theme::observe_system_appearance(window, cx).detach();
                        let demo = cx.new(|cx| Demo::new(window, cx));
                        demo.read(cx).editor.read(cx).focus(window);
                        demo
                    },
                )
                .expect("open demo window");
            cx.activate(true);
            automation::run_from_env(window.into(), cx);
        });
}
