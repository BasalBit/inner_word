//! A small host for the view, used by the tests and by the `shortcuts`
//! example (which includes this file): what the app does around
//! [`KeymapView`], in miniature.
//!
//! It binds a handful of pretend commands of an app beside the real
//! bindings of `writer-ui` and of the view, takes the assembled GPUI keymap
//! as the defaults, hands the view a listing made from it, and carries out
//! every request the view makes: it changes its `writer_keymap::Keymap`,
//! installs the result in GPUI (clear, then bind again in order) and hands
//! the view the new listing. So a key changed in the view is the key that
//! runs the command afterwards, the view's own keys included.

#![allow(dead_code)]

use std::rc::Rc;

use gpui::{
    App, Context, DummyKeyboardMapper, Entity, FocusHandle, Focusable, KeyBinding,
    KeyBindingContextPredicate, Subscription, Window, actions, div, prelude::*, px,
};
use writer_keymap::{
    Catalogue, Changes, DefaultBinding, Defaults, KeySequence, Keymap, Places, Target,
};
use writer_keymap_ui::{CommandRow, KeymapView, Listing, Request};
use writer_ui::{
    ContextMenuLayer, ModalLayer, TextField,
    theme::{ActiveTheme, UI_TEXT_BASE},
};

actions!(
    demo,
    [
        /// Open or close the keyboard shortcuts.
        OpenShortcuts,
        /// Make a new document.
        NewDocument,
        /// Open a document by name.
        QuickOpen,
        /// Quit.
        Quit,
        /// Show or hide comments.
        ToggleComments,
        /// Bold.
        Bold,
        /// Italic.
        Italic,
        /// Underline.
        Underline,
        /// Numbered list.
        NumberedList,
        /// Find.
        Find,
        /// Move the block up.
        MoveBlockUp,
        /// The context menu of the text.
        OpenContextMenu,
        /// Case sensitive, in the find bar.
        CaseSensitive,
        /// The version history: a command without a key.
        VersionHistory,
        /// The insert menu: opened by typing a character, not by a binding.
        OpenInsertMenu,
    ]
);

/// The pretend commands' keys, in the order an app binds them: the
/// window's first.
fn demo_bindings() -> Vec<KeyBinding> {
    let editor = Some("Editor");
    vec![
        KeyBinding::new("cmd-/", OpenShortcuts, None),
        KeyBinding::new("cmd-n", NewDocument, None),
        KeyBinding::new("cmd-p", QuickOpen, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-alt-c", ToggleComments, None),
        KeyBinding::new("cmd-b", Bold, editor),
        KeyBinding::new("cmd-i", Italic, editor),
        KeyBinding::new("cmd-u", Underline, editor),
        // One key in the two spellings a keyboard and a script send.
        KeyBinding::new("cmd-&", NumberedList, editor),
        KeyBinding::new("cmd-shift-7", NumberedList, editor),
        KeyBinding::new("cmd-f", Find, editor),
        KeyBinding::new("alt-up", MoveBlockUp, editor),
        KeyBinding::new("shift-f10", OpenContextMenu, editor),
        KeyBinding::new("ctrl-enter", OpenContextMenu, editor),
        KeyBinding::new("cmd-alt-c", CaseSensitive, Some("FindBar > TextField")),
    ]
}

/// What the pretend commands are called, and their category.
const TITLES: [(&str, &str, &str); 15] = [
    ("demo::OpenShortcuts", "Keyboard shortcuts", "View"),
    ("demo::NewDocument", "New document", "File"),
    ("demo::QuickOpen", "Quick open", "File"),
    ("demo::Quit", "Quit", "File"),
    ("demo::ToggleComments", "Show or hide comments", "View"),
    ("demo::Bold", "Bold", "Format"),
    ("demo::Italic", "Italic", "Format"),
    ("demo::Underline", "Underline", "Format"),
    ("demo::NumberedList", "Numbered list", "Format"),
    ("demo::Find", "Find", "Edit"),
    ("demo::MoveBlockUp", "Move block up", "Edit"),
    ("demo::OpenContextMenu", "Context menu", "Edit"),
    ("demo::CaseSensitive", "Case sensitive", ""),
    ("demo::VersionHistory", "Version history", "File"),
    ("demo::OpenInsertMenu", "Insert menu", "Insert"),
];

/// Why the insert menu's row cannot be changed.
pub const TYPED_REASON: &str = "It opens when / is typed in the text, which is not a shortcut.";

/// The line above the list.
pub const NOTE: &str = "Every command is also in a menu: right-click, or the menu bar";

/// Where a key context is, in words, in the order the list shows places.
fn place_of(context: Option<&str>) -> (usize, &'static str) {
    let Some(context) = context else {
        return (0, "Anywhere");
    };
    let has = |name: &str| context.contains(name);
    if has("KeymapChoices") {
        (7, "In a question of keyboard shortcuts")
    } else if has("KeymapView") {
        (6, "In keyboard shortcuts")
    } else if has("FindBar") {
        (2, "In the find bar")
    } else if has("Picker") {
        (3, "In a list or menu")
    } else if [
        "ConfirmDialog",
        "Button",
        "Toggle",
        "SegmentedControl",
        "TabNavigation",
        "ModalLayer",
    ]
    .into_iter()
    .any(has)
    {
        (4, "In a dialog")
    } else if has("Editor") {
        (1, "In the text")
    } else if has("TextField") {
        (5, "In a text field")
    } else {
        (8, "Elsewhere")
    }
}

/// `text_field::WordLeft` as "Word left".
fn humanize(name: &str) -> String {
    let short = name.rsplit("::").next().unwrap_or(name);
    let mut out = String::new();
    for (ix, ch) in short.chars().enumerate() {
        if ix > 0 && ch.is_uppercase() {
            out.push(' ');
        }
        if ix == 0 {
            out.extend(ch.to_uppercase());
        } else {
            out.extend(ch.to_lowercase());
        }
    }
    out
}

/// Binds everything in the app's order: the window's own keys, the
/// widgets', the view's. Call once per app.
pub fn init(cx: &mut App) {
    cx.bind_keys(demo_bindings());
    writer_ui::init(cx);
    writer_keymap_ui::init(cx);
}

/// The stand-in for the app's window: something with the keyboard behind
/// the view (a text field in an "Editor" context), the modal layer the
/// view is shown in, and the keymap.
pub struct Host {
    pub keymap: Keymap,
    /// GPUI's bindings as they were bound, in the order of the defaults.
    gpui_defaults: Vec<KeyBinding>,
    pub modal_layer: Entity<ModalLayer>,
    /// What has the keyboard while the view is closed.
    pub field: Entity<TextField>,
    pub view: Option<Entity<KeymapView>>,
    /// Every request the view made, in order.
    pub requests: Vec<Request>,
    /// The commands the view asked to run.
    pub ran: Vec<Target>,
    /// The changes as they were before the last request that changed any.
    pub undo: Option<Changes>,
    /// The file "the user chooses" when the view asks for one: its name
    /// and its text.
    pub import_file: Option<(String, String)>,
    /// The shortcuts file of the last export.
    pub exported: Option<String>,
    /// How often each pretend command ran.
    pub new_documents: usize,
    pub quits: usize,
    pub bolds: usize,
    _subscriptions: Vec<Subscription>,
}

impl Host {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let gpui_defaults: Vec<KeyBinding> =
            cx.key_bindings().borrow().bindings().cloned().collect();
        let mut defaults = Defaults::new();
        for binding in &gpui_defaults {
            let keys = binding
                .keystrokes()
                .iter()
                .map(|key| key.unparse())
                .collect::<Vec<_>>()
                .join(" ");
            let context = binding.predicate().map(|predicate| predicate.to_string());
            let input = binding.action_input();
            defaults.push(
                DefaultBinding::new(
                    binding.action().name(),
                    input.as_ref().map(|input| input.as_ref()),
                    context.as_deref(),
                    &keys,
                )
                .expect("the keys of the fixture are read"),
            );
        }
        defaults.add_keyless(Target::new("demo::VersionHistory", None));
        let places = Places::new()
            .typing("Editor")
            .typing("TextField")
            .inside("TextField", "FindBar")
            .inside("TextField", "KeymapView")
            .inside("TextField", "Picker")
            .inside("KeymapChoices", "KeymapView")
            .container("FindBar")
            .container("Picker");
        let field = cx.new(|cx| TextField::new(window, cx).with_placeholder("The document"));
        Self {
            keymap: Keymap::new(defaults, places),
            gpui_defaults,
            modal_layer: cx.new(|_| ModalLayer::new()),
            field,
            view: None,
            requests: Vec::new(),
            ran: Vec::new(),
            undo: None,
            import_file: None,
            exported: None,
            new_documents: 0,
            quits: 0,
            bolds: 0,
            _subscriptions: Vec::new(),
        }
    }

    /// The rows of the list and the names of every command.
    pub fn listing(&self) -> Listing {
        let mut rows: Vec<(usize, CommandRow)> = Vec::new();
        for target in self.keymap.defaults().targets() {
            let action = target.command.action.as_str();
            let (rank, place) = place_of(target.context.text());
            let named = TITLES.iter().find(|(name, _, _)| *name == action);
            let (title, category) = match named {
                Some((_, title, category)) => (title.to_string(), category.to_string()),
                None => (humanize(action), String::new()),
            };
            let mut row = CommandRow::new(target.clone(), title, place).category(category);
            if !action.starts_with("demo::") {
                row = row.not_runnable();
            }
            rows.push((rank, row));
        }
        // A command that is typed, not bound: listed, and said to be fixed.
        rows.push((
            1,
            CommandRow::new(
                Target::new("demo::OpenInsertMenu", Some("Editor")),
                "Insert menu",
                "In the text",
            )
            .category("Insert")
            .locked_with_keys(TYPED_REASON, vec![KeySequence::parse("/").unwrap()]),
        ));
        // By place, then by heading in the order the headings first appear.
        let mut headings: Vec<gpui::SharedString> = Vec::new();
        for (_, row) in &rows {
            if !headings.contains(&row.heading) {
                headings.push(row.heading.clone());
            }
        }
        rows.sort_by_key(|(rank, row)| {
            (
                *rank,
                headings.iter().position(|heading| *heading == row.heading),
            )
        });
        let rows: Vec<CommandRow> = rows.into_iter().map(|(_, row)| row).collect();
        let mut catalogue = Catalogue::new();
        for row in &rows {
            catalogue.add(row.target.clone(), &row.title, &row.place);
        }
        Listing::new(self.keymap.clone(), rows)
            .catalogue(catalogue)
            .note(NOTE)
    }

    /// Installs the keymap as it is in GPUI: clear, then every binding
    /// again in order, a changed key where its default stood.
    pub fn install(&self, cx: &mut App) {
        let effective = self.keymap.effective();
        let mut bindings = Vec::new();
        for binding in effective.bindings() {
            let (action, predicate, input) = match binding.source {
                Some(index) => {
                    let default = &self.gpui_defaults[index];
                    (
                        default.action().boxed_clone(),
                        default.predicate(),
                        default.action_input(),
                    )
                }
                None => {
                    let Ok(action) = cx.build_action(&binding.target.command.action, None) else {
                        continue;
                    };
                    let predicate = binding
                        .target
                        .context
                        .text()
                        .and_then(|text| KeyBindingContextPredicate::parse(text).ok())
                        .map(Rc::new);
                    (action, predicate, None)
                }
            };
            if let Ok(binding) = KeyBinding::load(
                &binding.spelling,
                action,
                predicate,
                false,
                input,
                &DummyKeyboardMapper,
            ) {
                bindings.push(binding);
            }
        }
        cx.clear_key_bindings();
        cx.bind_keys(bindings);
    }

    /// Shows the view (or closes it when it is open).
    pub fn toggle_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_layer.read(cx).is_open::<KeymapView>() {
            self.modal_layer
                .update(cx, |layer, cx| layer.dismiss(window, cx));
            self.view = None;
            return;
        }
        let listing = self.listing();
        let view = cx.new(|cx| KeymapView::new(listing, window, cx));
        self._subscriptions =
            vec![
                cx.subscribe_in(&view, window, |this, _, request: &Request, window, cx| {
                    this.carry_out(request.clone(), window, cx)
                }),
            ];
        self.view = Some(view.clone());
        self.modal_layer
            .update(cx, |layer, cx| layer.show(view, window, cx));
    }

    /// What the app does with a request of the view.
    fn carry_out(&mut self, request: Request, window: &mut Window, cx: &mut Context<Self>) {
        self.requests.push(request.clone());
        let Some(view) = self.view.clone() else {
            return;
        };
        match &request {
            Request::Export => {
                let listing = self.listing();
                let text = self.keymap.export(&listing.catalogue, "0.4.0");
                if let Ok(dir) = std::env::var("THE_WRITER_EXPORT_DIR") {
                    let path = std::path::Path::new(&dir).join(writer_keymap::FILE_NAME);
                    std::fs::create_dir_all(&dir).ok();
                    std::fs::write(path, &text).ok();
                }
                self.exported = Some(text);
            }
            Request::ChooseImportFile => {
                let Some((name, text)) = self.import_file.clone() else {
                    return;
                };
                match self.keymap.plan_import(&text) {
                    Ok(plan) => view.update(cx, |view, cx| {
                        view.show_import_preview(name, plan, window, cx)
                    }),
                    Err(error) => view.update(cx, |view, cx| {
                        view.show_import_error(name, &error, window, cx)
                    }),
                }
            }
            Request::Run { target } => {
                self.ran.push(target.clone());
            }
            change => {
                if let Some(before) = change.apply_to(&mut self.keymap) {
                    self.undo = Some(before);
                    self.install(cx);
                }
                let listing = self.listing();
                view.update(cx, |view, cx| view.set_listing(listing, window, cx));
            }
        }
    }

    /// Whether the view is shown.
    pub fn is_open(&self, cx: &App) -> bool {
        self.modal_layer.read(cx).is_open::<KeymapView>()
    }
}

impl Focusable for Host {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.field.focus_handle(cx)
    }
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        // The view went away by itself (escape, a click outside).
        if self.view.is_some() && !self.modal_layer.read(cx).is_open::<KeymapView>() {
            self.view = None;
        }
        div()
            .id("host")
            .key_context("Host")
            .relative()
            .size_full()
            .bg(colors.canvas)
            .font_family(theme.fonts.ui.clone())
            .text_size(UI_TEXT_BASE)
            .text_color(colors.ink)
            .on_action(
                cx.listener(|this, _: &OpenShortcuts, window, cx| {
                    this.toggle_shortcuts(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &NewDocument, _, cx| {
                this.new_documents += 1;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Quit, _, cx| {
                this.quits += 1;
                cx.notify();
            }))
            .child(
                div()
                    .key_context("Editor")
                    .on_action(cx.listener(|this, _: &Bold, _, cx| {
                        this.bolds += 1;
                        cx.notify();
                    }))
                    .p(px(40.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(div().w(px(420.)).child(self.field.clone()))
                    .child(div().text_color(colors.muted).child(format!(
                        "New documents: {}   Bold: {}   Quit asked: {}",
                        self.new_documents, self.bolds, self.quits
                    ))),
            )
            .child(self.modal_layer.clone())
            .child(ContextMenuLayer)
    }
}

/// A shortcuts file a person edited: two keys changed, one entry for a
/// command this version does not have, one key that cannot be read, one a
/// guard rail refuses.
pub fn edited_file(host: &Host) -> String {
    let exported = host.keymap.export(&host.listing().catalogue, "0.3.0");
    let mut value: serde_json_lite::Value = serde_json_lite::parse(&exported);
    value.set_keys("demo::Bold", &["cmd-alt-b"]);
    value.set_keys("demo::VersionHistory", &["cmd-alt-h"]);
    value.set_keys("demo::Italic", &["cmd-"]);
    value.set_keys("demo::Underline", &["u"]);
    value.push_entry("demo::Gone", "Gone", &["cmd-g"]);
    value.to_text()
}

/// The little of JSON editing the fixture needs, on the text of an
/// exported file (the crate has no JSON dependency of its own): the file
/// is written one field per line, so a field of an entry is found by its
/// line.
pub mod serde_json_lite {
    pub struct Value {
        lines: Vec<String>,
    }

    pub fn parse(text: &str) -> Value {
        Value {
            lines: text.lines().map(str::to_string).collect(),
        }
    }

    fn list(keys: &[&str]) -> String {
        let keys: Vec<String> = keys.iter().map(|key| format!("\"{key}\"")).collect();
        format!("[{}]", keys.join(", "))
    }

    impl Value {
        /// Replaces the `keys` of the first entry for `command`.
        pub fn set_keys(&mut self, command: &str, keys: &[&str]) {
            let wanted = format!("\"command\": \"{command}\"");
            let start = self
                .lines
                .iter()
                .position(|line| line.contains(&wanted))
                .unwrap_or_else(|| panic!("no entry for {command}"));
            let at = (start..self.lines.len())
                .find(|ix| self.lines[*ix].trim_start().starts_with("\"keys\":"))
                .expect("an entry has keys");
            self.lines[at] = format!("      \"keys\": {}", list(keys));
        }

        /// Adds an entry at the end of `shortcuts`.
        pub fn push_entry(&mut self, command: &str, title: &str, keys: &[&str]) {
            let end = self
                .lines
                .iter()
                .rposition(|line| line.trim() == "]")
                .expect("the list ends");
            let last = self.lines[..end]
                .iter()
                .rposition(|line| line.trim() == "}")
                .expect("an entry before the end");
            self.lines[last] = "    },".to_string();
            let entry = [
                "    {".to_string(),
                format!("      \"command\": \"{command}\","),
                format!("      \"title\": \"{title}\","),
                "      \"where\": \"Anywhere\",".to_string(),
                "      \"context\": null,".to_string(),
                "      \"default\": [],".to_string(),
                format!("      \"keys\": {}", list(keys)),
                "    }".to_string(),
            ];
            for (offset, line) in entry.into_iter().enumerate() {
                self.lines.insert(end + offset, line);
            }
        }

        pub fn to_text(&self) -> String {
            let mut text = self.lines.join("\n");
            text.push('\n');
            text
        }
    }
}
