# Debounce Rewrite Deadlock Fix

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix the tokio runtime deadlock caused by the debounce rewrite, add a stress test to prevent regression, and document lock ordering rules.

**Architecture:** The deadlock is a lock ordering cycle between DashMap shard locks (held during `pending.iter()`) and `std::sync::RwLock` awareness locks (acquired inside `is_folder_doc()`). The fix separates DashMap iteration from awareness lock acquisition by collecting keys first, dropping the iterator, then filtering. A stress test reproduces the deadlock scenario with 2 tokio worker threads.

**Tech Stack:** Rust, tokio (multi-thread runtime), DashMap, yrs (Y.Doc/Awareness with std::sync::RwLock)

---

## Context: Root Cause Analysis

### The deadlock cycle

Two lock types participate:
- **DashMap shard locks** — internal to `search_pending` (and link indexer's `self.pending`), acquired during `.iter()`, `.entry()`, etc.
- **Awareness RwLock** — `std::sync::RwLock` inside each `DocWithSyncKv`, acquired via `docs.get(id) → awareness().read()/write()`

**Thread A — search worker** (collecting ready items):
```
search_pending.iter()            → holds READ lock on shard S
  → filter calls is_folder_doc()
    → docs.get(doc_id)
      → awareness.read()         → BLOCKED (Thread B has WRITE)
```

**Thread B — WebSocket handler** (applying client Y.Doc update):
```
awareness.write()                → holds WRITE lock on awareness
  → Y.Doc transact_mut commits
    → observe_update_v1 callback fires SYNCHRONOUSLY
      → search_pending.entry()   → BLOCKED (Thread A has READ on shard S)
```

Same doc_id → same DashMap shard → deadlock. With only 2 tokio worker threads, the entire runtime freezes.

### Why the old code didn't deadlock

The old search worker called `is_folder_doc()` after `rx.recv().await` — no DashMap locks were held. The new poll-based loop calls `is_folder_doc()` inside `pending.iter().filter()`, which holds shard read guards.

### Why parking_lot deadlock detection won't help

DashMap 6.x uses `parking_lot_core` internally, but awareness uses `std::sync::RwLock`. The `parking_lot` deadlock detector only tracks its own locks — it can't see the cross-type cycle.

---

## Deferred work

The following items are documented here but **not part of this plan** — they will be done in a follow-up:

1. **Task: Fix the root cause** — Separate DashMap iteration from `is_folder_doc()` calls in both `search_worker` (`crates/relay/src/server.rs:240-247`) and `LinkIndexer::run_worker` (`crates/y-sweet-core/src/link_indexer.rs:1392-1400`). The fix: collect keys from `pending.iter()` first (no external locks), drop the iterator, then check each key against `is_folder_doc()`.

2. **Task: Explore parking_lot deadlock detection for awareness locks** — Investigate switching awareness from `std::sync::RwLock` to `parking_lot::RwLock` so the deadlock detector can see both sides of the cycle. This is a deeper change in y-sweet-core's `doc_sync.rs` / `sync/awareness.rs` (or upstream yrs).

---

## Task 1: Add lock ordering documentation

**Files:**
- Modify: `crates/relay/src/server.rs:239` (comment before search worker step 3)
- Modify: `crates/y-sweet-core/src/link_indexer.rs:1391` (comment before link indexer step 3)

**Step 1: Add lock ordering warning to search_worker**

In `crates/relay/src/server.rs`, add a comment block before line 240 (`let ready: Vec<String> = pending`):

```rust
        // ⚠️ LOCK ORDERING RULE: DashMap shard locks < awareness RwLock
        //
        // DashMap::iter() holds read guards on shards as it iterates.
        // is_folder_doc() acquires awareness read locks via docs.get() → awareness().read().
        // WebSocket handlers hold awareness WRITE locks and synchronously write to
        // search_pending in the update callback (server.rs:908-927).
        //
        // Calling is_folder_doc() inside pending.iter().filter() creates a lock ordering
        // cycle: worker holds shard READ, needs awareness READ; handler holds awareness
        // WRITE, needs shard WRITE. This deadlocks with 2 tokio worker threads.
        //
        // FIX REQUIRED: Collect keys from pending.iter() first (no external locks),
        // drop the iterator, THEN check is_folder_doc() on each key.
        // See docs/plans/2026-03-08-debounce-deadlock-fix.md for full analysis.
```

**Step 2: Add the same warning to link_indexer run_worker**

In `crates/y-sweet-core/src/link_indexer.rs`, add a comment block before line 1392 (`let ready: Vec<String> = self`):

```rust
            // ⚠️ LOCK ORDERING RULE: DashMap shard locks < awareness RwLock
            //
            // Same hazard as search_worker in server.rs — see that comment for details.
            // Currently less likely to deadlock because the WebSocket callback uses
            // tokio::spawn for link indexer notifications (not synchronous), but the
            // pattern is still fragile. Fix together with search_worker.
```

**Step 3: Add lock ordering warning to the callback registration**

In `crates/relay/src/server.rs`, add a comment before line 908 (`// Notify search index worker`):

```rust
                            // ⚠️ This runs synchronously inside an awareness write lock.
                            // Any lock acquired here must be LOWER than awareness in the
                            // lock ordering. DashMap shard locks (via .entry()) are lower,
                            // so this is safe — but code iterating this DashMap must NOT
                            // hold shard locks while acquiring awareness locks.
```

**Step 4: Commit**

```
jj describe -m "doc: add lock ordering warnings to search/indexer workers

Document the DashMap shard ↔ awareness RwLock deadlock cycle.
Mark the pending.iter().filter(is_folder_doc) pattern as known-broken."
```

---

## Task 2: Add stress test for search worker deadlock

This test reproduces the exact deadlock scenario: a search worker iterating `search_pending` while concurrent tasks update docs (triggering callbacks that write to `search_pending`).

**Files:**
- Create: `crates/relay/tests/search_deadlock.rs`

**Step 1: Write the stress test**

```rust
//! Stress test: search worker must not deadlock with concurrent doc updates.
//!
//! Reproduces the lock ordering cycle:
//! - Search worker: search_pending.iter() → is_folder_doc() → awareness.read()
//! - Doc update callback: awareness.write() → search_pending.entry()
//!
//! With worker_threads = 2, this reliably deadlocks on the unfixed code.

use dashmap::DashMap;
use std::sync::Arc;
use std::time::Duration;
use yrs::{Doc, Map, Text, Transact, WriteTxn};

/// Minimal reproduction: iterate a DashMap while calling is_folder_doc,
/// concurrently with tasks that hold awareness write locks and write to
/// the same DashMap via a synchronous callback.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_worker_does_not_deadlock_with_concurrent_updates() {
    use y_sweet_core::doc_sync::DocWithSyncKv;
    use y_sweet_core::link_indexer::{self, PendingEntry};

    let docs: Arc<DashMap<String, DocWithSyncKv>> = Arc::new(DashMap::new());
    let search_pending: Arc<DashMap<String, PendingEntry>> = Arc::new(DashMap::new());

    let relay_id = "cb696037-0f72-4e93-8717-4e433129d789";
    let folder_uuid = "b0000001-0000-4000-8000-000000000001";
    let folder_doc_id = format!("{}-{}", relay_id, folder_uuid);

    // Create a folder doc with filemeta
    let folder_dswk =
        DocWithSyncKv::new(&folder_doc_id, None, || (), None)
            .await
            .unwrap();
    {
        let awareness = folder_dswk.awareness();
        let guard = awareness.write().unwrap();
        let mut txn = guard.doc.transact_mut();
        let filemeta = txn.get_or_insert_map("filemeta_v0");
        let mut meta = std::collections::HashMap::new();
        meta.insert("id".to_string(), yrs::Any::String("uuid-content-1".into()));
        meta.insert("type".to_string(), yrs::Any::String("markdown".into()));
        meta.insert("version".to_string(), yrs::Any::Number(0.0));
        filemeta.insert(
            &mut txn,
            "/TestDoc.md",
            yrs::Any::Map(meta.into()),
        );
    }
    docs.insert(folder_doc_id.clone(), folder_dswk);

    // Create a few content docs
    for i in 0..5 {
        let content_uuid = format!("a000000{}-0000-4000-8000-000000000001", i);
        let content_doc_id = format!("{}-{}", relay_id, content_uuid);
        let content_dswk =
            DocWithSyncKv::new(&content_doc_id, None, || (), None)
                .await
                .unwrap();
        {
            let awareness = content_dswk.awareness();
            let guard = awareness.write().unwrap();
            let mut txn = guard.doc.transact_mut();
            let text = txn.get_or_insert_text("contents");
            text.insert(&mut txn, 0, &format!("Content of doc {}", i));
        }
        docs.insert(content_doc_id.clone(), content_dswk);

        // Pre-populate search_pending so the worker has items to iterate
        search_pending.insert(
            content_doc_id,
            PendingEntry::new(tokio::time::Instant::now()),
        );
    }

    let docs_for_worker = docs.clone();
    let pending_for_worker = search_pending.clone();
    let docs_for_updater = docs.clone();
    let pending_for_updater = search_pending.clone();
    let folder_id_for_updater = folder_doc_id.clone();

    // This is the core of the test: run both patterns concurrently with a timeout.
    // If the code deadlocks, the timeout fires and the test fails.
    let result = tokio::time::timeout(Duration::from_secs(5), async move {
        let worker_handle = tokio::spawn(async move {
            // Simulate search worker: iterate pending, call is_folder_doc for each
            for _ in 0..200 {
                let ready: Vec<String> = pending_for_worker
                    .iter()
                    .filter(|e| {
                        let _is_folder =
                            link_indexer::is_folder_doc(e.key(), &docs_for_worker).is_some();
                        true // don't actually remove, keep iterating
                    })
                    .map(|e| e.key().clone())
                    .collect();
                // Brief yield to let updater tasks run
                tokio::task::yield_now().await;
                // Re-populate pending for next iteration
                for key in &ready {
                    pending_for_worker.entry(key.clone()).or_insert_with(|| {
                        PendingEntry::new(tokio::time::Instant::now())
                    });
                }
            }
        });

        let updater_handle = tokio::spawn(async move {
            // Simulate WebSocket handler: hold awareness write lock, write to search_pending
            for _ in 0..200 {
                // Acquire awareness write lock (like a WebSocket update would)
                if let Some(doc_ref) = docs_for_updater.get(&folder_id_for_updater) {
                    let awareness = doc_ref.awareness();
                    let guard = awareness.write().unwrap();
                    // Simulate the callback: write to search_pending while holding awareness write
                    let now = tokio::time::Instant::now();
                    pending_for_updater
                        .entry(folder_id_for_updater.clone())
                        .and_modify(|e| e.last_updated = now)
                        .or_insert_with(|| PendingEntry::new(now));
                    drop(guard);
                }
                tokio::task::yield_now().await;
            }
        });

        worker_handle.await.unwrap();
        updater_handle.await.unwrap();
    })
    .await;

    assert!(
        result.is_ok(),
        "Deadlock detected: search worker and doc updater blocked for >5 seconds. \
         This indicates a lock ordering cycle between search_pending DashMap shards \
         and awareness RwLock. See docs/plans/2026-03-08-debounce-deadlock-fix.md"
    );
}
```

**Step 2: Run the test to verify it detects the deadlock**

```bash
CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo test --manifest-path=crates/Cargo.toml -p relay --test search_deadlock -- --nocapture
```

Expected: The test should **hang and then fail** with the timeout message (proving the deadlock exists). With only 2 worker threads and 200 iterations, the cycle should trigger quickly.

If the test passes (no deadlock triggered), increase iterations to 1000 or add more concurrent updater tasks.

**Step 3: Commit**

```
jj describe -m "test: add stress test that reproduces search worker deadlock

Runs search_pending.iter() + is_folder_doc() concurrently with
awareness.write() + search_pending.entry() on 2 tokio worker threads.
Currently expected to FAIL (deadlock detected) — will pass after the fix."
```

---

## Task 3: Verify the stress test catches the deadlock

**Step 1: Run the test and confirm it fails (deadlocks)**

```bash
CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo test --manifest-path=crates/Cargo.toml -p relay --test search_deadlock -- --nocapture 2>&1 | head -30
```

Expected output should include the timeout/deadlock assertion failure, OR the test hangs for 5 seconds then fails.

**Step 2: If the test passes (doesn't deadlock)**

The test may need tuning. Options:
- Increase iterations from 200 to 2000
- Add more updater tasks (3-4 concurrent)
- Add more content docs to increase DashMap shard contention
- Remove `yield_now()` from the worker loop to increase contention

Adjust and re-run until the test reliably fails on the current code.

**Step 3: Once the test reliably fails, commit the final version**

```
jj describe -m "test: stress test reliably reproduces search worker deadlock"
```

---

## Verification

After all tasks are complete:

1. The stress test reliably **fails** on the current code (proving it catches the deadlock)
2. Lock ordering comments are in place at all three locations
3. This plan document serves as the reference for the follow-up fix

When the fix (deferred Task 1) is implemented later, the stress test should **pass** — confirming the deadlock is resolved.
