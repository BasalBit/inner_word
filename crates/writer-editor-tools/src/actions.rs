//! The actions of the editor tools and their default key bindings.
//!
//! Unit structs in the `editor_tools` namespace (`editor_tools::Find`). They
//! are handled by the [`crate::EditorTools`] view, so they work while focus
//! is anywhere inside it: in the editor, in the find bar, or in an open
//! popup. [`key_bindings`] binds them; [`crate::init`] installs the
//! bindings.

use gpui::{KeyBinding, actions};

/// The key context of the [`crate::EditorTools`] view (the editor, the find
/// bar and the popups are all inside it).
pub const KEY_CONTEXT: &str = "EditorTools";
/// The key context of the find bar.
pub const FIND_BAR_CONTEXT: &str = "FindBar";
/// The key context of the link popover.
pub const LINK_CONTEXT: &str = "LinkPopover";

actions!(
    editor_tools,
    [
        /// Open the contextual format menu at the caret or selection.
        OpenFormatMenu,
        /// Open the insert menu at the caret (the same menu typing `/`
        /// opens).
        OpenInsertMenu,
        /// Add a link, or edit the link at the caret or selection.
        OpenLink,
        /// Remove the link at the caret or from the selection.
        RemoveLink,
        /// Open the URL in the link popover (or the link at the caret) in
        /// the browser.
        OpenLinkInBrowser,
        /// Open the font picker.
        OpenFontPicker,
        /// Insert today's date at the caret.
        InsertDate,
        /// Open the find bar.
        Find,
        /// Open the find bar with the replace field.
        FindAndReplace,
        /// Go to the next match.
        FindNext,
        /// Go to the previous match.
        FindPrevious,
        /// Switch case-sensitive search on or off.
        ToggleCaseSensitive,
        /// Switch whole-word search on or off.
        ToggleWholeWord,
        /// Replace the current match and go to the next one.
        ReplaceCurrent,
        /// Replace every match (one undo step).
        ReplaceAll,
        /// Move between the fields of the find bar.
        FocusNextField,
        /// Move between the fields of the find bar, backwards.
        FocusPreviousField,
        /// Close the open tool and return to the editor.
        CloseTool,
    ]
);

/// The default key bindings. Alternatives come before the canonical key,
/// because key hints show the binding added last.
///
/// `enter` and `escape` inside the tools are not bindings of this crate:
/// they arrive through the text fields (`TextField` submits on `enter` and
/// cancels on `escape`) and, for the insert menu, through the editor's key
/// interception.
pub fn key_bindings() -> Vec<KeyBinding> {
    let tools = Some(KEY_CONTEXT);
    let find = Some(FIND_BAR_CONTEXT);
    let link = Some(LINK_CONTEXT);
    vec![
        // Find bar (before the tools bindings, so the hints of "find
        // previous" and "find next" show cmd-shift-g and cmd-g).
        KeyBinding::new("shift-enter", FindPrevious, find),
        KeyBinding::new("tab", FocusNextField, find),
        KeyBinding::new("shift-tab", FocusPreviousField, find),
        KeyBinding::new("cmd-enter", ReplaceAll, find),
        KeyBinding::new("cmd-alt-c", ToggleCaseSensitive, find),
        KeyBinding::new("cmd-alt-w", ToggleWholeWord, find),
        // Link popover.
        KeyBinding::new("cmd-enter", OpenLinkInBrowser, link),
        // Anywhere inside the tools view.
        KeyBinding::new("cmd-.", OpenFormatMenu, tools),
        KeyBinding::new("cmd-k", OpenLink, tools),
        KeyBinding::new("cmd-shift-k", RemoveLink, tools),
        KeyBinding::new("cmd-t", OpenFontPicker, tools),
        KeyBinding::new("cmd-f", Find, tools),
        KeyBinding::new("cmd-alt-f", FindAndReplace, tools),
        KeyBinding::new("cmd-g", FindNext, tools),
        KeyBinding::new("cmd-shift-g", FindPrevious, tools),
    ]
}
