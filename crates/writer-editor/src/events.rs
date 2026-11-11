//! Events emitted by the [`crate::Editor`] entity.

use writer_core::Change;

/// What an [`crate::Editor`] tells its subscribers
/// (`cx.subscribe(&editor, |this, editor, event: &EditorEvent, cx| ..)`).
///
/// Events are emitted once per mutation, after the mutation is complete (see
/// [`crate::Editor::edit`]), in this order: `Edited`, `SettingsChanged`,
/// `SelectionChanged`. The enum is `#[non_exhaustive]`: match with a
/// wildcard arm.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum EditorEvent {
    /// Blocks or comment threads of the document changed (typing, a
    /// command, undo, an outside [`crate::Editor::edit`]). This is the
    /// autosave trigger. `change.blocks` is the changed block range,
    /// `change.comments` says whether threads or anchored text changed (the
    /// comments panel refreshes on it). Not emitted for pure selection or
    /// settings changes, nor by [`crate::Editor::set_document`].
    Edited {
        /// What changed, merged over the whole mutation.
        change: Change,
    },
    /// The selection differs from the one before the mutation.
    SelectionChanged,
    /// The editor's focus handle became focused (window active).
    Focused,
    /// The editor's focus handle lost focus. Also emitted when focus moves
    /// into the editor's own popup (see [`crate::Editor::show_popup`]).
    Blurred,
    /// The document settings changed (page setup, typography, page view).
    /// Settings are saved with the document, so autosave listens to this
    /// too.
    SettingsChanged,
    /// [`crate::Editor::set_document`] replaced the whole document. No
    /// `Edited` is emitted for it. Consumers drop everything they derived
    /// from the old document (find matches, thread lists, Vim state).
    DocumentReplaced,
    /// Compact mode with submit-on-enter: the user pressed `enter`. The
    /// text is still in the editor; read it with
    /// [`crate::Editor::plain_text`] and empty it with
    /// [`crate::Editor::clear`].
    Submitted,
}
