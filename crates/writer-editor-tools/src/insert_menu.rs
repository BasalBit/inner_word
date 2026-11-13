//! The insert menu: typing `/` at the start of a block or after whitespace
//! opens a list at the caret, filtered by what is typed after the slash.
//!
//! Focus stays in the editor. While the menu is open a key interceptor at
//! overlay priority takes the keys of a list (`up` / `down` / `enter` /
//! `escape` and `ctrl-p` / `ctrl-n` by default; asked of the keymap, see
//! [`InsertMenuKey`]) and `tab`; every other key keeps reaching the
//! document, and the menu follows the document through the editor's
//! events: the query is the text between the slash and the caret.

use gpui::{
    Action, App, AppContext, Context, Entity, Keystroke, SharedString, Subscription, Window, px,
};
use writer_core::{BlockId, BlockKind, Position, TextRange};
use writer_editor::{Editor, InterceptPriority, KeyDisposition, PopupAnchor, actions as editor};
use writer_ui::{KeyHint, keymap, picker};

use crate::{
    actions::{InsertDate, OpenLink},
    menu::{Menu, MenuEntry, MenuItem, MenuOptions, rank},
    tools::{EditorTools, PopupTool},
    util::today_string,
};

/// With no match, the menu closes once the query is this long.
const GIVE_UP_AFTER: usize = 2;

#[derive(Clone)]
enum Effect {
    Kind(BlockKind),
    Divider,
    PageBreak,
    /// Ask for picture files and insert them (the editor's "Image").
    Image,
    Link,
    Date,
}

struct InsertItem {
    label: SharedString,
    keywords: &'static str,
    detail: Option<SharedString>,
    key_hint: Option<KeyHint>,
    effect: Effect,
}

pub(crate) struct InsertMenu {
    pub(crate) menu: Entity<Menu>,
    items: Vec<InsertItem>,
    /// Where the trigger text starts: the slash, or the caret when the menu
    /// was opened by command.
    start: Position,
    block: BlockId,
    /// 1 when a slash was typed, 0 when opened by command.
    trigger_len: usize,
    _keys: Subscription,
    _subscription: Subscription,
}

impl InsertMenu {
    fn entries(&self, query: &str) -> Vec<MenuEntry> {
        let candidates = self
            .items
            .iter()
            .enumerate()
            .map(|(id, item)| (id, item.label.as_ref(), item.keywords));
        rank(query, candidates)
            .into_iter()
            .map(|(id, positions)| {
                let item = &self.items[id];
                MenuEntry::Item(MenuItem {
                    id,
                    label: item.label.clone(),
                    positions,
                    detail: item.detail.clone(),
                    key_hint: item.key_hint.clone(),
                    ..MenuItem::default()
                })
            })
            .collect()
    }
}

/// What a key does to the open insert menu.
///
/// The insert menu is a list like the other menus of the tools, but the
/// keyboard stays in the document while it is open (what is typed after the
/// slash is its filter), so the keys of a list do not reach it as actions:
/// it takes them from the editor, before the editor's own bindings. Which
/// keys those are is asked of the keymap each time, in the place the other
/// lists are in (`Picker`): the keys of "previous", "next", "confirm" and
/// "dismiss" there, whatever they are bound to now. One key is the menu's
/// own and is not a binding: `tab` takes the marked row, as it completes a
/// word elsewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InsertMenuKey {
    Previous,
    Next,
    /// Take the marked row.
    Take,
    Close,
}

impl InsertMenuKey {
    fn of(keystroke: &Keystroke, cx: &App) -> Option<Self> {
        let of_action = |action: &dyn Action| {
            let action = action.as_any();
            if action.is::<picker::SelectPrev>() {
                Some(Self::Previous)
            } else if action.is::<picker::SelectNext>() {
                Some(Self::Next)
            } else if action.is::<picker::Confirm>() {
                Some(Self::Take)
            } else if action.is::<picker::Dismiss>() {
                Some(Self::Close)
            } else {
                None
            }
        };
        let list = [keymap::context(picker::KEY_CONTEXT)];
        keymap::action_for_keystroke_in(keystroke, &list, cx, |action| of_action(action).is_some())
            .and_then(|action| of_action(action.as_ref()))
            .or_else(|| {
                (keystroke.key == "tab" && !keystroke.modifiers.modified()).then_some(Self::Take)
            })
    }
}

/// The rows of the insert menu. `pictures`: the editor takes pictures (see
/// [`takes_pictures`]); "Image" is offered only there.
fn insert_items(pictures: bool, cx: &App) -> Vec<InsertItem> {
    let kind = |label: &'static str, keywords, kind: BlockKind, hint: Option<KeyHint>| InsertItem {
        label: label.into(),
        keywords,
        detail: None,
        key_hint: hint,
        effect: Effect::Kind(kind),
    };
    let hint = |action: &dyn gpui::Action| KeyHint::for_action(action, cx);
    // The blocks a writer reaches for first are in view when the menu
    // opens (ten rows): the three headings a text usually has, the lists,
    // quote, code, and the two separators. The image, link and date
    // follow, then the kinds that are rarely asked for by menu (a paragraph
    // is what `enter` gives; headings 4 to 6).
    let mut items = vec![
        kind(
            "Heading 1",
            "h1 title",
            BlockKind::heading(1),
            hint(&editor::SetHeading1),
        ),
        kind(
            "Heading 2",
            "h2 section",
            BlockKind::heading(2),
            hint(&editor::SetHeading2),
        ),
        kind(
            "Heading 3",
            "h3 subsection",
            BlockKind::heading(3),
            hint(&editor::SetHeading3),
        ),
        kind(
            "Bullet list",
            "ul unordered points",
            BlockKind::BulletItem,
            hint(&editor::ToggleBulletList),
        ),
        kind(
            "Numbered list",
            "ol ordered steps",
            BlockKind::NumberedItem,
            hint(&editor::ToggleNumberedList),
        ),
        kind(
            "To-do list",
            "todo task checkbox checklist",
            BlockKind::todo(),
            hint(&editor::ToggleTodoList),
        ),
        kind(
            "Quote",
            "blockquote citation",
            BlockKind::Quote,
            hint(&editor::ToggleQuote),
        ),
        kind(
            "Code block",
            "pre snippet monospace",
            BlockKind::code(),
            hint(&editor::ToggleCodeBlock),
        ),
        InsertItem {
            label: "Divider".into(),
            keywords: "hr rule line separator",
            detail: None,
            key_hint: hint(&editor::InsertDivider),
            effect: Effect::Divider,
        },
        InsertItem {
            label: "Page break".into(),
            keywords: "new page",
            detail: None,
            key_hint: hint(&editor::InsertPageBreak),
            effect: Effect::PageBreak,
        },
        InsertItem {
            label: "Link".into(),
            keywords: "url hyperlink web address",
            detail: None,
            key_hint: hint(&OpenLink),
            effect: Effect::Link,
        },
        InsertItem {
            label: "Today's date".into(),
            keywords: "date today now day",
            detail: Some(today_string().into()),
            key_hint: hint(&InsertDate),
            effect: Effect::Date,
        },
        kind(
            "Paragraph",
            "text body plain normal",
            BlockKind::Paragraph,
            hint(&editor::SetParagraph),
        ),
        kind(
            "Heading 4",
            "h4",
            BlockKind::heading(4),
            hint(&editor::SetHeading4),
        ),
        kind(
            "Heading 5",
            "h5",
            BlockKind::heading(5),
            hint(&editor::SetHeading5),
        ),
        kind(
            "Heading 6",
            "h6",
            BlockKind::heading(6),
            hint(&editor::SetHeading6),
        ),
    ];
    if pictures {
        let after_page_break = items
            .iter()
            .position(|item| matches!(item.effect, Effect::PageBreak))
            .map_or(items.len(), |index| index + 1);
        items.insert(
            after_page_break,
            InsertItem {
                label: "Image".into(),
                keywords: "picture photo figure screenshot",
                detail: None,
                key_hint: hint(&editor::InsertImage),
                effect: Effect::Image,
            },
        );
    }
    items
}

/// Whether pictures can come into `editor`: a document editor that can be
/// changed and has somebody to store them (an image provider). Only such
/// an editor does anything with `editor::InsertImage`.
pub(crate) fn takes_pictures(editor: &Editor) -> bool {
    !editor.is_read_only() && !editor.config().mode.is_compact() && editor.has_image_provider()
}

impl EditorTools {
    /// Opens the insert menu at the caret, as typing `/` does, but without
    /// a slash: what is typed next filters the menu and is removed when an
    /// item is applied. Does nothing in a read-only editor.
    pub fn open_insert_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only(cx) {
            return;
        }
        self.make_room(window, cx);
        let caret = self.editor.update(cx, |editor, cx| {
            let selection = editor.selection();
            if !selection.is_collapsed() {
                editor.edit(|state| state.set_caret(selection.end()), cx);
            }
            editor.selection().head
        });
        self.editor.read(cx).focus(window);
        self.show_insert_menu(caret, 0, window, cx);
    }

    /// Called after an edit that followed a `/` key press: opens the menu
    /// when the slash landed at the start of a block or after whitespace.
    pub(crate) fn open_insert_menu_after_slash(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = self.editor.read(cx);
        let selection = editor.selection();
        if editor.is_read_only() || !selection.is_collapsed() {
            return;
        }
        let caret = selection.head;
        let Some(block) = editor.document().block(caret.block) else {
            return;
        };
        let Some(before_caret) = block.text().get(..caret.offset) else {
            return;
        };
        let Some(before_slash) = before_caret.strip_suffix('/') else {
            return;
        };
        let slash = before_slash.len();
        let at_boundary = before_slash
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        let in_code =
            block.kind().is_code() || block.attrs_at(slash).is_some_and(|attrs| attrs.code);
        if !at_boundary || in_code {
            return;
        }
        self.make_room(window, cx);
        self.show_insert_menu(Position::new(caret.block, slash), 1, window, cx);
    }

    fn show_insert_menu(
        &mut self,
        start: Position,
        trigger_len: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(block) = self
            .editor
            .read(cx)
            .document()
            .block(start.block)
            .map(|block| block.id())
        else {
            return;
        };
        let menu = cx.new(|cx| {
            Menu::new(
                MenuOptions {
                    placeholder: None,
                    width: px(280.),
                    max_rows: 10,
                    marks: false,
                    empty_text: "No matching block".into(),
                    footer: Vec::new(),
                },
                window,
                cx,
            )
        });
        let keys = self
            .editor
            .read(cx)
            .intercept_keys(InterceptPriority::OVERLAY, {
                let tools = cx.weak_entity();
                move |key, window, cx| {
                    let Some(menu_key) = InsertMenuKey::of(key, cx) else {
                        return KeyDisposition::Pass;
                    };
                    match tools.update(cx, |tools, cx| tools.insert_menu_key(menu_key, window, cx))
                    {
                        Ok(true) => KeyDisposition::Consumed,
                        _ => KeyDisposition::Pass,
                    }
                }
            });
        let tool = InsertMenu {
            menu: menu.clone(),
            items: insert_items(takes_pictures(self.editor.read(cx)), cx),
            start,
            block,
            trigger_len,
            _keys: keys,
            _subscription: cx.subscribe_in(&menu, window, Self::on_menu_event),
        };
        let entries = tool.entries("");
        menu.update(cx, |menu, cx| menu.set_entries(entries, None, cx));
        self.show_popup(
            PopupTool::Insert(tool),
            menu.into(),
            PopupAnchor::Position(start),
            cx,
        );
    }

    /// A menu key while the insert menu is open. Returns whether the key
    /// was used (otherwise it goes on to the document).
    fn insert_menu_key(
        &mut self,
        key: InsertMenuKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(PopupTool::Insert(tool)) = &self.popup else {
            return false;
        };
        let menu = tool.menu.clone();
        match key {
            InsertMenuKey::Previous => menu.update(cx, |menu, cx| menu.select_prev(cx)),
            InsertMenuKey::Next => menu.update(cx, |menu, cx| menu.select_next(cx)),
            InsertMenuKey::Close => self.close_popup(false, window, cx),
            InsertMenuKey::Take => {
                if menu.read(cx).selected_item().is_none() {
                    // Nothing to apply: the key belongs to the document.
                    self.close_popup(false, window, cx);
                    return false;
                }
                self.confirm_insert_menu(window, cx);
            }
        }
        true
    }

    /// The keymap changed under an open insert menu: the keys its rows
    /// show are asked again.
    pub(crate) fn rekey_insert_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(PopupTool::Insert(tool)) = &mut self.popup else {
            return;
        };
        tool.items = insert_items(takes_pictures(self.editor.read(cx)), cx);
        self.sync_insert_menu(window, cx);
    }

    /// Brings the open insert menu in line with the document: recomputes
    /// the query from the text after the slash, and closes the menu when
    /// the slash is gone, the caret left, or nothing matches any more.
    pub(crate) fn sync_insert_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(PopupTool::Insert(tool)) = &self.popup else {
            return;
        };
        let editor = self.editor.read(cx);
        let selection = editor.selection();
        let query = (|| {
            let caret = selection.head;
            if !selection.is_collapsed() || caret.block != tool.start.block {
                return None;
            }
            let block = editor.document().block(caret.block)?;
            let query_start = tool.start.offset + tool.trigger_len;
            if block.id() != tool.block || caret.offset < query_start {
                return None;
            }
            let trigger = block.text().get(tool.start.offset..query_start)?;
            if tool.trigger_len == 1 && trigger != "/" {
                return None;
            }
            let query = block.text().get(query_start..caret.offset)?;
            let valid = !query.contains('\n') && !query.starts_with(char::is_whitespace);
            valid.then(|| query.to_string())
        })();
        let entries = query.as_deref().map(|query| tool.entries(query));
        match (query, entries) {
            (Some(query), Some(entries))
                if !entries.is_empty() || query.chars().count() < GIVE_UP_AFTER =>
            {
                tool.menu
                    .update(cx, |menu, cx| menu.set_entries(entries, None, cx));
            }
            _ => self.close_popup(false, window, cx),
        }
    }

    /// Applies the selected item: removes the slash and the query, then
    /// does what the item says, as one undo step.
    pub(crate) fn confirm_insert_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(PopupTool::Insert(tool)) = &self.popup else {
            return;
        };
        let Some(effect) = tool
            .menu
            .read(cx)
            .selected_id()
            .map(|id| tool.items[id].effect.clone())
        else {
            return;
        };
        let start = tool.start;
        self.close_popup(false, window, cx);
        let today = today_string();
        self.editor.update(cx, |editor, cx| {
            if editor.is_read_only() {
                return;
            }
            editor.edit(
                |state| {
                    state.grouped(|state| {
                        let caret = state.selection().head;
                        if caret > start {
                            state.delete_range(TextRange::new(start, caret));
                        }
                        state.set_caret(start);
                        match &effect {
                            Effect::Kind(kind) => {
                                // `set_block_kind` toggles; the menu sets.
                                let same = state
                                    .document()
                                    .block(start.block)
                                    .is_some_and(|block| block.kind().same_type(kind));
                                if !same {
                                    state.set_block_kind(kind.clone());
                                }
                            }
                            Effect::Divider => {
                                state.insert_divider();
                            }
                            Effect::PageBreak => {
                                state.insert_page_break();
                            }
                            Effect::Date => {
                                state.insert_text(&today);
                            }
                            // What these two do comes after the edit:
                            // the slash and the query are gone by then.
                            Effect::Link | Effect::Image => {}
                        }
                    })
                },
                cx,
            );
        });
        match effect {
            Effect::Link => self.open_link(window, cx),
            // The editor's own command, where the keyboard is (in the
            // text: the insert menu never took it). It asks for files and
            // puts the pictures where the caret is by then: in the
            // paragraph the slash emptied, or after the caret's block.
            Effect::Image => window.dispatch_action(Box::new(editor::InsertImage), cx),
            _ => {}
        }
    }

    /// Inserts today's date at the caret (replacing the selection).
    pub fn insert_date(&mut self, cx: &mut Context<Self>) {
        let today = today_string();
        self.editor.update(cx, |editor, cx| {
            if !editor.is_read_only() {
                editor.edit(|state| state.insert_text(&today), cx);
            }
        });
    }
}
