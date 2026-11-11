//! [`EditorTools`]: the view that wraps an editor and gives it the tools.

use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyView, App, Context, Entity, EntityId, EventEmitter, FocusHandle, Focusable, IntoElement,
    Render, Subscription, Window, div, prelude::*,
};
use writer_editor::{
    Editor, EditorEvent, InterceptPriority, KeyDisposition, PopupAnchor, PopupPlacement,
};
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
/// One tool is open at a time. `escape` closes the open tool and returns
/// focus to the editor.
pub struct EditorTools {
    pub(crate) editor: Entity<Editor>,
    pub(crate) popup: Option<PopupTool>,
    pub(crate) find: FindState,
    pub(crate) find_bar: Option<FindBar>,
    find_placement: FindBarPlacement,
    /// Set by the key interceptor when `/` was just pressed; consumed by
    /// the next edit event.
    slash_typed: Rc<Cell<bool>>,
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
        let slash_typed = Rc::new(Cell::new(false));
        let subscriptions = vec![
            cx.subscribe_in(&editor, window, Self::on_editor_event),
            // Sees every key that reaches the document (after an open menu
            // and after Vim): remembers whether the last one typed a slash.
            editor
                .read(cx)
                .intercept_keys(InterceptPriority::DEFAULT, {
                    let slash_typed = slash_typed.clone();
                    move |key, _, _| {
                        let modifiers = key.modifiers;
                        slash_typed.set(
                            !modifiers.platform
                                && !modifiers.control
                                && !modifiers.function
                                && (key.key_char.as_deref() == Some("/")
                                    || (key.key_char.is_none() && key.key == "/")),
                        );
                        KeyDisposition::Pass
                    }
                }),
            // Highlight colours come from the theme.
            cx.observe_global::<GlobalTheme>(|this, cx| {
                if this.find_bar.is_some() {
                    this.apply_find_highlights(cx);
                }
                cx.notify();
            }),
        ];
        Self {
            editor,
            popup: None,
            find: FindState::default(),
            find_bar: None,
            find_placement: FindBarPlacement::default(),
            slash_typed,
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
            _ => self.popup.as_ref().is_some_and(|popup| popup.kind() == kind),
        }
    }

    /// Closes whatever is open and returns focus to the editor.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_popup(false, window, cx);
        self.close_find_bar(cx);
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
                    header: true,
                    mark: Mark::None,
                    key_hint: None,
                    selected: false,
                },
                MenuEntry::Item(item) => MenuRow {
                    label: item.label.to_string(),
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

    /// Makes room for a tool that is about to open: closes the open popup
    /// tool and, unless `keep_find`, the find bar. Focus is left alone (the
    /// new tool decides).
    pub(crate) fn make_room(&mut self, keep_find: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.close_popup(false, window, cx);
        if !keep_find {
            self.close_find_bar(cx);
        }
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
    pub(crate) fn close_popup(&mut self, refocus: bool, window: &mut Window, cx: &mut Context<Self>) {
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
            // Focus went somewhere else (a click in the document, another
            // panel): close without pulling focus back.
            MenuEvent::Blurred => self.close_popup(false, window, cx),
        }
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
            EditorEvent::Edited { change } => {
                let slash = self.slash_typed.replace(false);
                if change.blocks.is_some() {
                    if matches!(self.popup, Some(PopupTool::Insert(_))) {
                        self.sync_insert_menu(window, cx);
                    } else if slash && self.popup.is_none() {
                        self.open_insert_menu_after_slash(window, cx);
                    }
                    if self.find_bar.is_some() {
                        self.refresh_find(false, cx);
                    }
                }
            }
            EditorEvent::SelectionChanged => {
                if matches!(self.popup, Some(PopupTool::Insert(_))) {
                    self.sync_insert_menu(window, cx);
                }
                if self.find_bar.is_some() {
                    self.sync_find_current(cx);
                }
            }
            EditorEvent::Blurred => {
                // The insert menu lives on the editor's keyboard focus.
                if matches!(self.popup, Some(PopupTool::Insert(_))) {
                    self.close_popup(false, window, cx);
                }
            }
            EditorEvent::DocumentReplaced => {
                // The editor dropped its popup and its highlight layers.
                self.close_popup(false, window, cx);
                if self.find_bar.is_some() {
                    self.refresh_find(false, cx);
                }
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------
    // Action handlers
    // -----------------------------------------------------------------

    fn on_close_tool(&mut self, _: &CloseTool, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_tool().is_some() {
            self.close(window, cx);
        } else {
            cx.propagate();
        }
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
            .on_action(cx.listener(|this, _: &OpenInsertMenu, window, cx| {
                this.open_insert_menu(window, cx)
            }))
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
                this.open_find(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &FindNext, window, cx| this.find_next(window, cx)))
            .on_action(
                cx.listener(|this, _: &FindPrevious, window, cx| this.find_previous(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleCaseSensitive, _, cx| {
                this.toggle_case_sensitive(cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleWholeWord, _, cx| this.toggle_whole_word(cx)))
            .on_action(cx.listener(|this, _: &ReplaceCurrent, window, cx| {
                this.replace_current(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ReplaceAll, _, cx| this.replace_all(cx)))
            .on_action(cx.listener(|this, _: &FocusNextField, window, cx| {
                this.focus_other_find_field(window, cx)
            }))
            .on_action(cx.listener(|this, _: &FocusPreviousField, window, cx| {
                this.focus_other_find_field(window, cx)
            }))
            .on_action(cx.listener(Self::on_close_tool))
            .children(top)
            .child(div().flex_1().min_h_0().child(self.editor.clone()))
            .children(bottom)
    }
}
