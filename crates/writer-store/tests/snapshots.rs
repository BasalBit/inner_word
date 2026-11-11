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
