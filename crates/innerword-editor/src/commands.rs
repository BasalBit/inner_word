//! The editor's user-facing commands, for the command palette and the
//! keyboard-shortcuts overlay.

use gpui::Action;
use innerword_ui::CommandDescriptor;

use crate::actions::*;

/// Category of inline formatting commands.
pub const CATEGORY_FORMAT: &str = "Format";
/// Category of block kind commands.
pub const CATEGORY_BLOCK: &str = "Block";
/// Category of alignment, size, indent and block order commands.
pub const CATEGORY_LAYOUT: &str = "Layout";
/// Category of insertion commands.
pub const CATEGORY_INSERT: &str = "Insert";
/// Category of history, clipboard and selection commands.
pub const CATEGORY_EDIT: &str = "Edit";

/// Every editor command a user would look for by name: a title, a category
/// and the action. Shortcuts are not part of a descriptor; look them up in
/// the keymap with `CommandDescriptor::key_hint`. Plain caret movement and
/// deletion keys are not listed (they are in [`crate::actions`]).
///
/// The commands act on the focused editor: run one with
/// `window.dispatch_action(command.action(), cx)` while the editor has
/// focus.
pub fn commands() -> Vec<CommandDescriptor> {
    fn c(title: &'static str, category: &'static str, action: impl Action) -> CommandDescriptor {
        CommandDescriptor::new(title, category, action)
    }
    vec![
        // Inline formatting.
        c("Bold", CATEGORY_FORMAT, ToggleBold).keywords(["strong"]),
        c("Italic", CATEGORY_FORMAT, ToggleItalic).keywords(["emphasis"]),
        c("Underline", CATEGORY_FORMAT, ToggleUnderline),
        c("Strikethrough", CATEGORY_FORMAT, ToggleStrikethrough).keywords(["strike", "cross out"]),
        c("Inline Code", CATEGORY_FORMAT, ToggleInlineCode).keywords(["monospace"]),
        c("Highlight", CATEGORY_FORMAT, ToggleHighlight).keywords(["marker", "yellow"]),
        c("Clear Formatting", CATEGORY_FORMAT, ClearFormatting).keywords(["plain", "remove"]),
        // Block kinds.
        c("Paragraph", CATEGORY_BLOCK, SetParagraph).keywords(["body", "normal", "text"]),
        c("Heading 1", CATEGORY_BLOCK, SetHeading1).keywords(["title", "h1"]),
        c("Heading 2", CATEGORY_BLOCK, SetHeading2).keywords(["section", "h2"]),
        c("Heading 3", CATEGORY_BLOCK, SetHeading3).keywords(["subsection", "h3"]),
        c("Heading 4", CATEGORY_BLOCK, SetHeading4).keywords(["h4"]),
        c("Heading 5", CATEGORY_BLOCK, SetHeading5).keywords(["h5"]),
        c("Heading 6", CATEGORY_BLOCK, SetHeading6).keywords(["h6"]),
        c("Numbered List", CATEGORY_BLOCK, ToggleNumberedList).keywords(["ordered", "1."]),
        c("Bullet List", CATEGORY_BLOCK, ToggleBulletList).keywords(["unordered", "points"]),
        c("To-do List", CATEGORY_BLOCK, ToggleTodoList).keywords(["task", "checkbox", "checklist"]),
        c("Quote", CATEGORY_BLOCK, ToggleQuote).keywords(["blockquote", "citation"]),
        c("Code Block", CATEGORY_BLOCK, ToggleCodeBlock).keywords(["monospace", "snippet"]),
        c("Toggle To-do Checked", CATEGORY_BLOCK, ToggleTodoChecked)
            .keywords(["done", "complete", "tick"]),
        // Layout.
        c("Align Left", CATEGORY_LAYOUT, AlignLeft),
        c("Align Centre", CATEGORY_LAYOUT, AlignCenter).keywords(["center", "middle"]),
        c("Align Right", CATEGORY_LAYOUT, AlignRight),
        c("Justify", CATEGORY_LAYOUT, AlignJustify).keywords(["align"]),
        // One key means "larger" for whatever is selected: the text of the
        // blocks, or the image the caret is on. The titles say both,
        // because the list of keyboard shortcuts finds a command by its
        // title.
        c("Larger Text or Image", CATEGORY_LAYOUT, IncreaseTextSize)
            .keywords(["size", "bigger", "increase", "picture", "photo", "resize"]),
        c("Smaller Text or Image", CATEGORY_LAYOUT, DecreaseTextSize)
            .keywords(["size", "decrease", "picture", "photo", "resize"]),
        c("Reset Text Size", CATEGORY_LAYOUT, ResetTextSize).keywords(["default"]),
        c("Indent", CATEGORY_LAYOUT, Indent).keywords(["nest", "right"]),
        c("Outdent", CATEGORY_LAYOUT, Outdent).keywords(["unindent", "left"]),
        c("Move Block Up", CATEGORY_LAYOUT, MoveBlockUp).keywords(["paragraph", "reorder"]),
        c("Move Block Down", CATEGORY_LAYOUT, MoveBlockDown).keywords(["paragraph", "reorder"]),
        // The selected image.
        c("Image Width 25 %", CATEGORY_LAYOUT, ImageWidth25)
            .keywords(["picture", "size", "quarter", "small"]),
        c("Image Width 50 %", CATEGORY_LAYOUT, ImageWidth50).keywords(["picture", "size", "half"]),
        c("Image Width 75 %", CATEGORY_LAYOUT, ImageWidth75).keywords(["picture", "size"]),
        c("Image Width 100 %", CATEGORY_LAYOUT, ImageWidth100).keywords([
            "picture",
            "size",
            "full width",
            "column",
        ]),
        c("Image Original Size", CATEGORY_LAYOUT, ImageOriginalSize)
            .keywords(["picture", "actual", "natural", "reset"]),
        // Insert.
        c("Line Break", CATEGORY_INSERT, InsertSoftBreak).keywords(["soft break", "newline"]),
        c("Page Break", CATEGORY_INSERT, InsertPageBreak).keywords(["new page"]),
        c("Divider", CATEGORY_INSERT, InsertDivider).keywords(["rule", "separator", "line"]),
        c("Image", CATEGORY_INSERT, InsertImage).keywords([
            "picture",
            "photo",
            "figure",
            "screenshot",
            "insert",
        ]),
        // Edit.
        c("Undo", CATEGORY_EDIT, Undo),
        c("Redo", CATEGORY_EDIT, Redo),
        c("Cut", CATEGORY_EDIT, Cut),
        c("Copy", CATEGORY_EDIT, Copy),
        c("Paste", CATEGORY_EDIT, Paste),
        c("Paste as Plain Text", CATEGORY_EDIT, PasteAsPlainText).keywords(["unformatted"]),
        c("Select All", CATEGORY_EDIT, SelectAll),
        c("Context Menu", CATEGORY_EDIT, OpenContextMenu).keywords([
            "right click",
            "actions",
            "menu",
        ]),
    ]
}
