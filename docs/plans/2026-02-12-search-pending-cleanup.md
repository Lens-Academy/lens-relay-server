# Fix: Search pending entries never removed after folder-rename inline indexing

**Goal:** Prevent search results from going permanently stale after file renames.

**Architecture:** 1-line fix in `search_handle_folder_update`.

---

## The Bug

In `crates/relay/src/server.rs`, `search_handle_folder_update` (lines 395-405) handles renamed/added files by inserting into the shared `pending` DashMap and immediately calling `search_handle_content_update` inline:

```rust
pending.insert(content_id.clone(), tokio::time::Instant::now());
search_handle_content_update(&content_id, docs, search_index);
// BUG: pending entry is never removed
```

After this, the entry stays in `pending` forever. The next time that content doc is edited, the callback's `Entry::Occupied` branch fires (since the key already exists), sets `is_new = false`, and skips sending to the channel. The search worker never processes the update. Search results for that document become permanently stale.

The "first time seeing folder doc" path (lines 411-416) correctly avoids this bug by calling `search_handle_content_update` directly without inserting into `pending` first.

## The Fix

**File:** `crates/relay/src/server.rs`

After the inline `search_handle_content_update` call at line 402, remove the pending entry:

```rust
// BEFORE (lines 399-403):
if docs.contains_key(&content_id) {
    pending.insert(content_id.clone(), tokio::time::Instant::now());
    search_handle_content_update(&content_id, docs, search_index);
}

// AFTER:
if docs.contains_key(&content_id) {
    pending.insert(content_id.clone(), tokio::time::Instant::now());
    search_handle_content_update(&content_id, docs, search_index);
    // Remove pending entry so future edits trigger normal callback -> worker flow.
    // Without this, the callback's Entry::Occupied dedup suppresses all future sends.
    pending.remove(&content_id);
}
```

## Verification

Run: `cargo check --manifest-path=crates/Cargo.toml -p relay`

## Commit

```
jj describe -m "fix(search): remove pending entries after inline folder-rename indexing

search_handle_folder_update inserted pending entries for renamed docs
but never removed them. The callback's entry-based dedup then suppressed
all future search channel sends for those docs, making search results
permanently stale after renames." && jj new
```
