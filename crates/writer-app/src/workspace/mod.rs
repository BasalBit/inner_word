//! The workspace: the root view of the main window.
//!
//! It lays out the titlebar band, the library sidebar, the editor, the right
//! panel slot, the status bar and the modal and toast layers; it owns the
//! document session (open, autosave, title) and handles every app-level
//! action.
//!
//! # What is mounted
//!
//! The workspace owns one [`Editor`] entity for the lifetime of the window:
//! opening another document replaces the editor's document
//! ([`writer_editor::EditorEvent::DocumentReplaced`]). Everything that
//! extends the editor is attached to that entity once, in
//! [`Workspace::new`], and follows the document by itself:
//!
//! | What | Where it shows | Kept in step by |
//! |---|---|---|
//! | [`EditorTools`] (format menu, insert menu, link popover, font picker, find bar) | the editor area: it wraps the editor | itself; recent fonts are stored from the `FontLibrary` global |
//! | [`CommentsPanel`] | the right panel slot, while it has something to show | `on_comments_event`: the slot, the visibility setting, the "show resolved" setting |
//! | [`VimMode`] with its status item and command line | status bar (left), a strip under the editor | `apply_vim_setting` (the `editor.vim_mode` setting) and `on_vim_event` (`:w`, `:q`, named commands) |
//! | [`writer_spell::SpellChecker`] (only when the app has a spell service, see [`crate::spelling`]) | wavy underlines in the text, the top of the text context menu | `apply_spelling`: the `spelling.*` settings and the open document's own language |
//!
//! The slots themselves are public, so another view can be plugged in the
//! same way:
//!
//! | Slot | How |
//! |---|---|
//! | A wrapper around the editor | [`Workspace::set_editor_host`]: the view renders [`Workspace::editor`] itself, with whatever it adds |
//! | A panel at the right | [`Workspace::set_right_panel`], [`Workspace::set_right_panel_width`] |
//! | A strip under the editor | [`Workspace::set_editor_strip`], keyed, stacked in insertion order |
//! | An item in the status bar | [`Workspace::set_status_item`], keyed, shown at the left |
//! | Commands for the palette and the shortcuts overlay | [`crate::commands::register`] |
//! | Settings | [`Workspace::settings`], [`crate::WorkspaceEvent::SettingsChanged`], [`crate::settings::keys`] |
//! | Modals and toasts | [`Workspace::modal_layer`], [`Workspace::toast_layer`], [`Workspace::show_toast`] |
//!
//! # The mouse
//!
//! Everything is also on a menu: the native menu bar ([`crate::menus`]),
//! the menus of the title and the status bar items ([`MenuKind`],
//! [`Workspace::open_menu`]), and the items the app adds to the context
//! menus of the text, the library rows and the Vim chip (`menus.rs`).
//! Commands for the text that are chosen while the keyboard is elsewhere
//! are passed on to the text ([`Workspace::forward_to_text`]).
//!
//! # Saving
//!
//! The session (`session.rs`) marks the document dirty on every edit and
//! saves it after a pause. For a library on disk the save is written by a
//! background thread (`saver.rs`): the main thread only copies the
//! document. [`Workspace::flush`] is the synchronous, complete save used
//! when another document is opened, the window closes or the app quits.
//!
//! # Who takes a key
//!
//! With focus in the text: an open insert menu first (it intercepts at the
//! editor's overlay priority), then Vim mode (modal priority; it lets every
//! `cmd` chord through), then the key bindings from the focused element
//! outwards (editor, editor tools, workspace), then text input. A tool that
//! takes focus (format menu, link popover, font picker, find bar) and the
//! modals get their keys directly.

mod measure;
mod menus;
mod render;
mod saver;
mod session;
mod spelling;

use std::time::Duration;

use gpui::{
    AnyView, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, Pixels,
    PromptLevel, SharedString, Subscription, Task, Window, WindowBounds, px,
};
use serde::Serialize;
use writer_comments::{CommentsEvent, CommentsPanel};
use writer_core::Document;
use writer_editor::{Editor, EditorConfig, EditorEvent};
use writer_editor_tools::{EditorTools, FontLibrary};
use writer_library::{
    DocumentId, Library, LibraryEvent, LibraryItem, LibraryPanel, LibraryPanelEvent,
};
use writer_ui::{
    CommandDescriptor, ModalLayer, Picker, ThemeMode, Toast, ToastLayer,
    theme::{self},
};
use writer_vim_mode::{HostRequest, VimCommandLine, VimEvent, VimMode, VimStatusItem};

use crate::{
    actions::*,
    library_search::LibrarySearchDelegate,
    palette::PaletteDelegate,
    quick_open::QuickOpenDelegate,
    settings::{self, Settings, keys},
    settings_view::SettingsView,
    shortcuts::ShortcutsDelegate,
    version_history::VersionHistory,
    window::StoredBounds,
};

pub use menus::{Anchors, MenuKind};
pub use render::{
    MIN_EDITOR_WIDTH, MIN_RIGHT_PANEL_WIDTH, PanelLayout, STATUS_BAR_HEIGHT, SidePanel, StatusLine,
    counted, group_digits, limit_label, panel_layout,
};
pub use session::{
    AUTOSAVE_DELAY, CHANGED_OUTSIDE, MAX_AUTOSAVE_WAIT, RETRY_DELAY, SNAPSHOT_INTERVAL, SaveCost,
    SaveState, first_occurrence,
};
pub(crate) use session::{Session, display_title as session_title};
pub use spelling::CHECK_SPELLING_MENU_ITEM;

/// The key context of the workspace root element.
pub const KEY_CONTEXT: &str = "Workspace";

/// The category under which the library panel publishes its commands.
const LIBRARY_CATEGORY: &str = "Library";

/// The width of the app's modals (palette, quick open, shortcuts, export,
/// settings). One width, so the panel does not jump when one follows
/// another.
pub const MODAL_WIDTH: f32 = 600.;
/// The width of the wide modals: the library search with its snippets.
pub const WIDE_MODAL_WIDTH: f32 = 720.;

/// Default width of the right panel slot.
pub const DEFAULT_RIGHT_PANEL_WIDTH: f32 = 320.;

/// How much one "widen" or "narrow" step changes the sidebar width.
const SIDEBAR_STEP: f32 = 24.;

/// How long window moves and resizes settle before the bounds are stored.
const BOUNDS_SAVE_DELAY: Duration = Duration::from_millis(600);

/// How many commands the palette remembers as recently used.
const MAX_RECENT_COMMANDS: usize = 8;

/// The status-bar slot of the Vim mode indicator.
pub const VIM_STATUS_KEY: &str = "vim";
/// The strip slot of the Vim command line.
pub const VIM_COMMAND_LINE_KEY: &str = "vim-command-line";

/// What the workspace tells its subscribers.
#[derive(Clone, Debug, PartialEq)]
pub enum WorkspaceEvent {
    /// Another document is now open in the editor.
    DocumentOpened(DocumentId),
    /// No document is open any more.
    DocumentClosed,
    /// A user setting changed and has been applied; read
    /// [`Workspace::settings`].
    SettingsChanged,
}

/// See the [module docs](self).
pub struct Workspace {
    pub(crate) library: Entity<Library>,
    pub(crate) editor: Entity<Editor>,
    pub(crate) panel: Entity<LibraryPanel>,
    pub(crate) tools: Entity<EditorTools>,
    pub(crate) comments: Entity<CommentsPanel>,
    pub(crate) vim: Entity<VimMode>,
    /// The spell checker of the editor; `None` without a spell service.
    pub(crate) spell: Option<Entity<writer_spell::SpellChecker>>,
    /// What the open settings modal asks for (the language picker).
    settings_subscription: Option<Subscription>,
    pub(crate) modal_layer: Entity<ModalLayer>,
    pub(crate) toast_layer: Entity<ToastLayer>,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) settings: Settings,
    pub(crate) session: Option<Session>,
    /// The background half of the autosave.
    pub(crate) saver: saver::Saver,
    /// What the last save cost.
    pub(crate) last_save: Option<SaveCost>,
    pub(crate) sidebar_dragging: bool,
    pub(crate) editor_host: Option<AnyView>,
    pub(crate) right_panel: Option<AnyView>,
    pub(crate) right_panel_width: Pixels,
    /// The panel beside the text that was used last: it keeps its place in
    /// a window too narrow for both.
    pub(crate) last_side_panel: SidePanel,
    pub(crate) strips: Vec<(SharedString, AnyView)>,
    pub(crate) status_items: Vec<(SharedString, AnyView)>,
    /// The Vim mode indicator the workspace put in the status bar itself.
    pub(crate) vim_status_id: gpui::EntityId,
    /// Where the title and the status bar items were drawn last: their
    /// menus drop down from there.
    pub(crate) anchors: menus::Anchors,
    pub(crate) character_count: Option<(u64, writer_export::SocialCount)>,
    /// The per-block counts behind `character_count`, for the open document.
    pub(crate) limit_counter: Option<crate::character_limit::LimitCounter>,
    pub(crate) window_bounds: Option<WindowBounds>,
    pub(crate) window_title: String,
    bounds_task: Option<Task<()>>,
    /// The focused element that was not part of the last frame (see
    /// [`Workspace::keep_focus_on_screen`]).
    off_screen_focus: Option<FocusHandle>,
    /// The commands that act on the text (formatting, blocks, insertion,
    /// the tools). Chosen while the keyboard is elsewhere in the window
    /// they are passed on to the text: see [`Workspace::forward_to_text`].
    pub(crate) text_commands: Vec<Box<dyn gpui::Action>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// The editor configuration that follows from the settings.
pub fn editor_config(settings: &Settings) -> EditorConfig {
    EditorConfig::full()
        .input_rules(settings.input_rules)
        .smart_punctuation(settings.smart_punctuation)
        .comments_visible(settings.comments_visible)
        .placeholder("Start writing")
}

/// The stored content of a new, empty document with the user's defaults.
pub fn new_document_json(settings: &Settings) -> String {
    Document::new()
        .with_settings(settings.new_document_settings())
        .to_json()
}

impl Workspace {
    /// Builds the workspace over `library`, restores the last session (or
    /// creates the welcome document on the first run) and focuses the editor.
    ///
    /// Requires [`crate::init`] to have run.
    pub fn new(library: Entity<Library>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = Settings::load(library.read(cx));
        if theme::theme_mode(cx) != settings.theme {
            theme::set_theme_mode(settings.theme, cx);
        }

        let editor = cx.new(|cx| {
            Editor::new(
                Document::new(),
                editor_config(&settings).read_only(true),
                window,
                cx,
            )
        });
        let modal_layer = cx.new(|_| ModalLayer::new());
        // The sidebar reads what it stored at the last quit: not about
        // folders and documents that were deleted since.
        session::forget_missing_sidebar_items(&library, cx);
        let panel = cx.new(|cx| {
            let mut panel = LibraryPanel::new(library.clone(), cx);
            // One modal layer per window: the panel's pickers and
            // confirmations open in the same one as the palette.
            panel.set_modal_layer(modal_layer.clone(), cx);
            // What the panel's own "new document" key stores.
            panel.set_new_document_content(new_document_json(&settings));
            panel
        });
        let toast_layer = cx.new(|_| ToastLayer::new());
        ToastLayer::set_global(&toast_layer, cx);

        // Everything that extends the editor attaches to the one editor
        // entity here and follows `DocumentReplaced` by itself.
        let recent_fonts: Vec<String> = library.read(cx).setting_or(keys::RECENT_FONTS, Vec::new());
        writer_editor_tools::set_recent_fonts(recent_fonts.into_iter().map(SharedString::from), cx);
        let tools = cx.new(|cx| {
            let mut tools = EditorTools::new(editor.clone(), window, cx);
            tools.set_format_bar_enabled(settings.format_bar, cx);
            tools
        });
        let comments = cx.new(|cx| {
            let mut comments =
                CommentsPanel::new(editor.clone(), settings.comment_author.clone(), window, cx);
            let show_resolved = library
                .read(cx)
                .setting_or(keys::COMMENTS_SHOW_RESOLVED, false);
            comments.set_show_resolved(show_resolved, window, cx);
            comments
        });
        let vim = cx.new(|cx| VimMode::new(editor.clone(), cx));
        let vim_status = cx.new(|cx| {
            let mut status = VimStatusItem::new(vim.clone(), cx);
            // "Turn off Vim mode" in the chip's menu is the app's own
            // toggle (it has the key and stores the setting); the settings
            // are one more click away.
            status.set_toggle_action(ToggleVimMode);
            status.extend_menu(|items, _, _| {
                items.push(writer_ui::ContextMenuItem::separator());
                items.push(writer_ui::ContextMenuItem::action(
                    "Settings…",
                    OpenSettings,
                ));
            });
            status
        });
        let vim_command_line = cx.new(|cx| VimCommandLine::new(vim.clone(), cx));
        // The spell checker, when the app has a spell service: `main`
        // installs the Mac's own, a test installs a fake, and without one
        // nothing is checked.
        let spell = crate::spelling::service(cx).map(|service| {
            cx.new(|cx| writer_spell::SpellChecker::new(editor.clone(), service, cx))
        });

        let mut subscriptions = vec![
            cx.subscribe_in(&editor, window, Self::on_editor_event),
            cx.subscribe_in(&library, window, Self::on_library_event),
            cx.subscribe_in(&panel, window, Self::on_panel_event),
            cx.subscribe_in(&comments, window, Self::on_comments_event),
            cx.subscribe_in(&vim, window, Self::on_vim_event),
            cx.observe_global::<FontLibrary>(Self::store_recent_fonts),
            // A modal that closes gives focus back to where it was. When
            // that place is gone (a popup that closed meanwhile), focus
            // would be nowhere and no key would reach anything.
            cx.observe_in(&modal_layer, window, |this, layer, window, cx| {
                if !layer.read(cx).has_modal() && window.focused(cx).is_none() {
                    this.focus_editor(window, cx);
                }
            }),
            // Losing focus (another app or window comes to the front) is a
            // moment at which nothing must be left unsaved.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.flush(cx);
                }
            }),
            cx.observe_window_bounds(window, |this, window, cx| {
                this.window_bounds = Some(window.window_bounds());
                this.schedule_bounds_save(cx);
            }),
            // Quit: flush synchronously in the closure body; GPUI gives the
            // returned future only 100ms.
            cx.on_app_quit(|this, cx| {
                this.flush(cx);
                this.save_window_bounds(cx);
                async {}
            }),
        ];
        if let Some(spell) = &spell {
            subscriptions.push(cx.subscribe_in(spell, window, Self::on_spell_event));
        }
        // A library command that puts the keyboard in the sidebar while the
        // sidebar is not drawn (closed, or stepped aside in a narrow
        // window) asks for it to be shown.
        let weak = cx.weak_entity();
        panel.update(cx, |panel, _| {
            panel.set_reveal_handler(move |window, cx| {
                weak.update(cx, |this: &mut Self, cx| {
                    this.last_side_panel = SidePanel::Library;
                    if !this.settings.sidebar_visible {
                        this.set_sidebar_visible(true, window, cx);
                    }
                    cx.notify();
                })
                .ok();
            });
        });
        // Closing the window (red button, cmd-w through the system) flushes too.
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            weak.update(cx, |this, cx| {
                this.flush(cx);
                this.save_window_bounds(cx);
            })
            .ok();
            true
        });

        let saver = saver::Saver::new(library.read(cx));
        let mut this = Self {
            library,
            editor,
            panel,
            editor_host: Some(tools.clone().into()),
            tools,
            comments,
            vim,
            spell,
            settings_subscription: None,
            modal_layer,
            toast_layer,
            focus_handle: cx.focus_handle(),
            settings,
            session: None,
            saver,
            last_save: None,
            sidebar_dragging: false,
            right_panel: None,
            right_panel_width: px(DEFAULT_RIGHT_PANEL_WIDTH),
            last_side_panel: SidePanel::Right,
            strips: vec![(VIM_COMMAND_LINE_KEY.into(), vim_command_line.into())],
            vim_status_id: vim_status.entity_id(),
            anchors: menus::Anchors::default(),
            status_items: vec![(VIM_STATUS_KEY.into(), vim_status.into())],
            character_count: None,
            limit_counter: None,
            window_bounds: None,
            window_title: String::new(),
            bounds_task: None,
            off_screen_focus: None,
            text_commands: text_commands(),
            _subscriptions: subscriptions,
        };
        this.extend_the_menus_of_other_crates(cx);
        this.connect_vim_spelling(cx);
        this.restore_session(window, cx);
        this.offer_agents_hint(cx);
        this.sync_comments_panel(cx);
        this.apply_vim_setting(cx);
        this.apply_spelling(cx);
        this.focus_editor(window, cx);
        this
    }

    // ----- access ----------------------------------------------------------------

    /// The library this workspace shows.
    pub fn library(&self) -> &Entity<Library> {
        &self.library
    }

    /// The editor. One entity for the lifetime of the window: its document
    /// is replaced when another one is opened.
    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    /// The library sidebar.
    pub fn panel(&self) -> &Entity<LibraryPanel> {
        &self.panel
    }

    /// The editor's contextual tools: format menu, insert menu, link
    /// popover, font picker, find and replace. This view wraps the editor
    /// in the editor area.
    pub fn tools(&self) -> &Entity<EditorTools> {
        &self.tools
    }

    /// The comments panel. It sits in the right panel slot while it has
    /// something to show.
    pub fn comments(&self) -> &Entity<CommentsPanel> {
        &self.comments
    }

    /// Vim mode for the editor. Attached while the `editor.vim_mode`
    /// setting is on.
    pub fn vim(&self) -> &Entity<VimMode> {
        &self.vim
    }

    /// The modal layer of the window (palette, pickers, dialogs).
    pub fn modal_layer(&self) -> &Entity<ModalLayer> {
        &self.modal_layer
    }

    /// The toast layer of the window. It is also installed as the global
    /// layer, so `writer_ui::toast::show_toast` reaches it.
    pub fn toast_layer(&self) -> &Entity<ToastLayer> {
        &self.toast_layer
    }

    /// The current user settings.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The id of the open document, if one is open.
    pub fn open_document_id(&self) -> Option<DocumentId> {
        self.session.as_ref().map(|session| session.id)
    }

    /// The title of the open document, as the band above the text shows
    /// it: the title its text gives now, or the one the user chose. The
    /// library's row has it from the next save on.
    pub fn document_title(&self) -> Option<&str> {
        self.session.as_ref().map(|session| session.title.as_str())
    }

    /// Whether the open document has unsaved changes, and whether the last
    /// save failed.
    pub fn save_state(&self) -> Option<SaveState> {
        self.session.as_ref().map(Session::save_state)
    }

    // ----- mount points ----------------------------------------------------------

    /// Puts `view` in the editor area instead of the bare editor. The view is
    /// expected to render [`Workspace::editor`] itself (it wraps the editor)
    /// and to fill its container. `None` shows the bare editor again.
    pub fn set_editor_host(&mut self, view: Option<AnyView>, cx: &mut Context<Self>) {
        self.editor_host = view;
        cx.notify();
    }

    /// Shows `view` in the panel slot at the right of the editor, or removes
    /// the panel with `None`.
    pub fn set_right_panel(&mut self, view: Option<AnyView>, cx: &mut Context<Self>) {
        self.right_panel = view;
        cx.notify();
    }

    /// Whether the right panel slot is filled.
    pub fn has_right_panel(&self) -> bool {
        self.right_panel.is_some()
    }

    /// Sets the width of the right panel slot (clamped to 220..=560). In a
    /// narrow window the panel is drawn narrower, see [`panel_layout`].
    pub fn set_right_panel_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        self.right_panel_width = width.clamp(px(MIN_RIGHT_PANEL_WIDTH), px(560.));
        cx.notify();
    }

    /// Shows `view` as a strip under the editor (a find bar, the Vim command
    /// line), or removes the strip registered under `key` with `None`.
    /// Strips stack in the order their keys were first set.
    pub fn set_editor_strip(
        &mut self,
        key: impl Into<SharedString>,
        view: Option<AnyView>,
        cx: &mut Context<Self>,
    ) {
        set_keyed(&mut self.strips, key.into(), view);
        cx.notify();
    }

    /// Shows `view` at the left of the status bar (the Vim mode), or removes
    /// the item registered under `key` with `None`.
    pub fn set_status_item(
        &mut self,
        key: impl Into<SharedString>,
        view: Option<AnyView>,
        cx: &mut Context<Self>,
    ) {
        set_keyed(&mut self.status_items, key.into(), view);
        cx.notify();
    }

    // ----- toasts and modals -----------------------------------------------------

    /// Shows a toast at the bottom of the window.
    pub fn show_toast(&mut self, toast: Toast, cx: &mut Context<Self>) {
        self.toast_layer.update(cx, |layer, cx| {
            layer.push(toast, cx);
        });
    }

    pub(crate) fn toast_error(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.show_toast(
            Toast::new(message).error().duration(Duration::from_secs(8)),
            cx,
        );
    }

    /// Closes the open modal, if any, restoring the focus it took.
    pub fn dismiss_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modal_layer
            .update(cx, |layer, cx| layer.dismiss(window, cx));
    }

    // ----- focus -----------------------------------------------------------------

    /// Moves focus to the editor (closing any modal first). Without an open
    /// document the workspace itself takes focus, so app shortcuts and
    /// palette commands still have a target.
    pub fn focus_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_modal(window, cx);
        if self.session.is_some() {
            window.focus(&self.editor.focus_handle(cx));
        } else {
            window.focus(&self.focus_handle);
        }
    }

    /// Moves focus to the library sidebar, showing it when it is hidden.
    pub fn focus_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_modal(window, cx);
        if !self.settings.sidebar_visible {
            self.set_sidebar_visible(true, window, cx);
        }
        // In a window too narrow for both side panels the library is now
        // the one that is drawn.
        self.last_side_panel = SidePanel::Library;
        let panel = self.panel.clone();
        if panel.read(cx).selected().is_none() {
            let target = self
                .open_document_id()
                .map(LibraryItem::Document)
                .or_else(|| panel.read(cx).visible_items().first().copied());
            if let Some(target) = target {
                panel.update(cx, |panel, cx| {
                    if !panel.reveal(target, cx) {
                        let first = panel.visible_items().first().copied();
                        panel.select(first, cx);
                    }
                });
            }
        }
        window.focus(&panel.focus_handle(cx));
    }

    /// Focus must always be on something that is on screen: with nothing
    /// focused, or with the focus on an element that is not drawn, keys are
    /// dispatched at the window root and reach neither the text nor the
    /// handlers of the workspace. Called from `render`.
    ///
    /// The ways to get there: the focused element went away (a popup
    /// closed, a panel was removed, a modal closed and what it returns the
    /// keyboard to is gone too, such as a format menu or a rename field
    /// that closed when the modal took the keyboard). GPUI then reports
    /// either no focus at all, or a focus that is not part of the window.
    ///
    /// The second case shows only in a finished frame, and not in the first
    /// one: an element that has just been given the focus (a popup's field,
    /// the rename field in a row of the library's list) is drawn for the
    /// first time in the frame being built now, or in the one after. So the
    /// check runs after every frame, and gives up on a focused element only
    /// when it is missing from two frames in a row.
    pub(crate) fn keep_focus_on_screen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.focused(cx).is_none() {
            self.focus_fallback(window, cx);
        }
        cx.defer_in(window, |this, window, cx| {
            let focused = window.focused(cx);
            let on_screen = focused
                .as_ref()
                .is_some_and(|focused| this.focus_handle.contains(focused, window));
            if on_screen {
                this.off_screen_focus = None;
                this.close_modal_without_keyboard(window, cx);
                return;
            }
            match focused {
                Some(focused) if this.off_screen_focus.as_ref() != Some(&focused) => {
                    // Look again after one more frame.
                    this.off_screen_focus = Some(focused);
                    window.refresh();
                }
                _ => {
                    this.off_screen_focus = None;
                    this.focus_fallback(window, cx);
                }
            }
        });
    }

    /// A modal that is on screen has the keyboard. When the keyboard has
    /// gone somewhere else in the window (a window-wide key of another
    /// crate put it in the comments, a menu bar command opened a tool in
    /// the text), the modal closes: left open it would cover the window
    /// while `escape` and every other key went to what is behind it.
    ///
    /// Runs after a frame, with the focused element known to be in that
    /// frame. The keyboard stays where it went.
    fn close_modal_without_keyboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.modal_layer.read(cx).has_modal() {
            return;
        }
        let in_modal = window
            .context_stack()
            .iter()
            .any(|context| context.contains(writer_ui::modal::KEY_CONTEXT));
        if in_modal {
            return;
        }
        let focused = window.focused(cx);
        self.dismiss_modal(window, cx);
        if let Some(focused) = focused {
            window.focus(&focused);
        }
    }

    /// A key or a menu command that acts on the window behind an open modal
    /// closes the modal first (as `cmd-0`, `cmd-1` and `cmd-n` do), so
    /// nothing changes out of sight. Returns whether a modal was closed.
    pub(crate) fn close_modal_first(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let open = self.modal_layer.read(cx).has_modal();
        if open {
            self.dismiss_modal(window, cx);
        }
        open
    }

    /// Whether the keyboard is in the text of the document or in one of
    /// its tools (also: in any other editor, such as the field a comment
    /// is written in). Read from the last frame.
    pub(crate) fn keyboard_in_text(&self, window: &Window) -> bool {
        window.context_stack().iter().any(|context| {
            context.contains(writer_editor::actions::KEY_CONTEXT)
                || context.contains(writer_editor_tools::actions::KEY_CONTEXT)
        })
    }

    /// A command for the text arrived at the workspace because nothing on
    /// the way from the keyboard took it: the keyboard is in the library,
    /// in the comment list or in a modal. The palette and the menu bar
    /// offer these commands wherever the keyboard is, so they are passed on
    /// to the text, which takes the keyboard (an open modal closes).
    ///
    /// When the keyboard is in the text already and the command still came
    /// this far, the text had no use for it right now; it is left alone.
    pub(crate) fn forward_to_text(
        &mut self,
        action: &dyn gpui::Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session.is_none()
            || self.editor.read(cx).is_read_only()
            || self.keyboard_in_text(window)
        {
            cx.propagate();
            return;
        }
        self.dismiss_modal(window, cx);
        window.focus(&self.editor.focus_handle(cx));
        window.dispatch_action(action.boxed_clone(), cx);
    }

    /// Where the keyboard goes when it would otherwise be nowhere: the
    /// text, or the workspace itself while no document is open (or when
    /// the text is the thing that is not on screen).
    fn focus_fallback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.editor.focus_handle(cx);
        let target = match self.session {
            Some(_) if !editor.is_focused(window) => editor,
            _ => self.focus_handle.clone(),
        };
        window.focus(&target);
    }

    fn panel_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.panel.focus_handle(cx).contains_focused(window, cx)
    }

    // ----- sidebar ---------------------------------------------------------------

    /// Shows or hides the library sidebar and remembers the choice.
    pub fn set_sidebar_visible(
        &mut self,
        visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings.sidebar_visible == visible {
            return;
        }
        if !visible && self.panel_has_focus(window, cx) {
            self.focus_editor(window, cx);
        }
        self.settings.sidebar_visible = visible;
        self.write_setting(keys::SIDEBAR_VISIBLE, &visible, cx);
        cx.notify();
    }

    /// Sets the sidebar width (clamped) without storing it; used while
    /// dragging.
    pub(crate) fn set_sidebar_width_live(&mut self, width: f32, cx: &mut Context<Self>) {
        let width = width.clamp(settings::MIN_SIDEBAR_WIDTH, settings::MAX_SIDEBAR_WIDTH);
        if self.settings.sidebar_width != width {
            self.settings.sidebar_width = width;
            cx.notify();
        }
    }

    /// Sets the sidebar width (clamped to
    /// [`settings::MIN_SIDEBAR_WIDTH`]..=[`settings::MAX_SIDEBAR_WIDTH`]) and
    /// remembers it.
    pub fn set_sidebar_width(&mut self, width: f32, cx: &mut Context<Self>) {
        self.set_sidebar_width_live(width, cx);
        let width = self.settings.sidebar_width;
        self.write_setting(keys::SIDEBAR_WIDTH, &width, cx);
    }

    // ----- settings --------------------------------------------------------------

    /// Stores one setting. The change is applied before this returns; a
    /// failed write is reported in a toast.
    pub fn write_setting<T: Serialize + ?Sized>(
        &mut self,
        key: &str,
        value: &T,
        cx: &mut Context<Self>,
    ) {
        let result = self
            .library
            .update(cx, |library, cx| library.set_setting(key, value, cx));
        match result {
            Ok(()) => self.reload_settings(cx),
            Err(error) => self.toast_error(format!("Could not save the setting: {error}"), cx),
        }
    }

    /// Re-reads the settings from the library and applies what changed.
    pub(crate) fn reload_settings(&mut self, cx: &mut Context<Self>) {
        let new = Settings::load(self.library.read(cx));
        if new == self.settings {
            return;
        }
        let old = std::mem::replace(&mut self.settings, new);
        self.apply_settings(&old, cx);
        cx.emit(WorkspaceEvent::SettingsChanged);
        cx.notify();
    }

    fn apply_settings(&mut self, old: &Settings, cx: &mut Context<Self>) {
        let new = self.settings.clone();
        if old.theme != new.theme {
            theme::set_theme_mode(new.theme, cx);
        }
        if old.input_rules != new.input_rules
            || old.smart_punctuation != new.smart_punctuation
            || old.comments_visible != new.comments_visible
        {
            self.editor.update(cx, |editor, cx| {
                editor.update_config(
                    |config| {
                        config.input_rules = new.input_rules;
                        config.smart_punctuation = new.smart_punctuation;
                        config.comments_visible = new.comments_visible;
                    },
                    cx,
                );
            });
        }
        if old.vim_mode != new.vim_mode {
            self.apply_vim_setting(cx);
        }
        if old.format_bar != new.format_bar {
            self.tools.update(cx, |tools, cx| {
                tools.set_format_bar_enabled(new.format_bar, cx)
            });
        }
        if old.check_spelling != new.check_spelling
            || old.spelling_language != new.spelling_language
        {
            self.apply_spelling(cx);
        }
        if old.comment_author != new.comment_author {
            let author = new.comment_author.clone();
            self.comments
                .update(cx, |comments, _| comments.set_author(author));
        }
        if old.character_limit != new.character_limit {
            self.character_count = None;
            self.limit_counter = None;
        }
        if old.new_document_settings() != new.new_document_settings() {
            self.panel.update(cx, |panel, _| {
                panel.set_new_document_content(new_document_json(&new));
            });
        }
    }

    // ----- commands --------------------------------------------------------------

    /// Runs a command the way the palette does: closes the open modal (which
    /// restores focus to where it was), remembers the command as recently
    /// used and dispatches its action from the focused element.
    pub fn run_command(
        &mut self,
        command: &CommandDescriptor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remember_command(&command.label(), cx);
        // The library's commands act on the sidebar: make sure it is there.
        if command.category.as_ref() == LIBRARY_CATEGORY {
            // Also in a window too narrow for both side panels.
            self.last_side_panel = SidePanel::Library;
            if !self.settings.sidebar_visible {
                self.dismiss_modal(window, cx);
                self.set_sidebar_visible(true, window, cx);
            }
        }
        self.run_action(command.action(), window, cx);
    }

    /// Closes the open modal and dispatches `action` from the element that
    /// has focus afterwards.
    pub fn run_action(
        &mut self,
        action: Box<dyn gpui::Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_modal(window, cx);
        if window.focused(cx).is_none() {
            window.focus(&self.focus_handle);
        }
        window.dispatch_action(action, cx);
    }

    /// The labels of the commands run most recently from the palette.
    pub fn recent_commands(&self, cx: &App) -> Vec<String> {
        self.library
            .read(cx)
            .setting_or(keys::RECENT_COMMANDS, Vec::new())
    }

    fn remember_command(&mut self, label: &str, cx: &mut Context<Self>) {
        let mut recent = self.recent_commands(cx);
        recent.retain(|existing| existing != label);
        recent.insert(0, label.to_string());
        recent.truncate(MAX_RECENT_COMMANDS);
        self.library
            .update(cx, |library, cx| {
                library.set_setting(keys::RECENT_COMMANDS, &recent, cx)
            })
            .ok();
    }

    // ----- modals ----------------------------------------------------------------

    /// Opens the command palette, or closes it when it is open.
    pub fn toggle_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = cx.weak_entity();
        let recent = self.recent_commands(cx);
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle(window, cx, |window, cx| {
                let delegate = PaletteDelegate::new(workspace, recent, cx);
                let rows = picker_rows(window, 14);
                Picker::modal(delegate, window, cx)
                    .width(px(MODAL_WIDTH))
                    .max_visible_rows(rows)
            });
        });
    }

    /// Opens quick open, or closes it when it is open.
    pub fn toggle_quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = cx.weak_entity();
        let library = self.library.clone();
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle(window, cx, |window, cx| {
                let delegate = QuickOpenDelegate::new(workspace, &library, cx);
                let rows = picker_rows(window, 14);
                Picker::modal(delegate, window, cx)
                    .width(px(MODAL_WIDTH))
                    .max_visible_rows(rows)
            });
        });
    }

    /// Opens the library search, or closes it when it is open.
    pub fn toggle_library_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = cx.weak_entity();
        let library = self.library.clone();
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle(window, cx, |window, cx| {
                let delegate = LibrarySearchDelegate::new(workspace, library);
                let rows = picker_rows(window, 14);
                Picker::modal(delegate, window, cx)
                    .width(px(WIDE_MODAL_WIDTH))
                    .max_visible_rows(rows)
            });
        });
    }

    /// Opens the keyboard shortcuts overlay, or closes it when it is open.
    pub fn toggle_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = cx.weak_entity();
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle(window, cx, |window, cx| {
                let delegate = ShortcutsDelegate::new(workspace, cx);
                let rows = picker_rows(window, 20);
                Picker::modal(delegate, window, cx)
                    .width(px(MODAL_WIDTH))
                    .max_visible_rows(rows)
            });
        });
    }

    /// Opens the export picker for the open document, or closes it when it
    /// is open.
    pub fn toggle_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .modal_layer
            .read(cx)
            .is_open::<crate::export::ExportPicker>()
        {
            self.dismiss_modal(window, cx);
        } else {
            self.show_export(crate::export::ExportTarget::Open, window, cx);
        }
    }

    /// Opens the settings, or closes them when they are open. With a
    /// document open they include the "This document" section.
    pub fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_settings(None, window, cx);
    }

    /// Opens the settings at the section for the open document (its paper,
    /// margins and typography), or closes them when they are open.
    pub fn toggle_document_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.is_none() {
            self.show_toast(Toast::new("Open a document to change its settings"), cx);
            return;
        }
        self.show_settings(
            Some(crate::settings_view::SettingRow::DocumentPageView),
            window,
            cx,
        );
    }

    /// Opens the Agents panel (the command line tool, the skill, the MCP
    /// server), or closes it when it is open.
    pub fn toggle_agents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle(window, cx, |_, cx| crate::agents_view::AgentsView::new(cx));
        });
    }

    /// Opens the version history of the open document, or closes it when
    /// it is open. What is on screen is saved first, so the list is up to
    /// date.
    pub fn toggle_version_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.open_document_id() else {
            self.show_toast(Toast::new("Open a document to see its versions"), cx);
            return;
        };
        self.flush(cx);
        let workspace = cx.weak_entity();
        let library = self.library.clone();
        let title = self.display_title();
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle(window, cx, |_, cx| {
                VersionHistory::new(workspace, library, id, title, cx)
            });
        });
    }

    fn show_settings(
        &mut self,
        start: Option<crate::settings_view::SettingRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let library = self.library.clone();
        let document = self.session.is_some().then(|| self.editor.clone());
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle(window, cx, |window, cx| {
                let view = match document {
                    Some(editor) => SettingsView::for_document(library, editor, window, cx),
                    None => SettingsView::new(library, window, cx),
                };
                match start {
                    Some(row) => view.starting_at(row, cx),
                    None => view,
                }
            });
        });
        // The modal asks the workspace for the language picker.
        self.settings_subscription = self
            .modal_layer
            .read(cx)
            .active::<SettingsView>()
            .map(|view| cx.subscribe_in(&view, window, Self::on_settings_event));
    }

    // ----- view toggles ----------------------------------------------------------

    /// Switches the open document between the flowing column and page view.
    /// The choice is stored in the document.
    pub fn toggle_page_view(&mut self, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        self.editor
            .update(cx, |editor, cx| editor.toggle_page_view(cx));
        // The editor reports the change as SettingsChanged, which schedules a
        // save; make sure of it even if it stays silent.
        self.mark_dirty(cx);
    }

    /// Changes the open document's own settings (paper, typography) and
    /// says what they are now. The change is part of the document: it is
    /// saved with it and exported with it.
    pub fn change_document_settings(
        &mut self,
        change: impl FnOnce(&mut writer_core::DocumentSettings),
        describe: impl FnOnce(&writer_core::DocumentSettings) -> String,
        cx: &mut Context<Self>,
    ) {
        if self.session.is_none() || self.editor.read(cx).is_read_only() {
            return;
        }
        let mut settings = self.editor.read(cx).document().settings().clone();
        change(&mut settings);
        let settings = settings.sanitized();
        let message = describe(&settings);
        self.editor.update(cx, |editor, cx| {
            editor.edit_with(
                writer_editor::Autoscroll::None,
                |state| state.set_settings(settings),
                cx,
            );
        });
        self.mark_dirty(cx);
        self.show_toast(Toast::new(message).duration(Duration::from_secs(2)), cx);
    }

    fn toggle_orientation(&mut self, cx: &mut Context<Self>) {
        use writer_core::Orientation;
        self.change_document_settings(
            |settings| {
                settings.page.orientation = match settings.page.orientation {
                    Orientation::Portrait => Orientation::Landscape,
                    Orientation::Landscape => Orientation::Portrait,
                }
            },
            |settings| match settings.page.orientation {
                Orientation::Portrait => "Pages: portrait".to_string(),
                Orientation::Landscape => "Pages: landscape".to_string(),
            },
            cx,
        );
    }

    fn step_body_text_size(&mut self, points: f32, cx: &mut Context<Self>) {
        self.change_document_settings(
            |settings| {
                let size = (settings.typography.base_size + points).round();
                settings.typography.base_size = size.clamp(9., 36.);
            },
            |settings| format!("Body text: {} pt", settings.typography.base_size),
            cx,
        );
    }

    fn step_line_height(&mut self, step: f32, cx: &mut Context<Self>) {
        self.change_document_settings(
            |settings| {
                let height = ((settings.typography.line_height + step) * 10.).round() / 10.;
                settings.typography.line_height = height.clamp(1.0, 3.0);
            },
            |settings| format!("Line spacing: {:.1}", settings.typography.line_height),
            cx,
        );
    }

    /// Turns Vim mode on or off (a setting). The mode indicator in the
    /// status bar shows the result.
    pub fn toggle_vim_mode(&mut self, cx: &mut Context<Self>) {
        let on = !self.settings.vim_mode;
        self.write_setting(keys::VIM_MODE, &on, cx);
    }

    /// Attaches or detaches Vim mode as the setting says, and has held
    /// keys repeat (instead of offering accents) while it is on.
    fn apply_vim_setting(&mut self, cx: &mut Context<Self>) {
        let on = self.settings.vim_mode;
        self.vim.update(cx, |vim, cx| vim.set_attached(on, cx));
        let noted = crate::press_and_hold::state(cx).restart_note_due();
        crate::press_and_hold::sync(on, cx);
        // The system reads that choice once per launch (see
        // `press_and_hold`): say so when it was made too late for this one.
        if !noted && crate::press_and_hold::state(cx).restart_note_due() {
            let message = if on {
                "Held keys repeat instead of offering accents after a restart."
            } else {
                "The accent menu on held keys returns after a restart."
            };
            self.show_toast(Toast::new(message).duration(Duration::from_secs(6)), cx);
        }
    }

    /// Shows or hides comments: the tints in the text and the panel (a
    /// setting). `cmd-alt-c` does the same through the comments panel,
    /// which reports back with [`CommentsEvent::VisibilityChanged`].
    pub fn toggle_comments(&mut self, cx: &mut Context<Self>) {
        let visible = !self.settings.comments_visible;
        self.write_setting(keys::COMMENTS_VISIBLE, &visible, cx);
    }

    /// Puts the comments panel in the right slot while it has something to
    /// show, and takes it out otherwise.
    fn sync_comments_panel(&mut self, cx: &mut Context<Self>) {
        let open = self.comments.read(cx).is_open();
        if open != self.right_panel.is_some() {
            let view = open.then(|| self.comments.clone().into());
            self.set_right_panel(view, cx);
        }
    }

    /// The font picker's recently used families changed (or any other part
    /// of the font library): keep the stored list in step.
    fn store_recent_fonts(&mut self, cx: &mut Context<Self>) {
        let recent: Vec<String> = writer_editor_tools::recent_fonts(cx)
            .into_iter()
            .map(|font| font.to_string())
            .collect();
        let stored: Vec<String> = self
            .library
            .read(cx)
            .setting_or(keys::RECENT_FONTS, Vec::new());
        if recent != stored {
            self.library
                .update(cx, |library, cx| {
                    library.set_setting(keys::RECENT_FONTS, &recent, cx)
                })
                .ok();
        }
    }

    /// Stores the theme choice and applies it.
    pub fn set_theme(&mut self, mode: ThemeMode, cx: &mut Context<Self>) {
        self.write_setting(keys::THEME, mode.as_str(), cx);
    }

    // ----- window ----------------------------------------------------------------

    fn schedule_bounds_save(&mut self, cx: &mut Context<Self>) {
        self.bounds_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(BOUNDS_SAVE_DELAY).await;
            this.update(cx, |this, cx| this.save_window_bounds(cx)).ok();
        }));
    }

    pub(crate) fn save_window_bounds(&mut self, cx: &mut Context<Self>) {
        let Some(bounds) = self.window_bounds.take() else {
            return;
        };
        let stored = StoredBounds::from_window_bounds(bounds);
        self.library
            .update(cx, |library, cx| {
                library.set_setting(keys::WINDOW_BOUNDS, &stored, cx)
            })
            .ok();
    }

    fn show_about(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let detail = format!(
            "Version {}\nA keyboard-first place to write.",
            env!("CARGO_PKG_VERSION")
        );
        // The answer does not matter; dropping the receiver is fine.
        drop(window.prompt(PromptLevel::Info, "The Writer", Some(&detail), &["OK"], cx));
    }

    // ----- events ----------------------------------------------------------------

    fn on_editor_event(
        &mut self,
        _editor: &Entity<Editor>,
        event: &EditorEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            EditorEvent::Edited { .. } => self.mark_dirty(cx),
            EditorEvent::SettingsChanged => {
                self.mark_dirty(cx);
                // The document's own spelling language may be what changed.
                self.apply_spelling(cx);
            }
            // Another document, with a language of its own or without.
            EditorEvent::DocumentReplaced => {
                self.apply_spelling(cx);
                cx.notify();
            }
            _ => cx.notify(),
        }
    }

    fn on_comments_event(
        &mut self,
        _comments: &Entity<CommentsPanel>,
        event: &CommentsEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            CommentsEvent::OpenChanged(_) => self.sync_comments_panel(cx),
            // The panel shows or hides comments itself (cmd-alt-c, or "add
            // comment" while they are hidden): remember what it chose.
            CommentsEvent::VisibilityChanged(visible) => {
                if self.settings.comments_visible != *visible {
                    self.write_setting(keys::COMMENTS_VISIBLE, visible, cx);
                }
                self.sync_comments_panel(cx);
            }
            CommentsEvent::ShowResolvedChanged(show) => {
                self.library
                    .update(cx, |library, cx| {
                        library.set_setting(keys::COMMENTS_SHOW_RESOLVED, show, cx)
                    })
                    .ok();
            }
            _ => {}
        }
    }

    fn on_vim_event(
        &mut self,
        vim: &Entity<VimMode>,
        event: &VimEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            VimEvent::Request(request) => self.on_vim_request(vim, request, window, cx),
            // Vim was switched by something other than the setting (the Vim
            // crate's own toggle action): the setting follows.
            VimEvent::Attached | VimEvent::Detached => {
                let attached = vim.read(cx).is_attached();
                if self.settings.vim_mode != attached {
                    self.write_setting(keys::VIM_MODE, &attached, cx);
                }
            }
            _ => {}
        }
    }

    /// What the Vim command line asks of the app: `:w` saves now, `:q`
    /// closes the document (the window stays, with the library), `:wq` does
    /// both, and any other name is looked up among the commands of the
    /// palette (`:togglepageview`, `:export`, `:bold`).
    fn on_vim_request(
        &mut self,
        vim: &Entity<VimMode>,
        request: &HostRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match request {
            HostRequest::Save => {
                self.vim_save(vim, cx);
            }
            HostRequest::Close { .. } => {
                // Nothing can be lost by closing: the document is saved
                // first, with or without the bang.
                self.close_document(window, cx);
            }
            HostRequest::SaveAndClose => {
                if self.vim_save(vim, cx) {
                    self.close_document(window, cx);
                }
            }
            HostRequest::Command { name, .. } => match command_named(name, cx) {
                Some(command) => self.run_action(command.action(), window, cx),
                None => {
                    let error = format!("E492: Not an editor command: {name}");
                    vim.update(cx, |vim, cx| vim.show_error(error, cx));
                }
            },
        }
    }

    fn vim_save(&mut self, vim: &Entity<VimMode>, cx: &mut Context<Self>) -> bool {
        let saved = self.flush(cx);
        let title = self.display_title();
        vim.update(cx, |vim, cx| {
            if saved {
                // As Vim says it; "Saved" is already beside it in the bar.
                vim.show_message(format!("\u{201c}{title}\u{201d} written"), cx);
            } else {
                vim.show_error("E212: Could not save; the save is retried", cx);
            }
        });
        saved
    }

    fn on_panel_event(
        &mut self,
        _panel: &Entity<LibraryPanel>,
        event: &LibraryPanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            LibraryPanelEvent::OpenDocument { id, focus_editor } => {
                self.open_document(*id, *focus_editor, window, cx);
            }
            LibraryPanelEvent::Dismissed => self.focus_editor(window, cx),
        }
    }

    fn on_library_event(
        &mut self,
        _library: &Entity<Library>,
        event: &LibraryEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            LibraryEvent::SettingChanged { key } => {
                if keys::USER_SETTINGS.contains(&key.as_ref()) {
                    self.reload_settings(cx);
                }
            }
            LibraryEvent::DocumentRenamed(id) => self.document_renamed(*id, cx),
            LibraryEvent::DocumentSaved { id, .. } => {
                if self.open_document_id() == Some(*id) {
                    self.refresh_title(window, cx);
                }
            }
            LibraryEvent::DocumentTrashed(id) => {
                if self.open_document_id() == Some(*id) {
                    self.open_document_left(window, cx);
                }
            }
            LibraryEvent::FolderTrashed { documents, .. } => {
                if self
                    .open_document_id()
                    .is_some_and(|open| documents.contains(&open))
                {
                    self.open_document_left(window, cx);
                }
            }
            LibraryEvent::DocumentPurged(id) => {
                let key = keys::custom_title(*id);
                self.library
                    .update(cx, |library, cx| library.remove_setting(&key, cx))
                    .ok();
            }
            _ => {}
        }
    }

    // ----- action handlers (registered on the root element in render) -------------

    pub(crate) fn on_toggle_palette(
        &mut self,
        _: &ToggleCommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_command_palette(window, cx);
    }

    pub(crate) fn on_quick_open(
        &mut self,
        _: &QuickOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_quick_open(window, cx);
    }

    pub(crate) fn on_search_library(
        &mut self,
        _: &SearchLibrary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_library_search(window, cx);
    }

    pub(crate) fn on_show_shortcuts(
        &mut self,
        _: &ShowShortcuts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_shortcuts(window, cx);
    }

    pub(crate) fn on_export(&mut self, _: &Export, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_export(window, cx);
    }

    pub(crate) fn on_open_settings(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_settings(window, cx);
    }

    pub(crate) fn on_new_document(
        &mut self,
        _: &NewDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_document(window, cx);
    }

    pub(crate) fn on_new_folder(
        &mut self,
        _: &NewFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_folder(window, cx);
    }

    pub(crate) fn on_focus_library(
        &mut self,
        _: &FocusLibrary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_library(window, cx);
    }

    pub(crate) fn on_focus_editor(
        &mut self,
        _: &FocusEditor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_editor(window, cx);
    }

    pub(crate) fn on_toggle_sidebar(
        &mut self,
        _: &ToggleSidebar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_modal_first(window, cx);
        let visible = !self.settings.sidebar_visible;
        self.set_sidebar_visible(visible, window, cx);
    }

    pub(crate) fn on_widen_sidebar(
        &mut self,
        _: &WidenSidebar,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_sidebar_width(self.settings.sidebar_width + SIDEBAR_STEP, cx);
    }

    pub(crate) fn on_narrow_sidebar(
        &mut self,
        _: &NarrowSidebar,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_sidebar_width(self.settings.sidebar_width - SIDEBAR_STEP, cx);
    }

    pub(crate) fn on_toggle_page_view(
        &mut self,
        _: &TogglePageView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_modal_first(window, cx);
        self.toggle_page_view(cx);
    }

    pub(crate) fn on_toggle_vim(
        &mut self,
        _: &ToggleVimMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_modal_first(window, cx);
        self.toggle_vim_mode(cx);
    }

    pub(crate) fn on_show_agents(
        &mut self,
        _: &ShowAgents,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_agents(window, cx);
    }

    pub(crate) fn on_version_history(
        &mut self,
        _: &ShowVersionHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_version_history(window, cx);
    }

    pub(crate) fn on_document_settings(
        &mut self,
        _: &OpenDocumentSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_document_settings(window, cx);
    }

    pub(crate) fn on_toggle_orientation(
        &mut self,
        _: &TogglePageOrientation,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_orientation(cx);
    }

    pub(crate) fn on_larger_body_text(
        &mut self,
        _: &IncreaseBodyTextSize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_body_text_size(1., cx);
    }

    pub(crate) fn on_smaller_body_text(
        &mut self,
        _: &DecreaseBodyTextSize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_body_text_size(-1., cx);
    }

    pub(crate) fn on_more_line_height(
        &mut self,
        _: &IncreaseLineHeight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_line_height(0.1, cx);
    }

    pub(crate) fn on_less_line_height(
        &mut self,
        _: &DecreaseLineHeight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_line_height(-0.1, cx);
    }

    pub(crate) fn on_title_from_text(
        &mut self,
        _: &UseTitleFromText,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.use_title_from_text(window, cx);
    }

    pub(crate) fn on_light_theme(
        &mut self,
        _: &UseLightTheme,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_theme(ThemeMode::Light, cx);
    }

    pub(crate) fn on_dark_theme(
        &mut self,
        _: &UseDarkTheme,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_theme(ThemeMode::Dark, cx);
    }

    pub(crate) fn on_system_theme(
        &mut self,
        _: &UseSystemTheme,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_theme(ThemeMode::System, cx);
    }

    pub(crate) fn on_dismiss_notifications(
        &mut self,
        _: &DismissNotifications,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toast_layer.update(cx, |layer, cx| layer.clear(cx));
    }

    pub(crate) fn on_close_window(
        &mut self,
        _: &CloseWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush(cx);
        self.window_bounds = Some(window.window_bounds());
        self.save_window_bounds(cx);
        window.remove_window();
    }

    pub(crate) fn on_about(&mut self, _: &About, window: &mut Window, cx: &mut Context<Self>) {
        self.show_about(window, cx);
    }

    pub(crate) fn on_minimize(&mut self, _: &Minimize, window: &mut Window, _: &mut Context<Self>) {
        window.minimize_window();
    }

    pub(crate) fn on_zoom(&mut self, _: &Zoom, window: &mut Window, _: &mut Context<Self>) {
        window.zoom_window();
    }

    pub(crate) fn on_toggle_full_screen(
        &mut self,
        _: &ToggleFullScreen,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.toggle_fullscreen();
    }
}

/// The standard items of the Edit menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditCommand {
    /// Undo.
    Undo,
    /// Redo.
    Redo,
    /// Cut.
    Cut,
    /// Copy.
    Copy,
    /// Paste (also "paste as plain text": a field holds plain text only).
    Paste,
    /// Select all.
    SelectAll,
}

impl Workspace {
    /// The Edit menu acts on whatever has the keyboard.
    ///
    /// The menu items are the editor's actions (so they show the editor's
    /// shortcuts). The text and the comment fields handle them themselves.
    /// A plain text field (the palette's query, a rename field, the find
    /// bar, the settings) has its own actions for the same things, bound
    /// to the same keys, but does not know the editor's: so when one of
    /// the editor's edit actions is on its way to a text field, it is
    /// replaced here by the field's own. In the library, Undo brings back
    /// what was last moved to the trash, as `cmd-z` does there.
    ///
    /// Registered for the capture phase on the workspace root, which runs
    /// before anything between the root and the focused element.
    pub(crate) fn edit_where_the_keyboard_is(
        &mut self,
        command: EditCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use writer_ui::text_field as field;
        let for_field: Box<dyn gpui::Action> = match command {
            EditCommand::Undo => Box::new(field::Undo),
            EditCommand::Redo => Box::new(field::Redo),
            EditCommand::Cut => Box::new(field::Cut),
            EditCommand::Copy => Box::new(field::Copy),
            EditCommand::Paste => Box::new(field::Paste),
            EditCommand::SelectAll => Box::new(field::SelectAll),
        };
        if window.is_action_available(for_field.as_ref(), cx) {
            cx.stop_propagation();
            window.dispatch_action(for_field, cx);
        } else if command == EditCommand::Undo && self.panel_has_focus(window, cx) {
            cx.stop_propagation();
            window.dispatch_action(Box::new(writer_library::UndoTrash), cx);
        }
    }
}

/// How many rows a picker modal may show in this window: as many as
/// `wanted`, fewer when the window is short (the modal starts 12% down and
/// has a 48px query row).
pub(crate) fn picker_rows(window: &Window, wanted: usize) -> usize {
    let height = f32::from(window.viewport_size().height);
    let available = height * 0.88 - 48. - 8. - 24.;
    ((available / 32.).floor().max(0.) as usize).clamp(4, wanted.max(4))
}

/// The registered command an ex command line names: the command's title,
/// its action name or "category title", compared without case, spaces and
/// punctuation (`:togglepageview`, `:ToggleBold`, `:export`). The first
/// registered match wins.
pub fn command_named(name: &str, cx: &App) -> Option<CommandDescriptor> {
    fn squeeze(text: &str) -> String {
        text.chars()
            .filter(|ch| ch.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    }
    let wanted = squeeze(name);
    if wanted.is_empty() {
        return None;
    }
    crate::commands::all(cx).into_iter().find(|command| {
        let action = command.action_name();
        let action = action.rsplit("::").next().unwrap_or(action);
        squeeze(&command.title) == wanted
            || squeeze(action) == wanted
            || squeeze(&command.label()) == wanted
    })
}

/// The commands of the editor and its tools that act on the text and that
/// the workspace passes on to it (see [`Workspace::forward_to_text`]).
///
/// Left out: the Edit menu's standard items and the context menu. They
/// belong to whatever has the keyboard (undo in the library brings back a
/// trashed document, paste in a field pastes there), and are never sent
/// to the text from somewhere else.
pub fn text_commands() -> Vec<Box<dyn gpui::Action>> {
    writer_editor::commands()
        .into_iter()
        .filter(|command| command.category.as_ref() != writer_editor::CATEGORY_EDIT)
        .chain(writer_editor_tools::commands())
        .map(|command| command.action())
        .collect()
}

fn set_keyed(slots: &mut Vec<(SharedString, AnyView)>, key: SharedString, view: Option<AnyView>) {
    let position = slots.iter().position(|(existing, _)| *existing == key);
    match (position, view) {
        (Some(ix), Some(view)) => slots[ix].1 = view,
        (Some(ix), None) => {
            slots.remove(ix);
        }
        (None, Some(view)) => slots.push((key, view)),
        (None, None) => {}
    }
}
