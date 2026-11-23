//! The whiteboard's own keys (`docs/whiteboard-design.md`, 4.7): the chords Excalidraw 0.18.1
//! acts on (read in its actions' key tests and its `onKeyDown`), which on a whiteboard are
//! the whiteboard's. The app binds each to `gpui::NoAction` in `BoardView` after every other
//! binding (fixed: never a default, a row of the list's commands or an entry of the shortcuts
//! file), builds the menu bar without them while a whiteboard is open, and lists them as
//! fixed rows "On a whiteboard". The tool keys (letters and digits) are one row of their own
//! and are not here: no key without `cmd` can be bound where text is typed.

/// One key of Excalidraw's on a whiteboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoardKey {
    /// GPUI's spelling, in `innerword_keymap`'s normal form: `cmd-d`, `cmd-alt-[`.
    pub keys: &'static str,
    /// What it does there, as the list of shortcuts titles the row: "Duplicate".
    pub what: &'static str,
}

const fn key(keys: &'static str, what: &'static str) -> BoardKey {
    BoardKey { keys, what }
}

/// Every key of section 4.7's table, read again in 0.18.1.
///
/// Each is written once, in `innerword_keymap`'s normal form. **Bind each in every spelling
/// `innerword_keymap::Keystroke::spellings` gives for it, not in this text alone**: shift with
/// a digit or a punctuation key reaches GPUI as the shifted character without the shift
/// flag, so `cmd-shift-,` and `cmd-shift-.` arrive as `cmd-<` and `cmd->` on a US keyboard,
/// and a binding of the normal form alone would never match, letting the window's "smaller
/// or larger text" run on a whiteboard. (This crate depends on nothing of ours, so it
/// cannot call the keymap itself; its tests hold the two spellings of those keys.)
pub const WHITEBOARD_KEYS: &[BoardKey] = &[
    key("cmd-z", "Undo"),
    key("cmd-shift-z", "Redo"),
    key("cmd-c", "Copy"),
    key("cmd-x", "Cut"),
    key("cmd-v", "Paste"),
    key("cmd-shift-v", "Paste as plain text"),
    key("cmd-a", "Select all"),
    key("cmd-d", "Duplicate"),
    key("cmd-g", "Group"),
    key("cmd-shift-g", "Ungroup"),
    key("cmd-[", "Send backward"),
    key("cmd-]", "Bring forward"),
    key("cmd-alt-[", "Send to back"),
    key("cmd-alt-]", "Bring to front"),
    key("cmd-alt-c", "Copy styles"),
    key("cmd-alt-v", "Paste styles"),
    key("cmd-shift-up", "Align top"),
    key("cmd-shift-down", "Align bottom"),
    key("cmd-shift-left", "Align left"),
    key("cmd-shift-right", "Align right"),
    key("cmd-shift-,", "Smaller font"),
    key("cmd-shift-.", "Larger font"),
    key("cmd-'", "Show or hide the grid"),
    key("cmd-k", "Link of the selected element"),
    key("cmd-shift-l", "Lock or unlock"),
    key("cmd-up", "Add the next node of a flowchart above"),
    key("cmd-down", "Add the next node of a flowchart below"),
    key("cmd-left", "Add the next node of a flowchart to the left"),
    key("cmd-right", "Add the next node of a flowchart to the right"),
    key("cmd-enter", "Edit the points of a line or arrow; confirm in a dialog"),
    key("cmd-backspace", "Clear the canvas (asks first)"),
    key("cmd-delete", "Clear the canvas (asks first)"),
    key("alt-up", "Go to the next node of a flowchart above"),
    key("alt-down", "Go to the next node of a flowchart below"),
    key("alt-left", "Go to the next node of a flowchart to the left"),
    key("alt-right", "Go to the next node of a flowchart to the right"),
    key("alt-z", "Zen mode"),
    key("alt-s", "Snap to objects"),
    key("alt-/", "Statistics (position, size and angle in fields)"),
    key("alt-r", "View mode"),
    key("alt-h", "Distribute horizontally"),
    key("alt-v", "Distribute vertically"),
    key("alt-shift-c", "Copy as PNG"),
];

/// The title of the one row for the tool keys (the list of shortcuts).
pub const TOOL_KEYS_ROW: &str = "Tools (letters and digits)";

/// Whether a key in normal form is one of [`WHITEBOARD_KEYS`].
pub fn is_whiteboard_key(keys: &str) -> bool {
    WHITEBOARD_KEYS.iter().any(|key| key.keys == keys)
}
