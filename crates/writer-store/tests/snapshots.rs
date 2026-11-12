mod common;

use std::time::Duration;

use common::*;
use writer_store::{SnapshotId, SnapshotPolicy, StoreError};

#[test]
fn create_list_load_delete() {
    let (mut store, clock) = store();
    let d = doc(&mut store, None, "Draft", "one two");
    let s1 = store.create_snapshot(d.id).unwrap();
    assert_eq!(s1.document_id, d.id);
    assert_eq!(s1.word_count, 2);
    assert_eq!(s1.title, "Draft");
    clock.advance_ms(MINUTE);
    store
        .save_document(d.id, &content("Draft v2", "one two three", "{\"v\":2}"))
        .unwrap();
    let s2 = store.create_snapshot(d.id).unwrap();
    let list = store.snapshots(d.id).unwrap();
    assert_eq!(
        list.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![s2.id, s1.id]
    );
    let loaded = store.snapshot(s1.id).unwrap();
    assert_eq!(loaded.content, store_content_v1());
    assert_eq!(store.snapshot(s2.id).unwrap().content, "{\"v\":2}");
    store.delete_snapshot(s1.id).unwrap();
    assert_eq!(store.snapshots(d.id).unwrap().len(), 1);
    assert!(matches!(
        store.snapshot(SnapshotId(999)),
        Err(StoreError::NotFound { .. })
    ));
    assert!(matches!(
        store.delete_snapshot(s1.id),
        Err(StoreError::NotFound { .. })
    ));
}

fn store_content_v1() -> String {
    format!("{{\"text\":{:?}}}", "one two")
}

#[test]
fn snapshot_if_stale_rules() {
    let (mut store, clock) = store();
    let d = doc(&mut store, None, "Draft", "one");
    let ten = Duration::from_secs(600);
    // First call always snapshots.
    assert!(store.snapshot_if_stale(d.id, ten).unwrap().is_some());
    // Too recent.
    store
        .save_document(d.id, &content("Draft", "two", "{\"v\":2}"))
        .unwrap();
    clock.advance_ms(5 * MINUTE);
    assert!(store.snapshot_if_stale(d.id, ten).unwrap().is_none());
    // Old enough and different.
    clock.advance_ms(6 * MINUTE);
    let s = store.snapshot_if_stale(d.id, ten).unwrap().unwrap();
    assert_eq!(store.snapshot(s.id).unwrap().content, "{\"v\":2}");
    // Old enough but identical.
    clock.advance_ms(60 * MINUTE);
    assert!(store.snapshot_if_stale(d.id, ten).unwrap().is_none());
    assert_eq!(store.snapshots(d.id).unwrap().len(), 2);
    assert!(matches!(
        store.snapshot_if_stale(writer_store::DocumentId(404), ten),
        Err(StoreError::NotFound { .. })
    ));
}

#[test]
fn pruning_bounds_growth() {
    let (mut store, clock) = store();
    let d = doc(&mut store, None, "Draft", "x");
    // A snapshot every 3 hours for 120 days.
    for i in 0..(120 * 8) {
        store
            .save_document(d.id, &content("Draft", "x", &format!("{{\"v\":{i}}}")))
            .unwrap();
        store.create_snapshot(d.id).unwrap();
        clock.advance_ms(3 * HOUR);
    }
    assert_eq!(store.snapshots(d.id).unwrap().len(), 960);
    let deleted = store
        .prune_snapshots(d.id, &SnapshotPolicy::default())
        .unwrap();
    let kept = store.snapshots(d.id).unwrap();
    assert_eq!(deleted + kept.len(), 960);
    // ~8 from the last day, ~29 daily, ~13 weekly.
    assert!((40..=60).contains(&kept.len()), "kept {}", kept.len());
    // The newest one survives.
    assert_eq!(store.snapshot(kept[0].id).unwrap().content, "{\"v\":959}");
    // Pruning is idempotent.
    assert_eq!(
        store
            .prune_snapshots(d.id, &SnapshotPolicy::default())
            .unwrap(),
        0
    );

    let capped = SnapshotPolicy {
        max_per_document: 10,
        ..Default::default()
    };
    store.prune_snapshots(d.id, &capped).unwrap();
    assert_eq!(store.snapshots(d.id).unwrap().len(), 10);
}

#[test]
fn snapshot_if_stale_prunes_with_store_policy() {
    let (mut store, clock) = store();
    store.set_snapshot_policy(SnapshotPolicy {
        max_per_document: 3,
        ..Default::default()
    });
    let d = doc(&mut store, None, "Draft", "x");
    for i in 0..10 {
        store
            .save_document(d.id, &content("Draft", "x", &format!("{{\"v\":{i}}}")))
            .unwrap();
        store
            .snapshot_if_stale(d.id, Duration::from_secs(60))
            .unwrap()
            .unwrap();
        clock.advance_ms(2 * MINUTE);
    }
    assert_eq!(store.snapshots(d.id).unwrap().len(), 3);
}

#[test]
fn labels_say_why_a_snapshot_was_taken() {
    let (mut store, clock) = store();
    let d = doc(&mut store, None, "Draft", "one two");
    let plain = store.create_snapshot(d.id).unwrap();
    clock.advance_ms(MINUTE);
    let labelled = store
        .create_labeled_snapshot(d.id, "  Before an edit by Claude ")
        .unwrap();
    assert_eq!(labelled.document_id, d.id);
    assert_eq!(store.snapshot_label(plain.id).unwrap(), None);
    assert_eq!(
        store.snapshot_label(labelled.id).unwrap().as_deref(),
        Some("Before an edit by Claude")
    );
    let labels = store.snapshot_labels(d.id).unwrap();
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[&labelled.id], "Before an edit by Claude");
    // The snapshot itself is an ordinary one: listed, loaded and restored as any other.
    assert_eq!(store.snapshots(d.id).unwrap().len(), 2);
    assert_eq!(store.snapshot(labelled.id).unwrap().meta, labelled);
    // Another document's labels are its own.
    let other = doc(&mut store, None, "Other", "x");
    assert!(store.snapshot_labels(other.id).unwrap().is_empty());
    // An empty label is no label.
    clock.advance_ms(MINUTE);
    let bare = store.create_labeled_snapshot(d.id, "  ").unwrap();
    assert_eq!(store.snapshot_label(bare.id).unwrap(), None);
    assert_eq!(store.snapshot_label(SnapshotId(999)).unwrap(), None);

    // A snapshot that is there already is given a label afterwards, another one in its
    // place, or none again; nothing else about it changes.
    store
        .label_snapshot(plain.id, " Before an edit by Codex ")
        .unwrap();
    assert_eq!(
        store.snapshot_label(plain.id).unwrap().as_deref(),
        Some("Before an edit by Codex")
    );
    assert_eq!(store.snapshot(plain.id).unwrap().meta, plain);
    assert_eq!(store.snapshot_labels(d.id).unwrap().len(), 2);
    store.label_snapshot(plain.id, "Before a rewrite").unwrap();
    assert_eq!(
        store.snapshot_labels(d.id).unwrap()[&plain.id],
        "Before a rewrite"
    );
    store.label_snapshot(plain.id, "").unwrap();
    assert_eq!(store.snapshot_label(plain.id).unwrap(), None);
    assert_eq!(store.snapshots(d.id).unwrap().len(), 3);
    assert!(matches!(
        store.label_snapshot(SnapshotId(999), "x"),
        Err(StoreError::NotFound { .. })
    ));
    assert_eq!(store.snapshot_label(SnapshotId(999)).unwrap(), None);
}

#[test]
fn a_label_does_not_pass_to_a_later_snapshot_with_the_same_id() {
    let (mut store, clock) = store();
    let d = doc(&mut store, None, "Draft", "one two");
    let first = store
        .create_labeled_snapshot(d.id, "Before an edit")
        .unwrap();
    store.delete_snapshot(first.id).unwrap();
    // The label is left behind, and never shown.
    assert_eq!(store.snapshot_label(first.id).unwrap(), None);
    clock.advance_ms(MINUTE);
    // SQLite hands the id of the deleted row out again.
    let second = store.create_snapshot(d.id).unwrap();
    assert_eq!(second.id, first.id);
    assert_eq!(store.snapshot_label(second.id).unwrap(), None);
    assert!(store.snapshot_labels(d.id).unwrap().is_empty());

    // Housekeeping removes what belongs to no snapshot, and nothing else.
    let kept = store.create_labeled_snapshot(d.id, "Kept").unwrap();
    let gone = store.create_labeled_snapshot(d.id, "Gone").unwrap();
    store
        .set_setting("snapshot.labels_are_not_this", &true)
        .unwrap();
    store.set_setting("theme", "dark").unwrap();
    store.delete_snapshot(gone.id).unwrap();
    assert_eq!(store.remove_stale_snapshot_labels().unwrap(), 1);
    assert_eq!(
        store.snapshot_label(kept.id).unwrap().as_deref(),
        Some("Kept")
    );
    assert_eq!(
        store.setting::<String>("theme").unwrap().as_deref(),
        Some("dark")
    );
    assert_eq!(
        store
            .setting::<bool>("snapshot.labels_are_not_this")
            .unwrap(),
        Some(true)
    );
    assert_eq!(store.remove_stale_snapshot_labels().unwrap(), 0);
    // Deleting the document takes its snapshots along; their labels go with the next
    // housekeeping.
    store.trash_document(d.id).unwrap();
    store.purge_document(d.id).unwrap();
    assert_eq!(store.remove_stale_snapshot_labels().unwrap(), 2);
}
