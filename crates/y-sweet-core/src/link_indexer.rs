use crate::doc_sync::DocWithSyncKv;
use crate::link_parser::{compute_wikilink_rename_edits, extract_wikilinks};
use dashmap::DashMap;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};
use yrs::{Any, Doc, GetString, Map, MapRef, Out, ReadTxn, Text, Transact, WriteTxn};

// ---------------------------------------------------------------------------
// parse_doc_id
// ---------------------------------------------------------------------------

/// Parse a doc_id into (relay_id, doc_uuid).
///
/// Format: "relay_id-doc_uuid" where both are UUIDs (36 chars: 8-4-4-4-12)
/// Example: "cb696037-0f72-4e93-8717-4e433129d789-f7c85d0f-8bb4-4a03-80b5-408498d77c52"
///
/// Returns None if the format is invalid.
pub fn parse_doc_id(doc_id: &str) -> Option<(&str, &str)> {
    // UUID is 36 chars: 8-4-4-4-12 with hyphens
    // Full doc_id is: 36 (relay_id) + 1 (hyphen) + 36 (doc_uuid) = 73 chars
    if doc_id.len() >= 73 && doc_id.as_bytes()[36] == b'-' {
        let relay_id = &doc_id[..36];
        let doc_uuid = &doc_id[37..];
        Some((relay_id, doc_uuid))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the "id" field from a filemeta_v0 entry value.
///
/// filemeta_v0 entries can be stored as either:
/// - `Out::YMap(MapRef)` — from Rust/Yrs code using MapPrelim (unit tests, server-side writes)
/// - `Out::Any(Any::Map(HashMap))` — from JavaScript Y.js clients setting plain objects
pub fn extract_id_from_filemeta_entry(value: &Out, txn: &impl ReadTxn) -> Option<String> {
    match value {
        Out::YMap(meta_map) => {
            if let Some(Out::Any(Any::String(ref id))) = meta_map.get(txn, "id") {
                Some(id.to_string())
            } else {
                None
            }
        }
        Out::Any(Any::Map(ref map)) => {
            if let Some(Any::String(ref id)) = map.get("id") {
                Some(id.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Read a backlinks array for a given target UUID from a backlinks_v0 Y.Map.
pub fn read_backlinks_array(backlinks: &MapRef, txn: &impl ReadTxn, target_uuid: &str) -> Vec<String> {
    backlinks
        .get(txn, target_uuid)
        .and_then(|v| {
            if let Out::Any(Any::Array(arr)) = v {
                Some(
                    arr.iter()
                        .filter_map(|item| {
                            if let Any::String(s) = item {
                                Some(s.to_string())
                            } else {
                                None
                            }
                        })
                        .collect(),
                )
            } else {
                None
            }
        })
        .unwrap_or_default()
}

/// Resolve link names (e.g., "Note") to UUIDs using filemeta_v0.
///
/// filemeta_v0 structure:
///   "/Welcome.md" -> Y.Map { "id": "uuid", "type": "markdown", "version": 0 }
///   "/Notes/Ideas.md" -> Y.Map { "id": "other-uuid", ... }
fn resolve_links_to_uuids(
    link_names: &[String],
    filemeta: &MapRef,
    txn: &impl ReadTxn,
) -> Vec<String> {
    let mut uuids = Vec::new();

    for name in link_names {
        // Try exact match: "/{name}.md"
        let path = format!("/{}.md", name);

        if let Some(entry_value) = filemeta.get(txn, &*path) {
            if let Some(id) = extract_id_from_filemeta_entry(&entry_value, txn) {
                uuids.push(id);
                continue;
            }
        }

        // Try case-insensitive full-path match
        let mut found = false;
        for (entry_path, entry_value) in filemeta.iter(txn) {
            let entry_name = entry_path
                .strip_prefix('/')
                .and_then(|s| s.strip_suffix(".md"))
                .unwrap_or(entry_path);

            if entry_name.to_lowercase() == name.to_lowercase() {
                if let Some(id) = extract_id_from_filemeta_entry(&entry_value, txn) {
                    uuids.push(id);
                    found = true;
                    break;
                }
            }
        }
        if found {
            continue;
        }

        // Basename match (case-insensitive) — handles subdirectories
        // e.g. [[Ideas]] matches /Notes/Ideas.md
        for (entry_path, entry_value) in filemeta.iter(txn) {
            let basename = entry_path
                .rsplit('/')
                .next()
                .and_then(|s| s.strip_suffix(".md"))
                .unwrap_or("");

            if basename.to_lowercase() == name.to_lowercase() {
                if let Some(id) = extract_id_from_filemeta_entry(&entry_value, txn) {
                    uuids.push(id);
                    break; // first match wins
                }
            }
        }
        // If not found, the link is unresolvable — silently skip
    }

    uuids
}

// ---------------------------------------------------------------------------
// Folder doc scanning helpers
// ---------------------------------------------------------------------------

/// Find all loaded folder docs (docs with non-empty filemeta_v0).
/// Returns doc_ids of all folder docs.
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

/// Check if a doc is a folder doc (has non-empty filemeta_v0).
/// Returns the list of content doc UUIDs listed in filemeta_v0, or None.
pub fn is_folder_doc(doc_id: &str, docs: &DashMap<String, DocWithSyncKv>) -> Option<Vec<String>> {
    let doc_ref = docs.get(doc_id)?;
    let awareness = doc_ref.awareness();
    let guard = awareness.read().unwrap();
    let txn = guard.doc.transact();
    let filemeta = txn.get_map("filemeta_v0")?;
    if filemeta.len(&txn) == 0 {
        return None;
    }
    let mut content_uuids = Vec::new();
    for (_path, value) in filemeta.iter(&txn) {
        if let Some(id) = extract_id_from_filemeta_entry(&value, &txn) {
            content_uuids.push(id);
        }
    }
    Some(content_uuids)
}

// ---------------------------------------------------------------------------
// Core indexing function (testable with bare Y.Docs)
// ---------------------------------------------------------------------------

/// Core indexing logic — operates on bare Y.Docs for testability.
///
/// Single-folder convenience wrapper. All existing callers and tests continue to work.
pub fn index_content_into_folder(
    source_uuid: &str,
    content_doc: &Doc,
    folder_doc: &Doc,
) -> anyhow::Result<()> {
    index_content_into_folders(source_uuid, content_doc, &[folder_doc])
}

/// Multi-folder indexing: resolves wikilinks across all folder docs and writes
/// backlinks to the folder doc that owns each target.
pub fn index_content_into_folders(
    source_uuid: &str,
    content_doc: &Doc,
    folder_docs: &[&Doc],
) -> anyhow::Result<()> {
    // 1. Extract markdown content
    let markdown = {
        let txn = content_doc.transact();
        if let Some(contents) = txn.get_text("contents") {
            contents.get_string(&txn)
        } else {
            return Ok(()); // No content, nothing to index
        }
    };

    // 2. Parse wikilinks
    let link_names = extract_wikilinks(&markdown);
    tracing::info!(
        "Doc {}: content length={}, wikilinks={:?}",
        source_uuid, markdown.len(), link_names
    );

    // 3. Resolve link names to (target_uuid, folder_index) across all folder docs.
    //    Each target UUID is resolved to the folder that contains it.
    //    Priority: exact root match > case-insensitive full-path > basename match.
    //    First folder match wins (folders searched in order).
    let mut resolved: Vec<(String, usize)> = Vec::new(); // (target_uuid, folder_idx)

    for name in &link_names {
        let mut found = false;
        for (fi, folder_doc) in folder_docs.iter().enumerate() {
            let txn = folder_doc.transact();
            if let Some(filemeta) = txn.get_map("filemeta_v0") {
                let resolved_uuids = resolve_links_to_uuids(&[name.clone()], &filemeta, &txn);
                if let Some(uuid) = resolved_uuids.into_iter().next() {
                    resolved.push((uuid, fi));
                    found = true;
                    break; // first folder match wins
                }
            }
        }
        if !found {
            // Unresolvable — silently skip
        }
    }

    tracing::info!(
        "Doc {}: resolved {} links -> {} targets across {} folders",
        source_uuid, link_names.len(), resolved.len(), folder_docs.len()
    );

    // 4. Group resolved targets by folder index
    let mut targets_per_folder: Vec<HashSet<String>> = vec![HashSet::new(); folder_docs.len()];
    for (uuid, fi) in &resolved {
        targets_per_folder[*fi].insert(uuid.clone());
    }

    // All resolved target UUIDs (for stale cleanup)
    let all_new_targets: HashSet<&str> = resolved.iter().map(|(u, _)| u.as_str()).collect();

    // 5. Diff-update backlinks_v0 on each folder doc
    for (fi, folder_doc) in folder_docs.iter().enumerate() {
        let new_targets = &targets_per_folder[fi];
        let mut txn = folder_doc.transact_mut_with("link-indexer");
        let backlinks = txn.get_or_insert_map("backlinks_v0");

        // Add source to each new target's backlinks in this folder
        for target_uuid in new_targets {
            let current: Vec<String> = read_backlinks_array(&backlinks, &txn, target_uuid);
            if !current.contains(&source_uuid.to_string()) {
                let mut updated = current;
                updated.push(source_uuid.to_string());
                let arr: Vec<Any> = updated
                    .into_iter()
                    .map(|s| Any::String(s.into()))
                    .collect();
                backlinks.insert(&mut txn, target_uuid.as_str(), arr);
            }
        }

        // Remove source from targets it no longer links to in this folder
        let all_keys: Vec<String> = backlinks.keys(&txn).map(|k| k.to_string()).collect();
        for key in all_keys {
            if all_new_targets.contains(key.as_str()) {
                continue; // Still linked (possibly in another folder), skip
            }
            let current: Vec<String> = read_backlinks_array(&backlinks, &txn, &key);
            if current.contains(&source_uuid.to_string()) {
                let updated: Vec<String> =
                    current.into_iter().filter(|s| s != source_uuid).collect();
                if updated.is_empty() {
                    backlinks.remove(&mut txn, &key);
                } else {
                    let arr: Vec<Any> = updated
                        .into_iter()
                        .map(|s| Any::String(s.into()))
                        .collect();
                    backlinks.insert(&mut txn, key.as_str(), arr);
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Wikilink rename — apply rename edits to Y.Text
// ---------------------------------------------------------------------------

/// Read Y.Text("contents"), compute wikilink rename edits, and apply them.
///
/// This is a free function (not a method on LinkIndexer). It:
/// 1. Reads the plain text from Y.Text("contents")
/// 2. Calls `compute_wikilink_rename_edits()` to find matching wikilinks
/// 3. Applies edits in reverse order using `remove_range` / `insert`
///    (uses `transact_mut_with("link-indexer")` so the observer can identify
///    indexer-originated writes via the transaction origin)
/// 4. Returns the number of edits applied
///
/// Note: yrs defaults to `OffsetKind::Bytes` (UTF-8 byte offsets), which matches
/// the byte offsets from `compute_wikilink_rename_edits()` directly — no conversion needed.
pub fn update_wikilinks_in_doc(
    content_doc: &Doc,
    old_name: &str,
    new_name: &str,
) -> anyhow::Result<usize> {
    // 1. Read plain text
    let plain_text = {
        let txn = content_doc.transact();
        match txn.get_text("contents") {
            Some(text) => text.get_string(&txn),
            None => return Ok(0),
        }
    };

    // 2. Compute edits (already in reverse offset order)
    let edits = compute_wikilink_rename_edits(&plain_text, old_name, new_name);
    if edits.is_empty() {
        return Ok(0);
    }

    // 3. Apply edits in reverse offset order so earlier byte offsets stay valid.
    let mut txn = content_doc.transact_mut_with("link-indexer");
    let text = txn.get_or_insert_text("contents");

    for edit in &edits {
        text.remove_range(&mut txn, edit.offset as u32, edit.remove_len as u32);
        text.insert(&mut txn, edit.offset as u32, &edit.insert_text);
    }

    // 5. Return count
    Ok(edits.len())
}

// ---------------------------------------------------------------------------
// LinkIndexer — async server-side struct with debounced worker
// ---------------------------------------------------------------------------

const DEBOUNCE_DURATION: Duration = Duration::from_secs(2);

/// A rename event detected by diffing filemeta snapshots.
///
/// Emitted when the same UUID maps to a different basename across two snapshots.
pub(crate) struct RenameEvent {
    pub uuid: String,
    pub old_name: String,
    pub new_name: String,
}

pub struct LinkIndexer {
    pending: Arc<DashMap<String, Instant>>,
    index_tx: mpsc::Sender<String>,
    filemeta_cache: Arc<DashMap<String, HashMap<String, String>>>, // folder_doc_id -> (uuid -> basename)
}

impl LinkIndexer {
    pub fn new() -> (Self, mpsc::Receiver<String>) {
        let (index_tx, index_rx) = mpsc::channel(1000);
        (
            Self {
                pending: Arc::new(DashMap::new()),
                index_tx,
                filemeta_cache: Arc::new(DashMap::new()),
            },
            index_rx,
        )
    }

    pub async fn on_document_update(&self, doc_id: &str) {
        let already_pending = self.pending.contains_key(doc_id);
        self.pending.insert(doc_id.to_string(), Instant::now());
        // Only send to channel on the first update — subsequent updates just
        // reset the timestamp for debouncing without flooding the channel.
        if !already_pending {
            if let Err(e) = self.index_tx.send(doc_id.to_string()).await {
                tracing::error!(
                    "Link indexer channel send failed (receiver dropped — worker dead?): {}",
                    e
                );
            }
        }
    }

    fn is_ready(&self, doc_id: &str) -> bool {
        if let Some(entry) = self.pending.get(doc_id) {
            entry.elapsed() >= DEBOUNCE_DURATION
        } else {
            false
        }
    }

    fn mark_indexed(&self, doc_id: &str) {
        self.pending.remove(doc_id);
    }

    /// Diff current filemeta_v0 state against cache, emit RenameEvent for UUIDs
    /// whose basename has changed. Updates the cache after diffing.
    /// First call (no cache entry) seeds the cache and returns empty.
    pub(crate) fn detect_renames(&self, folder_doc_id: &str, folder_doc: &Doc) -> Vec<RenameEvent> {
        // 1. Read filemeta_v0 and build uuid -> basename map
        let current: HashMap<String, String> = {
            let txn = folder_doc.transact();
            let Some(filemeta) = txn.get_map("filemeta_v0") else {
                return Vec::new();
            };

            let mut map = HashMap::new();
            for (path, value) in filemeta.iter(&txn) {
                if let Some(uuid) = extract_id_from_filemeta_entry(&value, &txn) {
                    // Extract basename: strip leading "/", strip trailing ".md", take last component
                    let basename = path
                        .strip_prefix('/')
                        .unwrap_or(&path)
                        .strip_suffix(".md")
                        .unwrap_or(&path)
                        .rsplit('/')
                        .next()
                        .unwrap_or(&path)
                        .to_string();
                    map.insert(uuid, basename);
                }
            }
            map
        };

        // 2. Get old snapshot from cache
        let old_opt = self.filemeta_cache.get(folder_doc_id).map(|r| r.clone());

        // 3. Update cache with current snapshot
        self.filemeta_cache
            .insert(folder_doc_id.to_string(), current.clone());

        // 4. If no old snapshot, this is the seed call — return empty
        let Some(old) = old_opt else {
            return Vec::new();
        };

        // 5. Compare: for each uuid in BOTH old and new, if basename changed, emit RenameEvent
        let mut renames = Vec::new();
        for (uuid, new_basename) in &current {
            if let Some(old_basename) = old.get(uuid) {
                if old_basename != new_basename {
                    renames.push(RenameEvent {
                        uuid: uuid.clone(),
                        old_name: old_basename.clone(),
                        new_name: new_basename.clone(),
                    });
                }
            }
            // UUID not in old = new file, skip
        }
        // UUIDs in old but not in current = deleted, skip (we only iterate current)

        renames
    }

    /// Detect renames in a folder doc and update wikilinks in all backlinkers.
    ///
    /// This is the server-level glue that:
    /// 1. Calls `detect_renames()` to diff filemeta against the cache
    /// 2. For each rename, reads backlinks to find source docs
    /// 3. Looks up each source doc in the DashMap and calls `update_wikilinks_in_doc()`
    /// Returns `true` if renames were detected and processed.
    fn apply_rename_updates(&self, folder_doc_id: &str, docs: &DashMap<String, DocWithSyncKv>) -> bool {
        // 1. Get the folder doc and detect renames
        let renames = {
            let Some(doc_ref) = docs.get(folder_doc_id) else {
                return false;
            };
            let awareness = doc_ref.awareness();
            let guard = awareness.read().unwrap();
            self.detect_renames(folder_doc_id, &guard.doc)
        };

        if renames.is_empty() {
            return false;
        }

        let Some((relay_id, _)) = parse_doc_id(folder_doc_id) else {
            tracing::error!("Invalid folder_doc_id format: {}", folder_doc_id);
            return false;
        };

        tracing::info!(
            "Detected {} rename(s) in folder doc {}",
            renames.len(),
            folder_doc_id
        );

        // 2. For each rename, read backlinks and update content docs
        for rename in &renames {
            // Read backlinks for the renamed UUID from the folder doc
            let source_uuids = {
                let Some(doc_ref) = docs.get(folder_doc_id) else {
                    continue;
                };
                let awareness = doc_ref.awareness();
                let guard = awareness.read().unwrap();
                let txn = guard.doc.transact();
                if let Some(backlinks) = txn.get_map("backlinks_v0") {
                    read_backlinks_array(&backlinks, &txn, &rename.uuid)
                } else {
                    Vec::new()
                }
            };

            if source_uuids.is_empty() {
                tracing::info!(
                    "Rename {} -> {}: no backlinkers for uuid {}",
                    rename.old_name, rename.new_name, rename.uuid
                );
                continue;
            }

            tracing::info!(
                "Rename {} -> {}: updating {} backlinker(s)",
                rename.old_name, rename.new_name, source_uuids.len()
            );

            // 3. Update wikilinks in each source doc
            for source_uuid in &source_uuids {
                let content_doc_id = format!("{}-{}", relay_id, source_uuid);
                let Some(content_ref) = docs.get(&content_doc_id) else {
                    tracing::warn!(
                        "Backlinker doc {} not loaded, skipping rename update",
                        content_doc_id
                    );
                    continue;
                };

                let awareness = content_ref.awareness();
                let guard = awareness.write().unwrap();
                match update_wikilinks_in_doc(&guard.doc, &rename.old_name, &rename.new_name) {
                    Ok(count) => {
                        tracing::info!(
                            "Updated {} wikilink(s) in {} ({} -> {})",
                            count, content_doc_id, rename.old_name, rename.new_name
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to update wikilinks in {}: {:?}",
                            content_doc_id, e
                        );
                    }
                }
            }
        }
        true
    }

    /// Background worker that processes the indexing queue.
    ///
    /// Folder docs skip debounce (metadata changes are discrete events, not typing)
    /// and trigger rename detection before re-queuing content docs.
    /// Content docs debounce as before (typing produces rapid updates).
    pub async fn run_worker(
        self: Arc<Self>,
        mut rx: mpsc::Receiver<String>,
        docs: Arc<DashMap<String, DocWithSyncKv>>,
    ) {
        tracing::info!("Link indexer worker started");
        loop {
            match rx.recv().await {
                Some(doc_id) => {
                    let folder_content = is_folder_doc(&doc_id, &docs);

                    if folder_content.is_none() {
                        // Content doc — debounce as before.
                        // Wait until no updates have arrived for DEBOUNCE_DURATION.
                        // on_document_update resets the timestamp on each call,
                        // so we loop until elapsed >= DEBOUNCE_DURATION.
                        loop {
                            tokio::time::sleep(DEBOUNCE_DURATION).await;
                            if self.is_ready(&doc_id) {
                                break;
                            }
                            if !self.pending.contains_key(&doc_id) {
                                break; // Entry removed externally, bail out
                            }
                        }

                        if !self.pending.contains_key(&doc_id) {
                            continue; // Was removed, skip processing
                        }
                    }

                    // Re-check folder status after debounce (content doc could have
                    // been removed, or we need the content UUIDs fresh).
                    if let Some(content_uuids) =
                        folder_content.or_else(|| is_folder_doc(&doc_id, &docs))
                    {
                        // Folder doc — process immediately (no debounce)

                        // 1. Detect renames BEFORE re-queuing content docs
                        let had_renames = self.apply_rename_updates(&doc_id, &docs);

                        // 2. Re-queue loaded content docs — but SKIP when renames were
                        //    processed.  The rename system already updated wikilinks in
                        //    backlinker content docs.  Re-queuing them for re-indexing
                        //    would race with the next rename: the debounced re-indexer
                        //    would try to resolve the OLD name against the NEW metadata,
                        //    fail, and clear the backlinks.  The next non-rename folder
                        //    doc update (add/delete/backlinks write-back) will still
                        //    trigger re-queuing normally.
                        if !had_renames {
                            tracing::info!(
                                "Folder doc {} has {} content docs, re-queuing loaded ones",
                                doc_id, content_uuids.len()
                            );
                            let relay_id = parse_doc_id(&doc_id)
                                .map(|(r, _)| r)
                                .unwrap_or(&doc_id[..36.min(doc_id.len())]);
                            for uuid in content_uuids {
                                let content_id = format!("{}-{}", relay_id, uuid);
                                if docs.contains_key(&content_id) {
                                    tracing::info!("Re-queuing content doc: {}", content_id);
                                    self.on_document_update(&content_id).await;
                                }
                            }
                        } else {
                            tracing::info!(
                                "Folder doc {}: skipping content re-queue after rename (avoids stale backlink race)",
                                doc_id
                            );
                        }
                    } else {
                        // Content doc — index it
                        tracing::info!("Indexing content doc: {}", doc_id);
                        match self.index_document(&doc_id, &docs) {
                            Ok(()) => tracing::info!("Successfully indexed: {}", doc_id),
                            Err(e) => tracing::error!("Failed to index {}: {:?}", doc_id, e),
                        }
                    }
                    self.mark_indexed(&doc_id);
                }
                None => break,
            }
        }
    }

    /// Server glue: unwraps DocWithSyncKv, delegates to core function.
    /// Resolves links across ALL loaded folder docs (cross-folder backlinks).
    fn index_document(
        &self,
        doc_id: &str,
        docs: &DashMap<String, DocWithSyncKv>,
    ) -> anyhow::Result<()> {
        let (_relay_id, doc_uuid) = parse_doc_id(doc_id)
            .ok_or_else(|| anyhow::anyhow!("Invalid doc_id format: {}", doc_id))?;

        // Find all folder docs so we can resolve cross-folder links
        let folder_doc_ids = find_all_folder_docs(docs);
        if folder_doc_ids.is_empty() {
            return Err(anyhow::anyhow!("No folder docs found for indexing"));
        }

        let content_ref = docs
            .get(doc_id)
            .ok_or_else(|| anyhow::anyhow!("Content doc not found: {}", doc_id))?;

        // Collect refs to all folder docs (need to hold DashMap guards)
        let folder_refs: Vec<_> = folder_doc_ids
            .iter()
            .filter_map(|id| docs.get(id))
            .collect();

        // Get Y.Docs from DocWithSyncKv via awareness
        let content_awareness = content_ref.awareness();
        let content_guard = content_awareness.read().unwrap();

        // Build awareness guards for all folder docs
        let folder_awarnesses: Vec<_> = folder_refs
            .iter()
            .map(|r| r.awareness())
            .collect();
        // SAFETY: We acquire write locks on ALL folder docs simultaneously.
        // This is safe because run_worker processes docs sequentially (single loop iteration).
        // Do NOT parallelize index_document calls without introducing lock ordering.
        let folder_guards: Vec<_> = folder_awarnesses
            .iter()
            .map(|a| a.write().unwrap())
            .collect();
        let folder_doc_refs: Vec<&Doc> = folder_guards
            .iter()
            .map(|g| &g.doc)
            .collect();

        index_content_into_folders(doc_uuid, &content_guard.doc, &folder_doc_refs)
    }

    /// Reindex all backlinks by scanning every loaded document.
    ///
    /// Iterates all docs in the DashMap, indexes each content doc's wikilinks,
    /// and updates backlinks_v0 in the corresponding folder doc(s).
    /// Call after loading docs from storage on startup.
    pub fn reindex_all_backlinks(
        &self,
        docs: &DashMap<String, DocWithSyncKv>,
    ) -> anyhow::Result<()> {
        tracing::info!("Reindexing all backlinks...");
        let mut indexed = 0;
        let mut skipped = 0;

        for entry in docs.iter() {
            let doc_id = entry.key();
            match self.index_document(doc_id, docs) {
                Ok(()) => indexed += 1,
                Err(_) => skipped += 1, // Not all docs are content docs
            }
        }

        tracing::info!(
            "Backlink reindexing complete: {} content docs indexed, {} skipped",
            indexed,
            skipped
        );

        // Seed filemeta cache for all folder docs.
        // This prevents false renames on the first folder doc update after startup
        // (without a cached snapshot, detect_renames would have no baseline to diff against).
        let folder_doc_ids = find_all_folder_docs(docs);
        for folder_doc_id in &folder_doc_ids {
            if let Some(doc_ref) = docs.get(folder_doc_id) {
                let awareness = doc_ref.awareness();
                let guard = awareness.read().unwrap();
                self.detect_renames(folder_doc_id, &guard.doc);
            }
        }
        tracing::info!("Seeded filemeta cache for {} folder docs", folder_doc_ids.len());

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use yrs::{Doc, GetString, Map, Text, Transact, WriteTxn, Any};

    // === Debounce pipeline tests ===

    #[tokio::test]
    async fn rapid_updates_send_single_channel_message() {
        let (indexer, mut rx) = LinkIndexer::new();

        // Simulate rapid updates (like typing in editor)
        indexer.on_document_update("doc-1").await;
        indexer.on_document_update("doc-1").await;
        indexer.on_document_update("doc-1").await;

        // Should have exactly one message in channel (not three)
        assert!(rx.try_recv().is_ok(), "should have one message");
        assert!(rx.try_recv().is_err(), "should not have more messages");
    }

    #[tokio::test]
    async fn debounce_completes_after_updates_settle() {
        let (indexer, _rx) = LinkIndexer::new();

        // First update
        indexer.on_document_update("doc-1").await;

        // Simulate rapid updates over 500ms
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            indexer.on_document_update("doc-1").await;
        }

        // Not ready yet (last update was just now)
        assert!(!indexer.is_ready("doc-1"), "should not be ready during rapid updates");

        // Wait for full debounce duration
        tokio::time::sleep(DEBOUNCE_DURATION + Duration::from_millis(100)).await;

        // Now should be ready (no updates during the wait)
        assert!(indexer.is_ready("doc-1"), "should be ready after debounce settles");
    }

    #[tokio::test]
    async fn new_updates_after_indexing_requeue() {
        let (indexer, mut rx) = LinkIndexer::new();

        // First update cycle
        indexer.on_document_update("doc-1").await;
        assert!(rx.try_recv().is_ok());

        // Simulate indexing complete
        indexer.mark_indexed("doc-1");

        // New update should send a new channel message (not suppressed)
        indexer.on_document_update("doc-1").await;
        assert!(rx.try_recv().is_ok(), "should queue new message after mark_indexed");
    }

    // === parse_doc_id tests ===

    #[test]
    fn parses_valid_doc_id() {
        let doc_id = "cb696037-0f72-4e93-8717-4e433129d789-f7c85d0f-8bb4-4a03-80b5-408498d77c52";
        let result = parse_doc_id(doc_id);
        assert_eq!(
            result,
            Some((
                "cb696037-0f72-4e93-8717-4e433129d789",
                "f7c85d0f-8bb4-4a03-80b5-408498d77c52"
            ))
        );
    }

    #[test]
    fn returns_none_for_invalid_format() {
        assert_eq!(parse_doc_id("too-short"), None);
        assert_eq!(parse_doc_id(""), None);
    }

    // === Test Helpers ===

    /// Create a folder Y.Doc with filemeta_v0 populated.
    /// entries: &[("/path.md", "uuid")]
    ///
    /// Uses Any::Map (plain objects) to match what real Y.js clients produce.
    /// JavaScript `ymap.set(key, { id, type, version })` stores as Any::Map,
    /// not as a nested YMap.
    fn create_folder_doc(entries: &[(&str, &str)]) -> Doc {
        let doc = Doc::new();
        {
            let mut txn = doc.transact_mut();
            let filemeta = txn.get_or_insert_map("filemeta_v0");
            for (path, uuid) in entries {
                let mut map = HashMap::new();
                map.insert("id".to_string(), Any::String((*uuid).into()));
                map.insert("type".to_string(), Any::String("markdown".into()));
                map.insert("version".to_string(), Any::Number(0.0));
                filemeta.insert(&mut txn, *path, Any::Map(map.into()));
            }
        }
        doc
    }

    /// Create a content Y.Doc with Y.Text("contents").
    fn create_content_doc(markdown: &str) -> Doc {
        let doc = Doc::new();
        {
            let mut txn = doc.transact_mut();
            let text = txn.get_or_insert_text("contents");
            text.insert(&mut txn, 0, markdown);
        }
        doc
    }

    /// Read backlinks_v0 for a given target UUID from a folder doc.
    fn read_backlinks(folder_doc: &Doc, target_uuid: &str) -> Vec<String> {
        let txn = folder_doc.transact();
        if let Some(backlinks) = txn.get_map("backlinks_v0") {
            read_backlinks_array(&backlinks, &txn, target_uuid)
        } else {
            vec![]
        }
    }

    // === Unit+1 Tests (RED — these FAIL with stub index_content_into_folder) ===

    #[test]
    fn indexes_wikilink_into_backlinks() {
        // Setup: folder with two docs, content doc links to the other
        let folder_doc = create_folder_doc(&[
            ("/Notes.md", "uuid-notes"),
            ("/Ideas.md", "uuid-ideas"),
        ]);
        let content_doc = create_content_doc("See [[Ideas]] for more");

        // Act: index the content doc
        let result = index_content_into_folder("uuid-notes", &content_doc, &folder_doc);
        assert!(result.is_ok());

        // Assert: Ideas' backlinks contain Notes
        let backlinks = read_backlinks(&folder_doc, "uuid-ideas");
        assert_eq!(backlinks, vec!["uuid-notes"]);
    }

    #[test]
    fn reindex_after_adding_link() {
        let folder_doc = create_folder_doc(&[
            ("/Notes.md", "uuid-notes"),
            ("/Ideas.md", "uuid-ideas"),
            ("/Other.md", "uuid-other"),
        ]);
        let content_doc = create_content_doc("See [[Ideas]]");

        // First index
        index_content_into_folder("uuid-notes", &content_doc, &folder_doc).unwrap();

        // Edit: add another link
        {
            let mut txn = content_doc.transact_mut();
            let text = txn.get_or_insert_text("contents");
            let len = text.get_string(&txn).len();
            text.insert(&mut txn, len as u32, " and [[Other]]");
        }

        // Re-index
        index_content_into_folder("uuid-notes", &content_doc, &folder_doc).unwrap();

        // Assert: both targets have Notes as backlink
        assert_eq!(read_backlinks(&folder_doc, "uuid-ideas"), vec!["uuid-notes"]);
        assert_eq!(read_backlinks(&folder_doc, "uuid-other"), vec!["uuid-notes"]);
    }

    #[test]
    fn reindex_after_removing_link_cleans_stale() {
        let folder_doc = create_folder_doc(&[
            ("/Notes.md", "uuid-notes"),
            ("/Ideas.md", "uuid-ideas"),
            ("/Other.md", "uuid-other"),
        ]);
        let content_doc = create_content_doc("[[Ideas]] and [[Other]]");

        // First index: both targets have backlinks
        index_content_into_folder("uuid-notes", &content_doc, &folder_doc).unwrap();
        assert_eq!(read_backlinks(&folder_doc, "uuid-ideas"), vec!["uuid-notes"]);
        assert_eq!(read_backlinks(&folder_doc, "uuid-other"), vec!["uuid-notes"]);

        // Edit: remove the Other link
        {
            let mut txn = content_doc.transact_mut();
            let text = txn.get_or_insert_text("contents");
            // Replace entire content
            let len = text.get_string(&txn).len();
            text.remove_range(&mut txn, 0, len as u32);
            text.insert(&mut txn, 0, "[[Ideas]] only now");
        }

        // Re-index
        index_content_into_folder("uuid-notes", &content_doc, &folder_doc).unwrap();

        // Assert: Ideas still has backlink, Other's backlink is gone
        assert_eq!(read_backlinks(&folder_doc, "uuid-ideas"), vec!["uuid-notes"]);
        assert!(read_backlinks(&folder_doc, "uuid-other").is_empty());
    }

    #[test]
    fn multiple_sources_to_same_target() {
        let folder_doc = create_folder_doc(&[
            ("/Notes.md", "uuid-notes"),
            ("/Projects.md", "uuid-projects"),
            ("/Ideas.md", "uuid-ideas"),
        ]);
        let notes_doc = create_content_doc("See [[Ideas]]");
        let projects_doc = create_content_doc("Related: [[Ideas]]");

        // Index both source docs
        index_content_into_folder("uuid-notes", &notes_doc, &folder_doc).unwrap();
        index_content_into_folder("uuid-projects", &projects_doc, &folder_doc).unwrap();

        // Assert: Ideas has both as backlinks
        let mut backlinks = read_backlinks(&folder_doc, "uuid-ideas");
        backlinks.sort();
        assert_eq!(backlinks, vec!["uuid-notes", "uuid-projects"]);
    }

    #[test]
    fn unresolvable_link_skipped() {
        let folder_doc = create_folder_doc(&[("/Notes.md", "uuid-notes")]);
        let content_doc = create_content_doc("See [[NoSuchDoc]]");

        // Should not crash
        let result = index_content_into_folder("uuid-notes", &content_doc, &folder_doc);
        assert!(result.is_ok());

        // Assert: no backlinks created for non-existent target
        let txn = folder_doc.transact();
        let backlinks = txn.get_map("backlinks_v0");
        // backlinks_v0 should either not exist or be empty
        assert!(backlinks.is_none() || backlinks.unwrap().len(&txn) == 0);
    }

    #[test]
    fn ignores_links_in_code_blocks() {
        let folder_doc = create_folder_doc(&[
            ("/Notes.md", "uuid-notes"),
            ("/Fake.md", "uuid-fake"),
            ("/Real.md", "uuid-real"),
        ]);
        let content_doc = create_content_doc("```\n[[Fake]]\n```\n[[Real]]");

        index_content_into_folder("uuid-notes", &content_doc, &folder_doc).unwrap();

        // Assert: Real has backlink, Fake does not
        assert_eq!(read_backlinks(&folder_doc, "uuid-real"), vec!["uuid-notes"]);
        assert!(read_backlinks(&folder_doc, "uuid-fake").is_empty());
    }

    #[test]
    fn no_links_no_backlinks() {
        let folder_doc = create_folder_doc(&[
            ("/Notes.md", "uuid-notes"),
            ("/Other.md", "uuid-other"),
        ]);
        let content_doc = create_content_doc("Just plain text, no links");

        let result = index_content_into_folder("uuid-notes", &content_doc, &folder_doc);
        assert!(result.is_ok());

        // Assert: no backlinks_v0 entries
        assert!(read_backlinks(&folder_doc, "uuid-other").is_empty());
    }

    // === Subdirectory wikilink resolution tests ===

    #[test]
    fn resolves_wikilink_to_file_in_subdirectory() {
        let folder_doc = create_folder_doc(&[
            ("/Notes/Ideas.md", "uuid-ideas"),
            ("/Projects/Todo.md", "uuid-todo"),
        ]);
        let content_doc = create_content_doc("See [[Ideas]] for details.");

        index_content_into_folder("uuid-source", &content_doc, &folder_doc).unwrap();

        let backlinks = read_backlinks(&folder_doc, "uuid-ideas");
        assert_eq!(backlinks, vec!["uuid-source"]);
    }

    #[test]
    fn resolves_wikilink_case_insensitive_in_subdirectory() {
        let folder_doc = create_folder_doc(&[("/Notes/ideas.md", "uuid-ideas")]);
        let content_doc = create_content_doc("See [[Ideas]] for details.");
        index_content_into_folder("uuid-source", &content_doc, &folder_doc).unwrap();
        assert_eq!(read_backlinks(&folder_doc, "uuid-ideas"), vec!["uuid-source"]);
    }

    #[test]
    fn prefers_exact_root_match_over_basename_match() {
        // /Ideas.md should win over /Notes/Ideas.md for [[Ideas]]
        let folder_doc = create_folder_doc(&[
            ("/Ideas.md", "uuid-root"),
            ("/Notes/Ideas.md", "uuid-nested"),
        ]);
        let content_doc = create_content_doc("See [[Ideas]].");
        index_content_into_folder("uuid-source", &content_doc, &folder_doc).unwrap();
        let root_backlinks = read_backlinks(&folder_doc, "uuid-root");
        let nested_backlinks = read_backlinks(&folder_doc, "uuid-nested");
        assert_eq!(root_backlinks, vec!["uuid-source"]);
        assert!(nested_backlinks.is_empty());
    }

    #[test]
    fn resolves_explicit_path_wikilink() {
        // [[Notes/Ideas]] should match /Notes/Ideas.md
        let folder_doc = create_folder_doc(&[("/Notes/Ideas.md", "uuid-ideas")]);
        let content_doc = create_content_doc("See [[Notes/Ideas]].");
        index_content_into_folder("uuid-source", &content_doc, &folder_doc).unwrap();
        assert_eq!(read_backlinks(&folder_doc, "uuid-ideas"), vec!["uuid-source"]);
    }

    // === Cross-folder backlink tests ===

    #[test]
    fn cross_folder_link_creates_backlink_in_target_folder() {
        // Folder A contains source doc
        let folder_a = create_folder_doc(&[("/Welcome.md", "uuid-welcome")]);
        // Folder B contains target doc
        let folder_b = create_folder_doc(&[("/Syllabus.md", "uuid-syllabus")]);
        // Source doc links to target
        let content_doc = create_content_doc("See [[Syllabus]] for the course plan.");

        index_content_into_folders(
            "uuid-welcome",
            &content_doc,
            &[&folder_a, &folder_b],
        ).unwrap();

        // Backlink should be in folder B (the target's folder), NOT folder A
        let backlinks_b = read_backlinks(&folder_b, "uuid-syllabus");
        assert_eq!(backlinks_b, vec!["uuid-welcome"]);
        // Folder A should have no backlinks for uuid-syllabus (it's not in folder A)
        let backlinks_a = read_backlinks(&folder_a, "uuid-syllabus");
        assert!(backlinks_a.is_empty());
    }

    #[test]
    fn cross_folder_link_removal_cleans_target_folder() {
        let folder_a = create_folder_doc(&[("/Welcome.md", "uuid-welcome")]);
        let folder_b = create_folder_doc(&[("/Syllabus.md", "uuid-syllabus")]);

        // First: create the link
        let content_v1 = create_content_doc("See [[Syllabus]].");
        index_content_into_folders("uuid-welcome", &content_v1, &[&folder_a, &folder_b]).unwrap();
        assert_eq!(read_backlinks(&folder_b, "uuid-syllabus"), vec!["uuid-welcome"]);

        // Then: remove the link
        let content_v2 = create_content_doc("No links here.");
        index_content_into_folders("uuid-welcome", &content_v2, &[&folder_a, &folder_b]).unwrap();
        assert!(read_backlinks(&folder_b, "uuid-syllabus").is_empty());
    }

    #[test]
    fn within_folder_link_still_works_with_multi_folder() {
        // Same-folder links should still work when multiple folders are passed
        let folder_a = create_folder_doc(&[
            ("/Notes.md", "uuid-notes"),
            ("/Ideas.md", "uuid-ideas"),
        ]);
        let folder_b = create_folder_doc(&[("/Syllabus.md", "uuid-syllabus")]);
        let content_doc = create_content_doc("See [[Ideas]].");

        index_content_into_folders("uuid-notes", &content_doc, &[&folder_a, &folder_b]).unwrap();

        assert_eq!(read_backlinks(&folder_a, "uuid-ideas"), vec!["uuid-notes"]);
        assert!(read_backlinks(&folder_b, "uuid-ideas").is_empty());
    }

    #[test]
    fn link_to_docs_in_multiple_folders() {
        // One source links to targets in both folders
        let folder_a = create_folder_doc(&[
            ("/Welcome.md", "uuid-welcome"),
            ("/Resources.md", "uuid-resources"),
        ]);
        let folder_b = create_folder_doc(&[("/Syllabus.md", "uuid-syllabus")]);
        let content_doc = create_content_doc("See [[Syllabus]] and [[Resources]].");

        index_content_into_folders("uuid-welcome", &content_doc, &[&folder_a, &folder_b]).unwrap();

        assert_eq!(read_backlinks(&folder_b, "uuid-syllabus"), vec!["uuid-welcome"]);
        assert_eq!(read_backlinks(&folder_a, "uuid-resources"), vec!["uuid-welcome"]);
    }

    // === update_wikilinks_in_doc tests ===

    /// Helper: read Y.Text("contents") from a doc as a String.
    fn read_contents(doc: &Doc) -> String {
        let txn = doc.transact();
        txn.get_text("contents").unwrap().get_string(&txn)
    }

    #[test]
    fn replaces_simple_wikilink_in_ydoc() {
        let doc = create_content_doc("See [[Foo]] here");
        let count = update_wikilinks_in_doc(&doc, "Foo", "Bar").unwrap();
        assert_eq!(count, 1);
        assert_eq!(read_contents(&doc), "See [[Bar]] here");
    }

    #[test]
    fn replaces_wikilink_with_anchor_in_ydoc() {
        let doc = create_content_doc("[[Foo#Section]]");
        let count = update_wikilinks_in_doc(&doc, "Foo", "Bar").unwrap();
        assert_eq!(count, 1);
        assert_eq!(read_contents(&doc), "[[Bar#Section]]");
    }

    #[test]
    fn replaces_wikilink_with_alias_in_ydoc() {
        let doc = create_content_doc("[[Foo|Display]]");
        let count = update_wikilinks_in_doc(&doc, "Foo", "Bar").unwrap();
        assert_eq!(count, 1);
        assert_eq!(read_contents(&doc), "[[Bar|Display]]");
    }

    #[test]
    fn replaces_multiple_wikilinks_in_ydoc() {
        let doc = create_content_doc("[[Foo]] and [[Foo#Sec]]");
        let count = update_wikilinks_in_doc(&doc, "Foo", "Bar").unwrap();
        assert_eq!(count, 2);
        assert_eq!(read_contents(&doc), "[[Bar]] and [[Bar#Sec]]");
    }

    #[test]
    fn returns_zero_for_no_matches() {
        let doc = create_content_doc("[[Other]]");
        let count = update_wikilinks_in_doc(&doc, "Foo", "Bar").unwrap();
        assert_eq!(count, 0);
        assert_eq!(read_contents(&doc), "[[Other]]");
    }

    #[test]
    fn skips_code_blocks_in_ydoc() {
        let doc = create_content_doc("```\n[[Foo]]\n```\n[[Foo]]");
        let count = update_wikilinks_in_doc(&doc, "Foo", "Bar").unwrap();
        assert_eq!(count, 1);
        assert_eq!(read_contents(&doc), "```\n[[Foo]]\n```\n[[Bar]]");
    }

    #[test]
    fn handles_multibyte_chars_before_wikilink() {
        // U+00E9 (é) is 2 bytes in UTF-8 but 1 char in UTF-32
        let doc = create_content_doc("caf\u{00e9} [[Foo]] end");
        let count = update_wikilinks_in_doc(&doc, "Foo", "Bar").unwrap();
        assert_eq!(count, 1);
        assert_eq!(read_contents(&doc), "caf\u{00e9} [[Bar]] end");
    }

    // === detect_renames tests ===

    #[test]
    fn first_call_seeds_cache_returns_empty() {
        let (indexer, _rx) = LinkIndexer::new();
        let folder_doc = create_folder_doc(&[
            ("/Foo.md", "uuid-1"),
            ("/Bar.md", "uuid-2"),
        ]);

        let renames = indexer.detect_renames("folder-1", &folder_doc);
        assert!(renames.is_empty(), "first call should seed cache and return empty");
    }

    #[test]
    fn detects_basename_rename() {
        let (indexer, _rx) = LinkIndexer::new();

        // Seed cache with initial state
        let folder_v1 = create_folder_doc(&[("/Foo.md", "uuid-1")]);
        let renames = indexer.detect_renames("folder-1", &folder_v1);
        assert!(renames.is_empty(), "seed call should return empty");

        // Now rename: /Foo.md -> /Bar.md (same uuid)
        let folder_v2 = create_folder_doc(&[("/Bar.md", "uuid-1")]);
        let renames = indexer.detect_renames("folder-1", &folder_v2);
        assert_eq!(renames.len(), 1);
        assert_eq!(renames[0].uuid, "uuid-1");
        assert_eq!(renames[0].old_name, "Foo");
        assert_eq!(renames[0].new_name, "Bar");
    }

    #[test]
    fn ignores_folder_move_same_basename() {
        let (indexer, _rx) = LinkIndexer::new();

        // Seed: file in /Notes/Foo.md
        let folder_v1 = create_folder_doc(&[("/Notes/Foo.md", "uuid-1")]);
        indexer.detect_renames("folder-1", &folder_v1);

        // Move to /Archive/Foo.md — same basename "Foo"
        let folder_v2 = create_folder_doc(&[("/Archive/Foo.md", "uuid-1")]);
        let renames = indexer.detect_renames("folder-1", &folder_v2);
        assert!(renames.is_empty(), "folder move with same basename should not be a rename");
    }

    #[test]
    fn detects_multiple_renames() {
        let (indexer, _rx) = LinkIndexer::new();

        // Seed
        let folder_v1 = create_folder_doc(&[
            ("/Foo.md", "uuid-1"),
            ("/Bar.md", "uuid-2"),
        ]);
        indexer.detect_renames("folder-1", &folder_v1);

        // Rename both
        let folder_v2 = create_folder_doc(&[
            ("/Baz.md", "uuid-1"),
            ("/Qux.md", "uuid-2"),
        ]);
        let mut renames = indexer.detect_renames("folder-1", &folder_v2);
        renames.sort_by(|a, b| a.uuid.cmp(&b.uuid));
        assert_eq!(renames.len(), 2);
        assert_eq!(renames[0].uuid, "uuid-1");
        assert_eq!(renames[0].old_name, "Foo");
        assert_eq!(renames[0].new_name, "Baz");
        assert_eq!(renames[1].uuid, "uuid-2");
        assert_eq!(renames[1].old_name, "Bar");
        assert_eq!(renames[1].new_name, "Qux");
    }

    #[test]
    fn ignores_new_files() {
        let (indexer, _rx) = LinkIndexer::new();

        // Seed with one file
        let folder_v1 = create_folder_doc(&[("/Foo.md", "uuid-1")]);
        indexer.detect_renames("folder-1", &folder_v1);

        // Add a new file (new UUID)
        let folder_v2 = create_folder_doc(&[
            ("/Foo.md", "uuid-1"),
            ("/NewFile.md", "uuid-2"),
        ]);
        let renames = indexer.detect_renames("folder-1", &folder_v2);
        assert!(renames.is_empty(), "new files should not produce rename events");
    }

    #[test]
    fn ignores_deleted_files() {
        let (indexer, _rx) = LinkIndexer::new();

        // Seed with two files
        let folder_v1 = create_folder_doc(&[
            ("/Foo.md", "uuid-1"),
            ("/Bar.md", "uuid-2"),
        ]);
        indexer.detect_renames("folder-1", &folder_v1);

        // Remove uuid-2
        let folder_v2 = create_folder_doc(&[("/Foo.md", "uuid-1")]);
        let renames = indexer.detect_renames("folder-1", &folder_v2);
        assert!(renames.is_empty(), "deleted files should not produce rename events");
    }

    // === Rename pipeline integration tests ===
    // These test the full pipeline: detect_renames + read backlinks + update_wikilinks_in_doc

    #[test]
    fn rename_updates_wikilinks_in_backlinkers() {
        // 1. Create folder with Foo.md (uuid-foo) and Notes.md (uuid-notes)
        let folder_doc = create_folder_doc(&[
            ("/Foo.md", "uuid-foo"),
            ("/Notes.md", "uuid-notes"),
        ]);

        // 2. Create content doc for Notes with a link to Foo
        let notes_doc = create_content_doc("See [[Foo]] for details");

        // 3. Index Notes -> backlinks_v0[uuid-foo] = [uuid-notes]
        index_content_into_folder("uuid-notes", &notes_doc, &folder_doc).unwrap();
        assert_eq!(read_backlinks(&folder_doc, "uuid-foo"), vec!["uuid-notes"]);

        // 4. Seed the indexer's filemeta cache
        let (indexer, _rx) = LinkIndexer::new();
        let renames = indexer.detect_renames("folder-1", &folder_doc);
        assert!(renames.is_empty(), "seed call should return empty");

        // 5. Rename Foo -> Bar in filemeta (delete /Foo.md, add /Bar.md with same uuid)
        {
            let mut txn = folder_doc.transact_mut();
            let filemeta = txn.get_or_insert_map("filemeta_v0");
            filemeta.remove(&mut txn, "/Foo.md");
            let mut map = HashMap::new();
            map.insert("id".to_string(), Any::String("uuid-foo".into()));
            map.insert("type".to_string(), Any::String("markdown".into()));
            map.insert("version".to_string(), Any::Number(0.0));
            filemeta.insert(&mut txn, "/Bar.md", Any::Map(map.into()));
        }

        // 6. Detect renames
        let renames = indexer.detect_renames("folder-1", &folder_doc);
        assert_eq!(renames.len(), 1);
        assert_eq!(renames[0].uuid, "uuid-foo");
        assert_eq!(renames[0].old_name, "Foo");
        assert_eq!(renames[0].new_name, "Bar");

        // 7. For each rename, read backlinks and update wikilinks
        for rename in &renames {
            let txn = folder_doc.transact();
            let backlinks = txn.get_map("backlinks_v0").unwrap();
            let source_uuids = read_backlinks_array(&backlinks, &txn, &rename.uuid);
            drop(txn);

            // In a real scenario we'd look up the content doc by doc_id;
            // here we just match uuid-notes to notes_doc directly
            for source_uuid in &source_uuids {
                if source_uuid == "uuid-notes" {
                    update_wikilinks_in_doc(&notes_doc, &rename.old_name, &rename.new_name)
                        .unwrap();
                }
            }
        }

        // 8. Assert: Notes content now has [[Bar]] instead of [[Foo]]
        assert_eq!(read_contents(&notes_doc), "See [[Bar]] for details");
    }

    #[test]
    fn rename_preserves_anchors_and_aliases_in_backlinkers() {
        let folder_doc = create_folder_doc(&[
            ("/Foo.md", "uuid-foo"),
            ("/Notes.md", "uuid-notes"),
        ]);

        // Notes has both anchor and alias links to Foo
        let notes_doc = create_content_doc("See [[Foo#Section]] and [[Foo|Display]]");
        index_content_into_folder("uuid-notes", &notes_doc, &folder_doc).unwrap();

        // Seed cache
        let (indexer, _rx) = LinkIndexer::new();
        indexer.detect_renames("folder-1", &folder_doc);

        // Rename Foo -> Bar
        {
            let mut txn = folder_doc.transact_mut();
            let filemeta = txn.get_or_insert_map("filemeta_v0");
            filemeta.remove(&mut txn, "/Foo.md");
            let mut map = HashMap::new();
            map.insert("id".to_string(), Any::String("uuid-foo".into()));
            map.insert("type".to_string(), Any::String("markdown".into()));
            map.insert("version".to_string(), Any::Number(0.0));
            filemeta.insert(&mut txn, "/Bar.md", Any::Map(map.into()));
        }

        let renames = indexer.detect_renames("folder-1", &folder_doc);
        assert_eq!(renames.len(), 1);

        for rename in &renames {
            let txn = folder_doc.transact();
            let backlinks = txn.get_map("backlinks_v0").unwrap();
            let source_uuids = read_backlinks_array(&backlinks, &txn, &rename.uuid);
            drop(txn);

            for source_uuid in &source_uuids {
                if source_uuid == "uuid-notes" {
                    update_wikilinks_in_doc(&notes_doc, &rename.old_name, &rename.new_name)
                        .unwrap();
                }
            }
        }

        // Assert anchors and aliases preserved
        assert_eq!(
            read_contents(&notes_doc),
            "See [[Bar#Section]] and [[Bar|Display]]"
        );
    }

    #[test]
    fn rename_with_no_backlinkers_is_noop() {
        let folder_doc = create_folder_doc(&[
            ("/Foo.md", "uuid-foo"),
            ("/Notes.md", "uuid-notes"),
        ]);

        // Notes has NO link to Foo, so Foo has no backlinks
        let notes_doc = create_content_doc("Just some text");
        index_content_into_folder("uuid-notes", &notes_doc, &folder_doc).unwrap();
        assert!(read_backlinks(&folder_doc, "uuid-foo").is_empty());

        // Seed cache and rename Foo -> Bar
        let (indexer, _rx) = LinkIndexer::new();
        indexer.detect_renames("folder-1", &folder_doc);

        {
            let mut txn = folder_doc.transact_mut();
            let filemeta = txn.get_or_insert_map("filemeta_v0");
            filemeta.remove(&mut txn, "/Foo.md");
            let mut map = HashMap::new();
            map.insert("id".to_string(), Any::String("uuid-foo".into()));
            map.insert("type".to_string(), Any::String("markdown".into()));
            map.insert("version".to_string(), Any::Number(0.0));
            filemeta.insert(&mut txn, "/Bar.md", Any::Map(map.into()));
        }

        let renames = indexer.detect_renames("folder-1", &folder_doc);
        assert_eq!(renames.len(), 1);

        // Read backlinks for the renamed UUID — empty, so no content docs to update
        let txn = folder_doc.transact();
        let backlinks = txn.get_map("backlinks_v0");
        let source_uuids = if let Some(bl) = backlinks {
            read_backlinks_array(&bl, &txn, &renames[0].uuid)
        } else {
            vec![]
        };
        drop(txn);

        assert!(source_uuids.is_empty(), "no backlinkers means nothing to update");
        // Notes content unchanged
        assert_eq!(read_contents(&notes_doc), "Just some text");
    }

    #[test]
    fn rename_leaves_unrelated_links_untouched() {
        let folder_doc = create_folder_doc(&[
            ("/Foo.md", "uuid-foo"),
            ("/Other.md", "uuid-other"),
            ("/Notes.md", "uuid-notes"),
        ]);

        // Notes links to both Foo and Other
        let notes_doc = create_content_doc("See [[Foo]] and [[Other]]");
        index_content_into_folder("uuid-notes", &notes_doc, &folder_doc).unwrap();
        assert_eq!(read_backlinks(&folder_doc, "uuid-foo"), vec!["uuid-notes"]);
        assert_eq!(read_backlinks(&folder_doc, "uuid-other"), vec!["uuid-notes"]);

        // Seed cache and rename only Foo -> Bar
        let (indexer, _rx) = LinkIndexer::new();
        indexer.detect_renames("folder-1", &folder_doc);

        {
            let mut txn = folder_doc.transact_mut();
            let filemeta = txn.get_or_insert_map("filemeta_v0");
            filemeta.remove(&mut txn, "/Foo.md");
            let mut map = HashMap::new();
            map.insert("id".to_string(), Any::String("uuid-foo".into()));
            map.insert("type".to_string(), Any::String("markdown".into()));
            map.insert("version".to_string(), Any::Number(0.0));
            filemeta.insert(&mut txn, "/Bar.md", Any::Map(map.into()));
        }

        let renames = indexer.detect_renames("folder-1", &folder_doc);
        assert_eq!(renames.len(), 1);

        for rename in &renames {
            let txn = folder_doc.transact();
            let backlinks = txn.get_map("backlinks_v0").unwrap();
            let source_uuids = read_backlinks_array(&backlinks, &txn, &rename.uuid);
            drop(txn);

            for source_uuid in &source_uuids {
                if source_uuid == "uuid-notes" {
                    update_wikilinks_in_doc(&notes_doc, &rename.old_name, &rename.new_name)
                        .unwrap();
                }
            }
        }

        // Assert: [[Foo]] became [[Bar]], but [[Other]] is untouched
        assert_eq!(read_contents(&notes_doc), "See [[Bar]] and [[Other]]");
    }
}
