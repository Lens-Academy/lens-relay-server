# Crash Vector Fixes Implementation Plan

**Goal:** Eliminate the three most critical crash vectors found in the relay server's fork-specific code: a panicking dirty callback, RwLock poison cascades, and a non-functional search debounce.

**Architecture:** Three independent fixes. Task 1 is a 1-line fix. Task 2 is a mechanical replacement across 8 files (25 sites). Task 3 is a structural change to the search worker's debounce plumbing.

**Tech Stack:** Rust, yrs/y-sweet-core, tokio, DashMap

**Scope note:** `doc_connection.rs` has 9 awareness lock `.unwrap()` sites, but those are upstream code. We intentionally skip them to avoid merge conflicts on future upstream syncs. If an awareness lock gets poisoned, those upstream paths will still panic — but with Tasks 1 and 2 eliminating the poison sources, this should not happen in practice.

---

## Task 1: Fix `try_send(()).unwrap()` in dirty callback

**Files:**
- Modify: `crates/relay/src/server.rs:787`

**Context:**

The dirty callback signals the persistence worker that a doc has changed. It uses `send.try_send(()).unwrap()` which panics if the channel (capacity 1024) is full. Since this runs inside a Yrs `observe_update_v1` callback, a panic here poisons the document's `RwLock<Awareness>`, making the document permanently inaccessible.

The channel carries `()` — it's a "dirty" signal, not a queued message. If the channel is full, the persistence worker already has >1024 pending wakeups and will persist on its next iteration regardless. Dropping the signal is safe.

**Step 1: Replace unwrap with silent drop**

```rust
// BEFORE (server.rs:787):
send.try_send(()).unwrap();

// AFTER:
let _ = send.try_send(());
```

**Step 2: Verify compilation**

Run: `cargo check --manifest-path=crates/Cargo.toml -p relay`
Expected: compiles cleanly

**Step 3: Commit**

```
jj describe -m "fix(server): don't panic dirty callback when persistence channel is full" && jj new
```

---

## Task 2: Replace `.unwrap()` on RwLock guards with poison-clearing variants

**Files:**
- Modify: `crates/y-sweet-core/src/link_indexer.rs` (8 sites)
- Modify: `crates/y-sweet-core/src/doc_resolver.rs` (2 sites)
- Modify: `crates/y-sweet-core/src/doc_sync.rs` (4 sites)
- Modify: `crates/relay/src/server.rs` (5 sites)
- Modify: `crates/relay/src/mcp/tools/get_links.rs` (2 sites)
- Modify: `crates/relay/src/mcp/tools/grep.rs` (1 site)
- Modify: `crates/relay/src/mcp/tools/read.rs` (1 site)
- Modify: `crates/relay/src/mcp/tools/edit.rs` (2 sites)

**Context:**

`std::sync::RwLock` becomes "poisoned" when a thread panics while holding the lock. Every subsequent `.unwrap()` on that lock panics too, creating a cascade. Since all these locks guard `Awareness` objects (containing Y.Docs), a single panic makes the affected document permanently broken until server restart.

**Strategy:**

- **RwLock `.read().unwrap()` and `.write().unwrap()`** → `.read().unwrap_or_else(|e| e.into_inner())` / `.write().unwrap_or_else(|e| e.into_inner())`. This clears the poison and recovers the data. The Y.Doc inside is still valid — the panic that poisoned the lock was unrelated to data integrity.

- **`doc_sync.rs` lines 54, 57** — these are `.unwrap()` on `Result` from `push_update`/`flush_doc_with`, not RwLock. Replace with `if let Err(e)` + `tracing::error!` since these run inside a Yrs observer callback where panicking poisons the awareness lock. Note: if `push_update` fails, the update is lost from SyncKv persistence. The Y.Doc in memory still has the update applied (yrs applies it before the observer fires), and the next `persist()` call serializes the full doc state, so it will catch up.

**Step 1: Fix `doc_sync.rs` observer callback (lines 54-57)**

```rust
// BEFORE:
sync_kv.push_update(DOC_NAME, &event.update).unwrap();
sync_kv
    .flush_doc_with(DOC_NAME, Default::default())
    .unwrap();

// AFTER:
if let Err(e) = sync_kv.push_update(DOC_NAME, &event.update) {
    tracing::error!("Failed to push update for {}: {:?}", doc_key, e);
    return;
}
if let Err(e) = sync_kv.flush_doc_with(DOC_NAME, Default::default()) {
    tracing::error!("Failed to flush doc {}: {:?}", doc_key, e);
    return;
}
```

Note: The `doc_key` variable is already in scope from line 52 (`let doc_key = key.to_string();`).

**Step 2: Fix `doc_sync.rs` RwLock unwraps (lines 85, 94)**

```rust
// Line 85 (as_update):
let awareness_guard = self.awareness.read().unwrap_or_else(|e| e.into_inner());

// Line 94 (apply_update):
let awareness_guard = self.awareness.write().unwrap_or_else(|e| e.into_inner());
```

**Step 3: Fix `link_indexer.rs` RwLock unwraps (8 sites)**

All awareness lock sites in link_indexer.rs — replace `.unwrap()` with `.unwrap_or_else(|e| e.into_inner())`:

| Line | Context | Lock type |
|------|---------|-----------|
| 161 | `find_all_folder_docs` | `.read()` |
| 180 | `is_folder_doc` | `.read()` |
| 514 | `apply_rename_updates` (detect renames) | `.read()` |
| 541 | `apply_rename_updates` (read backlinks) | `.read()` |
| 575 | `apply_rename_updates` (update wikilinks) | `.write()` |
| 720 | `index_document` (content guard) | `.read()` |
| 732 | `index_document` (folder guards) | `.write()` |
| 776 | `reindex_all_backlinks` (seed cache) | `.read()` |

**Step 4: Fix `doc_resolver.rs` RwLock unwraps (2 sites)**

| Line | Context | Lock type |
|------|---------|-----------|
| 62 | `rebuild` | `.read()` |
| 154 | `update_folder` | `.read()` |

**Step 5: Fix `server.rs` RwLock unwraps (5 sites)**

| Line | Context | Lock type |
|------|---------|-----------|
| 264 | `search_handle_content_update` | `.read()` |
| 293 | `search_find_title_and_folder` | `.read()` |
| 342 | `search_handle_folder_update` | `.read()` |
| 935 | `startup_reindex` (folder docs) | `.read()` |
| 977 | `startup_reindex` (content docs) | `.read()` |

**Step 6: Fix MCP tool RwLock unwraps (6 sites)**

| File | Line | Context | Lock type |
|------|------|---------|-----------|
| `crates/relay/src/mcp/tools/get_links.rs` | 57 | `read_backlinks` | `.read()` |
| `crates/relay/src/mcp/tools/get_links.rs` | 84 | `read_forward_links` | `.read()` |
| `crates/relay/src/mcp/tools/grep.rs` | 188 | `read_doc_content` | `.read()` |
| `crates/relay/src/mcp/tools/read.rs` | 37 | `execute` | `.read()` |
| `crates/relay/src/mcp/tools/edit.rs` | 59 | `execute` (reading) | `.read()` |
| `crates/relay/src/mcp/tools/edit.rs` | 109 | `execute` (writing) | `.write()` |

**Step 7: Verify compilation and tests**

Run: `cargo check --manifest-path=crates/Cargo.toml -p relay`
Run: `cargo test --manifest-path=crates/Cargo.toml -p y-sweet-core`
Expected: compiles cleanly, all 218 tests pass

**Step 8: Commit**

```
jj describe -m "fix: clear poisoned RwLock guards instead of cascading panics

Replace .unwrap() with .unwrap_or_else(|e| e.into_inner()) on all
RwLock guards in fork-specific code (25 sites across 7 files).
Replace .unwrap() in doc_sync observer callback with error logging
to prevent panicking inside Yrs observer." && jj new
```

---

## Task 3: Fix search worker debounce (broken — no deduplication)

**Files:**
- Modify: `crates/relay/src/server.rs` (search worker spawn site + callback closures + struct field)

**Context:**

The search worker has a debounce mechanism modeled on the LinkIndexer, but it's broken:

1. **The `pending` DashMap is local to the worker** — created at line 539 and passed only to `search_worker()`. The callback closures (lines 744-748, 770-772) just do `tx.try_send(doc_id)` on every update with no deduplication.

2. **The debounce loop checks an empty map** — the worker's debounce at line 211 does `pending.get(&doc_id)` which always returns `None` (nothing writes to it from the callback), so it breaks immediately after one 2-second sleep. No actual coalescing happens.

3. **Channel floods under load** — without deduplication, a user typing rapidly sends hundreds of messages to the bounded(1000) channel. The worker processes one every ~2s. With 7+ active editors, the channel fills up and `try_send` starts returning errors, permanently dropping search updates.

**The LinkIndexer's working pattern** (for reference):
- `on_document_update()`: uses `pending.entry()` to atomically insert timestamp, only sends to channel on `Entry::Vacant` (first occurrence)
- Worker: sleeps, checks `pending.get(&doc_id).elapsed() >= DEBOUNCE_DURATION`, loops until ready

**Fix approach:** Share the `search_pending` DashMap between the callback and the worker. In the callback, use the entry() API to only send to the channel on first occurrence (matching LinkIndexer). The worker's debounce loop already checks `pending` correctly — once it's populated, it'll work.

### Step 1: Add `search_pending` field to Server struct

```rust
// In the Server struct (around line 434):
search_tx: Option<tokio::sync::mpsc::Sender<String>>,
search_pending: Option<Arc<DashMap<String, tokio::time::Instant>>>,
```

### Step 2: Store `search_pending` at construction (around lines 539-563)

Clone the Arc **before** the `tokio::spawn` moves it into the worker task:

```rust
// BEFORE (lines 539-540):
let search_pending: Arc<DashMap<String, tokio::time::Instant>> =
    Arc::new(DashMap::new());

// AFTER — add a clone for the Server struct BEFORE the spawn:
let search_pending: Arc<DashMap<String, tokio::time::Instant>> =
    Arc::new(DashMap::new());
let search_pending_for_struct = search_pending.clone();
```

The original `search_pending` is moved into the `tokio::spawn(async move { ... })` block at line 542. The clone `search_pending_for_struct` is what we store in the Server.

After the worker spawn (line 563), return the tuple:
```rust
// BEFORE:
Some(search_tx)
// AFTER:
(Some(search_tx), Some(search_pending_for_struct))
```

And in the `else` branch (line 565 — the path where search is disabled):
```rust
// BEFORE:
None
// AFTER:
(None, None)
```

Destructure the result into two variables:
```rust
let (search_tx_final, search_pending_final) = if let Some(ref si) = search_index {
    // ... existing code ...
    (Some(search_tx), Some(search_pending_for_struct))
} else {
    (None, None)
};
```

Update the struct initialization in `Server::new()` at line 591:
```rust
search_tx: search_tx_final,
search_pending: search_pending_final,
```

And in `new_for_test()` (line ~629) which also constructs a Server:
```rust
search_tx: None,
search_pending: None,
```

### Step 3: Update callback closures to use entry()-based deduplication

First, add the `Entry` import to the file-level imports (around line 19 where other dashmap imports are):

```rust
// Add to existing dashmap imports:
use dashmap::mapref::entry::Entry;
```

In `load_doc_with_user` (around line 704), clone `search_pending` alongside `search_tx`:

```rust
let search_tx_for_callback = self.search_tx.clone();
let search_pending_for_callback = self.search_pending.clone();
```

Replace the search notification code in both callback branches. The pattern (used at lines 744-748 and 770-772) changes from:

```rust
// BEFORE:
if let Some(ref tx) = search_tx_for_callback {
    if let Err(e) = tx.try_send(doc_key_for_indexer.clone()) {
        tracing::error!("Search index channel send failed (worker dead?): {e}");
    }
}
```

To:

```rust
// AFTER:
if let Some(ref tx) = search_tx_for_callback {
    if let Some(ref pending) = search_pending_for_callback {
        let is_new = match pending.entry(doc_key_for_indexer.clone()) {
            Entry::Occupied(mut e) => {
                e.insert(tokio::time::Instant::now());
                false
            }
            Entry::Vacant(e) => {
                e.insert(tokio::time::Instant::now());
                true
            }
        };
        if is_new {
            if let Err(e) = tx.try_send(doc_key_for_indexer.clone()) {
                tracing::error!("Search index channel send failed (worker dead?): {e}");
            }
        }
    }
}
```

This must be applied to **both** callback closures:
1. Lines 744-748 (inside the `if let Some(dispatcher)` branch)
2. Lines 770-772 (inside the `else` branch without dispatcher)

### Step 4: Fix the search worker's debounce to match LinkIndexer

The worker's content doc debounce (lines 208-218) needs to match the LinkIndexer pattern. Currently it sleeps once and breaks — with `pending` now populated, it will work, but we should make it robust:

```rust
// BEFORE (lines 207-218):
if folder_content.is_none() {
    // Content doc — debounce: wait until no updates for SEARCH_DEBOUNCE
    loop {
        tokio::time::sleep(SEARCH_DEBOUNCE).await;
        if let Some(entry) = pending.get(&doc_id) {
            if entry.elapsed() >= SEARCH_DEBOUNCE {
                break;
            }
        } else {
            break;
        }
    }
}

// AFTER:
if folder_content.is_none() {
    // Content doc — debounce: wait until no updates for SEARCH_DEBOUNCE
    loop {
        tokio::time::sleep(SEARCH_DEBOUNCE).await;
        if let Some(entry) = pending.get(&doc_id) {
            if entry.elapsed() >= SEARCH_DEBOUNCE {
                break;
            }
        } else {
            break; // Entry removed externally, bail out
        }
    }
    // Skip if entry was removed while we waited
    if !pending.contains_key(&doc_id) {
        continue;
    }
}
```

And add cleanup after processing (before the end of the `Some(doc_id)` match arm, around line 241):

```rust
// After processing (both folder and content), remove from pending:
pending.remove(&doc_id);
```

Currently the worker does `pending.remove(&doc_id)` at line 222, which is fine — but verify it's hit for both code paths.

### Step 5: Verify compilation and tests

Run: `cargo check --manifest-path=crates/Cargo.toml -p relay`
Run: `cargo test --manifest-path=crates/Cargo.toml -p y-sweet-core`
Expected: compiles cleanly, all tests pass

### Step 6: Commit

```
jj describe -m "fix(search): share pending DashMap with callbacks for proper debounce

The search worker's debounce was non-functional: the pending DashMap
was local to the worker, never populated by callbacks. Every document
update sent a new channel message with no deduplication. Under load
with 7+ active editors, the bounded(1000) channel would fill up and
all search updates would be silently dropped.

Fix: share the pending DashMap between callbacks and worker, using
the entry() API for atomic first-occurrence-only channel sends —
matching the LinkIndexer's working debounce pattern." && jj new
```
