//! Widget gallery: every writer-ui widget in one window.
//!
//! ```bash
//! cargo run -p writer-ui --example gallery
//! ```
//!
//! Keys: `cmd-t` toggles Paper / Night, `cmd-k` command palette, `cmd-p`
//! quick open (3000 documents), `cmd-shift-f` font picker (installed fonts),
//! `cmd-d` delete confirmation, `cmd-e` toast, `tab` / `shift-tab` move
//! between controls, `cmd-q` quits. The insert-menu demo filters a popup
//! picker from a separate field (external query).
//!
//! Visual QA: set `THE_WRITER_AUTOMATION` (see `automation.rs`), e.g.
//! `THE_WRITER_AUTOMATION="wait:800; theme:light; screenshot:/tmp/g.png; quit"`.

mod automation;

use gpui::{
    Action, App, Application, Bounds, Context, Entity, FocusHandle, Focusable, KeyBinding,
    SharedString, Subscription, WeakEntity, Window, WindowBounds, WindowOptions, actions, div,
    prelude::*, px, size,
};
use writer_ui::{
    Button, ButtonSize, ButtonVariant, Checkbox, ConfirmDialog, Divider, FieldAppearance, Icon,
    IconName, KeyHint, ModalLayer, Picker, PickerDelegate, PickerEntry, PickerItem, SectionLabel,
    SegmentedControl, Switch, TextField, TextFieldEvent, Toast, ToastLayer,
    focus::tab_navigation,
    fuzzy::{StringMatch, match_strings},
    key_hint::KeyHintStyle,
    picker,
    theme::{self, ActiveTheme, RADIUS_LG, ThemeMode, UI_TEXT_SM, UI_TEXT_TITLE},
};

actions!(
    gallery,
    [
        /// Switch between Paper and Night.
        ToggleTheme,
        /// Open the command palette.
        OpenPalette,
        /// Quick open a document.
        QuickOpen,
        /// Pick a font.
        PickFont,
        /// Ask before deleting.
        ConfirmDelete,
        /// Show a toast.
        ShowToast,
        /// Quit the gallery.
        Quit,
    ]
);

// ---------------------------------------------------------------------------
// Delegates
// ---------------------------------------------------------------------------

/// Commands: label + action, key hints from the keymap.
struct Commands {
    commands: Vec<(SharedString, Box<dyn Action>)>,
    matches: Vec<StringMatch>,
}

impl Commands {
    fn new() -> Self {
        let commands: Vec<(SharedString, Box<dyn Action>)> = vec![
            ("Toggle theme".into(), Box::new(ToggleTheme)),
            ("Quick open document".into(), Box::new(QuickOpen)),
            ("Pick font".into(), Box::new(PickFont)),
            ("Delete document".into(), Box::new(ConfirmDelete)),
            ("Show toast".into(), Box::new(ShowToast)),
            ("Quit gallery".into(), Box::new(Quit)),
        ];
        Self {
            commands,
            matches: Vec::new(),
        }
    }
}

impl PickerDelegate for Commands {
    fn update_matches(&mut self, query: &str, _: &mut Window, _: &mut Context<Picker<Self>>) {
        self.matches = match_strings(
            query,
            self.commands.iter().map(|(label, _)| label.as_ref()),
            usize::MAX,
        );
    }

    fn entry_count(&self) -> usize {
        self.matches.len()
    }

    fn entry(&self, ix: usize, cx: &App) -> PickerEntry {
        let m = &self.matches[ix];
        let (label, action) = &self.commands[m.candidate_index];
        PickerItem::new(label.clone())
            .positions(m.positions.clone())
            .key_hint(KeyHint::for_action(action.as_ref(), cx))
            .into()
    }

    fn confirm(&mut self, ix: usize, _: bool, window: &mut Window, cx: &mut Context<Picker<Self>>) -> bool {
        let action = self.commands[self.matches[ix].candidate_index].1.boxed_clone();
        // Deferred: runs after the palette closed and focus went back.
        window.dispatch_action(action, cx);
        true
    }

    fn placeholder(&self) -> SharedString {
        "Run a command".into()
    }
}

/// A plain list with optional headers when the query is empty.
struct Listing {
    placeholder: SharedString,
    items: Vec<(SharedString, Option<SharedString>, Option<IconName>)>,
    recent: usize,
    query_empty: bool,
    matches: Vec<StringMatch>,
    gallery: WeakEntity<Gallery>,
    verb: &'static str,
}

impl Listing {
    fn header_count(&self) -> usize {
        if self.query_empty && self.recent > 0 { 2 } else { 0 }
    }

    /// Entry index → match index (None for headers).
    fn match_ix(&self, ix: usize) -> Option<usize> {
        if self.header_count() == 0 {
            return Some(ix);
        }
        if ix == 0 || ix == self.recent + 1 {
            None
        } else if ix <= self.recent {
            Some(ix - 1)
        } else {
            Some(ix - 2)
        }
    }
}

impl PickerDelegate for Listing {
    fn update_matches(&mut self, query: &str, _: &mut Window, _: &mut Context<Picker<Self>>) {
        self.query_empty = query.trim().is_empty();
        self.matches = match_strings(
            query,
            self.items.iter().map(|(label, _, _)| label.as_ref()),
            usize::MAX,
        );
    }

    fn entry_count(&self) -> usize {
        self.matches.len() + self.header_count()
    }

    fn entry(&self, ix: usize, _: &App) -> PickerEntry {
        let Some(m) = self.match_ix(ix) else {
            return PickerEntry::Header(if ix == 0 { "Recent" } else { "All documents" }.into());
        };
        // Recent rows repeat the first items.
        let m = &self.matches[m.min(self.matches.len() - 1)];
        let (label, detail, icon) = &self.items[m.candidate_index];
        let mut item = PickerItem::new(label.clone()).positions(m.positions.clone());
        if let Some(detail) = detail {
            item = item.detail(detail.clone());
        }
        if let Some(icon) = icon {
            item = item.icon(*icon);
        }
        item.into()
    }

    fn is_selectable(&self, ix: usize, _: &App) -> bool {
        self.match_ix(ix).is_some()
    }

    fn confirm(&mut self, ix: usize, secondary: bool, window: &mut Window, cx: &mut Context<Picker<Self>>) -> bool {
        let Some(m) = self.match_ix(ix) else { return false };
        let label = self.items[self.matches[m].candidate_index].0.clone();
        let note = format!(
            "{} {label}{}",
            self.verb,
            if secondary { " (secondary, cmd-enter)" } else { "" }
        );
        let gallery = self.gallery.clone();
        window.defer(cx, move |_, cx| {
            gallery.update(cx, |gallery, cx| gallery.note(note, cx)).ok();
        });
        true
    }

    fn placeholder(&self) -> SharedString {
        self.placeholder.clone()
    }
}

/// The "/" insert menu: filtered by text typed elsewhere.
struct InsertMenu {
    blocks: Vec<(&'static str, &'static str)>,
    matches: Vec<StringMatch>,
    sections: bool,
    gallery: WeakEntity<Gallery>,
}

impl InsertMenu {
    const BLOCKS: &'static [(&'static str, &'static str)] = &[
        ("Heading 1", "Text"),
        ("Heading 2", "Text"),
        ("Quote", "Text"),
        ("Code block", "Text"),
        ("Bullet list", "Lists"),
        ("Numbered list", "Lists"),
        ("Todo list", "Lists"),
        ("Divider", "Layout"),
        ("Page break", "Layout"),
    ];

    /// With sections: Text header at 0, Lists header at 5, Layout header at 9.
    fn header(&self, ix: usize) -> Option<&'static str> {
        if !self.sections {
            return None;
        }
        match ix {
            0 => Some("Text"),
            5 => Some("Lists"),
            9 => Some("Layout"),
            _ => None,
        }
    }

    fn match_ix(&self, ix: usize) -> usize {
        if !self.sections {
            return ix;
        }
        ix - if ix > 9 { 3 } else if ix > 5 { 2 } else { 1 }
    }
}

impl PickerDelegate for InsertMenu {
    fn update_matches(&mut self, query: &str, _: &mut Window, _: &mut Context<Picker<Self>>) {
        self.sections = query.is_empty();
        self.matches = match_strings(query, self.blocks.iter().map(|(name, _)| *name), usize::MAX);
    }

    fn entry_count(&self) -> usize {
        self.matches.len() + if self.sections { 3 } else { 0 }
    }

    fn entry(&self, ix: usize, _: &App) -> PickerEntry {
        if let Some(header) = self.header(ix) {
            return PickerEntry::Header(header.into());
        }
        let m = &self.matches[self.match_ix(ix)];
        let (name, _) = self.blocks[m.candidate_index];
        let hint = match name {
            "Heading 1" => KeyHint::parse("cmd-alt-1"),
            "Heading 2" => KeyHint::parse("cmd-alt-2"),
            "Bullet list" => KeyHint::parse("cmd-shift-8"),
            "Page break" => KeyHint::parse("cmd-enter"),
            _ => None,
        };
        PickerItem::new(name).positions(m.positions.clone()).key_hint(hint).into()
    }

    fn is_selectable(&self, ix: usize, _: &App) -> bool {
        self.header(ix).is_none()
    }

    fn confirm(&mut self, ix: usize, _: bool, window: &mut Window, cx: &mut Context<Picker<Self>>) -> bool {
        let name = self.blocks[self.matches[self.match_ix(ix)].candidate_index].0;
        let gallery = self.gallery.clone();
        window.defer(cx, move |window, cx| {
            gallery
                .update(cx, |gallery, cx| {
                    gallery.note(format!("Inserted {name}"), cx);
                    gallery.reset_insert(window, cx);
                })
                .ok();
        });
        false
    }

    fn empty_text(&self, _: &str) -> SharedString {
        "No block matches".into()
    }
}

// ---------------------------------------------------------------------------
// Gallery view
// ---------------------------------------------------------------------------

struct Gallery {
    focus: FocusHandle,
    title_field: Entity<TextField>,
    filled_field: Entity<TextField>,
    rename_field: Entity<TextField>,
    find_field: Entity<TextField>,
    insert_field: Entity<TextField>,
    insert_menu: Entity<Picker<InsertMenu>>,
    vim: bool,
    autosave: bool,
    spellcheck: bool,
    view_mode: usize,
    modal_layer: Entity<ModalLayer>,
    toasts: Entity<ToastLayer>,
    last_action: SharedString,
    exports: usize,
    _subscriptions: Vec<Subscription>,
}

impl Gallery {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title_field = cx.new(|cx| TextField::new(window, cx).with_placeholder("Document title"));
        let filled_field = cx.new(|cx| {
            TextField::new(window, cx).with_text("https://example.com/a-rather-long-link-that-scrolls-horizontally")
        });
        let rename_field = cx.new(|cx| {
            TextField::new(window, cx)
                .with_text("Chapter 3 draft")
                .with_appearance(FieldAppearance::Inline)
        });
        let find_field = cx.new(|cx| {
            TextField::new(window, cx)
                .with_placeholder("Find in document")
                .with_appearance(FieldAppearance::Borderless)
        });
        let insert_field = cx.new(|cx| {
            let mut field = TextField::new(window, cx).with_placeholder("Type to filter blocks");
            field.set_list_navigation(true);
            field
        });
        let gallery = cx.weak_entity();
        let insert_menu = cx.new(|cx| {
            Picker::popup(
                InsertMenu {
                    blocks: InsertMenu::BLOCKS.to_vec(),
                    matches: Vec::new(),
                    sections: true,
                    gallery,
                },
                window,
                cx,
            )
            .external_query()
            .max_visible_rows(7)
        });
        let subscriptions = vec![
            cx.subscribe_in(&insert_field, window, |this, field, event, window, cx| match event {
                TextFieldEvent::Changed => {
                    let query = field.read(cx).text().to_string();
                    this.insert_menu
                        .update(cx, |menu, cx| menu.set_query(query, window, cx));
                }
                TextFieldEvent::Submitted => {
                    this.insert_menu.update(cx, |menu, cx| menu.confirm(false, window, cx))
                }
                _ => {}
            }),
            cx.subscribe_in(&rename_field, window, |this, field, event, _, cx| {
                if matches!(event, TextFieldEvent::Submitted | TextFieldEvent::Blurred) {
                    let name = field.read(cx).text().to_string();
                    this.note(format!("Renamed to “{name}”"), cx);
                }
            }),
            cx.subscribe_in(&title_field, window, |this, field, event, _, cx| {
                if *event == TextFieldEvent::Submitted {
                    let title = field.read(cx).text().to_string();
                    this.note(format!("Title submitted: “{title}”"), cx);
                }
            }),
        ];
        let toasts = cx.new(|_| ToastLayer::new());
        ToastLayer::set_global(&toasts, cx);
        Self {
            focus: cx.focus_handle(),
            title_field,
            filled_field,
            rename_field,
            find_field,
            insert_field,
            insert_menu,
            vim: true,
            autosave: true,
            spellcheck: false,
            view_mode: 0,
            modal_layer: cx.new(|_| ModalLayer::new()),
            toasts,
            last_action: "Nothing yet".into(),
            exports: 0,
            _subscriptions: subscriptions,
        }
    }

    fn note(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.last_action = text.into();
        cx.notify();
    }

    fn reset_insert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.insert_field.update(cx, |field, cx| field.set_text("", cx));
        self.insert_menu.update(cx, |menu, cx| menu.set_query("", window, cx));
    }

    fn toggle_theme(&mut self, _: &ToggleTheme, _: &mut Window, cx: &mut Context<Self>) {
        let next = if cx.theme().is_dark() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        theme::set_theme_mode(next, cx);
        self.note(format!("Theme: {}", next.label()), cx);
    }

    fn open_palette(&mut self, _: &OpenPalette, window: &mut Window, cx: &mut Context<Self>) {
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle(window, cx, |window, cx| Picker::modal(Commands::new(), window, cx))
        });
    }

    fn quick_open(&mut self, _: &QuickOpen, window: &mut Window, cx: &mut Context<Self>) {
        let folders = ["Drafts", "Essays", "Book/Part one", "Book/Part two", "Notes"];
        let items = (0..3000)
            .map(|i| {
                (
                    SharedString::from(format!("Chapter {} — scene {}", i / 12 + 1, i % 12 + 1)),
                    Some(SharedString::from(folders[i % folders.len()])),
                    Some(IconName::Document),
                )
            })
            .collect();
        let delegate = Listing {
            placeholder: "Open a document".into(),
            items,
            recent: 3,
            query_empty: true,
            matches: Vec::new(),
            gallery: cx.weak_entity(),
            verb: "Opened",
        };
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle(window, cx, |window, cx| Picker::modal(delegate, window, cx))
        });
    }

    fn pick_font(&mut self, _: &PickFont, window: &mut Window, cx: &mut Context<Self>) {
        let mut fonts = cx.text_system().all_font_names();
        fonts.sort();
        fonts.dedup();
        let items = fonts
            .into_iter()
            .filter(|name| !name.starts_with('.'))
            .map(|name| (SharedString::from(name), None, None))
            .collect();
        let delegate = Listing {
            placeholder: "Font family".into(),
            items,
            recent: 0,
            query_empty: true,
            matches: Vec::new(),
            gallery: cx.weak_entity(),
            verb: "Font set to",
        };
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle(window, cx, |window, cx| {
                Picker::modal(delegate, window, cx).width(px(420.))
            })
        });
    }

    fn confirm_delete(&mut self, _: &ConfirmDelete, window: &mut Window, cx: &mut Context<Self>) {
        let gallery = cx.weak_entity();
        let cancelled = cx.weak_entity();
        let dialog = cx.new(|cx| {
            ConfirmDialog::new(
                "Delete “Chapter 3 draft”?",
                "The document moves to the trash. You can restore it from there.",
                cx,
            )
            .confirm_label("Move to Trash")
            .danger()
            .on_confirm(move |_, cx| {
                gallery
                    .update(cx, |g, cx| g.note("Deleted “Chapter 3 draft”", cx))
                    .ok();
            })
            .on_cancel(move |_, cx| {
                cancelled.update(cx, |g, cx| g.note("Delete cancelled", cx)).ok();
            })
        });
        ConfirmDialog::show(dialog, &self.modal_layer, window, cx);
    }

    fn show_toast(&mut self, _: &ShowToast, _: &mut Window, cx: &mut Context<Self>) {
        self.exports += 1;
        let toast = if self.exports % 3 == 0 {
            Toast::new("Export failed: the folder is read-only").error()
        } else {
            Toast::new(format!("Exported to ~/Documents/Essay {}.md", self.exports)).success()
        };
        self.toasts.update(cx, |layer, cx| {
            layer.push(toast, cx);
        });
    }

    fn quit(&mut self, _: &Quit, _: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }

    fn card(title: &'static str, cx: &App) -> gpui::Div {
        let colors = cx.theme().colors;
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(16.))
            .w(px(420.))
            .bg(colors.elevated)
            .border_1()
            .border_color(colors.hairline)
            .rounded(RADIUS_LG)
            .child(SectionLabel::new(title))
    }
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = theme.colors;
        let caption = |text: &'static str| {
            div()
                .text_size(UI_TEXT_SM)
                .text_color(colors.muted)
                .child(text)
        };

        let fields = Self::card("Text fields", cx)
            .child(caption("Bordered, empty with placeholder"))
            .child(self.title_field.clone())
            .child(caption("Bordered, long text scrolls horizontally"))
            .child(self.filled_field.clone())
            .child(caption("Inline rename in a tree row"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(6.))
                    .bg(colors.accent_subtle)
                    .child(Icon::new(IconName::ChevronDown).size(px(14.)))
                    .child(Icon::new(IconName::Document).color(colors.ink))
                    .child(div().flex_1().child(self.rename_field.clone())),
            )
            .child(caption("Borderless, inside a find bar"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(36.))
                    .px(px(10.))
                    .rounded(px(6.))
                    .bg(colors.canvas)
                    .border_1()
                    .border_color(colors.hairline)
                    .child(Icon::new(IconName::Search))
                    .child(div().flex_1().child(self.find_field.clone()))
                    .child(div().text_size(UI_TEXT_SM).text_color(colors.muted).child("3 of 12"))
                    .child(Button::icon_only("find-close", IconName::Close).size(ButtonSize::Compact)),
            );

        let buttons = Self::card("Buttons", cx)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        Button::new("export", "Export")
                            .primary()
                            .icon(IconName::Export)
                            .key_hint(KeyHint::for_action(&ShowToast, cx))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_toast(&ShowToast, window, cx)
                            })),
                    )
                    .child(
                        Button::new("palette", "Commands")
                            .key_hint(KeyHint::for_action(&OpenPalette, cx))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_palette(&OpenPalette, window, cx)
                            })),
                    )
                    .child(
                        Button::new("delete", "Delete")
                            .danger()
                            .icon(IconName::Trash)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_delete(&ConfirmDelete, window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        Button::new("open", "Quick open")
                            .size(ButtonSize::Compact)
                            .key_hint(KeyHint::for_action(&QuickOpen, cx))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.quick_open(&QuickOpen, window, cx)
                            })),
                    )
                    .child(
                        Button::new("fonts", "Fonts")
                            .size(ButtonSize::Compact)
                            .key_hint(KeyHint::for_action(&PickFont, cx))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.pick_font(&PickFont, window, cx)
                            })),
                    )
                    .child(Button::new("disabled", "Disabled").disabled(true))
                    .child(Button::new("selected", "Active filter").selected(true).size(ButtonSize::Compact))
                    .child(Button::icon_only("add", IconName::Plus))
                    .child(Button::icon_only("more", IconName::More))
                    .child(Button::new("ghost", "Ghost").variant(ButtonVariant::Ghost)),
            );

        let toggles = Self::card("Switches, checkboxes, segmented control", cx)
            .child(
                Switch::new("vim", self.vim)
                    .label("Vim mode")
                    .description("Modal editing with normal, insert and visual modes")
                    .on_change(cx.listener(|this, on: &bool, _, cx| {
                        this.vim = *on;
                        cx.notify();
                    })),
            )
            .child(
                Switch::new("autosave", self.autosave)
                    .label("Autosave")
                    .on_change(cx.listener(|this, on: &bool, _, cx| {
                        this.autosave = *on;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("spell", self.spellcheck)
                    .label("Check spelling while typing")
                    .on_change(cx.listener(|this, on: &bool, _, cx| {
                        this.spellcheck = *on;
                        cx.notify();
                    })),
            )
            .child(Divider::horizontal())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(8.))
                    .child("View")
                    .child(
                        SegmentedControl::new("view", ["Flow", "Page", "Focus"], self.view_mode)
                            .on_select(cx.listener(|this, ix: &usize, _, cx| {
                                this.view_mode = *ix;
                                cx.notify();
                            })),
                    ),
            );

        let hint_row = |label: &'static str, hint: Option<KeyHint>| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .h(px(24.))
                .child(label)
                .children(hint)
        };
        let hints = Self::card("Key hints (from the keymap)", cx)
            .child(hint_row("Command palette", KeyHint::for_action(&OpenPalette, cx)))
            .child(hint_row("Toggle theme", KeyHint::for_action(&ToggleTheme, cx)))
            .child(hint_row("Undo in a field", KeyHint::for_action(&writer_ui::text_field::Undo, cx)))
            .child(hint_row(
                "Delete word",
                KeyHint::for_action(&writer_ui::text_field::DeleteWordLeft, cx)
                    .map(|h| h.style(KeyHintStyle::Boxed)),
            ))
            .child(hint_row("Chord (parsed)", KeyHint::parse("ctrl-x ctrl-s").map(KeyHint::boxed)))
            .child(hint_row(
                "Every glyph",
                KeyHint::parse("ctrl-alt-shift-cmd-enter backspace delete escape tab up down left right"),
            ));

        let icons = Self::card("Icons", cx).child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(4.))
                .children(IconName::ALL.iter().map(|icon| {
                    div()
                        .w(px(92.))
                        .h(px(52.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(6.))
                        .rounded(px(6.))
                        .hover(|s| s.bg(colors.hover))
                        .child(Icon::new(*icon).color(colors.ink))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(colors.muted)
                                .child(icon.name()),
                        )
                })),
        );

        let insert_field = self.insert_field.clone();
        let menu = self.insert_menu.clone();
        let insert = Self::card("Insert menu (popup picker, external query)", cx)
            .child(caption("Type here; up/down/ctrl-n/ctrl-p move, enter inserts, escape clears"))
            .child(
                div()
                    .key_context(picker::KEY_CONTEXT)
                    .on_action({
                        let menu = menu.clone();
                        move |_: &picker::SelectNext, window, cx| {
                            menu.update(cx, |m, cx| m.select_next(window, cx))
                        }
                    })
                    .on_action({
                        let menu = menu.clone();
                        move |_: &picker::SelectPrev, window, cx| {
                            menu.update(cx, |m, cx| m.select_prev(window, cx))
                        }
                    })
                    .on_action({
                        let menu = menu.clone();
                        move |_: &picker::SelectFirst, window, cx| {
                            menu.update(cx, |m, cx| m.select_first(window, cx))
                        }
                    })
                    .on_action({
                        let menu = menu.clone();
                        move |_: &picker::SelectLast, window, cx| {
                            menu.update(cx, |m, cx| m.select_last(window, cx))
                        }
                    })
                    .on_action(cx.listener(|this, _: &picker::Dismiss, window, cx| {
                        this.reset_insert(window, cx);
                        this.note("Insert menu dismissed", cx);
                    }))
                    .child(insert_field),
            )
            .child(self.insert_menu.clone());

        let overlays = Self::card("Overlays", cx)
            .child(caption("Each opens in the modal layer; escape returns focus here"))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(Button::new("o-palette", "Command palette").key_hint(KeyHint::for_action(&OpenPalette, cx)).on_click(cx.listener(|this, _, w, cx| this.open_palette(&OpenPalette, w, cx))))
                    .child(Button::new("o-open", "Quick open").key_hint(KeyHint::for_action(&QuickOpen, cx)).on_click(cx.listener(|this, _, w, cx| this.quick_open(&QuickOpen, w, cx))))
                    .child(Button::new("o-confirm", "Confirm dialog").key_hint(KeyHint::for_action(&ConfirmDelete, cx)).on_click(cx.listener(|this, _, w, cx| this.confirm_delete(&ConfirmDelete, w, cx))))
                    .child(Button::new("o-toast", "Toast").key_hint(KeyHint::for_action(&ShowToast, cx)).on_click(cx.listener(|this, _, w, cx| this.show_toast(&ShowToast, w, cx)))),
            )
            .child(Divider::horizontal())
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(div().text_color(colors.muted).child("Last action:"))
                    .child(self.last_action.clone()),
            );

        div()
            .key_context("Gallery")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::toggle_theme))
            .on_action(cx.listener(Self::open_palette))
            .on_action(cx.listener(Self::quick_open))
            .on_action(cx.listener(Self::pick_font))
            .on_action(cx.listener(Self::confirm_delete))
            .on_action(cx.listener(Self::show_toast))
            .on_action(cx.listener(Self::quit))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.canvas)
            .font_family(theme.fonts.ui.clone())
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(colors.ink)
            .child(
                div()
                    .flex_none()
                    .h(theme::TITLEBAR_HEIGHT)
                    .pl(theme::TRAFFIC_LIGHT_RESERVED_WIDTH)
                    .pr(px(16.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .bg(colors.sidebar)
                    .border_b_1()
                    .border_color(colors.hairline)
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(format!("writer-ui gallery · {}", theme.name)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_color(colors.muted)
                            .child("Toggle theme")
                            .children(KeyHint::for_action(&ToggleTheme, cx)),
                    ),
            )
            .child(
                tab_navigation(div().id("gallery-scroll"))
                    .flex_1()
                    .overflow_y_scroll()
                    .p(px(24.))
                    .child(
                        div()
                            .mb(px(20.))
                            .font_family(theme.fonts.document.clone())
                            .text_size(UI_TEXT_TITLE)
                            .line_height(px(28.))
                            .child("Paper and ink: the shared widgets"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_start()
                            .gap(px(20.))
                            .child(fields)
                            .child(buttons)
                            .child(toggles)
                            .child(insert)
                            .child(hints)
                            .child(overlays)
                            .child(icons),
                    ),
            )
            .child(self.modal_layer.clone())
            .child(self.toasts.clone())
    }
}

impl Focusable for Gallery {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn main() {
    Application::new()
        .with_assets(writer_ui::assets::Assets)
        .run(|cx: &mut App| {
            theme::init(ThemeMode::System, cx);
            let c = Some("Gallery");
            cx.bind_keys([
                KeyBinding::new("cmd-t", ToggleTheme, c),
                KeyBinding::new("cmd-k", OpenPalette, c),
                KeyBinding::new("cmd-p", QuickOpen, c),
                KeyBinding::new("cmd-shift-f", PickFont, c),
                KeyBinding::new("cmd-d", ConfirmDelete, c),
                KeyBinding::new("cmd-e", ShowToast, c),
                KeyBinding::new("cmd-q", Quit, None),
            ]);
            writer_ui::init(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let bounds = Bounds::centered(None, size(px(1400.), px(900.)), cx);
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        titlebar: Some(gpui::TitlebarOptions {
                            title: Some("writer-ui gallery".into()),
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
                        let gallery = cx.new(|cx| Gallery::new(window, cx));
                        let focus = gallery.read(cx).title_field.focus_handle(cx);
                        window.focus(&focus);
                        gallery
                    },
                )
                .expect("open gallery window");
            cx.activate(true);
            automation::run_from_env(window.into(), cx);
        });
}
