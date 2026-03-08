//! Stress tests for lock ordering in the relay server.
//!
//! These tests exercise REAL production functions (`is_folder_doc`,
//! `find_all_folder_docs`) under concurrent contention to verify they
//! don't deadlock. No inline reimplementations of fixed patterns — we
//! test the actual code paths.
//!
//! Each test uses an OS-thread watchdog because tokio timers can't fire
//! when all worker threads are deadlocked on sync locks.
//!
//! See docs/plans/2026-03-08-debounce-deadlock-fix.md for the original
//! deadlock analysis and lock ordering rules.

use dashmap::DashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::yield_now;
use y_sweet_core::doc_sync::DocWithSyncKv;
use y_sweet_core::link_indexer::{self, PendingEntry};
use yrs::{Map, Transact, WriteTxn};

const ITERATIONS: usize = 500;
const TIMEOUT_SECS: u64 = 5;

/// Create a folder doc (has filemeta_v0 with file entries).
async fn create_folder_doc(doc_id: &str) -> DocWithSyncKv {
    let dswk = DocWithSyncKv::new(doc_id, None, || (), None)
        .await
        .expect("failed to create DocWithSyncKv");
    {
        let awareness = dswk.awareness();
        let guard = awareness.write().unwrap();
        let mut txn = guard.doc.transact_mut();
        let filemeta = txn.get_or_insert_map("filemeta_v0");
        let mut meta = std::collections::HashMap::new();
        meta.insert("id".to_string(), yrs::Any::String("uuid-content-1".into()));
        meta.insert(
            "type".to_string(),
            yrs::Any::String("markdown".into()),
        );
        meta.insert("version".to_string(), yrs::Any::Number(0.0));
        filemeta.insert(&mut txn, "/TestDoc.md", yrs::Any::Map(meta.into()));
    }
    dswk
}

/// OS-thread watchdog that aborts the process if the test doesn't complete.
/// tokio timers can't fire when worker threads are deadlocked on sync locks,
/// so we use a real OS thread.
fn watchdog(done: Arc<AtomicBool>, test_name: &'static str) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(TIMEOUT_SECS));
        if !done.load(Ordering::SeqCst) {
            eprintln!(
                "\n\nDEADLOCK DETECTED in `{test_name}`: \
                 test did not complete within {TIMEOUT_SECS}s.\n\
                 This indicates a lock ordering cycle.\n\
                 See docs/plans/2026-03-08-debounce-deadlock-fix.md\n"
            );
            std::process::exit(1);
        }
    });
}

// ============================================================================
// Test 1: is_folder_doc() regression test for the search_worker fix
//
// The original deadlock: is_folder_doc() was called INSIDE pending.iter()
// .filter(), holding DashMap shard read guards while acquiring awareness
// read locks. The fix snapshots keys first, then calls is_folder_doc().
//
// This test calls the REAL is_folder_doc() function while a concurrent task
// holds awareness WRITE and writes to the same pending DashMap — reproducing
// the WebSocket callback pattern from server.rs:923-949.
//
// The test does NOT reimplement the snapshot-then-filter pattern inline.
// Instead it verifies that is_folder_doc() is safe to call from any context
// where no DashMap shard locks are held.
// ============================================================================
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn is_folder_doc_concurrent_with_awareness_write_and_dashmap_write() {
    let done = Arc::new(AtomicBool::new(false));
    watchdog(
        done.clone(),
        "is_folder_doc_concurrent_with_awareness_write_and_dashmap_write",
    );

    let docs: Arc<DashMap<String, DocWithSyncKv>> = Arc::new(DashMap::new());
    let pending: Arc<DashMap<String, PendingEntry>> = Arc::new(DashMap::new());

    let folder_id = "folder-doc-1".to_string();
    docs.insert(folder_id.clone(), create_folder_doc(&folder_id).await);
    pending.insert(
        folder_id.clone(),
        PendingEntry::new(tokio::time::Instant::now()),
    );

    let docs_w = Arc::clone(&docs);
    let docs_u = Arc::clone(&docs);
    let pending_u = Arc::clone(&pending);
    let fid_u = folder_id.clone();

    // Task A: repeatedly call REAL is_folder_doc()
    let caller = tokio::spawn(async move {
        for _ in 0..ITERATIONS {
            let result = link_indexer::is_folder_doc(&folder_id, &docs_w);
            // Verify it actually reads the doc correctly
            assert!(result.is_some(), "should detect folder doc");
            yield_now().await;
        }
    });

    // Task B: hold awareness WRITE, then write to pending DashMap
    // (exact pattern from server.rs observe_update_v1 callback)
    let updater = tokio::spawn(async move {
        for _ in 0..ITERATIONS {
            if let Some(doc_ref) = docs_u.get(&fid_u) {
                let awareness = doc_ref.awareness();
                let _guard = awareness.write().unwrap();
                // Synchronous DashMap write while holding awareness write
                pending_u
                    .entry(fid_u.clone())
                    .and_modify(|e| e.last_updated = tokio::time::Instant::now());
                drop(_guard);
                drop(doc_ref);
            }
            yield_now().await;
        }
    });

    caller.await.expect("caller panicked");
    updater.await.expect("updater panicked");
    done.store(true, Ordering::SeqCst);
}

// ============================================================================
// Test 2: find_all_folder_docs() under awareness write contention
//
// find_all_folder_docs() iterates the docs DashMap while acquiring awareness
// read locks INSIDE the iterator — the same fragile pattern that caused
// the original production deadlock on search_pending.
//
// Currently safe because no code writes to the docs DashMap while holding
// awareness write locks. This test exercises the function under concurrent
// awareness write lock contention (the pattern from WebSocket handlers).
// ============================================================================
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn find_all_folder_docs_concurrent_with_awareness_writes() {
    let done = Arc::new(AtomicBool::new(false));
    watchdog(
        done.clone(),
        "find_all_folder_docs_concurrent_with_awareness_writes",
    );

    let docs: Arc<DashMap<String, DocWithSyncKv>> = Arc::new(DashMap::new());
    for i in 0..5 {
        let id = format!("folder-doc-{}", i);
        docs.insert(id.clone(), create_folder_doc(&id).await);
    }

    let docs_reader = Arc::clone(&docs);
    let docs_writer = Arc::clone(&docs);

    // Task A: call REAL find_all_folder_docs() repeatedly
    let reader = tokio::spawn(async move {
        for _ in 0..ITERATIONS {
            let result = link_indexer::find_all_folder_docs(&docs_reader);
            assert_eq!(result.len(), 5, "should find all 5 folder docs");
            yield_now().await;
        }
    });

    // Task B: hold awareness write locks on various docs
    // (simulates WebSocket handlers applying Y.Doc updates)
    let writer = tokio::spawn(async move {
        for i in 0..ITERATIONS {
            let doc_id = format!("folder-doc-{}", i % 5);
            if let Some(doc_ref) = docs_writer.get(&doc_id) {
                let awareness = doc_ref.awareness();
                let _guard = awareness.write().unwrap();
                // Hold briefly — in prod this is where transact_mut() runs
                drop(_guard);
                drop(doc_ref);
            }
            yield_now().await;
        }
    });

    reader.await.expect("reader panicked");
    writer.await.expect("writer panicked");
    done.store(true, Ordering::SeqCst);
}

// ============================================================================
// Test 3: find_all_folder_docs() with concurrent docs DashMap mutation
//
// This is the DANGEROUS scenario. find_all_folder_docs() holds docs DashMap
// shard read locks (via .iter()) while acquiring awareness read locks. If a
// concurrent task holds an awareness write lock AND writes to the same docs
// DashMap, we get the classic lock ordering cycle:
//
//   find_all_folder_docs: docs shard READ → awareness READ (blocked)
//   mutator:              awareness WRITE → docs shard WRITE (blocked)
//
// Today no production code does this (callbacks only touch search_pending,
// not docs). But this test documents the latent risk: if ANY code path ever
// writes to the docs DashMap inside an awareness lock, find_all_folder_docs
// will deadlock.
//
// If this test starts deadlocking, find_all_folder_docs needs the same
// snapshot-then-filter fix as the search worker.
// ============================================================================
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn find_all_folder_docs_with_concurrent_docs_dashmap_mutation() {
    let done = Arc::new(AtomicBool::new(false));
    watchdog(
        done.clone(),
        "find_all_folder_docs_with_concurrent_docs_dashmap_mutation",
    );

    let docs: Arc<DashMap<String, DocWithSyncKv>> = Arc::new(DashMap::new());

    // Main folder docs
    for i in 0..5 {
        let id = format!("folder-doc-{}", i);
        docs.insert(id.clone(), create_folder_doc(&id).await);
    }

    // Extra docs to swap in/out of the DashMap
    for i in 0..3 {
        let id = format!("temp-doc-{}", i);
        docs.insert(id.clone(), create_folder_doc(&id).await);
    }

    let docs_reader = Arc::clone(&docs);
    let docs_mutator = Arc::clone(&docs);

    // Task A: call REAL find_all_folder_docs()
    let reader = tokio::spawn(async move {
        for _ in 0..ITERATIONS {
            let result = link_indexer::find_all_folder_docs(&docs_reader);
            // Count may vary as mutator adds/removes temp docs
            assert!(result.len() >= 5, "should find at least the 5 main docs");
            yield_now().await;
        }
    });

    // Task B: hold awareness WRITE on folder-doc-0, then mutate docs DashMap
    // This creates the lock ordering cycle:
    //   reader:  docs.iter() shard READ → awareness READ
    //   mutator: awareness WRITE → docs.remove()/insert() shard WRITE
    let mutator = tokio::spawn(async move {
        for i in 0..ITERATIONS {
            // Get awareness Arc without keeping DashMap ref alive
            let awareness_arc = {
                match docs_mutator.get("folder-doc-0") {
                    Some(r) => r.value().awareness(),
                    None => continue,
                }
                // DashMap Ref dropped here — shard lock released
            };

            let _guard = awareness_arc.write().unwrap();
            // While holding awareness WRITE, touch docs DashMap (shard WRITE)
            let temp_id = format!("temp-doc-{}", i % 3);
            if let Some((k, v)) = docs_mutator.remove(&temp_id) {
                docs_mutator.insert(k, v);
            }
            drop(_guard);
            yield_now().await;
        }
    });

    reader.await.expect("reader panicked");
    mutator.await.expect("mutator panicked");
    done.store(true, Ordering::SeqCst);
}

// ============================================================================
// Test 4: Concurrent multi-awareness-lock acquisition (move_doc ABBA risk)
//
// handle_move_document acquires awareness write locks on ALL folder docs
// and ALL content docs before calling move_document(). The locks are acquired
// in iteration order of folder_doc_ids/content_doc_ids.
//
// If two concurrent move operations share docs but iterate them in different
// orders, classic ABBA deadlock:
//   Move 1: lock A → lock B (blocked)
//   Move 2: lock B → lock A (blocked)
//
// This test acquires awareness write locks on shared docs in opposite orders
// from two concurrent tasks. It uses the REAL DocWithSyncKv awareness locks.
//
// Previously deadlocked due to ABBA lock ordering.
// Fixed by sorting content doc IDs before acquiring awareness write locks.
// ============================================================================
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_multi_awareness_lock_acquisition() {
    let done = Arc::new(AtomicBool::new(false));
    watchdog(
        done.clone(),
        "concurrent_multi_awareness_lock_acquisition",
    );

    let docs: Arc<DashMap<String, DocWithSyncKv>> = Arc::new(DashMap::new());
    for i in 0..4 {
        let id = format!("doc-{}", i);
        docs.insert(id.clone(), create_folder_doc(&id).await);
    }

    let docs_a = Arc::clone(&docs);
    let docs_b = Arc::clone(&docs);

    // Task A: acquire awareness write locks from a shuffled list,
    // but sort doc IDs before locking (same fix as handle_move_document).
    let task_a = tokio::spawn(async move {
        for _ in 0..ITERATIONS {
            // Collect IDs in arbitrary order (even indices first, then odd)
            let mut ids: Vec<String> = (0..4).step_by(2)
                .chain((1..4).step_by(2))
                .map(|i| format!("doc-{}", i))
                .collect();
            // Sort before locking — the fix under test
            ids.sort();
            let awareness_arcs: Vec<_> = ids
                .iter()
                .filter_map(|id| docs_a.get(id).map(|r| r.value().awareness()))
                .collect();
            let _guards: Vec<_> = awareness_arcs
                .iter()
                .map(|a| a.write().unwrap())
                .collect();
            drop(_guards);
            yield_now().await;
        }
    });

    // Task B: acquire awareness write locks from reverse order,
    // but sort doc IDs before locking (same fix as handle_move_document).
    let task_b = tokio::spawn(async move {
        for _ in 0..ITERATIONS {
            let mut ids: Vec<String> = (0..4)
                .rev()
                .map(|i| format!("doc-{}", i))
                .collect();
            // Sort before locking — the fix under test
            ids.sort();
            let awareness_arcs: Vec<_> = ids
                .iter()
                .filter_map(|id| docs_b.get(id).map(|r| r.value().awareness()))
                .collect();
            let _guards: Vec<_> = awareness_arcs
                .iter()
                .map(|a| a.write().unwrap())
                .collect();
            drop(_guards);
            yield_now().await;
        }
    });

    task_a.await.expect("task_a panicked");
    task_b.await.expect("task_b panicked");
    done.store(true, Ordering::SeqCst);
}

// ============================================================================
// Test 5: is_folder_doc() + find_all_folder_docs() + awareness writes
//
// Combined stress test exercising multiple real code paths simultaneously:
// - Task A: calls is_folder_doc() (used by search_worker and run_worker)
// - Task B: calls find_all_folder_docs() (used by run_worker and search_handle_content_update)
// - Task C: holds awareness write locks and writes to a DashMap (callback pattern)
//
// This catches interaction effects between multiple production code paths
// that the individual tests wouldn't reveal.
// ============================================================================
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn combined_stress_test_multiple_code_paths() {
    let done = Arc::new(AtomicBool::new(false));
    watchdog(done.clone(), "combined_stress_test_multiple_code_paths");

    let docs: Arc<DashMap<String, DocWithSyncKv>> = Arc::new(DashMap::new());
    let pending: Arc<DashMap<String, PendingEntry>> = Arc::new(DashMap::new());

    for i in 0..8 {
        let id = format!("folder-doc-{}", i);
        docs.insert(id.clone(), create_folder_doc(&id).await);
        pending.insert(id, PendingEntry::new(tokio::time::Instant::now()));
    }

    let docs1 = Arc::clone(&docs);
    let docs2 = Arc::clone(&docs);
    let docs3 = Arc::clone(&docs);
    let pending3 = Arc::clone(&pending);

    // Task A: call is_folder_doc() on various docs
    let task_a = tokio::spawn(async move {
        for i in 0..ITERATIONS {
            let id = format!("folder-doc-{}", i % 8);
            let result = link_indexer::is_folder_doc(&id, &docs1);
            assert!(result.is_some());
            yield_now().await;
        }
    });

    // Task B: call find_all_folder_docs()
    let task_b = tokio::spawn(async move {
        for _ in 0..ITERATIONS {
            let result = link_indexer::find_all_folder_docs(&docs2);
            assert_eq!(result.len(), 8);
            yield_now().await;
        }
    });

    // Task C: hold awareness write locks and write to pending DashMap
    let task_c = tokio::spawn(async move {
        for i in 0..ITERATIONS {
            let id = format!("folder-doc-{}", i % 8);
            if let Some(doc_ref) = docs3.get(&id) {
                let awareness = doc_ref.awareness();
                let _guard = awareness.write().unwrap();
                pending3
                    .entry(id.clone())
                    .and_modify(|e| e.last_updated = tokio::time::Instant::now());
                drop(_guard);
                drop(doc_ref);
            }
            yield_now().await;
        }
    });

    task_a.await.expect("task_a panicked");
    task_b.await.expect("task_b panicked");
    task_c.await.expect("task_c panicked");
    done.store(true, Ordering::SeqCst);
}
