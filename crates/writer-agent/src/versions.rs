//! History: the versions the library keeps of a document.

use writer_store::{Snapshot, SnapshotId, Store, StoreError};

use crate::content::{self, Form, Reason, version_info};
use crate::error::{Error, ItemKind, Result, quoted_excerpt};
use crate::library::{Library, Session, author_phrase};
use crate::live::Change;
use crate::resolve::{Index, title_of};
use crate::types::{
    ListVersionsRequest, ReadVersionRequest, RestoreVersionRequest, VersionList, VersionRestored,
    VersionText,
};
use crate::write_text::render;

fn snapshot(store: &Store, id: i64) -> Result<Snapshot> {
    match store.snapshot(SnapshotId(id)) {
        Ok(snapshot) => Ok(snapshot),
        Err(StoreError::NotFound { .. }) => Err(Error::NotFound {
            kind: ItemKind::Version,
            reference: format!("with the id {id}"),
            hint: Some("The history of a document lists its versions with their ids".to_string()),
        }),
        Err(other) => Err(other.into()),
    }
}

impl Library {
    /// The versions of a document, newest first.
    pub fn versions(&self, request: &ListVersionsRequest) -> Result<VersionList> {
        let session = self.reader()?;
        let store = &session.store;
        let index = Index::load(store)?;
        let meta = index.document(store, &request.document)?;
        let labels = store.snapshot_labels(meta.id)?;
        Ok(VersionList {
            document: index.document_summary(store, &meta),
            versions: store
                .snapshots(meta.id)?
                .iter()
                .map(|snapshot| version_info(snapshot, labels.get(&snapshot.id).cloned()))
                .collect(),
        })
    }

    /// The text of one version.
    pub fn read_version(&self, request: &ReadVersionRequest) -> Result<VersionText> {
        let session = self.reader()?;
        let store = &session.store;
        let index = Index::load(store)?;
        let snapshot = snapshot(store, request.version)?;
        let meta = store.document_meta(snapshot.meta.document_id)?;
        let document = content::parse(&meta, &snapshot.content)?;
        Ok(VersionText {
            document: index.document_summary(store, &meta),
            version: version_info(&snapshot.meta, store.snapshot_label(snapshot.meta.id)?),
            format: request.format,
            content: render(&document, &snapshot.meta.title, request.format),
        })
    }
}

/// Makes a version the text of its document again, after keeping the text as it is
/// now as a version. See [`RestoreVersionRequest`].
pub(crate) fn restore_version(
    session: &mut Session<'_>,
    request: &RestoreVersionRequest,
) -> Result<VersionRestored> {
    let index = Index::load(session.store)?;
    let store = &session.store;
    let snapshot = snapshot(store, request.version)?;
    let meta = index.live_document(
        store,
        &crate::types::Target::DocumentId(snapshot.meta.document_id.0),
    )?;
    if let Some(expected) = &request.document {
        let expected = index.document(store, expected)?;
        if expected.id != meta.id {
            return Err(Error::invalid(format!(
                "the version {} is a version of the document {} (doc:{}), not of {} \
                     (doc:{}); nothing was restored",
                request.version,
                quoted_excerpt(title_of(&meta)),
                meta.id,
                quoted_excerpt(title_of(&expected)),
                expected.id,
            )));
        }
    }
    let loaded = content::load(store, &meta)?;
    let restored = content::parse(&meta, &snapshot.content)?;
    let changed = loaded.document != restored;
    let words_before = loaded.document.stats().words as u64;
    let kept_title = loaded.custom_title.then(|| loaded.meta.title.clone());
    let form = Form::of(&restored, kept_title.as_deref());
    let words_after = form.words;
    let restored_version = version_info(&snapshot.meta, store.snapshot_label(snapshot.meta.id)?);
    let mut after = writer_store::DocumentMeta {
        title: form.title.clone(),
        word_count: form.words,
        char_count: form.characters,
        ..loaded.meta.clone()
    };
    let mut kept = None;
    if changed && !session.dry_run {
        let store = &mut session.store;
        kept = Some(content::keep_version(
            store,
            meta.id,
            &loaded.raw,
            &Reason::Restore.label(&author_phrase(&request.write)),
        )?);
        session
            .changes
            .push(Change::VersionKept { document: meta.id });
        after = store.save_document(meta.id, &form.content())?;
        session.changes.push(Change::DocumentText { id: meta.id });
        if loaded.document.settings() != restored.settings() {
            session
                .changes
                .push(Change::DocumentSettings { id: meta.id });
        }
    } else if !changed {
        after = loaded.meta.clone();
    }
    let result = VersionRestored {
        document: index.document_summary(session.store, &after),
        restored: restored_version,
        snapshot: kept,
        words_before,
        words_after,
        written: session.written(changed),
    };
    Ok(result)
}
