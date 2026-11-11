//! The commands of the tools, for the command palette and the shortcuts
//! overlay.

use writer_ui::CommandDescriptor;

use crate::actions::*;

/// Category of the format menu, link and font commands (the same name
/// `writer_editor` uses for its formatting commands).
pub const CATEGORY_FORMAT: &str = "Format";
/// Category of the insert commands.
pub const CATEGORY_INSERT: &str = "Insert";
/// Category of the find and replace commands.
pub const CATEGORY_SEARCH: &str = "Search";

/// Every user-facing command of this crate. The actions are handled by
/// [`crate::EditorTools`], so the palette runs them with
/// `window.dispatch_action(command.action(), cx)` after it has put focus
/// back in the editor.
pub fn commands() -> Vec<CommandDescriptor> {
    vec![
        CommandDescriptor::new("Format Menu", CATEGORY_FORMAT, OpenFormatMenu).keywords([
            "style",
            "contextual",
            "turn into",
        ]),
        CommandDescriptor::new("Add or Edit Link", CATEGORY_FORMAT, OpenLink)
            .keywords(["url", "hyperlink"]),
        CommandDescriptor::new("Remove Link", CATEGORY_FORMAT, RemoveLink)
            .keywords(["url", "unlink"]),
        CommandDescriptor::new("Open Link in Browser", CATEGORY_FORMAT, OpenLinkInBrowser)
            .keywords(["url", "follow", "visit"]),
        CommandDescriptor::new("Choose Font", CATEGORY_FORMAT, OpenFontPicker)
            .keywords(["typeface", "family"]),
        CommandDescriptor::new("Insert Menu", CATEGORY_INSERT, OpenInsertMenu)
            .keywords(["slash", "block", "add"]),
        CommandDescriptor::new("Today's Date", CATEGORY_INSERT, InsertDate)
            .keywords(["date", "today", "now"]),
        CommandDescriptor::new("Find", CATEGORY_SEARCH, Find).keywords(["search"]),
        CommandDescriptor::new("Find and Replace", CATEGORY_SEARCH, FindAndReplace)
            .keywords(["search", "substitute"]),
        CommandDescriptor::new("Find Next", CATEGORY_SEARCH, FindNext).keywords(["search"]),
        CommandDescriptor::new("Find Previous", CATEGORY_SEARCH, FindPrevious).keywords(["search"]),
        CommandDescriptor::new(
            "Toggle Case Sensitive",
            CATEGORY_SEARCH,
            ToggleCaseSensitive,
        )
        .keywords(["find", "match case"]),
        CommandDescriptor::new("Toggle Whole Word", CATEGORY_SEARCH, ToggleWholeWord)
            .keywords(["find", "match word"]),
        CommandDescriptor::new("Replace", CATEGORY_SEARCH, ReplaceCurrent).keywords([
            "find",
            "substitute",
            "current match",
        ]),
        CommandDescriptor::new("Replace All", CATEGORY_SEARCH, ReplaceAll)
            .keywords(["find", "substitute"]),
        CommandDescriptor::new("Close Find Bar", CATEGORY_SEARCH, CloseTool)
            .keywords(["find", "hide", "dismiss", "tool"]),
    ]
}
