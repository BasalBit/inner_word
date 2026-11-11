//! The context menu of the text: what a right-click in the document (or
//! `ctrl-enter`) offers.
//!
//! The editor adjusts the selection and asks
//! (`EditorEvent::ContextMenuRequested`); this module composes the menu for
//! that request and runs the row that is chosen. Every row runs the code
//! its key runs: rows with an action dispatch that action where the
//! keyboard is in the text, the formatting rows are the rows of the format
//! menu (`cmd-.`), and each shows the key the registry has for it.
//!
//! The rows, in order, each group only where it applies:
//!
//! 1. on a link: Open Link, Edit Link, Remove Link, Copy Link Address (the
//!    whole link, whatever part of it is selected);
//! 2. Cut, Copy, Paste, Paste as Plain Text;
//! 3. with a selection: Bold, Italic, Underline, Strikethrough, Inline
//!    Code, Highlight and Text Colour as submenus, Link, Font, Clear
//!    Formatting;
//! 4. the block: Turn Into, Align and Text Size as submenus, and on a
//!    to-do item Mark as Done / Mark as Not Done;
//! 5. Insert as a submenu (divider, page break, date, link);
//! 6. Find in Document, Select All;
//! 7. what the host adds ([`EditorTools::set_text_menu_extension`]).
//!
//! The menu itself is `writer_ui::ContextMenu`, the app's one menu widget:
//! it never takes the keyboard, so the editor keeps it, an action row
//! arrives where its key would, and closing the menu needs no focus
//! restore.
//!
//! A row that cannot apply right now (Cut without a selection, anything
//! that changes a read-only document, bold in a code block) is shown
//! disabled rather than left out, so the menu keeps its shape.

use std::rc::Rc;

use gpui::{
    Action, App, ClipboardItem, Context, DismissEvent, Entity, SharedString, Subscription,
    WeakEntity, Window,
};
use writer_core::{BlockKind, Choice, TextRange};
use writer_editor::{ContextMenuRequest, actions as editor};
use writer_ui::{ContextMenu, ContextMenuItem, KeyHint, MenuAnchor};

use crate::{
    actions::{Find, InsertDate, OpenLink, RemoveLink},
    format_menu::{Effect, FormatItem, Section, format_items},
    menu::Mark,
    tools::{EditorTools, ToolKind, ToolsEvent},
};

/// What the host adds to the text menu for a request.
pub(crate) type TextMenuExtension =
    Rc<dyn Fn(&ContextMenuRequest, &mut Window, &mut App) -> Vec<ContextMenuItem>>;

/// The text menu that is open.
pub(crate) struct OpenTextMenu {
    pub(crate) request: ContextMenuRequest,
    pub(crate) menu: Entity<ContextMenu>,
    _subscription: Subscription,
}

/// What choosing a row does, for the rows that are not a plain action.
#[derive(Clone)]
pub(crate) enum Run {
    /// A row of the format menu.
    Format(Effect),
    OpenUrl(String),
    EditLink(TextRange),
    RemoveLink(TextRange),
    CopyText(String),
}

enum What {
    /// Dispatched where the keyboard is in the text: the path of the
    /// action's key binding.
    Action(Rc<dyn Action>),
    Run(Run),
    Submenu(Vec<Row>),
}

/// A row while the menu is put together.
struct Row {
    label: SharedString,
    key_hint: Option<KeyHint>,
    /// `Some` for a row with a state: its check mark column.
    checked: Option<bool>,
    enabled: bool,
    what: What,
}

impl Row {
    fn new(label: impl Into<SharedString>, what: What) -> Self {
        Self {
            label: label.into(),
            key_hint: None,
            checked: None,
            enabled: true,
            what,
        }
    }

    fn run(label: &'static str, run: Run) -> Self {
        Self::new(label, What::Run(run))
    }

    /// A row that dispatches `action` and shows its key.
    fn action(label: &'static str, action: impl Action, cx: &App) -> Self {
        let key_hint = KeyHint::for_action(&action, cx);
        Self::new(label, What::Action(Rc::new(action))).key(key_hint)
    }

    /// A row of the format menu under its menu name. With `state`, it
    /// carries a check mark when it is on for the whole selection (a
    /// value that only part of the selection has is not checked).
    fn format(item: &FormatItem, state: bool) -> Self {
        let what = match &item.effect {
            Effect::Editor(action) => What::Action(action.clone()),
            effect => What::Run(Run::Format(effect.clone())),
        };
        let mut row = Self::new(item.short, what).key(item.key_hint.clone());
        if state {
            row.checked = Some(item.mark == Mark::Check);
        }
        row
    }

    /// A submenu. It is disabled when none of its rows can be chosen.
    fn submenu(label: &'static str, rows: Vec<Row>) -> Self {
        let enabled = rows.iter().any(|row| row.enabled);
        Self::new(label, What::Submenu(rows)).enabled(enabled)
    }

    fn key(mut self, hint: Option<KeyHint>) -> Self {
        self.key_hint = hint;
        self
    }

    fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    fn into_item(self, tools: &WeakEntity<EditorTools>) -> ContextMenuItem {
        let item = match self.what {
            What::Action(action) => ContextMenuItem::action_boxed(self.label, action.boxed_clone()),
            What::Run(run) => {
                let tools = tools.clone();
                ContextMenuItem::new(self.label, move |window, cx| {
                    tools
                        .update(cx, |tools, cx| {
                            tools.run_text_menu_row(run.clone(), window, cx)
                        })
                        .ok();
                })
            }
            What::Submenu(rows) => ContextMenuItem::submenu(
                self.label,
                rows.into_iter().map(|row| row.into_item(tools)).collect(),
            ),
        };
        let item = item.key_hint(self.key_hint).disabled(!self.enabled);
        match self.checked {
            Some(checked) => item.checked(checked),
            None => item,
        }
    }
}

impl EditorTools {
    /// Lets the host add its own items to the text context menu.
    ///
    /// `extension` is called each time the menu is about to open, with the
    /// editor's request: where the pointer is, whether there is a
    /// selection, the link and the comment threads under the pointer. The
    /// items it returns are shown after the menu's own, behind a
    /// separator; put [`ContextMenuItem::separator`] between groups of
    /// your own. An empty list adds nothing. By the time it is called the
    /// selection is what the menu acts on and the editor has the keyboard;
    /// both are still so when an item runs, so an action item arrives
    /// exactly as its key would.
    ///
    /// The extension runs outside any update of this view: it may read
    /// and update whatever it likes, this view included.
    ///
    /// ```ignore
    /// tools.update(cx, |tools, _| {
    ///     tools.set_text_menu_extension(move |request, _window, _cx| {
    ///         let mut items = Vec::new();
    ///         if request.has_selection {
    ///             items.push(ContextMenuItem::action("Add Comment", comments::AddComment));
    ///         }
    ///         if let Some(thread) = request.comments.first().copied() {
    ///             let panel = panel.clone();
    ///             items.push(ContextMenuItem::new("Go to Comment", move |window, cx| {
    ///                 panel.update(cx, |panel, cx| panel.reveal(thread, window, cx))
    ///             }));
    ///         }
    ///         items
    ///     })
    /// });
    /// ```
    pub fn set_text_menu_extension(
        &mut self,
        extension: impl Fn(&ContextMenuRequest, &mut Window, &mut App) -> Vec<ContextMenuItem> + 'static,
    ) {
        self.text_menu_extension = Some(Rc::new(extension));
    }

    /// Whether the text context menu is open.
    pub fn text_menu_open(&self) -> bool {
        self.text_menu.is_some()
    }

    /// The open text context menu (the shared widget: its items, bounds
    /// and selection), for tests and automation.
    pub fn text_menu(&self) -> Option<&Entity<ContextMenu>> {
        self.text_menu.as_ref().map(|open| &open.menu)
    }

    /// The request the open text context menu was composed for.
    pub fn text_menu_request(&self) -> Option<&ContextMenuRequest> {
        self.text_menu.as_ref().map(|open| &open.request)
    }

    /// Opens the text context menu for `request` (the editor's
    /// `EditorEvent::ContextMenuRequested` does this): this crate's items
    /// from [`EditorTools::text_menu_items`], then the host's. A
    /// right-click opens it at the pointer; `ctrl-enter` drops it down
    /// from the caret with the first item marked. An open popup tool
    /// closes; the find bar stays.
    ///
    /// The menu opens right after the current update (the host's
    /// extension runs outside it).
    pub fn open_text_menu(
        &mut self,
        request: ContextMenuRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_popup(false, window, cx);
        let mut items = self.text_menu_items(&request, cx);
        let extension = self.text_menu_extension.clone();
        let caret = self.editor.read(cx).caret_bounds();
        let anchor = match caret {
            Some(caret) if request.from_keyboard => MenuAnchor::Below(caret),
            _ => MenuAnchor::Point(request.position),
        };
        let this = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            if let Some(extension) = extension {
                let extra = extension(&request, window, cx);
                if !extra.is_empty() {
                    items.push(ContextMenuItem::separator());
                    items.extend(extra);
                }
            }
            let menu = ContextMenu::open(anchor, items, window, cx);
            if request.from_keyboard {
                menu.update(cx, |menu, cx| menu.select_first(cx));
            }
            this.update(cx, |this, cx| {
                let subscription = cx.subscribe(&menu, |this, menu, _: &DismissEvent, cx| {
                    if this
                        .text_menu
                        .as_ref()
                        .is_some_and(|open| open.menu == menu)
                    {
                        this.text_menu = None;
                        cx.emit(ToolsEvent::Closed(ToolKind::ContextMenu));
                        cx.notify();
                    }
                });
                this.text_menu = Some(OpenTextMenu {
                    request,
                    menu,
                    _subscription: subscription,
                });
                cx.emit(ToolsEvent::Opened(ToolKind::ContextMenu));
                cx.notify();
            })
            .ok();
        });
    }

    /// Closes the text context menu, if it is the menu open in the
    /// window. Focus is not touched (the menu never had it).
    pub(crate) fn close_text_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &self.text_menu else {
            return;
        };
        if ContextMenu::current(window, cx).is_some_and(|current| current == open.menu) {
            // The menu tells us (`DismissEvent`); the state follows there.
            ContextMenu::dismiss(window, cx);
        } else {
            self.text_menu = None;
            cx.notify();
        }
    }

    /// This crate's items of the text context menu for `request`, in
    /// order, groups separated by separators (without the host's items):
    /// the link under the pointer; cut, copy and paste; the formatting of
    /// the selection; the block; insert; find and select all. Items that
    /// cannot apply right now are disabled.
    pub fn text_menu_items(&self, request: &ContextMenuRequest, cx: &App) -> Vec<ContextMenuItem> {
        let mut items = Vec::new();
        for section in self.text_menu_sections(request, cx) {
            if !items.is_empty() {
                items.push(ContextMenuItem::separator());
            }
            items.extend(section.into_iter().map(|row| row.into_item(&self.this)));
        }
        items
    }

    fn text_menu_sections(&self, request: &ContextMenuRequest, cx: &App) -> Vec<Vec<Row>> {
        let editor = self.editor.read(cx);
        let state = editor.state();
        let document = editor.document();
        let editable = !editor.is_read_only();
        let has_selection = !editor.selection().is_collapsed();
        let format = state.format_state();
        let items = format_items(state, cx);
        let block_kind = document
            .block(request.block)
            .map(|block| block.kind().clone());
        let on_void = block_kind.as_ref().is_some_and(BlockKind::is_void);
        // A code block's text carries no inline formatting.
        let all_code = matches!(&format.kind, Choice::Same(kind) if kind.is_code());
        let section = |section: Section| items.iter().filter(move |item| item.section == section);
        let format_rows = |wanted: Section, state: bool, enabled: bool| -> Vec<Row> {
            section(wanted)
                .map(|item| Row::format(item, state).enabled(enabled))
                .collect()
        };
        let mut sections: Vec<Vec<Row>> = Vec::new();

        // 1. The link under the pointer.
        if let Some(url) = &request.link {
            let range = request.link_range.or_else(|| {
                document
                    .link_at(request.document_position)
                    .map(|link| link.range)
            });
            let mut rows = vec![Row::run("Open Link", Run::OpenUrl(url.clone()))];
            if let Some(range) = range {
                rows.push(
                    Row::run("Edit Link", Run::EditLink(range))
                        .key(KeyHint::for_action(&OpenLink, cx))
                        .enabled(editable),
                );
                rows.push(
                    Row::run("Remove Link", Run::RemoveLink(range))
                        .key(KeyHint::for_action(&RemoveLink, cx))
                        .enabled(editable),
                );
            }
            rows.push(Row::run("Copy Link Address", Run::CopyText(url.clone())));
            sections.push(rows);
        }

        // 2. The clipboard.
        let can_paste = editable && cx.read_from_clipboard().is_some();
        sections.push(vec![
            Row::action("Cut", editor::Cut, cx).enabled(editable && has_selection),
            Row::action("Copy", editor::Copy, cx).enabled(has_selection),
            Row::action("Paste", editor::Paste, cx).enabled(can_paste),
            Row::action("Paste as Plain Text", editor::PasteAsPlainText, cx).enabled(can_paste),
        ]);

        // 3. The selected text.
        if has_selection {
            let inline = editable && !all_code;
            let mut rows = Vec::new();
            let mut tail = Vec::new();
            for item in section(Section::Text) {
                match &item.effect {
                    // "Edit Link" above is this row for a link.
                    Effect::Link if request.link.is_some() => {}
                    Effect::Link | Effect::Font => {
                        tail.push(Row::format(item, false).enabled(inline))
                    }
                    _ if item.short == "Clear Formatting" => {
                        tail.push(Row::format(item, false).enabled(inline))
                    }
                    _ => rows.push(Row::format(item, true).enabled(inline)),
                }
            }
            let mut highlights = format_rows(Section::Highlight, true, inline);
            highlights.push(
                Row::run("None", Run::Format(Effect::NoHighlight))
                    .checked(matches!(&format.highlight, Choice::Same(None)))
                    .enabled(inline),
            );
            rows.push(Row::submenu("Highlight", highlights));
            rows.push(Row::submenu(
                "Text Colour",
                format_rows(Section::Color, true, inline),
            ));
            rows.extend(tail);
            sections.push(rows);
        }

        // 4. The block.
        let block = editable && !on_void;
        let mut rows = vec![
            Row::submenu("Turn Into", format_rows(Section::TurnInto, true, block)),
            Row::submenu("Align", format_rows(Section::Align, true, block)),
            Row::submenu("Text Size", format_rows(Section::Size, false, block)),
        ];
        if let Some(BlockKind::TodoItem { checked }) = block_kind {
            rows.push(
                Row::action(
                    if checked {
                        "Mark as Not Done"
                    } else {
                        "Mark as Done"
                    },
                    editor::ToggleTodoChecked,
                    cx,
                )
                .enabled(editable),
            );
        }
        sections.push(rows);

        // 5. Insert.
        sections.push(vec![Row::submenu(
            "Insert",
            vec![
                Row::action("Divider", editor::InsertDivider, cx).enabled(editable),
                Row::action("Page Break", editor::InsertPageBreak, cx).enabled(editable),
                Row::action("Today's Date", InsertDate, cx).enabled(editable),
                Row::action("Link", OpenLink, cx).enabled(editable),
            ],
        )]);

        // 6. The document.
        sections.push(vec![
            Row::action("Find in Document", Find, cx),
            Row::action("Select All", editor::SelectAll, cx),
        ]);

        sections
    }

    /// Does what a chosen row stands for. The menu is closed and the
    /// keyboard is in the text by now.
    pub(crate) fn run_text_menu_row(
        &mut self,
        run: Run,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match run {
            Run::Format(effect) => self.apply_format_effect(effect, window, cx),
            Run::OpenUrl(url) => cx.open_url(&url),
            Run::EditLink(range) => {
                // The whole link is what is edited, whatever part of it
                // the right-click selected.
                self.editor.update(cx, |editor, cx| {
                    editor.edit(|state| state.select_range(range), cx);
                });
                self.open_link(window, cx);
            }
            Run::RemoveLink(range) => self.editor.update(cx, |editor, cx| {
                if !editor.is_read_only() {
                    editor.edit(
                        |state| {
                            state.select_range(range);
                            state.remove_link()
                        },
                        cx,
                    );
                }
            }),
            Run::CopyText(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
        }
    }
}
