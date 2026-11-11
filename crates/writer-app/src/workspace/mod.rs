//! The workspace: the root view of the main window.
//!
//! It lays out the titlebar band, the library sidebar, the editor, the right
//! panel slot, the status bar and the modal and toast layers; it owns the
//! document session (open, autosave, title) and handles every app-level
//! action.
//!
//! # Mount points
//!
//! The workspace owns one [`Editor`] entity for the lifetime of the window:
//! opening another document replaces the editor's document
//! ([`writer_editor::EditorEvent::DocumentReplaced`]). Crates that extend the
//! editor attach to that entity once and plug their views in here:
//!
//! | What | How |
//! |---|---|
//! | A wrapper around the editor (format menu, insert menu, find) | [`Workspace::set_editor_host`]: the view renders [`Workspace::editor`] itself, with whatever it adds |
//! | A panel at the right (comments) | [`Workspace::set_right_panel`], [`Workspace::set_right_panel_width`] |
//! | A strip under the editor (find bar, Vim command line) | [`Workspace::set_editor_strip`], keyed, stacked in insertion order |
//! | An item in the status bar (Vim mode) | [`Workspace::set_status_item`], keyed, shown at the left |
//! | Commands for the palette and the shortcuts overlay | [`crate::commands::register`] |
//! | Settings | [`Workspace::settings`], [`crate::WorkspaceEvent::SettingsChanged`], [`crate::settings::keys`] |
//! | Modals and toasts | [`Workspace::modal_layer`], [`Workspace::toast_layer`], [`Workspace::show_toast`] |
//!
//! ```ignore
//! let workspace = window_handle.root(cx)?;
//! let (editor, library) = workspace.read_with(cx, |w, _| (w.editor().clone(), w.library().clone()));
//! let tools = cx.new(|cx| EditorTools::new(editor.clone(), window, cx));
//! workspace.update(cx, |workspace, cx| {
//!     workspace.set_editor_host(Some(tools.clone().into()), cx);
//!     workspace.set_status_item("vim", Some(vim_status.into()), cx);
//!     workspace.set_editor_strip("vim-command-line", Some(command_line.into()), cx);
//!     workspace.set_right_panel(Some(comments.into()), cx);
//! });
//! cx.subscribe(&workspace, |_, event: &WorkspaceEvent, cx| { /* SettingsChanged, DocumentOpened */ });
//! ```

mod render;
mod session;

use std::time::Duration;

use gpui::{
    AnyView, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, Pixels,
    PromptLevel, SharedString, Subscription, Task, Window, WindowBounds, px,
};
use serde::Serialize;
use writer_core::Document;
use writer_editor::{Editor, EditorConfig, EditorEvent};
use writer_library::{
    DocumentId, Library, LibraryEvent, LibraryItem, LibraryPanel, LibraryPanelEvent,
};
use writer_ui::{
    CommandDescriptor, ModalLayer, Picker, ThemeMode, Toast, ToastLayer,
    theme::{self},
};

use crate::{
    actions::*,
    export::ExportDelegate,
    library_search::LibrarySearchDelegate,
    palette::PaletteDelegate,
    quick_open::QuickOpenDelegate,
    settings::{self, Settings, keys},
    settings_view::SettingsView,
    shortcuts::ShortcutsDelegate,
    window::StoredBounds,
};

pub use render::{STATUS_BAR_HEIGHT, StatusLine, counted, group_digits};
pub use session::{AUTOSAVE_DELAY, RETRY_DELAY, SNAPSHOT_INTERVAL, SaveState, first_occurrence};
pub(crate) use session::{Session, display_title as session_title};

/// The key context of the workspace root element.
pub const KEY_CONTEXT: &str = "Workspace";

/// Default width of the right panel slot.
pub const DEFAULT_RIGHT_PANEL_WIDTH: f32 = 320.;

/// How much one "widen" or "narrow" step changes the sidebar width.
const SIDEBAR_STEP: f32 = 24.;

/// How long window moves and resizes settle before the bounds are stored.
const BOUNDS_SAVE_DELAY: Duration = Duration::from_millis(600);

/// How many commands the palette remembers as recently used.
const MAX_RECENT_COMMANDS: usize = 8;

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
    pub(crate) modal_layer: Entity<ModalLayer>,
    pub(crate) toast_layer: Entity<ToastLayer>,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) settings: Settings,
    pub(crate) session: Option<Session>,
    pub(crate) sidebar_dragging: bool,
    pub(crate) editor_host: Option<AnyView>,
    pub(crate) right_panel: Option<AnyView>,
    pub(crate) right_panel_width: Pixels,
    pub(crate) strips: Vec<(SharedString, AnyView)>,
    pub(crate) status_items: Vec<(SharedString, AnyView)>,
    pub(crate) character_count: Option<(u64, writer_export::SocialCount)>,
    pub(crate) window_bounds: Option<WindowBounds>,
    pub(crate) window_title: String,
    bounds_task: Option<Task<()>>,
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
        let panel = cx.new(|cx| LibraryPanel::new(library.clone(), cx));
        let modal_layer = cx.new(|_| ModalLayer::new());
        let toast_layer = cx.new(|_| ToastLayer::new());
        ToastLayer::set_global(&toast_layer, cx);

        let subscriptions = vec![
            cx.subscribe_in(&editor, window, Self::on_editor_event),
            cx.subscribe_in(&library, window, Self::on_library_event),
            cx.subscribe_in(&panel, window, Self::on_panel_event),
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

        let mut this = Self {
            library,
            editor,
            panel,
            modal_layer,
            toast_layer,
            focus_handle: cx.focus_handle(),
            settings,
            session: None,
            sidebar_dragging: false,
            editor_host: None,
            right_panel: None,
            right_panel_width: px(DEFAULT_RIGHT_PANEL_WIDTH),
            strips: Vec::new(),
            status_items: Vec::new(),
            character_count: None,
            window_bounds: None,
            window_title: String::new(),
            bounds_task: None,
            _subscriptions: subscriptions,
        };
        this.restore_session(window, cx);
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

    /// The title of the open document as the library shows it.
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

    /// Sets the width of the right panel slot (clamped to 220..=560).
    pub fn set_right_panel_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        self.right_panel_width = width.clamp(px(220.), px(560.));
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
        if old.character_limit != new.character_limit {
            self.character_count = None;
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
                    .width(px(600.))
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
                    .width(px(600.))
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
                    .width(px(720.))
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
                    .width(px(600.))
                    .max_visible_rows(rows)
            });
        });
    }

    /// Opens the export picker, or closes it when it is open.
    pub fn toggle_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.is_none() {
            self.show_toast(Toast::new("Open a document to export it"), cx);
            return;
        }
        let workspace = cx.weak_entity();
        let folder = self.export_folder_name(cx);
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle(window, cx, |window, cx| {
                let delegate = ExportDelegate::new(workspace, folder);
                let rows = picker_rows(window, 12);
                Picker::modal(delegate, window, cx).max_visible_rows(rows)
            });
        });
    }

    /// Opens the settings, or closes them when they are open.
    pub fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let library = self.library.clone();
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle(window, cx, |window, cx| {
                SettingsView::new(library, window, cx)
            });
        });
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

    /// Turns Vim mode on or off (a setting).
    pub fn toggle_vim_mode(&mut self, cx: &mut Context<Self>) {
        let on = !self.settings.vim_mode;
        self.write_setting(keys::VIM_MODE, &on, cx);
        self.show_toast(
            Toast::new(if on { "Vim mode on" } else { "Vim mode off" })
                .duration(Duration::from_secs(2)),
            cx,
        );
    }

    /// Shows or hides comments (a setting).
    pub fn toggle_comments(&mut self, cx: &mut Context<Self>) {
        let visible = !self.settings.comments_visible;
        self.write_setting(keys::COMMENTS_VISIBLE, &visible, cx);
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
        drop(window.prompt(
            PromptLevel::Info,
            "The Writer",
            Some(&detail),
            &["OK"],
            cx,
        ));
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
            EditorEvent::Edited { .. } | EditorEvent::SettingsChanged => self.mark_dirty(cx),
            _ => cx.notify(),
        }
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
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_page_view(cx);
    }

    pub(crate) fn on_toggle_vim(
        &mut self,
        _: &ToggleVimMode,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_vim_mode(cx);
    }

    pub(crate) fn on_toggle_comments(
        &mut self,
        _: &ToggleComments,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_comments(cx);
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

/// How many rows a picker modal may show in this window: as many as
/// `wanted`, fewer when the window is short (the modal starts 12% down and
/// has a 48px query row).
fn picker_rows(window: &Window, wanted: usize) -> usize {
    let height = f32::from(window.viewport_size().height);
    let available = height * 0.88 - 48. - 8. - 24.;
    ((available / 32.).floor().max(0.) as usize).clamp(4, wanted.max(4))
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
