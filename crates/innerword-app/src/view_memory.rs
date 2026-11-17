//! What the app remembers about a document between two times it is open,
//! and how it makes sure that what it remembers is still true.
//!
//! The library is not only written by this app. The `innerword` command line
//! tool lets agents change it while the app is closed: a document that was
//! open at the last quit may have been rewritten, renamed, moved, trashed
//! or deleted before the next launch. Two things are stored per document
//! and read back later, and both are checked against the document as it is
//! now:
//!
//! - **The caret and the scroll position** (the editor's [`ViewState`]).
//!   They are stored together with a [`TextStamp`] of the text they were
//!   taken on. A stamp that does not fit the stored text means that
//!   something else wrote the text since: the remembered caret would sit at
//!   an arbitrary place and a remembered selection would cover words the
//!   writer has never seen (and the next key would replace them). The
//!   document then opens at its start, and the workspace says that it was
//!   changed. A view state without a stamp (stored by a version that did
//!   not stamp yet) is taken as it is; the editor clamps positions that
//!   are gone.
//! - **"The title was chosen"** ([`TitleFlag`]): the flag that keeps a
//!   save from deriving the title from the text. It is stored under the
//!   document's id, and the library hands out the id of a deleted document
//!   again, so the flag carries the creation time of the document it was
//!   set for and does not hold for another one. A document whose stored
//!   title is not the one its text gives was named by someone
//!   ([`named_outside`]): the flag is set when the app first sees it, so
//!   the next save keeps the name.

use serde::{Deserialize, Serialize};
use innerword_core::Document;
use innerword_editor::{Editor, ViewState};
use innerword_library::DocumentMeta;

use crate::document_io::derived_title;

/// A cheap fingerprint of a text: enough to tell that a stored caret
/// position was taken on another text. Counting it is a walk over the
/// blocks without looking at their text (microseconds for a book).
///
/// A change that keeps both numbers (a word replaced by one of the same
/// length) is not seen, and does not need to be: every position is still
/// where it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextStamp {
    /// How many blocks the document has.
    pub blocks: usize,
    /// The length of all block texts together, in bytes.
    pub bytes: usize,
}

impl TextStamp {
    /// The stamp of `document`.
    pub fn of(document: &Document) -> Self {
        Self {
            blocks: document.block_count(),
            bytes: document
                .blocks()
                .iter()
                .map(|block| block.text().len())
                .sum(),
        }
    }
}

/// The view state as the library stores it: the editor's fields, and the
/// stamp of the text beside them (`"text": {"blocks": .., "bytes": ..}`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct StoredViewState {
    #[serde(flatten)]
    view: ViewState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<TextStamp>,
}

/// The view state of `editor` as it is stored: where the caret and the
/// viewport are, stamped with the text they are in.
pub fn stored_view_state(editor: &Editor) -> Option<String> {
    stamped(&editor.view_state(), editor.document())
}

/// `view`, taken on `document`, as it is stored.
pub fn stamped(view: &ViewState, document: &Document) -> Option<String> {
    serde_json::to_string(&StoredViewState {
        view: *view,
        text: Some(TextStamp::of(document)),
    })
    .ok()
}

/// What a stored view state is worth for the document as it is now.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Remembered {
    /// The caret and scroll position to come back to, when they still
    /// belong to this text.
    pub view: Option<ViewState>,
    /// The text is not the one the view state was taken on: it was written
    /// by something other than this app since.
    pub text_changed: bool,
}

/// Reads the stored view state `json` of a document whose text is now
/// `document`.
pub fn remembered(json: Option<&str>, document: &Document) -> Remembered {
    let Some(stored) = json.and_then(|json| serde_json::from_str::<StoredViewState>(json).ok())
    else {
        return Remembered::default();
    };
    match stored.text {
        Some(stamp) if stamp != TextStamp::of(document) => Remembered {
            view: None,
            text_changed: true,
        },
        _ => Remembered {
            view: Some(stored.view),
            text_changed: false,
        },
    }
}

/// The stored value of the "the title was chosen" flag of a document
/// (`document.custom_title.<id>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TitleFlag {
    /// As earlier versions stored it (`true`), and as anything that knows
    /// the key but not the document's creation time may store it.
    Plain(bool),
    /// The creation time (unix ms) of the document the flag was set for.
    For(i64),
}

impl TitleFlag {
    /// The flag of the document `meta`, set.
    pub fn of(meta: &DocumentMeta) -> Self {
        TitleFlag::For(meta.created_at)
    }

    /// Whether the flag says that the title of `meta` was chosen. A flag
    /// that was set for an earlier document with the same id does not.
    pub fn holds_for(self, meta: &DocumentMeta) -> bool {
        match self {
            TitleFlag::Plain(chosen) => chosen,
            TitleFlag::For(created_at) => created_at == meta.created_at,
        }
    }
}

/// Whether the stored title of a document is one that somebody gave it
/// while this app was not looking: it is not the title the text gives, and
/// not the name the library gives a duplicate either ("Title copy", which
/// goes back to following the text with the first edit, as it always has).
///
/// The app itself only ever stores the derived title unless the flag is
/// set, so a title like this was written by the command line tool: an
/// agent renamed the document, or created it under a name of its own.
pub fn named_outside(meta: &DocumentMeta, document: &Document) -> bool {
    let title = meta.title.trim();
    if title.is_empty() {
        return false;
    }
    let derived = derived_title(document);
    if title == derived {
        return false;
    }
    // "Title copy", "Title copy copy".
    let mut rest = title;
    while let Some(shorter) = rest.strip_suffix(" copy") {
        if shorter == derived {
            return false;
        }
        rest = shorter;
    }
    true
}
