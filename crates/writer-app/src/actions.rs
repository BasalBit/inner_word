//! App-level actions, their default key bindings and their command
//! descriptors.
//!
//! The bindings here have no key context, so they work wherever focus is in
//! the window. They are installed before the widget, editor and library
//! bindings: a binding with no context ties with the deepest context and the
//! later one wins, so a crate that needs one of these keys inside its own
//! context can still take it.
//!
//! Not here, although they work anywhere in the window: the comment keys
//! (`cmd-alt-m`, `cmd-alt-]`, `cmd-alt-[`, `cmd-alt-c`, `cmd-2`), which
//! `writer-comments` binds and handles itself, and the keys of the editor
//! tools (`cmd-.`, `cmd-k`, `cmd-t`, the find keys), which live in the
//! context of the editor area.

use gpui::{KeyBinding, actions};
use writer_ui::CommandDescriptor;

actions!(
    app,
    [
        /// Quit The Writer.
        Quit,
        /// Close the current window.
        CloseWindow,
        /// Show the version of The Writer.
        About,
        /// Hide The Writer.
        Hide,
        /// Hide every other application.
        HideOthers,
        /// Show all hidden applications.
        ShowAll,
        /// Minimize the window.
        Minimize,
        /// Zoom the window.
        Zoom,
        /// Enter or leave full screen.
        ToggleFullScreen,
    ]
);

actions!(
    workspace,
    [
        /// Open the command palette.
        ToggleCommandPalette,
        /// Open a document by name.
        QuickOpen,
        /// Create a document in the folder selected in the library.
        NewDocument,
        /// Create a folder in the folder selected in the library.
        NewFolder,
        /// Move focus to the library sidebar.
        FocusLibrary,
        /// Move focus to the editor.
        FocusEditor,
        /// Show or hide the library sidebar.
        ToggleSidebar,
        /// Make the library sidebar wider.
        WidenSidebar,
        /// Make the library sidebar narrower.
        NarrowSidebar,
        /// Search the text of every document.
        SearchLibrary,
        /// Export the current document or its folder.
        Export,
        /// Open the settings.
        OpenSettings,
        /// Show every keyboard shortcut.
        ShowShortcuts,
        /// Switch the current document between the flowing column and page view.
        TogglePageView,
        /// Turn Vim mode on or off.
        ToggleVimMode,
        /// Derive the document's title from its text again.
        UseTitleFromText,
        /// List the earlier versions of the current document.
        ShowVersionHistory,
        /// Open the settings of the current document: paper, margins, type.
        OpenDocumentSettings,
        /// Turn the pages of the current document: portrait or landscape.
        TogglePageOrientation,
        /// Make the body text of the current document one point larger.
        IncreaseBodyTextSize,
        /// Make the body text of the current document one point smaller.
        DecreaseBodyTextSize,
        /// Give the lines of the current document more room.
        IncreaseLineHeight,
        /// Give the lines of the current document less room.
        DecreaseLineHeight,
        /// Use the Paper (light) theme.
        UseLightTheme,
        /// Use the Night (dark) theme.
        UseDarkTheme,
        /// Follow the system appearance.
        UseSystemTheme,
        /// Dismiss the notifications at the bottom of the window.
        DismissNotifications,
    ]
);

/// Category of the commands about documents and folders.
pub const CATEGORY_FILE: &str = "File";
/// Category of the commands that change the open document as a whole:
/// its paper and its typography.
pub const CATEGORY_DOCUMENT: &str = "Document";
/// Category of the commands that move around the app.
pub const CATEGORY_GO: &str = "Go";
/// Category of the view toggles.
pub const CATEGORY_VIEW: &str = "View";
/// Category of the app-wide commands.
pub const CATEGORY_APP: &str = "App";

/// The app's context-free key bindings.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        // The message window shown when the library cannot be opened. First
        // in the list: key hints and menus show the binding added last, and
        // that must stay cmd-q.
        KeyBinding::new("enter", Quit, Some(crate::window::ERROR_WINDOW_KEY_CONTEXT)),
        KeyBinding::new(
            "escape",
            Quit,
            Some(crate::window::ERROR_WINDOW_KEY_CONTEXT),
        ),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("cmd-alt-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
        KeyBinding::new("cmd-shift-p", ToggleCommandPalette, None),
        KeyBinding::new("cmd-p", QuickOpen, None),
        KeyBinding::new("cmd-n", NewDocument, None),
        KeyBinding::new("cmd-shift-n", NewFolder, None),
        KeyBinding::new("cmd-0", FocusLibrary, None),
        KeyBinding::new("cmd-1", FocusEditor, None),
        KeyBinding::new("cmd-alt-s", ToggleSidebar, None),
        KeyBinding::new("cmd-alt-right", WidenSidebar, None),
        KeyBinding::new("cmd-alt-left", NarrowSidebar, None),
        KeyBinding::new("cmd-shift-f", SearchLibrary, None),
        KeyBinding::new("cmd-shift-s", Export, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-/", ShowShortcuts, None),
        KeyBinding::new("cmd-alt-p", TogglePageView, None),
        KeyBinding::new("cmd-alt-v", ToggleVimMode, None),
    ]
}

/// The app's commands for the palette and the shortcuts overlay.
pub fn commands() -> Vec<CommandDescriptor> {
    vec![
        CommandDescriptor::new("New document", CATEGORY_FILE, NewDocument)
            .keywords(["create", "add", "page", "file"]),
        CommandDescriptor::new("New folder", CATEGORY_FILE, NewFolder).keywords([
            "create",
            "add",
            "directory",
            "group",
        ]),
        CommandDescriptor::new("Export", CATEGORY_FILE, Export).keywords([
            "markdown",
            "word",
            "docx",
            "plain text",
            "save as",
            "copy",
        ]),
        CommandDescriptor::new("Version history", CATEGORY_FILE, ShowVersionHistory).keywords([
            "versions",
            "snapshots",
            "restore",
            "revert",
            "earlier",
            "previous",
            "go back",
            "recover",
        ]),
        CommandDescriptor::new("Take title from text", CATEGORY_FILE, UseTitleFromText)
            .keywords(["rename", "name", "derive", "reset"]),
        CommandDescriptor::new("Close window", CATEGORY_FILE, CloseWindow),
        CommandDescriptor::new("Document settings", CATEGORY_DOCUMENT, OpenDocumentSettings)
            .keywords([
                "page setup",
                "paper",
                "page size",
                "a4",
                "us letter",
                "custom",
                "margins",
                "orientation",
                "font",
                "text size",
                "line height",
                "paragraph spacing",
                "typography",
                "this document",
            ]),
        CommandDescriptor::new(
            "Switch portrait and landscape",
            CATEGORY_DOCUMENT,
            TogglePageOrientation,
        )
        .keywords(["orientation", "page setup", "rotate", "sideways"]),
        CommandDescriptor::new("Larger body text", CATEGORY_DOCUMENT, IncreaseBodyTextSize)
            .keywords(["font size", "base size", "bigger", "zoom", "typography"]),
        CommandDescriptor::new("Smaller body text", CATEGORY_DOCUMENT, DecreaseBodyTextSize)
            .keywords(["font size", "base size", "typography"]),
        CommandDescriptor::new("More line spacing", CATEGORY_DOCUMENT, IncreaseLineHeight)
            .keywords(["line height", "leading", "looser", "typography"]),
        CommandDescriptor::new("Less line spacing", CATEGORY_DOCUMENT, DecreaseLineHeight)
            .keywords(["line height", "leading", "tighter", "typography"]),
        CommandDescriptor::new("Open document", CATEGORY_GO, QuickOpen).keywords([
            "quick open",
            "switch",
            "find file",
            "go to",
        ]),
        CommandDescriptor::new("Search library", CATEGORY_GO, SearchLibrary).keywords([
            "find",
            "full text",
            "everywhere",
            "all documents",
        ]),
        CommandDescriptor::new("Focus library", CATEGORY_GO, FocusLibrary)
            .keywords(["sidebar", "folders", "tree"]),
        CommandDescriptor::new("Focus editor", CATEGORY_GO, FocusEditor)
            .keywords(["text", "document", "write"]),
        CommandDescriptor::new("Command palette", CATEGORY_GO, ToggleCommandPalette)
            .keywords(["commands", "actions", "run"]),
        CommandDescriptor::new("Show or hide library", CATEGORY_VIEW, ToggleSidebar)
            .keywords(["sidebar", "toggle", "folders"]),
        CommandDescriptor::new("Widen library", CATEGORY_VIEW, WidenSidebar)
            .keywords(["sidebar", "resize", "bigger"]),
        CommandDescriptor::new("Narrow library", CATEGORY_VIEW, NarrowSidebar)
            .keywords(["sidebar", "resize", "smaller"]),
        CommandDescriptor::new("Toggle page view", CATEGORY_VIEW, TogglePageView).keywords([
            "paper",
            "pages",
            "print layout",
            "flow",
        ]),
        CommandDescriptor::new("Toggle Vim mode", CATEGORY_VIEW, ToggleVimMode).keywords([
            "vi",
            "modal",
            "normal mode",
        ]),
        CommandDescriptor::new("Use Paper theme", CATEGORY_VIEW, UseLightTheme).keywords([
            "light",
            "appearance",
            "day",
        ]),
        CommandDescriptor::new("Use Night theme", CATEGORY_VIEW, UseDarkTheme)
            .keywords(["dark", "appearance"]),
        CommandDescriptor::new("Follow system appearance", CATEGORY_VIEW, UseSystemTheme)
            .keywords(["theme", "automatic", "light", "dark"]),
        CommandDescriptor::new(
            "Enter or leave full screen",
            CATEGORY_VIEW,
            ToggleFullScreen,
        ),
        CommandDescriptor::new("Settings", CATEGORY_APP, OpenSettings).keywords([
            "preferences",
            "options",
            "configure",
        ]),
        CommandDescriptor::new("Keyboard shortcuts", CATEGORY_APP, ShowShortcuts).keywords([
            "keys",
            "bindings",
            "help",
            "cheat sheet",
        ]),
        CommandDescriptor::new("Dismiss notifications", CATEGORY_APP, DismissNotifications)
            .keywords(["toast", "message", "clear"]),
        CommandDescriptor::new("About The Writer", CATEGORY_APP, About).keywords(["version"]),
        CommandDescriptor::new("Quit The Writer", CATEGORY_APP, Quit).keywords(["exit"]),
    ]
}
