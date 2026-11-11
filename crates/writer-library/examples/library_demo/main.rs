//! The library panel on a throwaway library, mounted the way the app mounts it: a sidebar
//! beside an "editor" placeholder, with the window's modal and toast layers.
//!
//! ```bash
//! cargo run -p writer-library --example library_demo
//! LIBRARY_DEMO_DOCS=5000 cargo run -p writer-library --example library_demo   # a big library
//! LIBRARY_DEMO_EMPTY=1 cargo run -p writer-library --example library_demo     # the empty state
//! LIBRARY_DEMO_LONG=1 cargo run -p writer-library --example library_demo      # long names, deep nesting
//! LIBRARY_DEMO_BENCH=5000 cargo run --release -p writer-library --example library_demo  # save cost, no window
//! ```
//!
//! The library lives in memory and is seeded with nested folders, documents and a few
//! trashed items, so nothing touches the user's data. Keys: `cmd-0` focuses the library,
//! `cmd-1` the placeholder editor, `cmd-t` toggles Paper / Night, `cmd-q` quits; everything
//! else is the panel's own keymap (see the `writer_library` panel docs).
//!
//! Visual QA: set `THE_WRITER_AUTOMATION` (see `automation.rs`), e.g.
//! `THE_WRITER_AUTOMATION="wait:800; theme:light; screenshot:/tmp/l.png; quit"`.

mod automation;
mod bench;

use gpui::{
    App, Application, Bounds, Context, Entity, FocusHandle, Focusable, KeyBinding, SharedString,
    Subscription, Window, WindowBounds, WindowOptions, actions, div, prelude::*, px, size,
};
use writer_library::{
    Clock, DocumentContent, DocumentId, FolderId, Library, LibraryMenuTarget, LibraryPanel,
    LibraryPanelEvent,
};
use writer_ui::{
    ContextMenuItem, ModalLayer, ToastLayer,
    theme::{self, ActiveTheme, ThemeMode, UI_TEXT_SM, UI_TEXT_TITLE},
};

actions!(
    library_demo,
    [
        /// Switch between Paper and Night.
        ToggleTheme,
        /// Focus the library panel.
        FocusLibrary,
        /// Focus the placeholder editor.
        FocusEditor,
        /// Quit the demo.
        Quit,
    ]
);

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

/// Creates a document whose last edit was `age_ms` ago.
#[allow(clippy::too_many_arguments)]
fn document(
    library: &mut Library,
    clock: &Clock,
    now: i64,
    folder: Option<FolderId>,
    title: &str,
    words: u64,
    age_ms: i64,
    cx: &mut Context<Library>,
) -> DocumentId {
    clock.set_ms(now - age_ms);
    let id = library
        .create_document(
            folder,
            &DocumentContent {
                content: "{}",
                title,
                plain_text: title,
                word_count: words,
                char_count: words * 6,
            },
            cx,
        )
        .expect("create document")
        .id;
    clock.set_ms(now);
    id
}

/// Fills the library with a writer's folders. Returns the document to show as open.
fn seed(library: &mut Library, clock: &Clock, cx: &mut Context<Library>) -> DocumentId {
    let now = clock.now_ms();
    let mut folder = |library: &mut Library, parent: Option<FolderId>, name: &str| {
        library
            .create_folder(parent, name, cx)
            .expect("create folder")
            .id
    };
    let novel = folder(library, None, "The Salt Road");
    let part_one = folder(library, Some(novel), "Part one");
    let part_two = folder(library, Some(novel), "Part two");
    let characters = folder(library, Some(novel), "Characters");
    let blog = folder(library, None, "Blog");
    let drafts = folder(library, Some(blog), "Drafts");
    let published = folder(library, Some(blog), "Published");
    let social = folder(library, None, "Social");
    let abandoned = folder(library, None, "Abandoned ideas");

    let mut doc = |library: &mut Library, parent: Option<FolderId>, title: &str, words, age| {
        document(library, clock, now, parent, title, words, age, cx)
    };
    doc(library, Some(part_one), "1. The harbour", 2140, 3 * HOUR);
    let open = doc(
        library,
        Some(part_one),
        "2. A letter from Mara",
        1865,
        25 * MINUTE,
    );
    doc(library, Some(part_one), "3. Low tide", 980, 2 * DAY);
    let old_opening = doc(library, Some(part_one), "Old opening", 410, 40 * DAY);
    doc(library, Some(part_two), "4. The inland road", 2410, 5 * DAY);
    doc(library, Some(part_two), "5. Salt and ash", 312, 0);
    doc(library, Some(characters), "Mara", 540, 9 * DAY);
    doc(library, Some(characters), "Tomas", 388, 16 * DAY);
    doc(
        library,
        Some(characters),
        "The harbourmaster",
        212,
        21 * DAY,
    );
    doc(library, Some(novel), "Outline", 640, 14 * DAY);
    doc(
        library,
        Some(novel),
        "Research notes on salt works and coastal trade in the 1840s",
        3120,
        70 * DAY,
    );
    doc(
        library,
        Some(drafts),
        "Why I write by hand first",
        730,
        6 * HOUR,
    );
    doc(library, Some(drafts), "On finishing things", 95, DAY);
    doc(
        library,
        Some(published),
        "A year of morning pages",
        1480,
        400 * DAY,
    );
    doc(library, Some(social), "Thread: writing tools", 180, 3 * DAY);
    doc(library, Some(social), "Launch post", 64, 8 * DAY);
    doc(
        library,
        Some(abandoned),
        "The lighthouse keeper",
        1210,
        200 * DAY,
    );
    doc(
        library,
        Some(abandoned),
        "Notes for a radio play",
        450,
        230 * DAY,
    );
    doc(library, None, "Reading list", 205, 4 * DAY);
    doc(library, None, "", 0, 2 * MINUTE);
    let scrap = doc(library, None, "Scrap", 12, 60 * DAY);

    // A few things in the trash.
    library.trash_document(scrap, cx).expect("trash");
    clock.advance_ms(1000);
    library.trash_document(old_opening, cx).expect("trash");
    clock.advance_ms(1000);
    library.trash_folder(abandoned, cx).expect("trash");
    clock.advance_ms(1000);

    // LIBRARY_DEMO_LONG=1 adds long names and deep nesting, to see how rows, the move
    // picker and the messages cope.
    if std::env::var_os("LIBRARY_DEMO_LONG").is_some() {
        let mut parent = library
            .create_folder(
                None,
                "Essays on the quiet hours between midnight and the first tram",
                cx,
            )
            .expect("create folder")
            .id;
        for name in [
            "Drafts that were abandoned and then picked up again a year later",
            "Second pass",
            "Notes from the editor, with my replies in the margins",
            "Cuts",
            "Things I could not bring myself to delete",
            "Level seven",
            "Level eight",
        ] {
            parent = library
                .create_folder(Some(parent), name, cx)
                .expect("create folder")
                .id;
            document(
                library,
                clock,
                now,
                Some(parent),
                "A paragraph about the sound the radiator makes when the heating comes on at five",
                1234567,
                3 * DAY,
                cx,
            );
        }
    }

    // LIBRARY_DEMO_DOCS=N adds a big archive to check that a large library stays fast.
    let extra: usize = std::env::var("LIBRARY_DEMO_DOCS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    if extra > 0 {
        let archive = library
            .create_folder(None, "Archive", cx)
            .expect("create folder")
            .id;
        let mut year = archive;
        for ix in 0..extra {
            if ix % 250 == 0 {
                year = library
                    .create_folder(Some(archive), &format!("Box {}", ix / 250 + 1), cx)
                    .expect("create folder")
                    .id;
            }
            document(
                library,
                clock,
                now,
                Some(year),
                &format!("Entry {:05}", ix + 1),
                (ix as u64 * 37) % 3000,
                (ix as i64 % 900) * DAY,
                cx,
            );
        }
    }
    open
}

/// The window root: the sidebar with the panel, and a placeholder where the editor goes.
struct Demo {
    library: Entity<Library>,
    panel: Entity<LibraryPanel>,
    modal_layer: Entity<ModalLayer>,
    toast_layer: Entity<ToastLayer>,
    editor_focus: FocusHandle,
    open: Option<DocumentId>,
    log: Vec<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl Demo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let clock = Clock::manual(Clock::system().now_ms());
        let mut library = Library::in_memory_with_clock(clock.clone()).expect("in-memory library");
        let mut open = None;
        let library = cx.new(|cx| {
            // LIBRARY_DEMO_EMPTY=1 starts with nothing, to see the empty state.
            if std::env::var_os("LIBRARY_DEMO_EMPTY").is_none() {
                open = Some(seed(&mut library, &clock, cx));
            }
            library
        });
        let modal_layer = cx.new(|_| ModalLayer::new());
        let toast_layer = cx.new(|_| ToastLayer::new());
        ToastLayer::set_global(&toast_layer, cx);
        let panel = cx.new(|cx| {
            let mut panel = LibraryPanel::new(library.clone(), cx);
            panel.set_modal_layer(modal_layer.clone(), cx);
            panel.set_new_document_content("{}");
            // What the app adds to the row menus (it exports and keeps versions; the
            // panel does not know about either). Here they only say that they ran.
            panel.set_menu_extension(|target, _window, _cx| {
                let say = |what: &'static str| {
                    move |_: &mut Window, cx: &mut App| {
                        writer_ui::toast::show_toast(writer_ui::Toast::new(what), cx);
                    }
                };
                match target {
                    LibraryMenuTarget::Document(_) => vec![
                        ContextMenuItem::new("Export…", say("The app would export it.")),
                        ContextMenuItem::new(
                            "Version History",
                            say("The app would show its versions."),
                        ),
                    ],
                    LibraryMenuTarget::Folder(_) => vec![ContextMenuItem::new(
                        "Export…",
                        say("The app would export the folder."),
                    )],
                    _ => Vec::new(),
                }
            });
            panel
        });
        panel.update(cx, |panel, cx| panel.set_active_document(open, cx));
        let subscriptions = vec![cx.subscribe_in(
            &panel,
            window,
            |this: &mut Self, panel, event: &LibraryPanelEvent, window, cx| {
                match event {
                    LibraryPanelEvent::OpenDocument { id, focus_editor } => {
                        this.open = Some(*id);
                        let id = *id;
                        panel.update(cx, |panel, cx| panel.set_active_document(Some(id), cx));
                        if *focus_editor {
                            window.focus(&this.editor_focus);
                        }
                        this.log.push(
                            format!("OpenDocument {{ id: {id}, focus_editor: {focus_editor} }}")
                                .into(),
                        );
                    }
                    LibraryPanelEvent::Dismissed => {
                        window.focus(&this.editor_focus);
                        this.log.push("Dismissed".into());
                    }
                }
                cx.notify();
            },
        )];
        Self {
            library,
            panel,
            modal_layer,
            toast_layer,
            editor_focus: cx.focus_handle(),
            open,
            log: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    fn toggle_theme(&mut self, _: &ToggleTheme, _: &mut Window, cx: &mut Context<Self>) {
        let next = if cx.theme().is_dark() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        theme::set_theme_mode(next, cx);
    }

    fn focus_library(&mut self, _: &FocusLibrary, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.panel.focus_handle(cx));
    }

    fn focus_editor(&mut self, _: &FocusEditor, window: &mut Window, _: &mut Context<Self>) {
        window.focus(&self.editor_focus);
    }
}

impl Render for Demo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let document_font = theme.fonts.document.clone();
        let ui_font = theme.fonts.ui.clone();
        let editor_focused = self.editor_focus.is_focused(window);
        let title: SharedString = match self.open {
            Some(id) => match self.library.read(cx).document_meta(id) {
                Ok(meta) if meta.title.trim().is_empty() => "Untitled".into(),
                Ok(meta) => meta.title.into(),
                Err(_) => "The open document is gone".into(),
            },
            None => "No document open".into(),
        };
        let log = self.log.iter().rev().take(6).cloned().collect::<Vec<_>>();

        writer_library::forward_actions(div().id("demo"), &self.panel)
            .key_context("Demo")
            .on_action(cx.listener(Self::toggle_theme))
            .on_action(cx.listener(Self::focus_library))
            .on_action(cx.listener(Self::focus_editor))
            .relative()
            .size_full()
            .flex()
            .bg(colors.canvas)
            .text_color(colors.ink)
            .font_family(ui_font)
            .child(
                div()
                    .w(px(288.))
                    .h_full()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .bg(colors.sidebar)
                    .border_r_1()
                    .border_color(colors.hairline)
                    // The traffic lights sit over the top of the sidebar.
                    .child(div().h(theme::TITLEBAR_HEIGHT).flex_none())
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .overflow_hidden()
                            .child(self.panel.clone()),
                    ),
            )
            .child(
                div()
                    .id("editor")
                    .track_focus(&self.editor_focus)
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .pt(px(96.))
                    .gap(px(12.))
                    .child(
                        div()
                            .w(px(560.))
                            .max_w_full()
                            .px(px(24.))
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .child(
                                div()
                                    .font_family(document_font)
                                    .text_size(UI_TEXT_TITLE)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_size(UI_TEXT_SM)
                                    .text_color(colors.muted)
                                    .child(if editor_focused {
                                        "The editor placeholder has the keyboard. cmd-0 goes to the library."
                                    } else {
                                        "The editor goes here. cmd-1 focuses it, cmd-t switches the theme."
                                    }),
                            )
                            .children(log.into_iter().map(|line| {
                                div()
                                    .text_size(UI_TEXT_SM)
                                    .text_color(colors.faint)
                                    .child(line)
                            })),
                    ),
            )
            .child(self.modal_layer.clone())
            .child(self.toast_layer.clone())
    }
}

fn main() {
    if bench::run_from_env() {
        return;
    }
    Application::new()
        .with_assets(writer_ui::assets::Assets)
        .run(|cx: &mut App| {
            theme::init(ThemeMode::System, cx);
            cx.bind_keys([
                KeyBinding::new("cmd-t", ToggleTheme, Some("Demo")),
                KeyBinding::new("cmd-0", FocusLibrary, Some("Demo")),
                KeyBinding::new("cmd-1", FocusEditor, Some("Demo")),
                KeyBinding::new("cmd-q", Quit, None),
            ]);
            writer_ui::init(cx);
            writer_library::init(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let bounds = Bounds::centered(None, size(px(1100.), px(760.)), cx);
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        titlebar: Some(gpui::TitlebarOptions {
                            title: Some("writer-library demo".into()),
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
                        let focus = demo.read(cx).panel.focus_handle(cx);
                        window.focus(&focus);
                        demo
                    },
                )
                .expect("open demo window");
            // A scripted run stays in the background: it must not take the keyboard away
            // from whoever is using the machine (their typing would land in this window).
            // The script's keystrokes are dispatched inside the app either way.
            if std::env::var_os(automation::ENV).is_none() {
                cx.activate(true);
            }
            automation::run_from_env(window.into(), cx);
        });
}
