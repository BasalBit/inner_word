//! The side chat's conversations: one per document, kept in the library.
//!
//! Each document has a conversation of its own. Switching documents
//! switches the conversation, and a relaunch brings it back: the transcript
//! (the messages, and the library items attached to it) is stored in the
//! library's settings under `assistant.conversation.<document id>`.
//!
//! - **Bounded.** What is stored is the recent end of the conversation: at
//!   most [`STORED_MESSAGES`] messages and [`STORED_CHARS`] characters,
//!   whole questions with their answers, the oldest dropped first. (In
//!   memory the engine's own, wider bounds apply.) An empty conversation is
//!   not stored at all.
//! - **Removed with the document.** The key goes when the document is
//!   deleted for good ([`forget`], from the library's event), and at
//!   launch every stored conversation whose document is gone is dropped
//!   ([`forget_missing`]): the command line tool can delete a document
//!   while the app is closed.
//! - **Never another document's.** The library may give the id of a deleted
//!   document to the next one that is created. A conversation is stored
//!   with the moment its document was created and is only handed back to
//!   a document created at that moment.

use gpui::{App, Entity};
use serde::{Deserialize, Serialize};
use writer_assistant::{Role, Transcript};
use writer_library::{DocumentId, Library};

use crate::settings::keys;

/// The most messages of a conversation that are stored.
pub const STORED_MESSAGES: usize = 80;
/// The most characters of message text of a conversation that are stored.
pub const STORED_CHARS: usize = 120_000;

/// A conversation as it is stored.
#[derive(Serialize, Deserialize)]
struct Stored {
    /// When the document the conversation is about was created (unix ms).
    document_created_at: i64,
    /// The conversation.
    transcript: Transcript,
}

/// The recent end of `transcript` that is stored: see the
/// [module docs](self). The attachments are kept whole.
pub fn bounded(transcript: &Transcript) -> Transcript {
    let mut bounded = transcript.clone();
    let size = |message: &writer_assistant::Message| {
        message.text.chars().count() + message.quote.as_ref().map_or(0, |q| q.chars().count())
    };
    // Whole exchanges only: a cut is made in front of a question.
    let mut chars: usize = bounded.messages.iter().map(size).sum();
    let mut drop = 0;
    let last_question = bounded
        .messages
        .iter()
        .rposition(|message| message.role == Role::User)
        .unwrap_or(0);
    while drop < last_question
        && (bounded.messages.len() - drop > STORED_MESSAGES || chars > STORED_CHARS)
    {
        chars = chars.saturating_sub(size(&bounded.messages[drop]));
        drop += 1;
    }
    while drop < last_question && bounded.messages[drop].role == Role::Assistant {
        drop += 1;
    }
    if drop > 0 {
        bounded.messages.drain(..drop);
        bounded.dropped += drop;
    }
    bounded
}

/// The stored conversation about document `id`; an empty one when nothing
/// is stored, when what is stored cannot be read, or when it belongs to an
/// earlier document that had the same id.
pub fn load(library: &Library, id: DocumentId) -> Transcript {
    let key = keys::assistant_conversation(id);
    let Some(stored) = library.setting::<Stored>(&key).ok().flatten() else {
        return Transcript::new();
    };
    let same_document = library
        .document_meta(id)
        .is_ok_and(|meta| meta.created_at == stored.document_created_at);
    if !same_document {
        return Transcript::new();
    }
    // Through the engine's own reader: answers that were being written
    // when the app quit come back as stopped, and its bounds are applied.
    Transcript::from_json(&stored.transcript.to_json()).unwrap_or_default()
}

/// Stores `transcript` as the conversation about document `id`, or removes
/// what was stored when there is nothing in it. Returns whether the
/// library holds the conversation afterwards.
pub fn store(
    library: &Entity<Library>,
    id: DocumentId,
    transcript: &Transcript,
    cx: &mut App,
) -> bool {
    let key = keys::assistant_conversation(id);
    if transcript.is_empty() && transcript.attachments.is_empty() {
        forget(library, id, cx);
        return false;
    }
    let Ok(meta) = library.read(cx).document_meta(id) else {
        return false;
    };
    let stored = Stored {
        document_created_at: meta.created_at,
        transcript: bounded(transcript),
    };
    library
        .update(cx, |library, cx| library.set_setting(&key, &stored, cx))
        .is_ok()
}

/// Removes the stored conversation about document `id`.
pub fn forget(library: &Entity<Library>, id: DocumentId, cx: &mut App) {
    let key = keys::assistant_conversation(id);
    let there = library
        .read(cx)
        .setting::<serde_json::Value>(&key)
        .ok()
        .flatten()
        .is_some();
    if there {
        library
            .update(cx, |library, cx| library.remove_setting(&key, cx))
            .ok();
    }
}

/// Whether a conversation about document `id` is stored.
pub fn is_stored(library: &Library, id: DocumentId) -> bool {
    library
        .setting::<serde_json::Value>(&keys::assistant_conversation(id))
        .ok()
        .flatten()
        .is_some()
}

/// Drops every stored conversation whose document is gone, or that
/// belongs to an earlier document with the same id. Called at launch.
pub fn forget_missing(library: &Entity<Library>, cx: &mut App) {
    let stale: Vec<String> = {
        let library = library.read(cx);
        library
            .setting_keys()
            .unwrap_or_default()
            .into_iter()
            .filter(|key| {
                let Some(id) = key
                    .strip_prefix(keys::ASSISTANT_CONVERSATION_PREFIX)
                    .and_then(|id| id.parse::<i64>().ok())
                else {
                    return false;
                };
                let stored: Option<Stored> = library.setting(key).ok().flatten();
                match (library.document_meta(DocumentId(id)), stored) {
                    (Ok(meta), Some(stored)) => meta.created_at != stored.document_created_at,
                    // Stored in a form this version does not read.
                    (Ok(_), None) => true,
                    (Err(writer_library::StoreError::NotFound { .. }), _) => true,
                    // Not known: leave it alone.
                    (Err(_), _) => false,
                }
            })
            .collect()
    };
    for key in stale {
        library
            .update(cx, |library, cx| library.remove_setting(&key, cx))
            .ok();
    }
}
