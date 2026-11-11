//! [`EditorTools`]: the view that wraps an editor and gives it the tools.

use gpui::{
    AnyView, App, Context, Entity, EntityId, EventEmitter, FocusHandle, Focusable, IntoElement,
    Render, Subscription, Window, div, prelude::*,
};
use writer_editor::{Editor, EditorEvent, PopupAnchor, PopupPlacement};
use writer_ui::theme::GlobalTheme;

use crate::{
    actions::*,
    find::{FindBar, FindState},
    font_picker::FontMenu,
    format_menu::FormatMenu,
    insert_menu::InsertMenu,
    link::LinkTool,
    menu::{Mark, Menu, MenuEntry, MenuEvent},
};

/// The tools of this crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolKind {
    /// The contextual format menu (`cmd-.`).
    Format,
    /// The insert menu (`/`).
    Insert,
    /// The link popover (`cmd-k`).
    Link,
    /// The font picker (`cmd-t`).
    Font,
    /// The find and replace bar (`cmd-f`, `cmd-alt-f`).
    Find,
}

/// Emitted by [`EditorTools`] when a tool opens or closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolsEvent {
    Opened(ToolKind),
    Closed(ToolKind),
}

/// Where [`EditorTools`] puts the find bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FindBarPlacement {
    /// Above the editor.
    #[default]
    Top,
    /// Below the editor.
    Bottom,
}

/// One row of the open menu, for tests and automation
/// ([`EditorTools::menu_rows`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuRow {
    pub label: String,
    /// Secondary text after the label (a current value, a sample).
    pub detail: Option<String>,
    /// A section header (not selectable).
    pub header: bool,
    pub mark: Mark,
    /// The key binding shown at the right, as glyphs ("⌘B").
    pub key_hint: Option<String>,
    pub selected: bool,
}

/// The popup tool that is open (the editor hosts one popup at a time).
pub(crate) enum PopupTool {
    Format(FormatMenu),
    Insert(InsertMenu),
    Link(LinkTool),
    Font(FontMenu),
}

impl PopupTool {
    pub(crate) fn kind(&self) -> ToolKind {
        match self {
            PopupTool::Format(_) => ToolKind::Format,
            PopupTool::Insert(_) => ToolKind::Insert,
            PopupTool::Link(_) => ToolKind::Link,
            PopupTool::Font(_) => ToolKind::Font,
        }
    }

    /// The entity shown as the editor's popup.
    fn view_id(&self) -> EntityId {
        match self {
            PopupTool::Format(tool) => tool.menu.entity_id(),
            PopupTool::Insert(tool) => tool.menu.entity_id(),
            PopupTool::Link(tool) => tool.popover.entity_id(),
            PopupTool::Font(tool) => tool.menu.entity_id(),
        }
    }

    pub(crate) fn menu(&self) -> Option<&Entity<Menu>> {
        match self {
            PopupTool::Format(tool) => Some(&tool.menu),
            PopupTool::Insert(tool) => Some(&tool.menu),
            PopupTool::Font(tool) => Some(&tool.menu),
            PopupTool::Link(_) => None,
        }
    }
}

/// An editor with its contextual tools: the format menu, the insert menu,
/// the link popover, the font picker and the find and replace bar.
///
/// Mount this view where the editor would go. It renders the editor (and
/// the find bar above or below it while that is open), handles the actions
/// in [`crate::actions`] and floats the menus through the editor's popup
/// seam. See the crate documentation for the snippet.
///
/// One popup tool is open at a time; the find bar stays open beside it.
/// `escape` closes the open tool and returns focus to the editor.
pub struct EditorTools {
    pub(crate) editor: Entity<Editor>,
    pub(crate) popup: Option<PopupTool>,
    pub(crate) find: FindState,
    pub(crate) find_bar: Option<FindBar>,
    find_placement: FindBarPlacement,
    /// The text input that the next `Edited` event belongs to was a single
    /// `/` (set by `EditorEvent::TextInput`, consumed by the `Edited` that
    /// always follows it).
    slash_typed: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ToolsEvent> for EditorTools {}

impl Focusable for EditorTools {
    /// The editor's focus handle: focusing the tools view focuses the
    /// editor.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl EditorTools {
    /// Wraps `editor`. Create it with
    /// `cx.new(|cx| EditorTools::new(editor.clone(), window, cx))` and
    /// render the returned entity instead of the editor.
    pub fn new(editor: Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.subscribe_in(&editor, window, Self::on_editor_event),
            // The highlight colours and the swatches of an open format menu
            // come from the theme.
            cx.observe_global::<GlobalTheme>(|this, cx| {
                this.apply_find_highlights(cx);
                this.retheme_format_menu(cx);
                cx.notify();
            }),
        ];
        Self {
            editor,
            popup: None,
            find: FindState::default(),
            find_bar: None,
            find_placement: FindBarPlacement::default(),
            slash_typed: false,
            _subscriptions: subscriptions,
        }
    }

    /// The wrapped editor.
    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    /// Puts the find bar above or below the editor.
    pub fn set_find_bar_placement(&mut self, placement: FindBarPlacement, cx: &mut Context<Self>) {
        if self.find_placement != placement {
            self.find_placement = placement;
            cx.notify();
        }
    }

    /// Where the find bar goes.
    pub fn find_bar_placement(&self) -> FindBarPlacement {
        self.find_placement
    }

    /// The open tool: a popup tool if one is open, else the find bar.
    pub fn active_tool(&self) -> Option<ToolKind> {
        match &self.popup {
            Some(popup) => Some(popup.kind()),
            None => self.find_bar.as_ref().map(|_| ToolKind::Find),
        }
    }

    /// Whether `kind` is open.
    pub fn is_open(&self, kind: ToolKind) -> bool {
        match kind {
            ToolKind::Find => self.find_bar.is_some(),
            _ => self
                .popup
                .as_ref()
                .is_some_and(|popup| popup.kind() == kind),
        }
    }

    /// Closes whatever is open and returns focus to the editor.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_popup(false, window, cx);
        self.close_find_bar(cx);
        self.end_find_flash(cx);
        self.editor.read(cx).focus(window);
    }

    /// The rows of the open menu (format menu, insert menu or font picker),
    /// in display order. Empty when no menu is open. For tests and
    /// automation.
    pub fn menu_rows(&self, cx: &App) -> Vec<MenuRow> {
        let Some(menu) = self.popup.as_ref().and_then(PopupTool::menu) else {
            return Vec::new();
        };
        let menu = menu.read(cx);
        let selected = menu.selected_index();
        menu.entries()
            .iter()
            .enumerate()
            .map(|(index, entry)| match entry {
                MenuEntry::Header(title) => MenuRow {
                    label: title.to_string(),
                    detail: None,
                    header: true,
                    mark: Mark::None,
                    key_hint: None,
                    selected: false,
                },
                MenuEntry::Item(item) => MenuRow {
                    label: item.label.to_string(),
                    detail: item.detail.as_ref().map(|detail| detail.to_string()),
                    header: false,
                    mark: item.mark,
                    key_hint: item.key_hint.as_ref().map(|hint| hint.text()),
                    selected: selected == Some(index),
                },
            })
            .collect()
    }

    /// The label of the selected row of the open menu.
    pub fn selected_menu_label(&self, cx: &App) -> Option<String> {
        let menu = self.popup.as_ref().and_then(PopupTool::menu)?;
        menu.read(cx)
            .selected_item()
            .map(|item| item.label.to_string())
    }

    // -----------------------------------------------------------------
    // Popup plumbing
    // -----------------------------------------------------------------

    pub(crate) fn read_only(&self, cx: &App) -> bool {
        self.editor.read(cx).is_read_only()
    }

    /// Makes room for a popup tool that is about to open: closes the open
    /// one (the editor hosts one popup). The find bar stays: it is not in
    /// the way, and closing it would lose the search the user is in the
    /// middle of. Focus is left alone (the new tool decides).
    pub(crate) fn make_room(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_popup(false, window, cx);
    }

    /// Shows `tool` as the editor's popup.
    pub(crate) fn show_popup(
        &mut self,
        tool: PopupTool,
        view: AnyView,
        anchor: PopupAnchor,
        cx: &mut Context<Self>,
    ) {
        let kind = tool.kind();
        self.editor.update(cx, |editor, cx| {
            editor.show_popup(view, anchor, PopupPlacement::Below, cx)
        });
        self.popup = Some(tool);
        cx.emit(ToolsEvent::Opened(kind));
        cx.notify();
    }

    /// Closes the open popup tool, if any. With `refocus`, focus goes back
    /// to the editor.
    pub(crate) fn close_popup(
        &mut self,
        refocus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tool) = self.popup.take() else {
            return;
        };
        let view_id = tool.view_id();
        self.editor.update(cx, |editor, cx| {
            // Another crate may have replaced the popup meanwhile.
            if editor
                .popup_view()
                .is_some_and(|view| view.entity_id() == view_id)
            {
                editor.hide_popup(cx);
            }
        });
        if refocus {
            self.editor.read(cx).focus(window);
        }
        cx.emit(ToolsEvent::Closed(tool.kind()));
        cx.notify();
    }

    /// Whether keyboard focus is inside the open popup tool.
    fn popup_has_focus(&self, window: &Window, cx: &App) -> bool {
        match &self.popup {
            Some(PopupTool::Link(tool)) => tool.popover.read(cx).field.read(cx).is_focused(window),
            Some(PopupTool::Insert(_)) | None => false,
            Some(tool) => tool
                .menu()
                .is_some_and(|menu| menu.read(cx).focus_handle(cx).is_focused(window)),
        }
    }

    /// Whether the popup tool showing `menu` is still the open one.
    fn is_current_menu(&self, menu: &Entity<Menu>) -> bool {
        self.popup
            .as_ref()
            .and_then(PopupTool::menu)
            .is_some_and(|current| current == menu)
    }

    pub(crate) fn on_menu_event(
        &mut self,
        menu: &Entity<Menu>,
        event: &MenuEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_current_menu(menu) {
            return;
        }
        match event {
            MenuEvent::QueryChanged => match &self.popup {
                Some(PopupTool::Format(_)) => self.refresh_format_menu(cx),
                Some(PopupTool::Font(_)) => self.refresh_font_menu(cx),
                _ => {}
            },
            MenuEvent::Confirmed { secondary } => match &self.popup {
                Some(PopupTool::Format(_)) => self.confirm_format_menu(window, cx),
                Some(PopupTool::Insert(_)) => self.confirm_insert_menu(window, cx),
                Some(PopupTool::Font(_)) => self.confirm_font_menu(*secondary, window, cx),
                _ => {}
            },
            MenuEvent::Dismissed => self.close_popup(true, window, cx),
            MenuEvent::Blurred => self.popup_lost_focus(window, cx),
            MenuEvent::ClickedOutside => self.popup_clicked_outside(window, cx),
        }
    }

    /// A mouse button went down outside the open popup tool: the tool
    /// closes. Where the press itself took the keyboard somewhere (into
    /// the text, a field of the find bar, another panel) it stays there;
    /// where it did not (a button, the window's chrome) the keyboard would
    /// be left in a field that is gone, so it returns to the editor.
    pub(crate) fn popup_clicked_outside(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let keyboard_in_popup = self.popup_has_focus(window, cx);
        self.close_popup(keyboard_in_popup, window, cx);
    }

    /// The field of the open popup tool lost focus.
    ///
    /// GPUI reports a blur both when focus moves inside the window and when
    /// the window is deactivated. Switching to another app must not close
    /// the tool (the link popover has just opened the browser, say): it
    /// keeps its focus and is still there when the user comes back. When
    /// focus moved somewhere else in the window (a click in the document,
    /// another panel), the tool closes without pulling focus back; when
    /// focus went nowhere, it goes to the editor.
    pub(crate) fn popup_lost_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !window.is_window_active() {
            return;
        }
        let nowhere = window.focused(cx).is_none();
        self.close_popup(nowhere, window, cx);
    }

    // -----------------------------------------------------------------
    // Editor events
    // -----------------------------------------------------------------

    fn on_editor_event(
        &mut self,
        _: &Entity<Editor>,
        event: &EditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            EditorEvent::TextInput { text, .. } => {
                // Announces the `Edited` of the same insertion.
                self.slash_typed = text == "/";
            }
            EditorEvent::Edited { change } => {
                // Only typing opens the insert menu: an edit that was not
                // announced by a text input (paste, undo, a command, another
                // crate) finds the flag down.
                let slash = std::mem::take(&mut self.slash_typed);
                if change.blocks.is_some() {
                    if matches!(self.popup, Some(PopupTool::Insert(_))) {
                        self.sync_insert_menu(window, cx);
                    } else if slash && self.popup.is_none() {
                        self.open_insert_menu_after_slash(window, cx);
                    }
                    if self.find_results_visible() {
                        self.find_document_edited(change, cx);
                    }
                }
            }
            EditorEvent::SelectionChanged => {
                if matches!(self.popup, Some(PopupTool::Insert(_))) {
                    self.sync_insert_menu(window, cx);
                }
                if self.find_results_visible() {
                    self.sync_find_current(cx);
                }
            }
            EditorEvent::Blurred => {
                // The insert menu lives on the editor's keyboard focus. A
                // deactivated window is not a reason to close it.
                if matches!(self.popup, Some(PopupTool::Insert(_))) && window.is_window_active() {
                    self.close_popup(false, window, cx);
                }
            }
            EditorEvent::DocumentReplaced => {
                // The editor dropped its popup and its highlight layers. If
                // the popup had the keyboard, the editor gets it back.
                let had_focus = self.popup_has_focus(window, cx);
                self.close_popup(had_focus, window, cx);
                self.slash_typed = false;
                if self.find_bar.is_some() {
                    self.refresh_find(false, cx);
                } else {
                    self.end_find_flash(cx);
                }
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------
    // Action handlers
    // -----------------------------------------------------------------

    /// `escape` with the keyboard in the document (the fields of the tools
    /// take their own `escape`): closes what is open, one thing at a time,
    /// the popup tool before the find bar. With nothing open the key goes
    /// on.
    fn on_close_tool(&mut self, _: &CloseTool, window: &mut Window, cx: &mut Context<Self>) {
        if self.popup.is_some() {
            self.close_popup(true, window, cx);
        } else if self.find_bar.is_some() {
            self.close_find(window, cx);
        } else if !self.end_find_flash(cx) {
            cx.propagate();
        }
    }

    /// The find actions that need the bar: with the bar closed they open
    /// it (with the replace row when `replace`) instead of doing nothing,
    /// so a command from the palette always shows what it did.
    fn with_find_bar(
        &mut self,
        replace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>),
    ) {
        if self.find_bar.is_none() {
            self.open_find(replace, window, cx);
        }
        then(self, window, cx);
    }
}

impl Render for EditorTools {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bar = self.render_find_bar(window, cx);
        let (top, bottom) = match self.find_placement {
            FindBarPlacement::Top => (bar, None),
            FindBarPlacement::Bottom => (None, bar),
        };
        div()
            .key_context(KEY_CONTEXT)
            .size_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(|this, _: &OpenFormatMenu, window, cx| {
                this.toggle_format_menu(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &OpenInsertMenu, window, cx| {
                    this.open_insert_menu(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &OpenLink, window, cx| this.toggle_link(window, cx)))
            .on_action(cx.listener(|this, _: &RemoveLink, window, cx| this.remove_link(window, cx)))
            .on_action(cx.listener(|this, _: &OpenLinkInBrowser, window, cx| {
                this.open_link_in_browser(window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenFontPicker, window, cx| {
                this.toggle_font_picker(window, cx)
            }))
            .on_action(cx.listener(|this, _: &InsertDate, _, cx| this.insert_date(cx)))
            .on_action(cx.listener(|this, _: &Find, window, cx| this.open_find(false, window, cx)))
            .on_action(cx.listener(|this, _: &FindAndReplace, window, cx| {
                this.toggle_find_replace(window, cx)
            }))
            .on_action(cx.listener(|this, _: &FindNext, window, cx| this.find_next(window, cx)))
            .on_action(
                cx.listener(|this, _: &FindPrevious, window, cx| this.find_previous(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleCaseSensitive, window, cx| {
                this.with_find_bar(false, window, cx, |this, _, cx| {
                    this.toggle_case_sensitive(cx)
                })
            }))
            .on_action(cx.listener(|this, _: &ToggleWholeWord, window, cx| {
                this.with_find_bar(false, window, cx, |this, _, cx| this.toggle_whole_word(cx))
            }))
            .on_action(cx.listener(|this, _: &ReplaceCurrent, window, cx| {
                // Opening the bar is all the first press does: nothing is
                // replaced before the user has seen what and with what.
                if this.find_bar.is_none() {
                    this.open_find(true, window, cx);
                } else {
                    this.replace_current(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ReplaceAll, window, cx| {
                if this.find_bar.is_none() {
                    this.open_find(true, window, cx);
                } else {
                    this.replace_all(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &FocusNextField, window, cx| {
                this.focus_other_find_field(window, cx)
            }))
            .on_action(cx.listener(|this, _: &FocusPreviousField, window, cx| {
                this.focus_other_find_field(window, cx)
            }))
            .on_action(cx.listener(Self::on_close_tool))
            .children(top)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(self.editor.clone())
                    .children(self.render_find_flash(cx)),
            )
            .children(bottom)
    }
}
