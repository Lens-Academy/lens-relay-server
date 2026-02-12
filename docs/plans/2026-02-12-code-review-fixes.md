# Code Review Fixes Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix all critical and important issues identified in the fork-specific code review, plus the high-value suggestions.

**Architecture:** Fixes are ordered by severity and mostly independent. Task 2 is a prerequisite for Task 3 (Task 3 relies on `find_all_folder_docs` returning sorted results). Each task can be committed separately and is safe to deploy incrementally. No new crates or major refactors.

**Tech Stack:** Rust (yrs, tokio, tantivy, dashmap), TypeScript/React (lens-editor)

---

## Task 1: Replace `thread_local!` IndexingGuard with transaction origin check

The `IndexingGuard` uses `thread_local!` to prevent the indexer's own writes from re-triggering indexing. This works because yrs observers fire synchronously on the same thread, but the safety invariant is invisible. The yrs `observe_update_v1` callback already receives `&TransactionMut` which exposes `.origin() -> Option<&Origin>`. Since the indexer already uses `transact_mut_with("link-indexer")`, we can check the origin string directly.

**Files:**
- Modify: `crates/y-sweet-core/src/webhook.rs:355` (update `WebhookCallback` type alias)
- Modify: `crates/y-sweet-core/src/webhook.rs:357-361` (update `create_webhook_callback` signature)
- Modify: `crates/y-sweet-core/src/webhook.rs:574-582` (update `create_debounced_webhook_callback` signature)
- Modify: `crates/y-sweet-core/src/doc_sync.rs:53` (pass `txn` origin to callback)
- Modify: `crates/relay/src/server.rs:738,764` (check origin instead of `should_index()`)
- Modify: `crates/y-sweet-core/src/link_indexer.rs:37-61` (remove `thread_local!`, `IndexingGuard`, `should_index()`)
- Modify: `crates/y-sweet-core/src/link_indexer.rs:301` (remove `_guard` in `index_content_into_folders`)
- Modify: `crates/y-sweet-core/src/link_indexer.rs:385` (remove `_guard` in `update_wikilinks_in_doc`)

**Step 1: Change `doc_sync.rs` observer to pass origin to the webhook callback**

The observer currently ignores the `TransactionMut` first parameter. Change it to extract the origin and pass it through to the callback.

First, update the `WebhookCallback` type to accept an optional origin. In `crates/y-sweet-core/src/webhook.rs`, find the type alias:

```rust
// Current:
pub type WebhookCallback = Arc<dyn Fn(DocumentUpdatedEvent) + Send + Sync>;

// New:
pub type WebhookCallback = Arc<dyn Fn(DocumentUpdatedEvent, bool) + Send + Sync>;
```

The second `bool` parameter is `is_indexer_origin` — `true` when the transaction origin is `"link-indexer"`.

Also update the two factory functions in `webhook.rs` that construct `WebhookCallback` values. They are currently unused but must compile:

```rust
// Line 357 — create_webhook_callback:
pub fn create_webhook_callback(dispatcher: Arc<WebhookDispatcher>) -> WebhookCallback {
    Arc::new(move |event: crate::event::DocumentUpdatedEvent, _is_indexer: bool| {
        dispatcher.send_webhooks(event.doc_id.clone());
    })
}

// Line 574 — create_debounced_webhook_callback:
pub fn create_debounced_webhook_callback(queue: Arc<DebouncedWebhookQueue>) -> WebhookCallback {
    Arc::new(move |event: crate::event::DocumentUpdatedEvent, _is_indexer: bool| {
        let queue_clone = queue.clone();
        let doc_id = event.doc_id.clone();
        tokio::spawn(async move {
            queue_clone.queue_webhook(doc_id).await;
        });
    })
}
```

In `crates/y-sweet-core/src/doc_sync.rs:53`, change the observer:

```rust
// Current:
doc.observe_update_v1(move |_, event| {
    // ...
    if let Some(ref callback) = webhook_callback {
        let event = DocumentUpdatedEvent::new(doc_key.clone())
            .with_metadata(&sync_kv)
            .with_update(event.update.to_vec());
        callback(event);
    }
})

// New:
doc.observe_update_v1(move |txn, event| {
    // ...
    if let Some(ref callback) = webhook_callback {
        let is_indexer = txn.origin()
            .map(|o| o.as_ref() == b"link-indexer")
            .unwrap_or(false);
        let event = DocumentUpdatedEvent::new(doc_key.clone())
            .with_metadata(&sync_kv)
            .with_update(event.update.to_vec());
        callback(event, is_indexer);
    }
})
```

**Step 2: Update callback call sites in `server.rs`**

In `crates/relay/src/server.rs`, both callback closures (around lines 710 and 757) need updating:

```rust
// Current (both closures):
Some(Arc::new(move |event: DocumentUpdatedEvent| {
    // ...
    if y_sweet_core::link_indexer::should_index() {
        // notify indexer + search
    }
}))

// New (both closures):
Some(Arc::new(move |event: DocumentUpdatedEvent, is_indexer: bool| {
    // ...
    if !is_indexer {
        // notify indexer + search
    }
}))
```

**Step 3: Remove `thread_local!` infrastructure from `link_indexer.rs`**

Delete the entire block at lines 37-61:
- `thread_local! { static INDEXING_IN_PROGRESS ... }`
- `pub fn should_index() -> bool`
- `pub struct IndexingGuard`
- `impl IndexingGuard`
- `impl Drop for IndexingGuard`

Remove the `_guard` lines:
- Line 301: delete `let _guard = IndexingGuard::new();`
- Line 385: delete `let _guard = IndexingGuard::new();`

Remove unused imports (`RefCell` if only used by the thread_local).

**Step 4: Verify compilation and tests**

Run: `CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo check --manifest-path=crates/Cargo.toml`
Expected: Clean compilation (warnings OK if pre-existing)

Run: `CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo test --manifest-path=crates/Cargo.toml -p y-sweet-core`
Expected: All 217 tests pass

**Step 5: Commit**

```
jj describe -m "refactor(link-indexer): replace thread_local IndexingGuard with transaction origin check

The IndexingGuard used thread_local! to prevent the indexer's own writes
from re-triggering indexing. Replace with checking TransactionMut::origin()
for the \"link-indexer\" string, which is already set by transact_mut_with().
This is correct regardless of which thread the observer fires on."
```

---

## Task 2: Sort `find_all_folder_docs()` return value

`find_all_folder_docs()` iterates a `DashMap`, returning folder doc IDs in non-deterministic order. Two call sites in `server.rs` use the index position to determine folder names ("Lens" vs "Lens Edu"), causing non-deterministic assignment.

**Files:**
- Modify: `crates/y-sweet-core/src/link_indexer.rs:185-201` (sort before returning)
- Modify: `crates/y-sweet-core/src/doc_resolver.rs:57-58` (remove now-redundant sort)
- Modify: `crates/relay/src/server.rs:~305,~930` (replace inline folder naming with `derive_folder_name()`)

**Step 1: Sort inside `find_all_folder_docs()`**

```rust
// In crates/y-sweet-core/src/link_indexer.rs, find_all_folder_docs():
pub fn find_all_folder_docs(docs: &DashMap<String, DocWithSyncKv>) -> Vec<String> {
    let mut result: Vec<String> = docs.iter()
        .filter_map(|entry| {
            let awareness = entry.value().awareness();
            let guard = awareness.read().unwrap();
            let txn = guard.doc.transact();
            if let Some(filemeta) = txn.get_map("filemeta_v0") {
                if filemeta.len(&txn) > 0 {
                    return Some(entry.key().clone());
                }
            }
            None
        })
        .collect();
    result.sort();
    result
}
```

**Step 2: Remove redundant sort in `doc_resolver.rs`**

In `crates/y-sweet-core/src/doc_resolver.rs:57-58`:

```rust
// Remove this line:
folder_doc_ids.sort(); // Deterministic folder ordering by doc_id
```

The `let mut` can also become `let` since we no longer mutate it.

**Step 3: Replace hardcoded folder naming in `server.rs`**

In `search_find_title_and_folder()` (~line 305):

```rust
// Current:
let folder_name = if folder_idx == 0 {
    "Lens".to_string()
} else {
    "Lens Edu".to_string()
};

// New:
let folder_name = doc_resolver::derive_folder_name(folder_idx).to_string();
```

Add import at top of file: `use y_sweet_core::doc_resolver;`

In `startup_reindex()` (~line 930):

```rust
// Current:
let folder_name = if folder_idx == 0 {
    "Lens".to_string()
} else {
    "Lens Edu".to_string()
};

// New:
let folder_name = doc_resolver::derive_folder_name(folder_idx).to_string();
```

**Step 4: Verify compilation and tests**

Run: `CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo check --manifest-path=crates/Cargo.toml`

Run: `CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo test --manifest-path=crates/Cargo.toml -p y-sweet-core`

**Step 5: Commit**

```
jj describe -m "fix: sort find_all_folder_docs() for deterministic folder ordering

find_all_folder_docs() iterated DashMap in arbitrary order. Call sites
in server.rs used index position to assign folder names, causing
non-deterministic 'Lens' vs 'Lens Edu' assignment. Sort the result
inside the function and use derive_folder_name() everywhere."
```

---

## Task 3: Keep DocumentResolver updated on folder doc changes

The `DocumentResolver` is only built at startup. When files are created, renamed, or deleted through Obsidian/Relay, the resolver becomes stale and MCP tools can't find new documents.

The link indexer already processes every folder doc update. Add an `update_folder()` call there.

**Files:**
- Modify: `crates/y-sweet-core/src/link_indexer.rs` (add `doc_resolver` field, call `update_folder` in worker)
- Modify: `crates/relay/src/server.rs` (pass `doc_resolver` to `LinkIndexer::new()` or `run_worker()`)

**Step 1: Pass `doc_resolver` to `run_worker()`**

Change the `run_worker` signature to accept a `DocumentResolver`:

```rust
// In crates/y-sweet-core/src/link_indexer.rs:
pub async fn run_worker(
    self: Arc<Self>,
    mut rx: mpsc::Receiver<String>,
    docs: Arc<DashMap<String, DocWithSyncKv>>,
    doc_resolver: Arc<DocumentResolver>,
) {
```

Add import at top of file: `use crate::doc_resolver::DocumentResolver;`

**Step 2: Call `update_folder()` after folder doc processing**

Inside the folder doc branch of `run_worker` (after the rename/re-queue logic, around line 684), add:

```rust
// After the if/else block for had_renames, before the closing brace:
// Update DocumentResolver so MCP tools see current paths
let mut all_folder_ids = find_all_folder_docs(&docs);
// find_all_folder_docs already returns sorted
if let Some(folder_idx) = all_folder_ids.iter().position(|id| id == &doc_id) {
    doc_resolver.update_folder(&doc_id, folder_idx, &docs);
}
```

**Step 3: Update spawn site in `server.rs`**

**IMPORTANT ordering issue:** `doc_resolver` is currently created inline at line 596 inside the `Ok(Self { ... })` block, which is AFTER the link indexer spawn at ~line 502. You must create a single `Arc<DocumentResolver>` before the spawn and reuse the same instance in the `Self` struct — otherwise you'd get two separate resolvers that don't share state.

Concrete fix:

1. Before the link indexer spawn block (~line 499), add:
```rust
let doc_resolver = Arc::new(DocumentResolver::new());
```

2. In the spawn block, clone it:
```rust
let resolver_for_indexer = doc_resolver.clone();
tokio::spawn(async move {
    let result = std::panic::AssertUnwindSafe(
        indexer_for_worker.run_worker(index_rx, docs_for_indexer, resolver_for_indexer),
    );
    // ... (existing catch_unwind handling)
});
```

3. In the `Ok(Self { ... })` struct at line 596, change:
```rust
// Current:
doc_resolver: Arc::new(DocumentResolver::new()),

// New:
doc_resolver,
```

4. The second `Ok(Self { ... })` block (around line 634, the no-auth path) also needs the same treatment. Create `doc_resolver` before that block too, or hoist it above both code paths.

**Prerequisite:** Task 2 must be completed first — this code calls `find_all_folder_docs(&docs)` and relies on it returning sorted results for correct `folder_idx` values.

**Step 4: Verify compilation and tests**

Run: `CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo check --manifest-path=crates/Cargo.toml`

Run: `CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo test --manifest-path=crates/Cargo.toml -p y-sweet-core`

**Step 5: Commit**

```
jj describe -m "fix: update DocumentResolver incrementally on folder doc changes

DocumentResolver was only built at startup, causing stale path-to-UUID
mappings for MCP tools. Call update_folder() from the link indexer
worker whenever a folder doc is processed."
```

---

## Task 4: Replace `panic!()` with `Err` in auth channel validation

Two functions in `auth.rs` panic on invalid channel names. Since the functions already return `Result<String, AuthError>`, just return an error variant instead.

**Files:**
- Modify: `crates/y-sweet-core/src/auth.rs:75-105` (add `InvalidChannelName` variant)
- Modify: `crates/y-sweet-core/src/auth.rs:~1021` (replace panic in `gen_doc_token_cwt`)
- Modify: `crates/y-sweet-core/src/auth.rs:~1116` (replace panic in `gen_file_token_cwt`)

**Step 1: Add `AuthError::InvalidChannelName` variant**

In `crates/y-sweet-core/src/auth.rs`, add to the `AuthError` enum (after `NoSigningKey`):

```rust
    #[error("Invalid channel name: must contain only alphanumeric characters, hyphens, and underscores")]
    InvalidChannelName,
```

**Step 2: Replace both `panic!()` calls**

In `gen_doc_token_cwt()` (~line 1021):

```rust
// Current:
if !crate::api_types::validate_key(channel_name) {
    panic!("Invalid channel name: must contain only alphanumeric characters, hyphens, and underscores");
}

// New:
if !crate::api_types::validate_key(channel_name) {
    return Err(AuthError::InvalidChannelName);
}
```

In `gen_file_token_cwt()` (~line 1116), same change.

**Step 3: Verify compilation and tests**

Run: `CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo check --manifest-path=crates/Cargo.toml`

Run: `CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo test --manifest-path=crates/Cargo.toml -p y-sweet-core`

**Step 4: Commit**

```
jj describe -m "fix(auth): return error instead of panicking on invalid channel names

gen_doc_token_cwt and gen_file_token_cwt panicked on invalid channel
names, which would crash the server on malformed client input. Return
AuthError::InvalidChannelName instead."
```

---

## Task 5: Fix `on_document_update` TOCTOU race with `entry()` API

The `on_document_update` method has a time-of-check-time-of-use race between `contains_key()` and `insert()`. Two concurrent calls with the same `doc_id` could both see `already_pending = false` and both send to the channel.

**Files:**
- Modify: `crates/y-sweet-core/src/link_indexer.rs:432-445`

**Step 1: Replace with `entry()` API**

```rust
// Current:
pub async fn on_document_update(&self, doc_id: &str) {
    let already_pending = self.pending.contains_key(doc_id);
    self.pending.insert(doc_id.to_string(), Instant::now());
    if !already_pending {
        if let Err(e) = self.index_tx.send(doc_id.to_string()).await {
            tracing::error!(
                "Link indexer channel send failed (receiver dropped — worker dead?): {}",
                e
            );
        }
    }
}

// New:
pub async fn on_document_update(&self, doc_id: &str) {
    use dashmap::mapref::entry::Entry;
    let is_new = match self.pending.entry(doc_id.to_string()) {
        Entry::Occupied(mut e) => {
            e.insert(Instant::now());
            false
        }
        Entry::Vacant(e) => {
            e.insert(Instant::now());
            true
        }
    };
    if is_new {
        if let Err(e) = self.index_tx.send(doc_id.to_string()).await {
            tracing::error!(
                "Link indexer channel send failed (receiver dropped — worker dead?): {}",
                e
            );
        }
    }
}
```

**Step 2: Verify compilation and tests**

Run: `CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo test --manifest-path=crates/Cargo.toml -p y-sweet-core`

**Step 3: Commit**

```
jj describe -m "fix(link-indexer): use entry() API to eliminate TOCTOU race in on_document_update

The contains_key + insert pattern allowed concurrent calls for the same
doc_id to both see 'not pending' and double-send to the channel."
```

---

## Task 6: Batch search index commits

`SearchIndex::add_document()` and `remove_document()` each call `writer.commit()` and `reader.reload()`. During startup reindex of hundreds of documents, this is extremely expensive.

Add buffered variants and a `flush()` method.

**Files:**
- Modify: `crates/y-sweet-core/src/search_index.rs:110-142` (add buffered methods + flush)
- Modify: `crates/relay/src/server.rs` (use buffered methods in startup_reindex, flush at end)

**Step 1: Add `add_document_buffered`, `remove_document_buffered`, and `flush` methods**

In `crates/y-sweet-core/src/search_index.rs`, after the existing `remove_document` method:

```rust
    /// Add a document without committing. Call `flush()` after a batch.
    pub fn add_document_buffered(
        &self,
        doc_id: &str,
        title: &str,
        body: &str,
        folder: &str,
    ) -> Result<()> {
        let mut writer = self.writer.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        let term = Term::from_field_text(self.doc_id_field, doc_id);
        writer.delete_term(term);
        writer.add_document(doc!(
            self.doc_id_field => doc_id,
            self.title_field => title,
            self.body_field => body,
            self.folder_field => folder,
        ))?;
        Ok(())
    }

    /// Remove a document without committing. Call `flush()` after a batch.
    pub fn remove_document_buffered(&self, doc_id: &str) -> Result<()> {
        let mut writer = self.writer.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        let term = Term::from_field_text(self.doc_id_field, doc_id);
        writer.delete_term(term);
        Ok(())
    }

    /// Commit buffered changes and reload the reader.
    pub fn flush(&self) -> Result<()> {
        let mut writer = self.writer.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }
```

**Step 2: Use buffered methods in `startup_reindex()`**

In `crates/relay/src/server.rs`, in the `startup_reindex()` method where documents are added to the search index in a loop (~line 995):

```rust
// Current:
if let Err(e) = search_index.add_document(uuid, title, &body, folder_name) {
    tracing::error!("Failed to add doc to search index: {}", e);
} else {
    indexed += 1;
}

// New:
if let Err(e) = search_index.add_document_buffered(uuid, title, &body, folder_name) {
    tracing::error!("Failed to add doc to search index: {}", e);
} else {
    indexed += 1;
}
```

After the loop ends, flush:

```rust
// After the loop:
if let Err(e) = search_index.flush() {
    tracing::error!("Failed to flush search index: {}", e);
}
tracing::info!("Search index built: {} documents indexed", indexed);
```

Leave the incremental `add_document`/`remove_document` calls in the search worker unchanged — individual updates should still commit immediately for real-time search.

**Step 3: Verify compilation and tests**

Run: `CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo check --manifest-path=crates/Cargo.toml`

Run: `CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo test --manifest-path=crates/Cargo.toml -p y-sweet-core`

**Step 4: Commit**

```
jj describe -m "perf(search): batch commits during startup reindex

Add add_document_buffered/remove_document_buffered/flush to SearchIndex.
Use buffered adds during startup_reindex to avoid per-document commit+
reload overhead."
```

---

## Task 7: Fix XSS in search snippets

`render_snippet_with_mark()` wraps highlighted terms in `<mark>` tags without escaping the surrounding text. The frontend renders snippets via `dangerouslySetInnerHTML`. Document content containing HTML would be injected into the page.

**Files:**
- Modify: `crates/y-sweet-core/src/search_index.rs:200-230` (escape HTML in fragment text)

**Step 1: Add HTML escaping to `render_snippet_with_mark()`**

No new dependency needed — a minimal escape function covers the 5 HTML special characters:

```rust
/// Escape HTML special characters for safe embedding in markup.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn render_snippet_with_mark(snippet: &tantivy::snippet::Snippet) -> String {
    let fragment = snippet.fragment();
    let highlighted = snippet.highlighted();

    if highlighted.is_empty() {
        return escape_html(fragment);
    }

    let mut result = String::new();
    let mut pos = 0;

    for range in highlighted {
        if range.start > pos {
            result.push_str(&escape_html(&fragment[pos..range.start]));
        }
        result.push_str("<mark>");
        result.push_str(&escape_html(&fragment[range.start..range.end]));
        result.push_str("</mark>");
        pos = range.end;
    }

    if pos < fragment.len() {
        result.push_str(&escape_html(&fragment[pos..]));
    }

    result
}
```

**Step 2: Add a test**

In the test module of `search_index.rs`:

```rust
#[test]
fn snippet_escapes_html_special_characters() {
    let index = create_index();  // uses SearchIndex::new_in_memory() helper defined in test module
    index
        .add_document("doc1", "XSS Test", "try <script>alert(1)</script> here", "Lens")
        .unwrap();
    let results = index.search("script", 10).unwrap();
    assert_eq!(results.len(), 1);
    // The snippet should contain escaped HTML, not raw tags
    assert!(!results[0].snippet.contains("<script>"));
    assert!(results[0].snippet.contains("&lt;script&gt;"));
}
```

**Step 3: Also update existing test `snippet_does_not_contain_bold_tags`**

Check if this test needs updating — the `<b>` tags test may now produce escaped output. Read the test and adjust if needed.

**Step 4: Verify tests**

Run: `CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo test --manifest-path=crates/Cargo.toml -p y-sweet-core -- search_index`

**Step 5: Commit**

```
jj describe -m "fix(search): escape HTML in search snippets to prevent XSS

render_snippet_with_mark() now escapes HTML special characters in
fragment text before wrapping highlights in <mark> tags. The frontend
uses dangerouslySetInnerHTML to render these snippets."
```

---

## Task 8: Fix `generateNewDocPath` missing leading slash

The TypeScript `generateNewDocPath()` returns paths without a leading `/`, inconsistent with the `filemeta_v0` convention used by the Rust server.

**Files:**
- Modify: `lens-editor/src/lib/document-resolver.ts:48-52`

**Step 1: Add leading slash**

```typescript
// Current:
export function generateNewDocPath(pageName: string): string {
  const safeName = pageName.replace(/[/\\?%*:|"<>]/g, '-');
  return `${safeName}.md`;
}

// New:
export function generateNewDocPath(pageName: string): string {
  const safeName = pageName.replace(/[/\\?%*:|"<>]/g, '-');
  return `/${safeName}.md`;
}
```

**Step 2: Update test expectations**

In `lens-editor/src/lib/document-resolver.test.ts`, update all `generateNewDocPath` test expectations to include the leading slash:

```typescript
// Line 151:
expect(generateNewDocPath('New Page')).toBe('/New Page.md');
// Line 155:
expect(generateNewDocPath('What is this?')).toBe('/What is this-.md');
// Line 159:
expect(generateNewDocPath('A/B')).toBe('/A-B.md');
// Line 163:
expect(generateNewDocPath('A\\B')).toBe('/A-B.md');
// Line 167:
expect(generateNewDocPath('Time: 10:00')).toBe('/Time- 10-00.md');
// Line 172:
expect(generateNewDocPath('Course YAML examples')).toBe('/Course YAML examples.md');
// Line 173-174: (multi-line assertion)
expect(generateNewDocPath('Dev, Staging, and Production environments')).toBe(
  '/Dev, Staging, and Production environments.md'
);
```

**Step 3: Check for callers that might already add the slash**

Search for `generateNewDocPath` in the codebase and verify no caller adds its own `/` prefix. (Currently only called in tests.)

**Step 4: Commit**

```
jj describe -m "fix(lens-editor): add leading slash to generateNewDocPath

Matches the /filename.md convention used in filemeta_v0 by the Rust
server for link resolution."
```

---

## Summary

| Task | Issue | Severity | Est. Difficulty |
|------|-------|----------|-----------------|
| 1 | thread_local IndexingGuard -> origin check | Critical | Medium (touches 3 files, callback signature change) |
| 2 | Non-deterministic folder ordering | Critical | Easy (one-line fix + cleanup) |
| 3 | Stale DocumentResolver | Critical | Medium (plumbing new param through) |
| 4 | Auth panic on invalid channel | Important | Easy (2 line changes) |
| 5 | TOCTOU race in on_document_update | Important | Easy (entry API swap) |
| 6 | Per-document search commits | Important | Easy (add methods, change one call site) |
| 7 | XSS in search snippets | Suggestion | Easy (add escape function) |
| 8 | Missing leading slash | Suggestion | Trivial |
