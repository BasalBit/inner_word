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
        /// In a message window: take the way out it suggests.
        AcceptMessage,
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
        /// Save the open document now, without waiting for the autosave.
        SaveNow,
        /// Rename the open document: its row in the library becomes a field.
        RenameDocument,
        /// Show the open document's row in the library.
        RevealInLibrary,
        /// Open the welcome document; it is written again when it is gone.
        OpenWelcomeDocument,
        /// Choose the language the open document's spelling is checked in.
        ChooseSpellingLanguage,
        /// Show what agents on this Mac can do with the library: the command
        /// line tool, the skill and the MCP server.
        ShowAgents,
        /// Ask the assistant in a small chat at the selection: about the
        /// selected text, or a plain question when nothing is selected.
        AskAssistant,
        /// Go to the assistant's side chat, opening it when it is closed.
        FocusAssistant,
        /// Show or hide the assistant's side chat.
        ToggleAssistant,
        /// Start a new chat about the open document in the side chat.
        NewAssistantChat,
        /// Clear the side chat's conversation about the open document.
        ClearAssistantChat,
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
/// Category of the assistant's commands. They are listed only while the
/// assistant is shown (its engine is installed and the setting is on):
/// see [`crate::commands::set_category_hidden`].
pub const CATEGORY_ASSISTANT: &str = "Assistant";

/// The app's context-free key bindings.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        // The message window shown when the library cannot be opened. First
        // in the list: key hints and menus show the binding added last, and
        // that must stay cmd-q.
        KeyBinding::new(
            "enter",
            AcceptMessage,
            Some(crate::window::ERROR_WINDOW_KEY_CONTEXT),
        ),
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
        KeyBinding::new("cmd-s", SaveNow, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-/", ShowShortcuts, None),
        KeyBinding::new("cmd-alt-p", TogglePageView, None),
        KeyBinding::new("cmd-alt-v", ToggleVimMode, None),
        KeyBinding::new("cmd-alt-a", ShowAgents, None),
        KeyBinding::new("cmd-j", AskAssistant, None),
        KeyBinding::new("cmd-3", FocusAssistant, None),
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
        CommandDescriptor::new("Save now", CATEGORY_FILE, SaveNow)
            .keywords(["write", "store", "autosave"]),
        CommandDescriptor::new("Rename document", CATEGORY_FILE, RenameDocument).keywords([
            "name",
            "title",
            "this document",
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
        CommandDescriptor::new("Reveal in library", CATEGORY_GO, RevealInLibrary).keywords([
            "show",
            "sidebar",
            "where",
            "folder",
            "this document",
        ]),
        CommandDescriptor::new("Go to library", CATEGORY_GO, FocusLibrary)
            .keywords(["focus", "sidebar", "folders", "tree"]),
        CommandDescriptor::new("Go to text", CATEGORY_GO, FocusEditor)
            .keywords(["focus", "editor", "document", "write"]),
        CommandDescriptor::new("Command palette", CATEGORY_GO, ToggleCommandPalette)
            .keywords(["commands", "actions", "run"]),
        CommandDescriptor::new("Show or hide library", CATEGORY_VIEW, ToggleSidebar)
            .keywords(["sidebar", "toggle", "folders"]),
        CommandDescriptor::new("Widen library", CATEGORY_VIEW, WidenSidebar)
            .keywords(["sidebar", "resize", "bigger"]),
        CommandDescriptor::new("Narrow library", CATEGORY_VIEW, NarrowSidebar)
            .keywords(["sidebar", "resize", "smaller"]),
        CommandDescriptor::new("Turn page view on or off", CATEGORY_VIEW, TogglePageView)
            .keywords(["toggle", "paper", "pages", "print layout", "flow"]),
        CommandDescriptor::new("Turn Vim mode on or off", CATEGORY_VIEW, ToggleVimMode).keywords([
            "toggle",
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
        CommandDescriptor::new("Agents", CATEGORY_APP, ShowAgents).keywords([
            "ai",
            "assistant",
            "claude",
            "codex",
            "gemini",
            "cursor",
            "copilot",
            "opencode",
            "mcp",
            "server",
            "skill",
            "install",
            "command line",
            "cli",
            "terminal",
            "tool",
            "writer",
        ]),
        CommandDescriptor::new("Welcome document", CATEGORY_APP, OpenWelcomeDocument).keywords([
            "help",
            "introduction",
            "getting started",
            "tour",
            "guide",
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

/// The app's own spelling command, listed after the checker's
/// (`writer_spell::commands`) in their category: the picker for the
/// language of the open document.
pub fn spelling_commands() -> Vec<CommandDescriptor> {
    vec![
        CommandDescriptor::new(
            "Spelling language",
            writer_spell::CATEGORY_SPELLING,
            ChooseSpellingLanguage,
        )
        .keywords([
            "spell",
            "dictionary",
            "proofing",
            "english",
            "automatic",
            "detect",
            "this document",
        ]),
    ]
}

/// The assistant's commands: the quick chat at the selection and the side
/// chat beside the text. In the registry they are in a category of their
/// own, which is hidden while the assistant is not shown.
pub fn assistant_commands() -> Vec<CommandDescriptor> {
    vec![
        CommandDescriptor::new("Ask the assistant", CATEGORY_ASSISTANT, AskAssistant).keywords([
            "ai",
            "quick chat",
            "question",
            "selection",
            "rewrite",
            "rephrase",
            "synonym",
            "word",
            "sentence",
            "claude",
        ]),
        CommandDescriptor::new("Go to assistant", CATEGORY_ASSISTANT, FocusAssistant).keywords([
            "ai",
            "side chat",
            "focus",
            "open",
            "panel",
            "conversation",
            "claude",
        ]),
        CommandDescriptor::new(
            "Show or hide assistant",
            CATEGORY_ASSISTANT,
            ToggleAssistant,
        )
        .keywords(["ai", "side chat", "toggle", "panel", "close", "claude"]),
        CommandDescriptor::new("New chat", CATEGORY_ASSISTANT, NewAssistantChat).keywords([
            "ai",
            "side chat",
            "start",
            "fresh",
            "conversation",
            "assistant",
        ]),
        CommandDescriptor::new("Clear chat", CATEGORY_ASSISTANT, ClearAssistantChat).keywords([
            "ai",
            "side chat",
            "empty",
            "delete",
            "forget",
            "conversation",
            "assistant",
        ]),
    ]
}
