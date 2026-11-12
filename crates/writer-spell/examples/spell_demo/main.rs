//! The spell checker over the Mac's real system checker, in a window: an
//! editor with a document in several languages, the real text context
//! menu with the suggestions on top, and a status line.
//!
//! ```text
//! cargo run -p writer-spell --example spell_demo
//! ```
//!
//! Keys: `cmd-;` / `cmd-shift-;` next and previous misspelling,
//! `ctrl-enter` the context menu (suggestions first on a marked word),
//! `cmd-alt-g` the suggestions alone, `cmd-alt-l` the next language
//! (Automatic, the user's first language, then a few fixed ones),
//! `cmd-alt-s` checking on and off, `cmd-alt-t` the theme, `cmd-q` quit.
//!
//! A scripted run (`THE_WRITER_AUTOMATION`, see `automation.rs`) stays in
//! the background and never learns a word: the menu's "Learn Spelling" is
//! not chosen by any script in this repository.

mod automation;

use std::rc::Rc;
use std::time::Instant;

use gpui::{
    App, AppContext as _, Application, Bounds, Context, Entity, InteractiveElement, IntoElement,
    KeyBinding, ParentElement, Render, Styled, Window, WindowBounds, WindowOptions, actions, div,
    px, size,
};
use writer_core::{Block, BlockKind, Document, InlineAttrs};
use writer_editor::{Editor, EditorConfig};
use writer_editor_tools::EditorTools;
use writer_spell::{
    SpellChecker, SpellEvent, SpellLanguage, SpellService, SystemSpellService, language_name,
};
use writer_ui::theme::{self, ActiveTheme as _, ThemeMode};
use writer_ui::toast::ToastLayer;

actions!(spell_demo, [ToggleTheme, NextLanguage, Quit]);

/// The demo document: a few languages, code, a short line.
fn document() -> Document {
    let code = InlineAttrs {
        code: true,
        ..InlineAttrs::default()
    };
    let bold = InlineAttrs {
        bold: true,
        ..InlineAttrs::default()
    };
    Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), "Spelling, checked by the Mac"),
        Block::paragraph(
            "Ths is a sentense with teh mispelled words, a URL https://example.com, and a name \
             like Eraldo. The harbour was a plain grey fact and nobody owed the town anything.",
        ),
        Block::paragraph("A word that is ")
            .with_span("partly bo", bold.clone())
            .with_span("ld: a ", InlineAttrs::default())
            .with_span("sent", bold)
            .with_span("ense. Inline code is left alone: ", InlineAttrs::default())
            .with_span("let teh_wrod = mispelled();", code)
            .with_span(" and the text goes on with a wrod.", InlineAttrs::default()),
        Block::with_text(BlockKind::heading(2), "Chaptr two"),
        Block::paragraph("Le batau est arrivé au port ce matin et persone ne l'attendait."),
        Block::paragraph("Das Schif kam heute Morgen im Hafen an und niemand hat es erwartet."),
        Block::paragraph("El barco llegó al puerto esta manana y nadie lo esperava."),
        Block::paragraph("La nave è arivata al porto stamattina e nessuno la aspettava."),
        Block::with_text(
            BlockKind::code(),
            "fn mian() {\n    // a code blok is not checked: teh, wrod, mispelled\n}",
        ),
        Block::with_text(
            BlockKind::Quote,
            "A quoet with an eror in it, 😀 and an emoji before the mistaek.",
        ),
        Block::with_text(BlockKind::todo(), "Buy mlik"),
        Block::paragraph("Ths sentense"),
        Block::paragraph(""),
    ])
}

/// A book for measuring: `blocks` paragraphs of about sixty words, a
/// misspelled word in every seventh.
pub fn book(blocks: usize) -> Document {
    let sentences = [
        "The boats came in before the weather turned, and the harbour went quiet.",
        "Nobody on the quay had expected them back so early in the season.",
        "She counted the crates twice, wrote the number down, and counted them again.",
        "By evening the wind had dropped and the lamps along the wall were lit.",
        "He said nothing about the letter, and she did not ask him what it said.",
    ];
    Document::from_blocks(
        (0..blocks)
            .map(|index| {
                let mut text = format!("Paragraph {index}. ");
                for step in 0..5 {
                    text.push_str(sentences[(index + step) % sentences.len()]);
                    text.push(' ');
                }
                if index % 7 == 0 {
                    text.push_str("One wrod here is mispelled.");
                }
                Block::paragraph(text.trim_end())
            })
            .collect(),
    )
}

pub struct Demo {
    pub editor: Entity<Editor>,
    tools: Entity<EditorTools>,
    pub checker: Entity<SpellChecker>,
    toasts: Entity<ToastLayer>,
    service: Rc<SystemSpellService>,
    languages: Vec<SpellLanguage>,
    language: usize,
    last_event: String,
}

impl Demo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let toasts = cx.new(|_| ToastLayer::new());
        ToastLayer::set_global(&toasts, cx);
        let editor = cx.new(|cx| Editor::new(document(), EditorConfig::full(), window, cx));
        let tools = cx.new(|cx| EditorTools::new(editor.clone(), window, cx));

        let service = Rc::new(SystemSpellService::new());
        let dyn_service: Rc<dyn SpellService> = service.clone();
        let checker = cx.new(|cx| SpellChecker::new(editor.clone(), dyn_service, cx));
        // The suggestions lead the text menu, as in the app.
        tools.update(cx, |tools, _| {
            let checker = checker.clone();
            tools.set_text_menu_leading_extension(move |request, _, cx| {
                checker.update(cx, |checker, cx| checker.context_menu_items(request, cx))
            });
        });
        cx.subscribe(&checker, |this, _, event: &SpellEvent, cx| {
            if !matches!(event, SpellEvent::MarksChanged) {
                this.last_event = format!("{event:?}");
            }
            cx.notify();
        })
        .detach();

        let mut languages = vec![SpellLanguage::Automatic];
        for id in service.preferred_languages().into_iter().take(1).chain([
            "en".to_string(),
            "de".to_string(),
            "fr".to_string(),
        ]) {
            let language = SpellLanguage::Fixed(id);
            if !languages.contains(&language) {
                languages.push(language);
            }
        }
        Demo {
            editor,
            tools,
            checker,
            toasts,
            service,
            languages,
            language: 0,
            last_event: String::new(),
        }
    }

    pub fn set_language(&mut self, language: SpellLanguage, cx: &mut Context<Self>) {
        if let Some(index) = self.languages.iter().position(|known| *known == language) {
            self.language = index;
        } else {
            self.languages.push(language.clone());
            self.language = self.languages.len() - 1;
        }
        self.checker
            .update(cx, |checker, cx| checker.set_language(language, cx));
        cx.notify();
    }

    fn next_language(&mut self, _: &NextLanguage, _: &mut Window, cx: &mut Context<Self>) {
        let next = self.languages[(self.language + 1) % self.languages.len()].clone();
        self.set_language(next, cx);
    }

    fn toggle_theme(&mut self, _: &ToggleTheme, _: &mut Window, cx: &mut Context<Self>) {
        let mode = if cx.theme().is_dark() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        theme::set_theme_mode(mode, cx);
    }

    fn language_label(&self, cx: &App) -> String {
        match self.checker.read(cx).language() {
            SpellLanguage::Automatic => "Automatic".to_string(),
            SpellLanguage::Fixed(id) => format!("{} ({id})", language_name(&*self.service, id)),
        }
    }

    /// One line about the checker, for the status line and scripted runs.
    pub fn state_line(&self, menu: Option<Vec<String>>, cx: &App) -> String {
        let checker = self.checker.read(cx);
        let editor = self.editor.read(cx);
        let selection = editor.selection();
        let selected: String = editor
            .document()
            .text_in(selection.range())
            .chars()
            .take(40)
            .collect();
        let (slices, longest) = checker.slice_stats();
        format!(
            "language {} enabled {} marks {} pending {} settled {} slices {slices} longest {longest:?} \
             selection {}:{}-{}:{} selected {selected:?} blocks {} menu {menu:?} notice {:?}",
            self.language_label(cx),
            checker.is_enabled(),
            checker.misspelling_count(cx),
            checker.pending_blocks(cx),
            checker.is_settled(cx),
            selection.anchor.block,
            selection.anchor.offset,
            selection.head.block,
            selection.head.offset,
            editor.document().block_count(),
            checker.last_notice(),
        )
    }
}

impl Render for Demo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let checker = self.checker.read(cx);
        let status = format!(
            "{}  ·  {}  ·  {} marked{}",
            self.language_label(cx),
            if checker.is_enabled() {
                "checking while typing"
            } else {
                "checking is off"
            },
            checker.misspelling_count(cx),
            if checker.is_settled(cx) || !checker.is_enabled() {
                String::new()
            } else {
                "  ·  checking the document".to_string()
            },
        );
        let hints = "⌘;  next    ⇧⌘;  previous    ⌃↩  menu    ⌥⌘G  suggestions    ⌥⌘L  language    ⌥⌘S  on / off    ⌥⌘T  theme";
        writer_spell::handle_actions(div(), &self.checker)
            .on_action(cx.listener(Self::toggle_theme))
            .on_action(cx.listener(Self::next_language))
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.canvas)
            .font_family(theme.fonts.ui.clone())
            .text_size(theme::UI_TEXT_SM)
            .text_color(colors.muted)
            // The window content draws under the titlebar.
            .child(div().h(px(36.)).flex_none())
            .child(div().flex_1().min_h_0().child(self.tools.clone()))
            .child(
                div()
                    .flex_none()
                    .h(px(28.))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(colors.hairline)
                    .child(status)
                    .child(div().text_color(colors.faint).child(hints)),
            )
            .child(self.toasts.clone())
    }
}

fn main() {
    Application::new()
        .with_assets(writer_ui::assets::Assets)
        .run(|cx: &mut App| {
            theme::init(ThemeMode::System, cx);
            cx.bind_keys([
                KeyBinding::new("cmd-alt-t", ToggleTheme, None),
                KeyBinding::new("cmd-alt-l", NextLanguage, None),
                KeyBinding::new(
                    "cmd-alt-s",
                    writer_spell::actions::ToggleSpellChecking,
                    None,
                ),
                KeyBinding::new("cmd-alt-g", writer_spell::actions::ShowSuggestions, None),
                KeyBinding::new("cmd-q", Quit, None),
            ]);
            writer_ui::init(cx);
            writer_editor::init(cx);
            writer_editor_tools::init(cx);
            writer_spell::init(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            // What the first use of the system checker costs here, in a
            // running app (asked for by a scripted run that measures).
            if std::env::var_os("SPELL_DEMO_WARM_UP").is_some() {
                let service = SystemSpellService::new();
                let started = Instant::now();
                service.warm_up();
                eprintln!(
                    "[spell_demo] warm_up on the main thread: {:?}",
                    started.elapsed()
                );
                let started = Instant::now();
                service.warm_up();
                eprintln!("[spell_demo] warm_up again: {:?}", started.elapsed());
            }

            let bounds = Bounds::centered(None, size(px(1000.), px(860.)), cx);
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        titlebar: Some(gpui::TitlebarOptions {
                            title: Some("writer-spell demo".into()),
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
            // A scripted run stays in the background: it must not take the
            // keyboard from whoever is using the machine.
            if std::env::var_os(automation::ENV).is_none() {
                cx.activate(true);
            }
            automation::run_from_env(window.into(), cx);
        });
}
