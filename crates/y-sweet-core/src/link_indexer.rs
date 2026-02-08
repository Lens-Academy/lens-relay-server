use crate::doc_sync::DocWithSyncKv;
use crate::link_parser::extract_wikilinks;
use dashmap::DashMap;
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};
use yrs::{Any, Doc, GetString, Map, MapRef, Out, ReadTxn, Transact, WriteTxn};

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
// IndexingGuard — thread-local flag to prevent infinite loops
// ---------------------------------------------------------------------------

thread_local! {
    /// Flag to prevent infinite loop when indexer writes trigger observer
    static INDEXING_IN_PROGRESS: RefCell<bool> = RefCell::new(false);
}

/// Check if we should index this update (not from our own write)
pub fn should_index() -> bool {
    INDEXING_IN_PROGRESS.with(|flag| !*flag.borrow())
}

/// Guard that sets flag during indexer writes. Automatically clears on Drop.
pub struct IndexingGuard;

impl IndexingGuard {
    pub fn new() -> Self {
        INDEXING_IN_PROGRESS.with(|flag| *flag.borrow_mut() = true);
        Self
    }
}

impl Drop for IndexingGuard {
    fn drop(&mut self) {
        INDEXING_IN_PROGRESS.with(|flag| *flag.borrow_mut() = false);
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
fn extract_id_from_filemeta_entry(value: &Out, txn: &impl ReadTxn) -> Option<String> {
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
fn read_backlinks_array(backlinks: &MapRef, txn: &impl ReadTxn, target_uuid: &str) -> Vec<String> {
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
fn find_all_folder_docs(docs: &DashMap<String, DocWithSyncKv>) -> Vec<String> {
    docs.iter()
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
        .collect()
}

/// Check if a doc is a folder doc (has non-empty filemeta_v0).
/// Returns the list of content doc UUIDs listed in filemeta_v0, or None.
fn is_folder_doc(doc_id: &str, docs: &DashMap<String, DocWithSyncKv>) -> Option<Vec<String>> {
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
    let _guard = IndexingGuard::new();
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
// LinkIndexer — async server-side struct with debounced worker
// ---------------------------------------------------------------------------

const DEBOUNCE_DURATION: Duration = Duration::from_secs(2);

pub struct LinkIndexer {
    pending: Arc<DashMap<String, Instant>>,
    index_tx: mpsc::Sender<String>,
}

impl LinkIndexer {
    pub fn new() -> (Self, mpsc::Receiver<String>) {
        let (index_tx, index_rx) = mpsc::channel(1000);
        (
            Self {
                pending: Arc::new(DashMap::new()),
                index_tx,
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
            let _ = self.index_tx.send(doc_id.to_string()).await;
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

    /// Background worker that processes the indexing queue.
    pub async fn run_worker(
        self: Arc<Self>,
        mut rx: mpsc::Receiver<String>,
        docs: Arc<DashMap<String, DocWithSyncKv>>,
    ) {
        tracing::info!("Link indexer worker started");
        loop {
            match rx.recv().await {
                Some(doc_id) => {
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

                    if let Some(content_uuids) = is_folder_doc(&doc_id, &docs) {
                        // Folder doc updated — re-queue loaded content docs
                        tracing::info!(
                            "Folder doc {} has {} content docs, re-queuing loaded ones",
                            doc_id, content_uuids.len()
                        );
                        let relay_id = &doc_id[..36];
                        for uuid in content_uuids {
                            let content_id = format!("{}-{}", relay_id, uuid);
                            if docs.contains_key(&content_id) {
                                tracing::info!("Re-queuing content doc: {}", content_id);
                                self.on_document_update(&content_id).await;
                            }
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
        let folder_guards: Vec<_> = folder_awarnesses
            .iter()
            .map(|a| a.read().unwrap())
            .collect();
        let folder_doc_refs: Vec<&Doc> = folder_guards
            .iter()
            .map(|g| &g.doc)
            .collect();

        index_content_into_folders(doc_uuid, &content_guard.doc, &folder_doc_refs)
    }

    /// Rebuild the entire backlinks index on startup.
    pub fn rebuild_all(
        &self,
        docs: &DashMap<String, DocWithSyncKv>,
    ) -> anyhow::Result<()> {
        tracing::info!("Rebuilding backlinks index...");
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
            "Backlinks index rebuild complete: {} indexed, {} skipped",
            indexed,
            skipped
        );
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
}
