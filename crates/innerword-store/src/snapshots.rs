//! Version snapshots of document content and their pruning policy.

use std::collections::HashSet;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::Store;
use crate::error::{ItemKind, Result, StoreError, not_found};
use crate::ids::{DocumentId, SnapshotId, count_from_sql};

/// Snapshot metadata (no content).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotMeta {
    /// The snapshot's id.
    pub id: SnapshotId,
    /// The document it belongs to.
    pub document_id: DocumentId,
    /// When it was taken, unix ms.
    pub created_at: i64,
    /// The document title at that time.
    pub title: String,
    /// Word count at that time.
    pub word_count: u64,
    /// Character count at that time.
    pub char_count: u64,
}

/// A snapshot with its content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Metadata.
    pub meta: SnapshotMeta,
    /// The opaque content JSON as it was saved at `meta.created_at`.
    pub content: String,
}

/// How snapshots are thinned out over time. Buckets are UTC calendar days and weeks.
///
/// 1. The newest snapshot is always kept.
/// 2. Everything younger than `keep_all_for` is kept.
/// 3. Up to `daily_for` old, only the newest snapshot of each day is kept.
/// 4. Older than that, only the newest snapshot of each week is kept.
/// 5. Finally, at most `max_per_document` snapshots are kept (the oldest go first).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotPolicy {
    /// Keep every snapshot younger than this. Default: 1 day.
    pub keep_all_for: Duration,
    /// Keep one per day for snapshots younger than this. Default: 30 days.
    pub daily_for: Duration,
    /// Hard cap per document. Default: 200. Values below 1 are treated as 1.
    pub max_per_document: usize,
}

impl Default for SnapshotPolicy {
    fn default() -> Self {
        Self {
            keep_all_for: Duration::from_secs(24 * 60 * 60),
            daily_for: Duration::from_secs(30 * 24 * 60 * 60),
            max_per_document: 200,
        }
    }
}

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const WEEK_MS: i64 = 7 * DAY_MS;

fn duration_ms(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

impl SnapshotPolicy {
    /// Given `(id, created_at)` pairs sorted newest first, returns the ids to delete.
    pub(crate) fn select_deletions(&self, now_ms: i64, snapshots: &[(i64, i64)]) -> Vec<i64> {
        let keep_all = duration_ms(self.keep_all_for);
        let daily = duration_ms(self.daily_for);
        let mut seen_buckets: HashSet<(u8, i64)> = HashSet::new();
        let mut kept = 0usize;
        let cap = self.max_per_document.max(1);
        let mut delete = Vec::new();
        for (index, &(id, created_at)) in snapshots.iter().enumerate() {
            let age = now_ms.saturating_sub(created_at);
            let keep = if index == 0 || age < keep_all {
                true
            } else if age < daily {
                seen_buckets.insert((0, created_at.div_euclid(DAY_MS)))
            } else {
                seen_buckets.insert((1, created_at.div_euclid(WEEK_MS)))
            };
            if keep && kept < cap {
                kept += 1;
            } else {
                delete.push(id);
            }
        }
        delete
    }
}

/// The settings key of a snapshot's label is this prefix followed by the snapshot's id
/// (`snapshot.label.42`). The value is an object `{"label": "...", "created_at": <unix
/// ms>}`; `created_at` is the creation time of the snapshot the label was given to, so a
/// label left behind by a deleted snapshot does not pass to a later one with the same id.
/// Use [`Store::snapshot_label`] and [`Store::snapshot_labels`] rather than the key.
pub const SNAPSHOT_LABEL_PREFIX: &str = "snapshot.label.";

/// A snapshot's label as the settings table holds it.
#[derive(Serialize, Deserialize)]
struct StoredLabel {
    label: String,
    created_at: i64,
}

fn label_key(id: SnapshotId) -> String {
    format!("{SNAPSHOT_LABEL_PREFIX}{}", id.0)
}

fn write_label(conn: &Connection, meta: &SnapshotMeta, label: &str, now: i64) -> Result<()> {
    let value = serde_json::to_string(&StoredLabel {
        label: label.to_owned(),
        created_at: meta.created_at,
    })?;
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value,
             updated_at = excluded.updated_at",
        params![label_key(meta.id), value, now],
    )?;
    Ok(())
}

/// The label in a stored value, when it was given to the snapshot created at `created_at`.
fn label_for(raw: &str, created_at: i64) -> Option<String> {
    let stored: StoredLabel = serde_json::from_str(raw).ok()?;
    (stored.created_at == created_at && !stored.label.is_empty()).then_some(stored.label)
}

const SNAPSHOT_COLS: &str = "id, document_id, created_at, title, word_count, char_count";

fn snapshot_meta_from_row(row: &Row<'_>) -> rusqlite::Result<SnapshotMeta> {
    Ok(SnapshotMeta {
        id: row.get(0)?,
        document_id: row.get(1)?,
        created_at: row.get(2)?,
        title: row.get(3)?,
        word_count: count_from_sql(row.get(4)?),
        char_count: count_from_sql(row.get(5)?),
    })
}

fn insert_snapshot(conn: &Connection, document: DocumentId, now: i64) -> Result<SnapshotMeta> {
    let inserted = conn.execute(
        "INSERT INTO snapshots (document_id, created_at, title, word_count, char_count, content)
         SELECT id, ?2, title, word_count, char_count, content FROM documents WHERE id = ?1",
        params![document, now],
    )?;
    if inserted == 0 {
        return Err(StoreError::NotFound {
            kind: ItemKind::Document,
            id: document.0,
        });
    }
    let id = SnapshotId(conn.last_insert_rowid());
    Ok(conn.query_row(
        &format!("SELECT {SNAPSHOT_COLS} FROM snapshots WHERE id = ?1"),
        [id],
        snapshot_meta_from_row,
    )?)
}

fn prune(
    conn: &Connection,
    document: DocumentId,
    policy: &SnapshotPolicy,
    now: i64,
) -> Result<usize> {
    let rows: Vec<(i64, i64)> = {
        let mut stmt = conn.prepare_cached(
            "SELECT id, created_at FROM snapshots WHERE document_id = ?1
             ORDER BY created_at DESC, id DESC",
        )?;
        stmt.query_map([document], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    let doomed = policy.select_deletions(now, &rows);
    let mut stmt = conn.prepare_cached("DELETE FROM snapshots WHERE id = ?1")?;
    for id in &doomed {
        stmt.execute([id])?;
    }
    Ok(doomed.len())
}

impl Store {
    /// Takes a snapshot of the document's currently saved content (works for trashed
    /// documents too). Does not prune.
    pub fn create_snapshot(&mut self, document: DocumentId) -> Result<SnapshotMeta> {
        let now = self.now();
        let tx = self.conn.transaction()?;
        let meta = insert_snapshot(&tx, document, now)?;
        tx.commit()?;
        Ok(meta)
    }

    /// Takes a snapshot like [`Store::create_snapshot`] and gives it a label: a few words
    /// that say why it was taken ("Before an edit by Claude"), for whoever looks at the
    /// versions of the document later. An empty label is no label.
    ///
    /// The schema knows no labels; they are kept in the settings table (see
    /// [`SNAPSHOT_LABEL_PREFIX`]), so a library with labels opens in every version of the
    /// app, and versions that do not know them show the snapshot without one.
    pub fn create_labeled_snapshot(
        &mut self,
        document: DocumentId,
        label: &str,
    ) -> Result<SnapshotMeta> {
        let now = self.now();
        let tx = self.conn.transaction()?;
        let meta = insert_snapshot(&tx, document, now)?;
        if label.trim().is_empty() {
            tx.execute("DELETE FROM settings WHERE key = ?1", [label_key(meta.id)])?;
        } else {
            write_label(&tx, &meta, label.trim(), now)?;
        }
        tx.commit()?;
        Ok(meta)
    }

    /// Gives a snapshot that is there already a label, in place of the one it has. An
    /// empty label takes the label away. Fails with [`StoreError::NotFound`] when there
    /// is no such snapshot.
    ///
    /// For a snapshot that turns out to be what a labelled one would have been: the app
    /// took it a moment ago, and an agent is about to change the text it holds.
    pub fn label_snapshot(&mut self, id: SnapshotId, label: &str) -> Result<()> {
        let now = self.now();
        let tx = self.conn.transaction()?;
        let meta = tx
            .query_row(
                &format!("SELECT {SNAPSHOT_COLS} FROM snapshots WHERE id = ?1"),
                [id],
                snapshot_meta_from_row,
            )
            .optional()?
            .ok_or(StoreError::NotFound {
                kind: ItemKind::Snapshot,
                id: id.0,
            })?;
        if label.trim().is_empty() {
            tx.execute("DELETE FROM settings WHERE key = ?1", [label_key(meta.id)])?;
        } else {
            write_label(&tx, &meta, label.trim(), now)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The label of a snapshot, `None` when it has none (or does not exist).
    pub fn snapshot_label(&self, id: SnapshotId) -> Result<Option<String>> {
        let row: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT s.created_at, t.value FROM snapshots s
                 JOIN settings t ON t.key = ?2
                 WHERE s.id = ?1",
                params![id, label_key(id)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(row.and_then(|(created_at, raw)| label_for(&raw, created_at)))
    }

    /// The labels of a document's snapshots, by snapshot id. Snapshots without a label
    /// are not in the map.
    pub fn snapshot_labels(
        &self,
        document: DocumentId,
    ) -> Result<std::collections::BTreeMap<SnapshotId, String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT s.id, s.created_at, t.value FROM snapshots s
             JOIN settings t ON t.key = ?2 || s.id
             WHERE s.document_id = ?1",
        )?;
        let rows = stmt.query_map(params![document, SNAPSHOT_LABEL_PREFIX], |r| {
            Ok((
                r.get::<_, SnapshotId>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut labels = std::collections::BTreeMap::new();
        for row in rows {
            let (id, created_at, raw) = row?;
            if let Some(label) = label_for(&raw, created_at) {
                labels.insert(id, label);
            }
        }
        Ok(labels)
    }

    /// Removes the labels of snapshots that are gone (pruned, deleted, or deleted with
    /// their document); returns how many were removed. Labels are small and a stale one is
    /// never shown, so this is housekeeping: call it after taking labelled snapshots.
    pub fn remove_stale_snapshot_labels(&mut self) -> Result<usize> {
        let removed = self.conn.execute(
            "DELETE FROM settings
             WHERE key >= ?1 AND key < ?2
               AND NOT EXISTS (SELECT 1 FROM snapshots s WHERE ?1 || s.id = settings.key)",
            params![
                SNAPSHOT_LABEL_PREFIX,
                format!("{SNAPSHOT_LABEL_PREFIX}\u{10ffff}")
            ],
        )?;
        Ok(removed)
    }

    /// Snapshot metadata for a document, newest first. Empty when there are none or the
    /// document does not exist.
    pub fn snapshots(&self, document: DocumentId) -> Result<Vec<SnapshotMeta>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {SNAPSHOT_COLS} FROM snapshots WHERE document_id = ?1
             ORDER BY created_at DESC, id DESC"
        ))?;
        let rows = stmt
            .query_map([document], snapshot_meta_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Loads one snapshot with its content.
    pub fn snapshot(&self, id: SnapshotId) -> Result<Snapshot> {
        self.conn
            .query_row(
                &format!("SELECT {SNAPSHOT_COLS}, content FROM snapshots WHERE id = ?1"),
                [id],
                |row| {
                    Ok(Snapshot {
                        meta: snapshot_meta_from_row(row)?,
                        content: row.get(6)?,
                    })
                },
            )
            .map_err(not_found(ItemKind::Snapshot, id.0))
    }

    /// Deletes one snapshot.
    pub fn delete_snapshot(&mut self, id: SnapshotId) -> Result<()> {
        let n = self
            .conn
            .execute("DELETE FROM snapshots WHERE id = ?1", [id])?;
        if n == 0 {
            return Err(StoreError::NotFound {
                kind: ItemKind::Snapshot,
                id: id.0,
            });
        }
        Ok(())
    }

    /// Applies `policy` to a document's snapshots; returns how many were deleted.
    pub fn prune_snapshots(
        &mut self,
        document: DocumentId,
        policy: &SnapshotPolicy,
    ) -> Result<usize> {
        let now = self.now();
        let tx = self.conn.transaction()?;
        let n = prune(&tx, document, policy, now)?;
        tx.commit()?;
        Ok(n)
    }

    /// Takes a snapshot when the document has none, or when its newest snapshot is at least
    /// `min_age` old and its content differs from the saved content. After taking one it
    /// prunes with [`Store::snapshot_policy`]. Returns the new snapshot, or `None` when none
    /// was due. `NotFound` when the document does not exist.
    pub fn snapshot_if_stale(
        &mut self,
        document: DocumentId,
        min_age: Duration,
    ) -> Result<Option<SnapshotMeta>> {
        let now = self.now();
        let policy = self.snapshot_policy.clone();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Only whether it exists: the metadata sits behind the text in the row.
        let exists: Option<i64> = tx
            .query_row("SELECT 1 FROM documents WHERE id = ?1", [document], |r| {
                r.get(0)
            })
            .optional()?;
        if exists.is_none() {
            return Err(StoreError::NotFound {
                kind: ItemKind::Document,
                id: document.0,
            });
        }
        // The age first: it is answered from an index. Comparing the content reads both
        // copies of the text, which is not something to do after every autosave.
        let last: Option<(SnapshotId, i64)> = tx
            .query_row(
                "SELECT id, created_at FROM snapshots
                 WHERE document_id = ?1 ORDER BY created_at DESC, id DESC LIMIT 1",
                [document],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((snapshot, created_at)) = last {
            if now.saturating_sub(created_at) < duration_ms(min_age) {
                return Ok(None);
            }
            let same: bool = tx.query_row(
                "SELECT s.content = d.content
                 FROM snapshots s JOIN documents d ON d.id = s.document_id
                 WHERE s.id = ?1",
                [snapshot],
                |r| r.get(0),
            )?;
            if same {
                return Ok(None);
            }
        }
        let meta = insert_snapshot(&tx, document, now)?;
        prune(&tx, document, &policy, now)?;
        tx.commit()?;
        Ok(Some(meta))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 60 * 60 * 1000;

    #[test]
    fn keeps_newest_and_recent() {
        let policy = SnapshotPolicy::default();
        let now = 1000 * DAY_MS;
        let snaps: Vec<(i64, i64)> = (0..10).map(|i| (i, now - i * HOUR)).collect();
        assert!(policy.select_deletions(now, &snaps).is_empty());
    }

    #[test]
    fn thins_to_daily_then_weekly() {
        let policy = SnapshotPolicy::default();
        let now = 1000 * DAY_MS;
        // One snapshot every 6 hours for 100 days, newest first.
        let snaps: Vec<(i64, i64)> = (0..400).map(|i| (i, now - i * 6 * HOUR)).collect();
        let doomed: HashSet<i64> = policy.select_deletions(now, &snaps).into_iter().collect();
        let kept: Vec<&(i64, i64)> = snaps.iter().filter(|s| !doomed.contains(&s.0)).collect();
        // 1 day of everything (4) + ~29 daily + ~10 weekly.
        assert!(kept.len() >= 38 && kept.len() <= 48, "kept {}", kept.len());
        // No two kept older-than-a-day snapshots share a day.
        let mut days = HashSet::new();
        for (_, t) in kept
            .iter()
            .filter(|(_, t)| now - t >= DAY_MS && now - t < 30 * DAY_MS)
        {
            assert!(days.insert(t.div_euclid(DAY_MS)));
        }
    }

    #[test]
    fn hard_cap_drops_oldest() {
        let policy = SnapshotPolicy {
            max_per_document: 3,
            ..Default::default()
        };
        let now = 1000 * DAY_MS;
        let snaps: Vec<(i64, i64)> = (0..10).map(|i| (i, now - i * 1000)).collect();
        let doomed = policy.select_deletions(now, &snaps);
        assert_eq!(doomed, vec![3, 4, 5, 6, 7, 8, 9]);
    }
}
