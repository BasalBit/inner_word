//! The side chat beside a document, on the fake backend: for looking at it
//! in Paper and Night without the app and without an agent. Nothing here
//! talks to a real agent.
//!
//! ```bash
//! cargo run -p writer-assistant-ui --example assistant
//! ```
//!
//! Keys: `cmd-3` the side chat, `cmd-1` the text, `cmd-t` the theme,
//! `cmd-q` quit. Every message is answered from a small script of answers
//! (formatted text, a tool call, a long answer), in turn.
//!
//! Scripted (the window stays in the background):
//!
//! ```bash
//! THE_WRITER_AUTOMATION="wait:800; keys:cmd-3; type:Where does it drag?; keys:enter; \
//!   demo:advance 6; screenshot:/some/tmp/streaming.png; demo:finish; \
//!   theme:dark; screenshot:/some/tmp/night.png; quit" ./target/debug/examples/assistant
//! ```
//!
//! `demo:` steps: `hold` (answers arrive only when told), `advance <n>`,
//! `finish`, `release`, `fail <limit|signin|network|exit>` (the next answer
//! fails), `signout` / `signin` / `missing` (what "Check Again" finds, and
//! what the panel is told now), `privacy`, `notice <text>`, `doc <1|2>`
//! (another document), `attach` (two library items), `selection <on|off>`,
//! `fill <n>` (n exchanges at once), `state` (one line about the panel).

mod automation;

use std::sync::Arc;

use gpui::{
    App, Application, Bounds, Context, Entity, KeyBinding, Render, Window, WindowBounds,
    WindowOptions, actions, div, prelude::*, px, size,
};
use writer_assistant::{
    AssistantError, Attachment, Backend, FakeBackend, FakeReply, OpenDocument, SignIn, Transcript,
};
use writer_assistant_ui::{
    ApplyKind, AssistantPanel, AssistantPanelEvent, AssistantPanelOptions, DocumentRef,
};
use writer_core::{Block, BlockKind, Document};
use writer_editor::{Editor, EditorConfig};
use writer_export::{MarkdownImportOptions, from_markdown};
use writer_ui::{
    Toast, ToastLayer,
    theme::{self, ActiveTheme, TITLEBAR_HEIGHT, ThemeMode},
};

actions!(assistant_demo, [FocusChat, FocusText, ToggleTheme, Quit]);

const PANEL_WIDTH: f32 = 380.;

const ANSWERS: [&str; 3] = [
    "The middle of the chapter loses pace in two places.\n\n\
     ## The harbour scene\n\n\
     Mara waits for the ferry for *four paragraphs* before anything happens. The \
     description is good, but it says the same thing three times:\n\n\
     - the water is grey\n\
     - the gulls are loud\n\
     - she is cold\n\n\
     Cut it to one paragraph and let the **ferryman's first line** arrive sooner.\n\n\
     ## The letter\n\n\
     > She read it once, and then again, and then a third time.\n\n\
     One reading is enough for the reader. Try something like `She read it once.` and \
     move straight to what she does next.\n\n\
     1. Shorten the wait.\n\
     2. Read the letter once.\n\
     3. End the scene on the door closing.",
    "A shorter version:\n\n\
     Mara stood at the rail until the ferry was a smudge on the water. Then she \
     turned up her collar and went to find the ferryman.",
    "Three words that carry the same weight: **weary**, *spent*, and worn out. \
     \"Weary\" keeps the rhythm of your sentence best.",
];

struct Demo {
    editor: Entity<Editor>,
    panel: Entity<AssistantPanel>,
    toasts: Entity<ToastLayer>,
    fake: FakeBackend,
    panel_open: bool,
    scripted: bool,
    document: usize,
    stored: [Transcript; 2],
    said: Vec<String>,
    next_answer: usize,
}

fn chapter(number: usize) -> Document {
    let (title, lines): (&str, [&str; 4]) = if number == 0 {
        (
            "The Harbour",
            [
                "Mara came down to the harbour before the light did. The water was grey, \
                 the gulls were loud, and the cold had got into her gloves.",
                "She waited. The ferry was late, as it always was in the first week of \
                 the month, and nobody on the quay seemed to mind but her.",
                "When the letter came it was the ferryman who brought it, folded twice, \
                 the paper soft from his pocket. She read it once, and then again, and \
                 then a third time.",
                "Then she turned up her collar and went to find out who had sent it.",
            ],
        )
    } else {
        (
            "The Letter",
            [
                "It was not a long letter. Four lines, a name she did not know, and a \
                 street in a town two days north.",
                "She put it in the tin with the others and shut the lid on it.",
                "By evening she had packed.",
                "The door closed behind her more quietly than she had meant it to.",
            ],
        )
    };
    let mut blocks = vec![Block::with_text(BlockKind::heading(1), title)];
    blocks.extend(lines.iter().map(|line| Block::paragraph(line)));
    Document::from_blocks(blocks)
}

fn document_ref(number: usize) -> DocumentRef {
    DocumentRef::new(
        number as i64 + 1,
        if number == 0 {
            "The Harbour"
        } else {
            "The Letter"
        },
    )
}

impl Demo {
    fn new(scripted: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let fake = FakeBackend::new();
        let mut info = fake.info();
        info.name = "Claude Code".into();
        info.vendor = "Anthropic".into();
        info.sign_in_help = "Open Terminal, run claude and sign in.".into();
        info.install_help =
            "Install Claude Code, then open the settings of The Writer again.".into();
        fake.set_info(info);
        fake.set_sign_in(SignIn::SignedIn {
            account: Some("Claude Max plan".into()),
        });
        let mut catalog = fake.catalog();
        catalog.agent_default_label = "Claude Code's default".into();
        for (model, name) in catalog.models.iter_mut().zip(["Sonnet", "Opus"]) {
            model.name = name.into();
        }
        fake.set_catalog(catalog);

        let editor = cx.new(|cx| Editor::new(chapter(0), EditorConfig::full(), window, cx));
        let toasts = cx.new(|_| ToastLayer::new());
        ToastLayer::set_global(&toasts, cx);
        let backend: Arc<dyn Backend> = Arc::new(fake.clone());
        let panel = cx.new(|cx| {
            let mut panel = AssistantPanel::new(
                backend,
                AssistantPanelOptions {
                    availability: Some(fake.availability()),
                    ..AssistantPanelOptions::default()
                },
                window,
                cx,
            );
            panel.set_document_source({
                let editor = editor.downgrade();
                move |cx| {
                    let editor = editor.upgrade()?;
                    let document = editor.read(cx).document().clone();
                    Some(OpenDocument {
                        id: Some(1),
                        title: document
                            .blocks()
                            .first()
                            .map(|block| block.text().to_string())
                            .unwrap_or_default(),
                        path: "The Salt Road/Part one".into(),
                        markdown: document.plain_text(),
                    })
                }
            });
            panel.set_library_source(|_| {
                vec![
                    Attachment::folder(10, "The Salt Road", "The Salt Road"),
                    Attachment::folder(11, "Part one", "The Salt Road/Part one"),
                    Attachment::document(1, "The Harbour", "The Salt Road/Part one/The Harbour"),
                    Attachment::document(2, "The Letter", "The Salt Road/Part one/The Letter"),
                    Attachment::document(3, "Characters", "The Salt Road/Notes/Characters"),
                    Attachment::document(4, "Timeline", "The Salt Road/Notes/Timeline"),
                    Attachment::document(5, "A walk in March", "Blog/A walk in March"),
                    Attachment::folder(12, "Blog", "Blog"),
                ]
            });
            panel.set_document(Some(document_ref(0)), None, window, cx);
            panel
        });
        cx.subscribe_in(&panel, window, Self::panel_event).detach();
        cx.subscribe(&editor, |this: &mut Demo, editor, _, cx| {
            let has_selection = !editor.read(cx).selection().is_collapsed();
            this.panel.update(cx, |panel, cx| {
                panel.set_apply_state(true, has_selection, cx)
            });
        })
        .detach();
        Self {
            editor,
            panel,
            toasts,
            fake,
            panel_open: true,
            scripted,
            document: 0,
            stored: [Transcript::new(), Transcript::new()],
            said: Vec::new(),
            next_answer: 0,
        }
    }

    fn panel_event(
        &mut self,
        _: &Entity<AssistantPanel>,
        event: &AssistantPanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AssistantPanelEvent::Apply { kind, markdown, .. } => match kind {
                ApplyKind::InsertAtCaret | ApplyKind::ReplaceSelection => {
                    let fragment =
                        from_markdown(markdown, &MarkdownImportOptions::default()).fragment();
                    self.editor.update(cx, |editor, cx| {
                        editor.edit(|state| state.paste(&fragment), cx);
                    });
                    self.editor.read(cx).focus(window);
                    self.said.push(format!("{kind:?}"));
                }
                ApplyKind::Copy => {
                    // A scripted run leaves the clipboard of whoever is at
                    // the machine alone.
                    if !self.scripted {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(markdown.clone()));
                    }
                    self.toasts.update(cx, |toasts, cx| {
                        toasts.push(Toast::new("The answer was copied").success(), cx);
                    });
                    self.said.push("Copy".into());
                }
            },
            AssistantPanelEvent::CopyText(text) => {
                if !self.scripted {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
                }
                self.said.push("CopyText".into());
            }
            AssistantPanelEvent::TranscriptChanged {
                document,
                transcript,
            } => {
                if let Some(slot) = document
                    .and_then(|id| usize::try_from(id - 1).ok())
                    .and_then(|index| self.stored.get_mut(index))
                {
                    *slot = transcript.clone();
                }
            }
            AssistantPanelEvent::FocusTextRequested => self.editor.read(cx).focus(window),
            AssistantPanelEvent::CloseRequested => {
                self.panel_open = false;
                self.panel.update(cx, |panel, cx| {
                    panel.set_shown(false, cx);
                    panel.end_idle_processes(cx);
                });
                self.editor.read(cx).focus(window);
            }
            other => self.said.push(format!("{other:?}")),
        }
        cx.notify();
    }

    /// Scripts the answer to the next message.
    fn script_next(&mut self) {
        let answer = ANSWERS[self.next_answer % ANSWERS.len()];
        let reply = FakeReply::text(answer);
        self.fake
            .push(if self.next_answer.is_multiple_of(ANSWERS.len()) {
                reply.with_tool("read_document", "Reading The Harbour")
            } else {
                reply
            });
        self.next_answer += 1;
    }

    /// A `demo:` step of a scripted run.
    fn command(&mut self, command: &str, window: &mut Window, cx: &mut Context<Self>) -> String {
        let (name, arg) = command.split_once(' ').unwrap_or((command, ""));
        let arg = arg.trim();
        match name {
            "hold" => self.fake.hold(),
            "advance" => {
                let count = self.fake.advance(arg.parse().unwrap_or(1));
                return format!("{count} events");
            }
            "finish" => {
                self.fake.finish();
            }
            "release" => self.fake.release(),
            "fail" => {
                let error = match arg {
                    "limit" => AssistantError::UsageLimit {
                        resets_at: None,
                        detail: "Your limit resets at 6pm.".into(),
                    },
                    "signin" => AssistantError::NotSignedIn,
                    "network" => AssistantError::Network {
                        detail: "offline".into(),
                    },
                    _ => AssistantError::Exited {
                        code: Some(1),
                        detail: "killed".into(),
                    },
                };
                self.fake.fail(error);
                return "the next answer fails".into();
            }
            "signout" | "signin" | "missing" => {
                match name {
                    "signout" => self.fake.set_sign_in(SignIn::SignedOut),
                    "signin" => self.fake.set_sign_in(SignIn::SignedIn {
                        account: Some("Claude Max plan".into()),
                    }),
                    _ => self
                        .fake
                        .set_availability(writer_assistant::Availability::NotInstalled),
                }
                let found = self.fake.recheck();
                self.panel
                    .update(cx, |panel, cx| panel.set_availability(found, cx));
            }
            "privacy" => self
                .panel
                .update(cx, |panel, cx| panel.set_privacy_notice(true, cx)),
            "notice" => self.fake.notice(arg),
            "doc" => {
                let number = arg.parse::<usize>().unwrap_or(1).clamp(1, 2) - 1;
                self.document = number;
                self.editor
                    .update(cx, |editor, cx| editor.set_document(chapter(number), cx));
                let stored = self.stored[number].clone();
                self.panel.update(cx, |panel, cx| {
                    panel.set_document(Some(document_ref(number)), Some(stored), window, cx)
                });
            }
            "attach" => self.panel.update(cx, |panel, cx| {
                panel.attach(
                    Attachment::folder(11, "Part one", "The Salt Road/Part one"),
                    cx,
                );
                panel.attach(
                    Attachment::document(3, "Characters", "The Salt Road/Notes/Characters"),
                    cx,
                );
            }),
            "selection" => self
                .panel
                .update(cx, |panel, cx| panel.set_apply_state(true, arg == "on", cx)),
            "fill" => {
                let count = arg.parse::<usize>().unwrap_or(3);
                for index in 0..count {
                    self.script_next();
                    self.panel.update(cx, |panel, cx| {
                        panel.send(format!("Question {}", index + 1), window, cx)
                    });
                }
            }
            "answer" => self.script_next(),
            "state" => {
                let said = std::mem::take(&mut self.said).join(", ");
                let state = self.panel.read(cx).debug_state(window, cx);
                return format!("{state} host=[{said}]");
            }
            other => return format!("unknown demo step {other:?}"),
        }
        cx.notify();
        String::new()
    }
}

impl Render for Demo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        div()
            .key_context("AssistantDemo")
            .on_action(cx.listener(|this, _: &FocusChat, window, cx| {
                this.panel_open = true;
                this.panel.update(cx, |panel, cx| {
                    panel.set_shown(true, cx);
                    panel.focus(window, cx);
                });
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FocusText, window, cx| {
                this.editor.read(cx).focus(window);
            }))
            .on_action(|_: &ToggleTheme, _, cx| {
                let next = if cx.theme().is_dark() {
                    ThemeMode::Light
                } else {
                    ThemeMode::Dark
                };
                theme::set_theme_mode(next, cx);
            })
            .relative()
            .size_full()
            .flex()
            .pt(TITLEBAR_HEIGHT)
            .bg(colors.canvas)
            .child(div().flex_1().min_w_0().h_full().child(self.editor.clone()))
            .when(self.panel_open, |root| {
                root.child(
                    div()
                        .flex_none()
                        .w(px(PANEL_WIDTH))
                        .h_full()
                        .border_l_1()
                        .border_color(colors.hairline)
                        .child(self.panel.clone()),
                )
            })
            .child(self.toasts.clone())
    }
}

fn main() {
    Application::new()
        .with_assets(writer_ui::assets::Assets)
        .run(|cx: &mut App| {
            theme::init(ThemeMode::Light, cx);
            let context = Some("AssistantDemo");
            cx.bind_keys([
                KeyBinding::new("cmd-3", FocusChat, context),
                KeyBinding::new("cmd-1", FocusText, context),
                KeyBinding::new("cmd-t", ToggleTheme, context),
                KeyBinding::new("cmd-q", Quit, None),
            ]);
            writer_ui::init(cx);
            writer_editor::init(cx);
            writer_assistant_ui::init(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            // A scripted run stays in the background: it does not take the
            // keyboard from whoever is at the machine.
            let scripted = std::env::var_os(automation::ENV).is_some();
            let bounds = Bounds::centered(None, size(px(1100.), px(820.)), cx);
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        titlebar: Some(gpui::TitlebarOptions {
                            title: Some("The assistant's side chat".into()),
                            appears_transparent: true,
                            traffic_light_position: Some(gpui::point(
                                theme::TRAFFIC_LIGHT_X,
                                theme::TRAFFIC_LIGHT_Y,
                            )),
                        }),
                        focus: !scripted,
                        ..Default::default()
                    },
                    |window, cx| {
                        let demo = cx.new(|cx| Demo::new(scripted, window, cx));
                        demo.read(cx).editor.read(cx).focus(window);
                        demo
                    },
                )
                .expect("open the window");
            if !scripted {
                cx.activate(true);
                // By hand every message gets one of the scripted answers.
                window
                    .update(cx, |demo, _, _| {
                        for _ in 0..64 {
                            demo.script_next();
                        }
                    })
                    .ok();
            }
            let demo = window.root(cx).expect("the demo view");
            automation::run_from_env(
                window.into(),
                move |command, window, cx| {
                    demo.update(cx, |demo, cx| demo.command(command, window, cx))
                },
                cx,
            );
        });
}
