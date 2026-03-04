use crate::doc_resolver::{read_folder_name, DocInfo, DocumentResolver};
use crate::doc_sync::DocWithSyncKv;
use crate::link_parser::{
    compute_wikilink_move_edits, compute_wikilink_rename_edits,
    compute_wikilink_rename_edits_resolved, extract_wikilinks,
};
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

/// Resolve a page name relative to the directory containing `current_file_path`.
/// Returns an absolute filemeta path with `.md` extension.
///
/// Port of frontend's `resolveRelative()` in document-resolver.ts.
///
/// Example: `resolve_relative("/Notes/Source.md", "../Ideas")` → `"/Ideas.md"`
pub fn resolve_relative(current_file_path: &str, page_name: &str) -> String {
    let last_slash = current_file_path.rfind('/').unwrap_or(0);
    let dir = &current_file_path[..last_slash];
    let mut segments: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();

    for part in page_name.split('/') {
        if part == ".." {
            if !segments.is_empty() {
                segments.pop();
            }
        } else if part != "." && !part.is_empty() {
            segments.push(part);
        }
    }

    if segments.is_empty() {
        // Edge case: resolved to root with just a filename
        format!("/.md")
    } else {
        format!("/{}.md", segments.join("/"))
    }
}

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

/// Extract the "type" field from a filemeta_v0 entry value.
///
/// Parallel to `extract_id_from_filemeta_entry`. Handles both `Out::YMap` and `Out::Any(Any::Map)`.
pub fn extract_type_from_filemeta_entry(value: &Out, txn: &impl ReadTxn) -> Option<String> {
    match value {
        Out::YMap(meta_map) => {
            if let Some(Out::Any(Any::String(ref t))) = meta_map.get(txn, "type") {
                Some(t.to_string())
            } else {
                None
            }
        }
        Out::Any(Any::Map(ref map)) => {
            if let Some(Any::String(ref t)) = map.get("type") {
                Some(t.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Find the filemeta path key for a given UUID by scanning all entries.
/// Returns `None` if the UUID is not found in the filemeta map.
pub fn find_path_for_uuid(filemeta: &MapRef, txn: &impl ReadTxn, uuid: &str) -> Option<String> {
    for (path, value) in filemeta.iter(txn) {
        if let Some(id) = extract_id_from_filemeta_entry(&value, txn) {
            if id == uuid {
                return Some(path.to_string());
            }
        }
    }
    None
}

/// Read a backlinks array for a given target UUID from a backlinks_v0 Y.Map.
pub fn read_backlinks_array(
    backlinks: &MapRef,
    txn: &impl ReadTxn,
    target_uuid: &str,
) -> Vec<String> {
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

/// An entry in the virtual filesystem tree that unifies all folder docs.
/// Virtual paths include the folder name prefix, e.g., "/Relay Folder 1/Notes/Ideas.md".
#[derive(Debug, Clone)]
pub struct VirtualEntry {
    /// Virtual path: "/{folder_name}{filemeta_path}", e.g., "/Lens/Notes/Ideas.md"
    pub virtual_path: String,
    /// Entry type: "markdown", "folder", etc.
    pub entry_type: String,
    /// Document UUID
    pub id: String,
    /// Index of the folder doc this entry came from
    pub folder_idx: usize,
}

/// Resolve a wikilink in the virtual filesystem tree.
///
/// Algorithm (matches frontend's `resolvePageName()` exactly):
/// 1. Relative: resolve link_name from source's directory, case-insensitive, markdown-only
/// 2. Absolute (fallback): /{link_name}.md, case-insensitive, markdown-only
pub fn resolve_in_virtual_tree<'a>(
    link_name: &str,
    source_virtual_path: Option<&str>,
    entries: &'a [VirtualEntry],
) -> Option<&'a VirtualEntry> {
    let relative_path = source_virtual_path.map(|svp| resolve_relative(svp, link_name));
    let absolute_path = format!("/{}.md", link_name);

    let lower_relative = relative_path.as_ref().map(|p| p.to_lowercase());
    let lower_absolute = absolute_path.to_lowercase();

    let mut absolute_match: Option<&VirtualEntry> = None;

    for entry in entries {
        if entry.entry_type != "markdown" {
            continue;
        }

        let lower_entry = entry.virtual_path.to_lowercase();

        // Priority 1: relative match — return immediately
        if let Some(ref lr) = lower_relative {
            if lower_entry == *lr {
                return Some(entry);
            }
        }

        // Priority 2: absolute match — save as fallback
        if absolute_match.is_none() && lower_entry == lower_absolute {
            absolute_match = Some(entry);
        }
    }

    absolute_match
}

/// Compute wikilink text that resolves from `source_virtual_path` to `target_virtual_path`.
///
/// Both paths include folder prefix: "/{folder}/{path}.md"
/// Returns the page-name portion (no .md extension) for use inside `[[ ]]`.
///
/// Examples:
/// - `("/Lens/Getting Started.md", "/Lens/Archive/Welcome.md")` → `"Archive/Welcome"`
/// - `("/Lens/Notes/Ideas.md", "/Lens/Archive/Welcome.md")` → `"../Archive/Welcome"`
/// - `("/Lens/Getting Started.md", "/Lens Edu/Welcome.md")` → `"../Lens Edu/Welcome"`
/// - `("/Lens/Getting Started.md", "/Lens/Welcome.md")` → `"Welcome"`
pub fn compute_relative_wikilink(source_virtual_path: &str, target_virtual_path: &str) -> String {
    // Extract source directory segments (everything before last '/')
    let source_dir = &source_virtual_path[..source_virtual_path.rfind('/').unwrap_or(0)];
    let source_segments: Vec<&str> = source_dir.split('/').filter(|s| !s.is_empty()).collect();

    // Extract target segments: strip .md, split by '/', skip leading empty
    let target_no_ext = target_virtual_path
        .strip_suffix(".md")
        .unwrap_or(target_virtual_path);
    let target_segments: Vec<&str> = target_no_ext.split('/').filter(|s| !s.is_empty()).collect();

    // Find common prefix length (case-insensitive)
    let common_len = source_segments
        .iter()
        .zip(target_segments.iter())
        .take_while(|(a, b)| a.to_lowercase() == b.to_lowercase())
        .count();

    // Build path: ".." for each remaining source segment, then remaining target segments
    let ups = source_segments.len() - common_len;
    let remaining_target = &target_segments[common_len..];

    let mut parts: Vec<&str> = Vec::new();
    for _ in 0..ups {
        parts.push("..");
    }
    parts.extend_from_slice(remaining_target);

    parts.join("/")
}

/// Build a flat list of virtual entries from multiple folder docs.
///
/// Virtual paths are constructed as "/{folder_name}{filemeta_path}",
/// e.g., "/Lens/Notes/Ideas.md" for filemeta path "/Notes/Ideas.md" in folder "Lens".
pub fn build_virtual_entries(folder_docs: &[&Doc], folder_names: &[&str]) -> Vec<VirtualEntry> {
    let mut entries = Vec::new();
    for (fi, folder_doc) in folder_docs.iter().enumerate() {
        let txn = folder_doc.transact();
        if let Some(filemeta) = txn.get_map("filemeta_v0") {
            let folder_name = folder_names[fi];
            for (path, value) in filemeta.iter(&txn) {
                let entry_type = extract_type_from_filemeta_entry(&value, &txn)
                    .unwrap_or_else(|| "unknown".to_string());
                let id = match extract_id_from_filemeta_entry(&value, &txn) {
                    Some(id) => id,
                    None => continue,
                };
                let virtual_path = format!("/{}{}", folder_name, path);
                entries.push(VirtualEntry {
                    virtual_path,
                    entry_type,
                    id,
                    folder_idx: fi,
                });
            }
        }
    }
    entries
}

// ---------------------------------------------------------------------------
// Folder doc scanning helpers
// ---------------------------------------------------------------------------

/// Find all loaded folder docs (docs with non-empty filemeta_v0).
/// Returns doc_ids of all folder docs.
pub fn find_all_folder_docs(docs: &DashMap<String, DocWithSyncKv>) -> Vec<String> {
    let mut result: Vec<String> = docs
        .iter()
        .filter_map(|entry| {
            let awareness = entry.value().awareness();
            let guard = awareness.read().unwrap_or_else(|e| e.into_inner());
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
    let guard = awareness.read().unwrap_or_else(|e| e.into_inner());
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
    let markdown = {
        let txn = content_doc.transact();
        if let Some(contents) = txn.get_text("contents") {
            contents.get_string(&txn)
        } else {
            return Ok(());
        }
    };
    let link_names = extract_wikilinks(&markdown);
    tracing::info!(
        "Doc {}: content length={}, wikilinks={:?}",
        source_uuid,
        markdown.len(),
        link_names
    );
    let folder_name_strings: Vec<String> = folder_docs
        .iter()
        .map(|doc| read_folder_name(doc, ""))
        .collect();
    let folder_name_strs: Vec<&str> = folder_name_strings.iter().map(|s| s.as_str()).collect();
    index_content_into_folders_from_text(
        source_uuid,
        &markdown,
        &link_names,
        folder_docs,
        &folder_name_strs,
    )
}

/// Core indexing logic: resolves wikilinks against folder docs and updates backlinks_v0.
/// Used by `reindex_all_backlinks` to avoid holding a content read lock while
/// taking folder write locks (which deadlocks when content doc == folder doc).
fn index_content_into_folders_from_text(
    source_uuid: &str,
    _markdown: &str,
    link_names: &[String],
    folder_docs: &[&Doc],
    folder_names: &[&str],
) -> anyhow::Result<()> {
    // Build virtual tree from all folder docs
    let entries = build_virtual_entries(folder_docs, folder_names);

    // Find source's virtual path
    let source_virtual_path: Option<String> = entries
        .iter()
        .find(|e| e.id == source_uuid)
        .map(|e| e.virtual_path.clone());

    // Resolve each link in the virtual tree
    let mut resolved: Vec<(String, usize)> = Vec::new();

    for name in link_names {
        if let Some(entry) = resolve_in_virtual_tree(name, source_virtual_path.as_deref(), &entries)
        {
            resolved.push((entry.id.clone(), entry.folder_idx));
        }
    }

    tracing::info!(
        "Doc {}: resolved {} links -> {} targets across {} folders",
        source_uuid,
        link_names.len(),
        resolved.len(),
        folder_docs.len()
    );

    // Group resolved targets by folder index
    let mut targets_per_folder: Vec<HashSet<String>> = vec![HashSet::new(); folder_docs.len()];
    for (uuid, fi) in &resolved {
        targets_per_folder[*fi].insert(uuid.clone());
    }

    let all_new_targets: HashSet<&str> = resolved.iter().map(|(u, _)| u.as_str()).collect();

    // Diff-update backlinks_v0 on each folder doc
    for (fi, folder_doc) in folder_docs.iter().enumerate() {
        let new_targets = &targets_per_folder[fi];
        let mut txn = folder_doc.transact_mut_with("link-indexer");
        let backlinks = txn.get_or_insert_map("backlinks_v0");

        for target_uuid in new_targets {
            let current: Vec<String> = read_backlinks_array(&backlinks, &txn, target_uuid);
            if !current.contains(&source_uuid.to_string()) {
                let mut updated = current;
                updated.push(source_uuid.to_string());
                let arr: Vec<Any> = updated.into_iter().map(|s| Any::String(s.into())).collect();
                backlinks.insert(&mut txn, target_uuid.as_str(), arr);
            }
        }

        let all_keys: Vec<String> = backlinks.keys(&txn).map(|k| k.to_string()).collect();
        for key in all_keys {
            if all_new_targets.contains(key.as_str()) {
                continue;
            }
            let current: Vec<String> = read_backlinks_array(&backlinks, &txn, &key);
            if current.contains(&source_uuid.to_string()) {
                let updated: Vec<String> =
                    current.into_iter().filter(|s| s != source_uuid).collect();
                if updated.is_empty() {
                    backlinks.remove(&mut txn, &key);
                } else {
                    let arr: Vec<Any> =
                        updated.into_iter().map(|s| Any::String(s.into())).collect();
                    backlinks.insert(&mut txn, key.as_str(), arr);
                }
            }
        }
    }

    Ok(())
}

/// Remove all backlink entries for a given source UUID from all folder docs.
///
/// Scans backlinks_v0 on each folder doc and removes source_uuid from every
/// backlink array. Removes empty arrays entirely. Idempotent: calling on a
/// UUID with no backlinks is a no-op.
///
/// Returns the number of backlink arrays modified.
pub fn remove_doc_from_backlinks(source_uuid: &str, folder_docs: &[&Doc]) -> anyhow::Result<usize> {
    let mut modified_count = 0;

    for folder_doc in folder_docs {
        let mut txn = folder_doc.transact_mut_with("link-indexer");
        let backlinks = txn.get_or_insert_map("backlinks_v0");

        let all_keys: Vec<String> = backlinks.keys(&txn).map(|k| k.to_string()).collect();
        for key in all_keys {
            let current: Vec<String> = read_backlinks_array(&backlinks, &txn, &key);
            if current.contains(&source_uuid.to_string()) {
                let updated: Vec<String> =
                    current.into_iter().filter(|s| s != source_uuid).collect();
                if updated.is_empty() {
                    backlinks.remove(&mut txn, &key);
                } else {
                    let arr: Vec<Any> =
                        updated.into_iter().map(|s| Any::String(s.into())).collect();
                    backlinks.insert(&mut txn, key.as_str(), arr);
                }
                modified_count += 1;
            }
        }
    }

    Ok(modified_count)
}

// ---------------------------------------------------------------------------
// Document move — filemeta update + index cascade + wikilink rewriting
// ---------------------------------------------------------------------------

/// Result of a document move operation.
pub struct MoveResult {
    /// Previous filemeta path (e.g. "/Photosynthesis.md")
    pub old_path: String,
    /// New filemeta path
    pub new_path: String,
    /// Folder name the document was in before the move
    pub old_folder_name: String,
    /// Folder name the document is in after the move
    pub new_folder_name: String,
    /// Total wikilink edits across all backlinker docs
    pub links_rewritten: usize,
}

/// Move a document to a new path within or across folders.
///
/// Operates on bare Y.Docs (no DocWithSyncKv, no async). Steps:
/// 1. Find UUID in source_folder_doc's filemeta_v0, extract old path + metadata
/// 2. Update filemeta_v0 (within-folder: remove old + insert new; cross-folder: remove from source, add to target)
/// 3. Update DocumentResolver with new path
/// 4. Build virtual tree with OLD paths for backlink resolution
/// 5. Read backlinks for the moved UUID, rewrite wikilinks in each backlinker
/// 6. Re-index the moved doc's own backlinks (wikilinks may resolve differently at new location)
///
/// Does NOT update SearchIndex (caller can do that with content doc text).
#[allow(clippy::too_many_arguments)]
pub fn move_document(
    uuid: &str,
    new_path: &str,
    source_folder_doc: &Doc,
    target_folder_doc: &Doc,
    all_folder_docs: &[&Doc],
    all_folder_names: &[&str],
    doc_resolver: &DocumentResolver,
    content_docs: &HashMap<String, &Doc>,
) -> anyhow::Result<MoveResult> {
    // 1. Find the UUID in source filemeta_v0, extract old path + metadata fields
    let (old_path, meta_fields) = {
        let txn = source_folder_doc.transact();
        let filemeta = txn
            .get_map("filemeta_v0")
            .ok_or_else(|| anyhow::anyhow!("source folder doc has no filemeta_v0"))?;

        let mut found: Option<(String, HashMap<String, Any>)> = None;
        for (path, value) in filemeta.iter(&txn) {
            if let Some(id) = extract_id_from_filemeta_entry(&value, &txn) {
                if id == uuid {
                    // Extract all fields from the metadata entry
                    let fields = extract_filemeta_fields(&value, &txn);
                    found = Some((path.to_string(), fields));
                    break;
                }
            }
        }
        found.ok_or_else(|| anyhow::anyhow!("UUID {} not found in source filemeta_v0", uuid))?
    };

    // Determine folder names
    let source_folder_name = read_folder_name(source_folder_doc, "");
    let target_folder_name = read_folder_name(target_folder_doc, "");
    let is_cross_folder = !std::ptr::eq(source_folder_doc, target_folder_doc);

    // 2. Update filemeta_v0 and legacy "docs" map
    // Both maps must stay in sync — Obsidian treats entries only in filemeta_v0 as orphaned
    if is_cross_folder {
        // Cross-folder: remove from source, add to target
        {
            let mut txn = source_folder_doc.transact_mut_with("link-indexer");
            let filemeta = txn.get_or_insert_map("filemeta_v0");
            filemeta.remove(&mut txn, &old_path);
            let docs_map = txn.get_or_insert_map("docs");
            docs_map.remove(&mut txn, &old_path);
        }
        {
            let mut txn = target_folder_doc.transact_mut_with("link-indexer");
            let filemeta = txn.get_or_insert_map("filemeta_v0");
            filemeta.insert(&mut txn, new_path, Any::Map(meta_fields.clone().into()));
            let docs_map = txn.get_or_insert_map("docs");
            docs_map.insert(&mut txn, new_path, Any::String(uuid.into()));
        }
    } else {
        // Within-folder: remove old, insert new in one transaction
        let mut txn = source_folder_doc.transact_mut_with("link-indexer");
        let filemeta = txn.get_or_insert_map("filemeta_v0");
        filemeta.remove(&mut txn, &old_path);
        filemeta.insert(&mut txn, new_path, Any::Map(meta_fields.clone().into()));
        let docs_map = txn.get_or_insert_map("docs");
        docs_map.remove(&mut txn, &old_path);
        docs_map.insert(&mut txn, new_path, Any::String(uuid.into()));
    }

    // 3. Update DocumentResolver
    let new_stripped = new_path.strip_prefix('/').unwrap_or(new_path);
    let new_full_path = format!("{}/{}", target_folder_name, new_stripped);

    // Build DocInfo for the resolver -- we need relay_id and folder_doc_id
    // Try to get them from the existing resolver entry, or construct minimal ones
    let (relay_id, folder_doc_id) = if let Some(old_info) = doc_resolver
        .path_for_uuid(uuid)
        .and_then(|p| doc_resolver.resolve_path(&p))
    {
        (
            old_info.relay_id.clone(),
            if is_cross_folder {
                // For cross-folder, we don't know the new folder_doc_id from here,
                // but we can try to find it from the resolver's existing entries
                // for the target folder. For now, keep old folder_doc_id and let
                // the HTTP handler update it. In tests, this field isn't checked.
                old_info.folder_doc_id.clone()
            } else {
                old_info.folder_doc_id.clone()
            },
        )
    } else {
        (String::new(), String::new())
    };

    let doc_info = DocInfo {
        uuid: uuid.to_string(),
        relay_id: relay_id.clone(),
        folder_doc_id,
        folder_name: target_folder_name.clone(),
        doc_id: if relay_id.is_empty() {
            uuid.to_string()
        } else {
            format!("{}-{}", relay_id, uuid)
        },
    };
    doc_resolver.upsert_doc(uuid, &new_full_path, doc_info);

    // 4. Build virtual tree with OLD paths for backlink resolution
    //    (filemeta already has new path, so we need to patch the moved doc's entry back)
    let mut entries = build_virtual_entries(all_folder_docs, all_folder_names);

    // Patch virtual tree: restore old path for the moved document
    let old_virtual_path = format!("/{}{}", source_folder_name, old_path);
    for entry in entries.iter_mut() {
        if entry.id == uuid {
            entry.virtual_path = old_virtual_path.clone();
        }
    }

    // 5. Read backlinks for the moved UUID from all folder docs
    let mut all_backlinker_uuids: Vec<String> = Vec::new();
    for folder_doc in all_folder_docs {
        let txn = folder_doc.transact();
        if let Some(backlinks) = txn.get_map("backlinks_v0") {
            let backlnks = read_backlinks_array(&backlinks, &txn, uuid);
            for bl in backlnks {
                if !all_backlinker_uuids.contains(&bl) {
                    all_backlinker_uuids.push(bl);
                }
            }
        }
    }

    // 6. For each backlinker, rewrite wikilinks (handles both renames and directory moves)
    let new_virtual_path = format!("/{}{}", target_folder_name, new_path);
    let mut total_rewritten = 0usize;
    for backlinker_uuid in &all_backlinker_uuids {
        if let Some(content_doc) = content_docs.get(backlinker_uuid) {
            // Find the backlinker's virtual path for resolution context
            let source_virtual_path: Option<&str> = entries
                .iter()
                .find(|e| e.id == *backlinker_uuid)
                .map(|e| e.virtual_path.as_str());

            if let Some(svp) = source_virtual_path {
                match rewrite_wikilinks_for_move(
                    content_doc,
                    svp,
                    &old_virtual_path,
                    &new_virtual_path,
                    &entries,
                ) {
                    Ok(count) => total_rewritten += count,
                    Err(e) => {
                        tracing::error!(
                            "Failed to update wikilinks in backlinker {}: {:?}",
                            backlinker_uuid,
                            e
                        );
                    }
                }
            }
        }
    }

    // 6b. Rewrite outgoing links in the moved document itself
    if let Some(content_doc) = content_docs.get(uuid) {
        match rewrite_outgoing_links_for_move(
            content_doc,
            &old_virtual_path,
            &new_virtual_path,
            &entries,
        ) {
            Ok(count) => total_rewritten += count,
            Err(e) => tracing::error!("Failed to rewrite outgoing links: {:?}", e),
        }
    }

    // 6c. For cross-folder moves: transfer the moved doc's backlink target entry
    //     from source folder's backlinks_v0 to target folder's backlinks_v0
    if is_cross_folder {
        // Read + remove in one transaction to avoid TOCTOU window
        let backlinker_uuids = {
            let mut txn = source_folder_doc.transact_mut_with("link-indexer");
            let backlinks = txn.get_or_insert_map("backlinks_v0");
            let uuids = read_backlinks_array(&backlinks, &txn, uuid);
            backlinks.remove(&mut txn, uuid);
            uuids
        };

        // Add to target folder (merge with any existing entries)
        if !backlinker_uuids.is_empty() {
            let mut txn = target_folder_doc.transact_mut_with("link-indexer");
            let backlinks = txn.get_or_insert_map("backlinks_v0");
            let existing: Vec<String> = read_backlinks_array(&backlinks, &txn, uuid);
            let mut merged = existing;
            for bl in backlinker_uuids {
                if !merged.contains(&bl) {
                    merged.push(bl);
                }
            }
            let arr: Vec<Any> = merged.into_iter().map(|s| Any::String(s.into())).collect();
            backlinks.insert(&mut txn, uuid, arr);
        }
    }

    // 7. Re-index the moved doc's own backlinks (its wikilinks may resolve differently at new location)
    if let Some(content_doc) = content_docs.get(uuid) {
        // Re-index using the updated folder docs (filemeta already has new path)
        let _ = index_content_into_folders(uuid, content_doc, all_folder_docs);
    }

    Ok(MoveResult {
        old_path,
        new_path: new_path.to_string(),
        old_folder_name: source_folder_name,
        new_folder_name: target_folder_name,
        links_rewritten: total_rewritten,
    })
}

/// Extract all fields from a filemeta entry value as a flat HashMap<String, Any>.
///
/// Handles both Out::YMap (from Rust/Yrs) and Out::Any(Any::Map) (from JS clients).
fn extract_filemeta_fields(value: &Out, txn: &impl ReadTxn) -> HashMap<String, Any> {
    let mut fields = HashMap::new();
    match value {
        Out::YMap(meta_map) => {
            for (key, val) in meta_map.iter(txn) {
                if let Out::Any(any_val) = val {
                    fields.insert(key.to_string(), any_val);
                }
            }
        }
        Out::Any(Any::Map(ref map)) => {
            for (key, val) in map.iter() {
                fields.insert(key.clone(), val.clone());
            }
        }
        _ => {}
    }
    fields
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
/// Update wikilinks in a Y.Doc -- matches all basename occurrences (no resolution filter).
/// Use `update_wikilinks_in_doc_resolved` for disambiguation in multi-folder setups.
pub fn update_wikilinks_in_doc(
    content_doc: &Doc,
    old_name: &str,
    new_name: &str,
) -> anyhow::Result<usize> {
    // Delegate with empty resolution context -- matches all basenames
    update_wikilinks_in_doc_resolved(content_doc, old_name, new_name, None, &[], "")
}

/// Update wikilinks in a Y.Doc with resolution-based disambiguation.
///
/// For each wikilink whose basename matches `old_name`, resolves it against
/// the virtual tree. Only edits links that resolve to `old_target_virtual_path`.
/// When `old_target_virtual_path` is empty, falls back to matching all basenames.
pub fn update_wikilinks_in_doc_resolved(
    content_doc: &Doc,
    old_name: &str,
    new_name: &str,
    source_virtual_path: Option<&str>,
    entries: &[VirtualEntry],
    old_target_virtual_path: &str,
) -> anyhow::Result<usize> {
    let plain_text = {
        let txn = content_doc.transact();
        match txn.get_text("contents") {
            Some(text) => text.get_string(&txn),
            None => return Ok(0),
        }
    };

    let edits = if old_target_virtual_path.is_empty() {
        // No resolution context -- match all basenames (legacy behavior)
        compute_wikilink_rename_edits(&plain_text, old_name, new_name)
    } else {
        // Resolution-aware -- only edit links that resolve to the renamed file
        let old_target_lower = old_target_virtual_path.to_lowercase();
        compute_wikilink_rename_edits_resolved(&plain_text, old_name, new_name, |link_name| {
            resolve_in_virtual_tree(link_name, source_virtual_path, entries)
                .map(|e| e.virtual_path.to_lowercase() == old_target_lower)
                .unwrap_or(false)
        })
    };

    if edits.is_empty() {
        return Ok(0);
    }

    let mut txn = content_doc.transact_mut_with("link-indexer");
    let text = txn.get_or_insert_text("contents");

    for edit in &edits {
        text.remove_range(&mut txn, edit.offset as u32, edit.remove_len as u32);
        text.insert(&mut txn, edit.offset as u32, &edit.insert_text);
    }

    Ok(edits.len())
}

/// Rewrite wikilinks in a content doc after a document move.
///
/// For each wikilink that resolves to `old_target_virtual_path` (in the pre-move
/// virtual tree), computes the new relative path from the source to the target's
/// new location and replaces the entire page-name portion.
fn rewrite_wikilinks_for_move(
    content_doc: &Doc,
    source_virtual_path: &str,
    old_target_virtual_path: &str,
    new_target_virtual_path: &str,
    entries: &[VirtualEntry],
) -> anyhow::Result<usize> {
    let plain_text = {
        let txn = content_doc.transact();
        match txn.get_text("contents") {
            Some(text) => text.get_string(&txn),
            None => return Ok(0),
        }
    };

    let old_target_lower = old_target_virtual_path.to_lowercase();
    let new_name = compute_relative_wikilink(source_virtual_path, new_target_virtual_path);

    let edits = compute_wikilink_move_edits(
        &plain_text,
        |link_name| {
            resolve_in_virtual_tree(link_name, Some(source_virtual_path), entries)
                .map(|e| e.virtual_path.to_lowercase() == old_target_lower)
                .unwrap_or(false)
        },
        |_| new_name.clone(),
    );

    if edits.is_empty() {
        return Ok(0);
    }

    let mut txn = content_doc.transact_mut_with("link-indexer");
    let text = txn.get_or_insert_text("contents");

    for edit in &edits {
        text.remove_range(&mut txn, edit.offset as u32, edit.remove_len as u32);
        text.insert(&mut txn, edit.offset as u32, &edit.insert_text);
    }

    Ok(edits.len())
}

/// Rewrite outgoing wikilinks in a moved document.
///
/// For each wikilink in the content doc, resolves it from the OLD source location.
/// If it resolves to a target, computes the correct relative path from the NEW
/// source location to that same target and replaces the link if it changed.
fn rewrite_outgoing_links_for_move(
    content_doc: &Doc,
    old_source_virtual_path: &str,
    new_source_virtual_path: &str,
    entries: &[VirtualEntry],
) -> anyhow::Result<usize> {
    let plain_text = {
        let txn = content_doc.transact();
        match txn.get_text("contents") {
            Some(text) => text.get_string(&txn),
            None => return Ok(0),
        }
    };

    let edits = compute_wikilink_move_edits(
        &plain_text,
        |link_name| {
            // Resolve from OLD location — does this link find a target?
            let target = resolve_in_virtual_tree(link_name, Some(old_source_virtual_path), entries);
            if let Some(t) = target {
                // Compute what the link text should be from the NEW location
                let new_link = compute_relative_wikilink(new_source_virtual_path, &t.virtual_path);
                // Only edit if the link text would actually change
                new_link != link_name
            } else {
                false
            }
        },
        |link_name| {
            // Resolve from OLD location to find the target
            let target = resolve_in_virtual_tree(link_name, Some(old_source_virtual_path), entries)
                .expect("should_edit already confirmed resolution");
            compute_relative_wikilink(new_source_virtual_path, &target.virtual_path)
        },
    );

    if edits.is_empty() {
        return Ok(0);
    }

    let mut txn = content_doc.transact_mut_with("link-indexer");
    let text = txn.get_or_insert_text("contents");

    for edit in &edits {
        text.remove_range(&mut txn, edit.offset as u32, edit.remove_len as u32);
        text.insert(&mut txn, edit.offset as u32, &edit.insert_text);
    }

    Ok(edits.len())
}

// ---------------------------------------------------------------------------
// LinkIndexer — async server-side struct with debounced worker
// ---------------------------------------------------------------------------

const DEBOUNCE_DURATION: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// A rename event detected by diffing filemeta snapshots.
///
/// Emitted when the same UUID maps to a different basename across two snapshots.
pub(crate) struct RenameEvent {
    pub uuid: String,
    pub old_name: String,
    pub new_name: String,
    /// Full filemeta path of the old file, e.g. "/Foo.md"
    pub old_path: String,
}

pub struct PendingEntry {
    pub first_queued: Instant,
    pub last_updated: Instant,
}

impl PendingEntry {
    pub fn new(now: Instant) -> Self {
        Self {
            first_queued: now,
            last_updated: now,
        }
    }
}

pub struct LinkIndexer {
    pending: Arc<DashMap<String, PendingEntry>>,
    index_tx: mpsc::Sender<String>,
    filemeta_cache: Arc<DashMap<String, HashMap<String, (String, String)>>>, // folder_doc_id -> (uuid -> (basename, path))
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
        use dashmap::mapref::entry::Entry;
        let now = Instant::now();
        // Atomically check-and-insert to avoid TOCTOU race where two concurrent
        // calls both see "not pending" and double-send to the channel.
        let is_new = match self.pending.entry(doc_id.to_string()) {
            Entry::Occupied(mut e) => {
                e.get_mut().last_updated = now;
                false
            }
            Entry::Vacant(e) => {
                e.insert(PendingEntry::new(now));
                true
            }
        };
        // Only send to channel on the first update — subsequent updates just
        // reset the timestamp for debouncing without flooding the channel.
        if is_new {
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
            entry.last_updated.elapsed() >= DEBOUNCE_DURATION // user paused
                || entry.first_queued.elapsed() >= DEBOUNCE_DURATION // ceiling: continuous editing
        } else {
            false
        }
    }

    fn mark_indexed(&self, doc_id: &str) {
        self.pending.remove(doc_id);
    }

    /// Clear all pending entries. Called after startup_reindex to discard stale
    /// timestamps so the worker starts clean.
    pub fn clear_pending(&self) {
        self.pending.clear();
    }

    /// Diff current filemeta_v0 state against cache, emit RenameEvent for UUIDs
    /// whose basename has changed. Updates the cache after diffing.
    /// First call (no cache entry) seeds the cache and returns empty.
    pub(crate) fn detect_renames(&self, folder_doc_id: &str, folder_doc: &Doc) -> Vec<RenameEvent> {
        // 1. Read filemeta_v0 and build uuid -> (basename, path) map
        let current: HashMap<String, (String, String)> = {
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
                    map.insert(uuid, (basename, path.to_string()));
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
        for (uuid, (new_basename, _new_path)) in &current {
            if let Some((old_basename, old_path)) = old.get(uuid) {
                if old_basename != new_basename {
                    renames.push(RenameEvent {
                        uuid: uuid.clone(),
                        old_name: old_basename.clone(),
                        new_name: new_basename.clone(),
                        old_path: old_path.clone(),
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
    /// 2. Builds virtual tree from all folder docs for link resolution
    /// 3. For each rename, reads backlinks to find source docs
    /// 4. Resolves each wikilink to confirm it points to the renamed file
    /// 5. Calls `update_wikilinks_in_doc_resolved()` for disambiguation
    /// Returns `true` if renames were detected and processed.
    fn apply_rename_updates(
        &self,
        folder_doc_id: &str,
        docs: &DashMap<String, DocWithSyncKv>,
    ) -> bool {
        // 1. Get the folder doc, detect renames, read folder name
        let (renames, folder_name) = {
            let Some(doc_ref) = docs.get(folder_doc_id) else {
                return false;
            };
            let awareness = doc_ref.awareness();
            let guard = awareness.read().unwrap_or_else(|e| e.into_inner());
            let renames = self.detect_renames(folder_doc_id, &guard.doc);
            let folder_name = read_folder_name(&guard.doc, "");
            (renames, folder_name)
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

        // 2. Build virtual entries from all folder docs
        //    Snapshot entries one folder at a time to avoid holding multiple locks.
        let folder_doc_ids = find_all_folder_docs(docs);
        let mut entries: Vec<VirtualEntry> = Vec::new();
        for fid in &folder_doc_ids {
            if let Some(doc_ref) = docs.get(fid) {
                let awareness = doc_ref.awareness();
                let guard = awareness.read().unwrap_or_else(|e| e.into_inner());
                let fname = read_folder_name(&guard.doc, "");
                let txn = guard.doc.transact();
                if let Some(filemeta) = txn.get_map("filemeta_v0") {
                    for (path, value) in filemeta.iter(&txn) {
                        if let Some(uuid) = extract_id_from_filemeta_entry(&value, &txn) {
                            let entry_type =
                                extract_type_from_filemeta_entry(&value, &txn).unwrap_or_default();
                            entries.push(VirtualEntry {
                                virtual_path: format!("/{}{}", fname, path),
                                entry_type,
                                id: uuid,
                                folder_idx: 0, // not needed for resolution
                            });
                        }
                    }
                }
            }
        }

        // 3. Patch virtual entries to reflect pre-rename state.
        //    By the time we get here, filemeta already has the new paths.
        //    Resolution must be against the old paths to correctly identify
        //    which links pointed to the renamed file.
        for rename in &renames {
            let old_virtual_path = format!("/{}{}", folder_name, rename.old_path);
            for entry in entries.iter_mut() {
                if entry.id == rename.uuid {
                    entry.virtual_path = old_virtual_path.clone();
                }
            }
        }

        // 4. For each rename, read backlinks and update content docs
        for rename in &renames {
            let old_virtual_path = format!("/{}{}", folder_name, rename.old_path);

            let source_uuids = {
                let Some(doc_ref) = docs.get(folder_doc_id) else {
                    continue;
                };
                let awareness = doc_ref.awareness();
                let guard = awareness.read().unwrap_or_else(|e| e.into_inner());
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
                    rename.old_name,
                    rename.new_name,
                    rename.uuid
                );
                continue;
            }

            tracing::info!(
                "Rename {} -> {}: updating {} backlinker(s)",
                rename.old_name,
                rename.new_name,
                source_uuids.len()
            );

            // 5. Update wikilinks in each source doc with resolution context
            for source_uuid in &source_uuids {
                let content_doc_id = format!("{}-{}", relay_id, source_uuid);
                let Some(content_ref) = docs.get(&content_doc_id) else {
                    tracing::warn!(
                        "Backlinker doc {} not loaded, skipping rename update",
                        content_doc_id
                    );
                    continue;
                };

                let source_virtual_path = entries
                    .iter()
                    .find(|e| e.id == *source_uuid)
                    .map(|e| e.virtual_path.clone());

                let awareness = content_ref.awareness();
                let guard = awareness.write().unwrap_or_else(|e| e.into_inner());
                match update_wikilinks_in_doc_resolved(
                    &guard.doc,
                    &rename.old_name,
                    &rename.new_name,
                    source_virtual_path.as_deref(),
                    &entries,
                    &old_virtual_path,
                ) {
                    Ok(count) => {
                        tracing::info!(
                            "Updated {} wikilink(s) in {} ({} -> {})",
                            count,
                            content_doc_id,
                            rename.old_name,
                            rename.new_name
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to update wikilinks in {}: {:?}",
                            content_doc_id,
                            e
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
        doc_resolver: Arc<DocumentResolver>,
    ) {
        tracing::info!("Link indexer worker started");
        loop {
            // 1. Wait for work: either a new channel message or poll timeout
            tokio::select! {
                msg = rx.recv() => {
                    match msg {
                        Some(_) => { /* already in pending map, nothing extra to do */ }
                        None => break, // channel closed
                    }
                }
                _ = tokio::time::sleep(POLL_INTERVAL) => {}
            }

            // 2. Drain any remaining channel messages (non-blocking)
            while rx.try_recv().is_ok() {}

            // 3. Collect all ready doc_ids
            let ready: Vec<String> = self
                .pending
                .iter()
                .filter(|e| {
                    let is_folder = is_folder_doc(e.key(), &docs).is_some();
                    is_folder || self.is_ready(e.key())
                })
                .map(|e| e.key().clone())
                .collect();

            // 4. Process each ready doc (sequentially, but NO sleeping between them)
            for doc_id in ready {
                // Re-check folder status (need fresh content UUIDs).
                if let Some(content_uuids) = is_folder_doc(&doc_id, &docs) {
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
                            doc_id,
                            content_uuids.len()
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

                    // Update DocumentResolver so MCP tools see current paths
                    doc_resolver.update_folder(&doc_id, &docs);
                } else {
                    // Content doc — index it
                    tracing::info!("Indexing content doc: {}", doc_id);
                    let folder_doc_ids = find_all_folder_docs(&docs);
                    match self.index_document(&doc_id, &docs, &folder_doc_ids) {
                        Ok(()) => tracing::info!("Successfully indexed: {}", doc_id),
                        Err(e) => tracing::error!("Failed to index {}: {:?}", doc_id, e),
                    }
                }
                self.mark_indexed(&doc_id);
            }
        }
    }

    /// Server glue: unwraps DocWithSyncKv, delegates to core function.
    /// Resolves links across ALL loaded folder docs (cross-folder backlinks).
    fn index_document(
        &self,
        doc_id: &str,
        docs: &DashMap<String, DocWithSyncKv>,
        folder_doc_ids: &[String],
    ) -> anyhow::Result<()> {
        let (_relay_id, doc_uuid) = parse_doc_id(doc_id)
            .ok_or_else(|| anyhow::anyhow!("Invalid doc_id format: {}", doc_id))?;

        if folder_doc_ids.is_empty() {
            return Err(anyhow::anyhow!("No folder docs found for indexing"));
        }

        // Phase 1: Extract content text under a short-lived read lock.
        // We must NOT hold a content read lock while taking folder write locks,
        // because the content doc might also be a folder doc — holding read + write
        // on the same std::sync::RwLock deadlocks.
        let markdown = {
            let content_ref = docs
                .get(doc_id)
                .ok_or_else(|| anyhow::anyhow!("Content doc not found: {}", doc_id))?;
            let awareness = content_ref.awareness();
            let guard = awareness.read().unwrap_or_else(|e| e.into_inner());
            let txn = guard.doc.transact();
            if let Some(contents) = txn.get_text("contents") {
                contents.get_string(&txn)
            } else {
                return Ok(()); // No content, nothing to index
            }
        }; // content read lock + DashMap guard dropped here

        let link_names = extract_wikilinks(&markdown);
        tracing::info!(
            "Doc {}: content length={}, wikilinks={:?}",
            doc_uuid,
            markdown.len(),
            link_names
        );

        // Phase 2: Take write locks on folder docs to resolve links and update backlinks.
        let folder_refs: Vec<_> = folder_doc_ids
            .iter()
            .filter_map(|id| docs.get(id))
            .collect();
        let folder_awarenesses: Vec<_> = folder_refs.iter().map(|r| r.awareness()).collect();
        let folder_guards: Vec<_> = folder_awarenesses
            .iter()
            .map(|a| a.write().unwrap_or_else(|e| e.into_inner()))
            .collect();
        let folder_doc_refs: Vec<&Doc> = folder_guards.iter().map(|g| &g.doc).collect();

        let folder_name_strings: Vec<String> = folder_doc_refs
            .iter()
            .zip(folder_doc_ids.iter())
            .map(|(doc, id)| read_folder_name(doc, id))
            .collect();
        let folder_name_strs: Vec<&str> = folder_name_strings.iter().map(|s| s.as_str()).collect();
        index_content_into_folders_from_text(
            doc_uuid,
            &markdown,
            &link_names,
            &folder_doc_refs,
            &folder_name_strs,
        )
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

        // Pre-compute folder doc IDs once (avoids re-scanning on every iteration).
        let folder_doc_ids = find_all_folder_docs(docs);
        tracing::info!("Found {} folder docs for indexing", folder_doc_ids.len());

        let doc_ids: Vec<String> = docs.iter().map(|e| e.key().clone()).collect();
        for doc_id in &doc_ids {
            match self.index_document(doc_id, docs, &folder_doc_ids) {
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
        for folder_doc_id in &folder_doc_ids {
            if let Some(doc_ref) = docs.get(folder_doc_id) {
                let awareness = doc_ref.awareness();
                let guard = awareness.read().unwrap_or_else(|e| e.into_inner());
                self.detect_renames(folder_doc_id, &guard.doc);
            }
        }
        tracing::info!(
            "Seeded filemeta cache for {} folder docs",
            folder_doc_ids.len()
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
    use yrs::{Any, Doc, GetString, Map, Text, Transact, WriteTxn};

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
        assert!(
            !indexer.is_ready("doc-1"),
            "should not be ready during rapid updates"
        );

        // Wait for full debounce duration
        tokio::time::sleep(DEBOUNCE_DURATION + Duration::from_millis(100)).await;

        // Now should be ready (no updates during the wait)
        assert!(
            indexer.is_ready("doc-1"),
            "should be ready after debounce settles"
        );
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
        assert!(
            rx.try_recv().is_ok(),
            "should queue new message after mark_indexed"
        );
    }

    #[tokio::test]
    async fn continuous_editing_hits_ceiling() {
        let (indexer, _rx) = LinkIndexer::new();

        // First update
        indexer.on_document_update("doc-1").await;

        // Simulate continuous typing: update every 500ms for 3 seconds.
        // Each update resets last_updated, but first_queued stays at the original time.
        for _ in 0..6 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            indexer.on_document_update("doc-1").await;
        }

        // At this point ~3s have elapsed since first_queued.
        // last_updated was just reset, so debounce (last_updated >= 2s) would NOT fire.
        // But ceiling (first_queued >= 2s) should fire.
        assert!(
            indexer.is_ready("doc-1"),
            "should be ready via ceiling even though last_updated was just reset"
        );
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

    /// Set the folder display name in a folder doc's folder_config Y.Map.
    fn set_folder_name(doc: &Doc, name: &str) {
        let mut txn = doc.transact_mut();
        let config = txn.get_or_insert_map("folder_config");
        config.insert(&mut txn, "name", Any::String(name.into()));
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
        let folder_doc =
            create_folder_doc(&[("/Notes.md", "uuid-notes"), ("/Ideas.md", "uuid-ideas")]);
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
        assert_eq!(
            read_backlinks(&folder_doc, "uuid-ideas"),
            vec!["uuid-notes"]
        );
        assert_eq!(
            read_backlinks(&folder_doc, "uuid-other"),
            vec!["uuid-notes"]
        );
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
        assert_eq!(
            read_backlinks(&folder_doc, "uuid-ideas"),
            vec!["uuid-notes"]
        );
        assert_eq!(
            read_backlinks(&folder_doc, "uuid-other"),
            vec!["uuid-notes"]
        );

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
        assert_eq!(
            read_backlinks(&folder_doc, "uuid-ideas"),
            vec!["uuid-notes"]
        );
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
        let folder_doc =
            create_folder_doc(&[("/Notes.md", "uuid-notes"), ("/Other.md", "uuid-other")]);
        let content_doc = create_content_doc("Just plain text, no links");

        let result = index_content_into_folder("uuid-notes", &content_doc, &folder_doc);
        assert!(result.is_ok());

        // Assert: no backlinks_v0 entries
        assert!(read_backlinks(&folder_doc, "uuid-other").is_empty());
    }

    // === Subdirectory wikilink resolution tests ===

    #[test]
    fn resolves_wikilink_to_sibling_in_subdirectory() {
        // Source is in /Notes/, so [[Ideas]] resolves relatively to /Notes/Ideas.md
        let folder_doc = create_folder_doc(&[
            ("/Notes/Source.md", "uuid-source"),
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
        // Source is in /Notes/, so [[Ideas]] resolves relatively to /Notes/ideas.md (case-insensitive)
        let folder_doc = create_folder_doc(&[
            ("/Notes/Source.md", "uuid-source"),
            ("/Notes/ideas.md", "uuid-ideas"),
        ]);
        let content_doc = create_content_doc("See [[Ideas]] for details.");
        index_content_into_folder("uuid-source", &content_doc, &folder_doc).unwrap();
        assert_eq!(
            read_backlinks(&folder_doc, "uuid-ideas"),
            vec!["uuid-source"]
        );
    }

    #[test]
    fn absolute_match_when_source_not_in_filemeta() {
        // Source not in filemeta → no relative resolution, no source path.
        // [[Ideas]] absolute → /Ideas.md → doesn't match /Lens/Ideas.md in virtual tree → no match.
        let folder_doc = create_folder_doc(&[
            ("/Ideas.md", "uuid-root"),
            ("/Notes/Ideas.md", "uuid-nested"),
        ]);
        let content_doc = create_content_doc("See [[Ideas]].");
        index_content_into_folder("uuid-source", &content_doc, &folder_doc).unwrap();
        assert!(read_backlinks(&folder_doc, "uuid-root").is_empty());
        assert!(read_backlinks(&folder_doc, "uuid-nested").is_empty());
    }

    #[test]
    fn resolves_explicit_path_wikilink() {
        // Source at root, [[Notes/Ideas]] resolves via relative to /Lens/Notes/Ideas.md
        let folder_doc = create_folder_doc(&[
            ("/Source.md", "uuid-source"),
            ("/Notes/Ideas.md", "uuid-ideas"),
        ]);
        let content_doc = create_content_doc("See [[Notes/Ideas]].");
        index_content_into_folder("uuid-source", &content_doc, &folder_doc).unwrap();
        assert_eq!(
            read_backlinks(&folder_doc, "uuid-ideas"),
            vec!["uuid-source"]
        );
    }

    // === Cross-folder backlink tests ===

    #[test]
    fn cross_folder_link_creates_backlink_in_target_folder() {
        let folder_a = create_folder_doc(&[("/Welcome.md", "uuid-welcome")]);
        set_folder_name(&folder_a, "Lens");
        let folder_b = create_folder_doc(&[("/Syllabus.md", "uuid-syllabus")]);
        set_folder_name(&folder_b, "Lens Edu");
        // Cross-folder link using absolute path with folder name
        let content_doc = create_content_doc("See [[Lens Edu/Syllabus]] for the course plan.");

        index_content_into_folders("uuid-welcome", &content_doc, &[&folder_a, &folder_b]).unwrap();

        let backlinks_b = read_backlinks(&folder_b, "uuid-syllabus");
        assert_eq!(backlinks_b, vec!["uuid-welcome"]);
        let backlinks_a = read_backlinks(&folder_a, "uuid-syllabus");
        assert!(backlinks_a.is_empty());
    }

    #[test]
    fn cross_folder_link_removal_cleans_target_folder() {
        let folder_a = create_folder_doc(&[("/Welcome.md", "uuid-welcome")]);
        set_folder_name(&folder_a, "Lens");
        let folder_b = create_folder_doc(&[("/Syllabus.md", "uuid-syllabus")]);
        set_folder_name(&folder_b, "Lens Edu");

        let content_v1 = create_content_doc("See [[Lens Edu/Syllabus]].");
        index_content_into_folders("uuid-welcome", &content_v1, &[&folder_a, &folder_b]).unwrap();
        assert_eq!(
            read_backlinks(&folder_b, "uuid-syllabus"),
            vec!["uuid-welcome"]
        );

        let content_v2 = create_content_doc("No links here.");
        index_content_into_folders("uuid-welcome", &content_v2, &[&folder_a, &folder_b]).unwrap();
        assert!(read_backlinks(&folder_b, "uuid-syllabus").is_empty());
    }

    #[test]
    fn within_folder_link_still_works_with_multi_folder() {
        // Same-folder links should still work when multiple folders are passed
        let folder_a =
            create_folder_doc(&[("/Notes.md", "uuid-notes"), ("/Ideas.md", "uuid-ideas")]);
        set_folder_name(&folder_a, "Lens");
        let folder_b = create_folder_doc(&[("/Syllabus.md", "uuid-syllabus")]);
        set_folder_name(&folder_b, "Lens Edu");
        let content_doc = create_content_doc("See [[Ideas]].");

        index_content_into_folders("uuid-notes", &content_doc, &[&folder_a, &folder_b]).unwrap();

        assert_eq!(read_backlinks(&folder_a, "uuid-ideas"), vec!["uuid-notes"]);
        assert!(read_backlinks(&folder_b, "uuid-ideas").is_empty());
    }

    #[test]
    fn link_to_docs_in_multiple_folders() {
        let folder_a = create_folder_doc(&[
            ("/Welcome.md", "uuid-welcome"),
            ("/Resources.md", "uuid-resources"),
        ]);
        set_folder_name(&folder_a, "Lens");
        let folder_b = create_folder_doc(&[("/Syllabus.md", "uuid-syllabus")]);
        set_folder_name(&folder_b, "Lens Edu");
        let content_doc = create_content_doc("See [[Lens Edu/Syllabus]] and [[Resources]].");

        index_content_into_folders("uuid-welcome", &content_doc, &[&folder_a, &folder_b]).unwrap();

        assert_eq!(
            read_backlinks(&folder_b, "uuid-syllabus"),
            vec!["uuid-welcome"]
        );
        assert_eq!(
            read_backlinks(&folder_a, "uuid-resources"),
            vec!["uuid-welcome"]
        );
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
        let folder_doc = create_folder_doc(&[("/Foo.md", "uuid-1"), ("/Bar.md", "uuid-2")]);

        let renames = indexer.detect_renames("folder-1", &folder_doc);
        assert!(
            renames.is_empty(),
            "first call should seed cache and return empty"
        );
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
        assert!(
            renames.is_empty(),
            "folder move with same basename should not be a rename"
        );
    }

    #[test]
    fn detects_multiple_renames() {
        let (indexer, _rx) = LinkIndexer::new();

        // Seed
        let folder_v1 = create_folder_doc(&[("/Foo.md", "uuid-1"), ("/Bar.md", "uuid-2")]);
        indexer.detect_renames("folder-1", &folder_v1);

        // Rename both
        let folder_v2 = create_folder_doc(&[("/Baz.md", "uuid-1"), ("/Qux.md", "uuid-2")]);
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
        let folder_v2 = create_folder_doc(&[("/Foo.md", "uuid-1"), ("/NewFile.md", "uuid-2")]);
        let renames = indexer.detect_renames("folder-1", &folder_v2);
        assert!(
            renames.is_empty(),
            "new files should not produce rename events"
        );
    }

    #[test]
    fn ignores_deleted_files() {
        let (indexer, _rx) = LinkIndexer::new();

        // Seed with two files
        let folder_v1 = create_folder_doc(&[("/Foo.md", "uuid-1"), ("/Bar.md", "uuid-2")]);
        indexer.detect_renames("folder-1", &folder_v1);

        // Remove uuid-2
        let folder_v2 = create_folder_doc(&[("/Foo.md", "uuid-1")]);
        let renames = indexer.detect_renames("folder-1", &folder_v2);
        assert!(
            renames.is_empty(),
            "deleted files should not produce rename events"
        );
    }

    // === Rename pipeline integration tests ===
    // These test the full pipeline: detect_renames + read backlinks + update_wikilinks_in_doc

    #[test]
    fn rename_updates_wikilinks_in_backlinkers() {
        // 1. Create folder with Foo.md (uuid-foo) and Notes.md (uuid-notes)
        let folder_doc = create_folder_doc(&[("/Foo.md", "uuid-foo"), ("/Notes.md", "uuid-notes")]);

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
        let folder_doc = create_folder_doc(&[("/Foo.md", "uuid-foo"), ("/Notes.md", "uuid-notes")]);

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
        let folder_doc = create_folder_doc(&[("/Foo.md", "uuid-foo"), ("/Notes.md", "uuid-notes")]);

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

        assert!(
            source_uuids.is_empty(),
            "no backlinkers means nothing to update"
        );
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
        assert_eq!(
            read_backlinks(&folder_doc, "uuid-other"),
            vec!["uuid-notes"]
        );

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

    // === resolve_relative unit tests ===

    #[test]
    fn resolve_relative_sibling() {
        // Source at /Notes/Source.md, link "Ideas" → /Notes/Ideas.md
        assert_eq!(
            resolve_relative("/Notes/Source.md", "Ideas"),
            "/Notes/Ideas.md"
        );
    }

    #[test]
    fn resolve_relative_parent() {
        // Source at /Notes/Source.md, link "../Welcome" → /Welcome.md
        assert_eq!(
            resolve_relative("/Notes/Source.md", "../Welcome"),
            "/Welcome.md"
        );
    }

    #[test]
    fn resolve_relative_cousin() {
        // Source at /Notes/Source.md, link "../Projects/Todo" → /Projects/Todo.md
        assert_eq!(
            resolve_relative("/Notes/Source.md", "../Projects/Todo"),
            "/Projects/Todo.md"
        );
    }

    #[test]
    fn resolve_relative_root_clamping() {
        // Source at /Source.md, link "../../Deep" → can't go above root → /Deep.md
        assert_eq!(resolve_relative("/Source.md", "../../Deep"), "/Deep.md");
    }

    #[test]
    fn resolve_relative_dot_segment() {
        // Source at /Notes/Source.md, link "./Ideas" → /Notes/Ideas.md
        assert_eq!(
            resolve_relative("/Notes/Source.md", "./Ideas"),
            "/Notes/Ideas.md"
        );
    }

    #[test]
    fn resolve_relative_root_level_file() {
        // Source at /Source.md, link "Ideas" → /Ideas.md
        assert_eq!(resolve_relative("/Source.md", "Ideas"), "/Ideas.md");
    }

    #[test]
    fn resolve_relative_deep_nesting() {
        // Source at /A/B/C/Source.md, link "../../X/Y" → /A/X/Y.md
        assert_eq!(
            resolve_relative("/A/B/C/Source.md", "../../X/Y"),
            "/A/X/Y.md"
        );
    }

    // === New link resolution behavior tests ===

    #[test]
    fn resolves_relative_sibling_link() {
        // Source at /Notes/Source.md, link [[Ideas]], target /Notes/Ideas.md → backlink
        let folder_doc = create_folder_doc(&[
            ("/Notes/Source.md", "uuid-source"),
            ("/Notes/Ideas.md", "uuid-ideas"),
        ]);
        let content_doc = create_content_doc("See [[Ideas]].");
        index_content_into_folder("uuid-source", &content_doc, &folder_doc).unwrap();
        assert_eq!(
            read_backlinks(&folder_doc, "uuid-ideas"),
            vec!["uuid-source"]
        );
    }

    #[test]
    fn resolves_relative_parent_link() {
        // Source at /Notes/Source.md, link [[../Welcome]], target /Welcome.md → backlink
        let folder_doc = create_folder_doc(&[
            ("/Notes/Source.md", "uuid-source"),
            ("/Welcome.md", "uuid-welcome"),
        ]);
        let content_doc = create_content_doc("See [[../Welcome]].");
        index_content_into_folder("uuid-source", &content_doc, &folder_doc).unwrap();
        assert_eq!(
            read_backlinks(&folder_doc, "uuid-welcome"),
            vec!["uuid-source"]
        );
    }

    #[test]
    fn relative_match_beats_absolute() {
        // Source at /Notes/Source.md, [[Ideas]]
        // Both /Ideas.md and /Notes/Ideas.md exist
        // → relative match /Notes/Ideas.md should win
        let folder_doc = create_folder_doc(&[
            ("/Notes/Source.md", "uuid-source"),
            ("/Ideas.md", "uuid-root"),
            ("/Notes/Ideas.md", "uuid-nested"),
        ]);
        let content_doc = create_content_doc("See [[Ideas]].");
        index_content_into_folder("uuid-source", &content_doc, &folder_doc).unwrap();
        assert_eq!(
            read_backlinks(&folder_doc, "uuid-nested"),
            vec!["uuid-source"]
        );
        assert!(read_backlinks(&folder_doc, "uuid-root").is_empty());
    }

    #[test]
    fn bare_name_no_basename_matching() {
        // Source not in filemeta, link [[Ideas]], only /Notes/Ideas.md exists
        // → absolute "/Ideas.md" doesn't match /Notes/Ideas.md, no basename fallback
        let folder_doc = create_folder_doc(&[("/Notes/Ideas.md", "uuid-ideas")]);
        let content_doc = create_content_doc("See [[Ideas]].");
        index_content_into_folder("uuid-source", &content_doc, &folder_doc).unwrap();
        assert!(read_backlinks(&folder_doc, "uuid-ideas").is_empty());
    }

    #[test]
    fn type_filter_skips_folders() {
        // A folder-type entry with matching path should not resolve
        let doc = Doc::new();
        {
            let mut txn = doc.transact_mut();
            let filemeta = txn.get_or_insert_map("filemeta_v0");
            // Add a "folder" type entry at /Ideas.md
            let mut map = HashMap::new();
            map.insert("id".to_string(), Any::String("uuid-folder".into()));
            map.insert("type".to_string(), Any::String("folder".into()));
            map.insert("version".to_string(), Any::Number(0.0));
            filemeta.insert(&mut txn, "/Ideas.md", Any::Map(map.into()));
        }
        let content_doc = create_content_doc("See [[Ideas]].");
        index_content_into_folder("uuid-source", &content_doc, &doc).unwrap();
        assert!(read_backlinks(&doc, "uuid-folder").is_empty());
    }

    #[test]
    fn resolves_link_with_spaces_in_path() {
        let folder_doc = create_folder_doc(&[
            ("/Source.md", "uuid-source"),
            ("/Course YAML examples.md", "uuid-yaml"),
        ]);
        let content_doc = create_content_doc("See [[Course YAML examples]].");
        index_content_into_folder("uuid-source", &content_doc, &folder_doc).unwrap();
        assert_eq!(
            read_backlinks(&folder_doc, "uuid-yaml"),
            vec!["uuid-source"]
        );
    }

    #[test]
    fn cross_folder_uses_absolute_only() {
        // Source in folder_a, target in folder_b.
        // [[Lens Edu/Ideas]] should resolve via absolute path in virtual tree.
        let folder_a = create_folder_doc(&[("/Notes/Source.md", "uuid-source")]);
        set_folder_name(&folder_a, "Lens");
        let folder_b = create_folder_doc(&[
            ("/Notes/Ideas.md", "uuid-nested"),
            ("/Ideas.md", "uuid-root"),
        ]);
        set_folder_name(&folder_b, "Lens Edu");
        let content_doc = create_content_doc("See [[Lens Edu/Ideas]].");
        index_content_into_folders("uuid-source", &content_doc, &[&folder_a, &folder_b]).unwrap();
        // Absolute /Lens Edu/Ideas.md matches /Ideas.md in folder_b
        assert_eq!(read_backlinks(&folder_b, "uuid-root"), vec!["uuid-source"]);
        assert!(read_backlinks(&folder_b, "uuid-nested").is_empty());
    }

    // === Cross-folder relative link with UI-visible folder name ===
    //
    // The folder name is stored in the Y.Doc's folder_config map and read by
    // read_folder_name(). This ensures virtual tree paths match what users see.
    #[test]
    fn cross_folder_relative_link_with_visible_folder_name() {
        let folder_a = create_folder_doc(&[("/Welcome.md", "uuid-welcome")]);
        let folder_b = create_folder_doc(&[("/Resources/Links.md", "uuid-links")]);

        // Set the folder names the user sees in the UI/sidebar.
        set_folder_name(&folder_a, "Relay Folder 1");
        set_folder_name(&folder_b, "Relay Folder 2");
        let content_doc =
            create_content_doc("Check [[../Relay Folder 2/Resources/Links]] for resources.");

        index_content_into_folders("uuid-welcome", &content_doc, &[&folder_a, &folder_b]).unwrap();

        // This SHOULD create a backlink in folder_b for Links.md
        assert_eq!(
            read_backlinks(&folder_b, "uuid-links"),
            vec!["uuid-welcome"],
            "cross-folder relative link using visible folder name should resolve"
        );
    }

    mod virtual_tree_tests {
        use super::super::*;

        fn spec_entries() -> Vec<VirtualEntry> {
            vec![
                VirtualEntry {
                    virtual_path: "/Relay Folder 1/Welcome.md".into(),
                    entry_type: "markdown".into(),
                    id: "W".into(),
                    folder_idx: 0,
                },
                VirtualEntry {
                    virtual_path: "/Relay Folder 1/Getting Started.md".into(),
                    entry_type: "markdown".into(),
                    id: "GS".into(),
                    folder_idx: 0,
                },
                VirtualEntry {
                    virtual_path: "/Relay Folder 1/Notes".into(),
                    entry_type: "folder".into(),
                    id: "f-notes".into(),
                    folder_idx: 0,
                },
                VirtualEntry {
                    virtual_path: "/Relay Folder 1/Notes/Ideas.md".into(),
                    entry_type: "markdown".into(),
                    id: "I".into(),
                    folder_idx: 0,
                },
                VirtualEntry {
                    virtual_path: "/Relay Folder 1/Projects".into(),
                    entry_type: "folder".into(),
                    id: "f-proj".into(),
                    folder_idx: 0,
                },
                VirtualEntry {
                    virtual_path: "/Relay Folder 1/Projects/Roadmap.md".into(),
                    entry_type: "markdown".into(),
                    id: "R".into(),
                    folder_idx: 0,
                },
                VirtualEntry {
                    virtual_path: "/Relay Folder 2/Course Notes.md".into(),
                    entry_type: "markdown".into(),
                    id: "CN".into(),
                    folder_idx: 1,
                },
                VirtualEntry {
                    virtual_path: "/Relay Folder 2/Syllabus.md".into(),
                    entry_type: "markdown".into(),
                    id: "S".into(),
                    folder_idx: 1,
                },
                VirtualEntry {
                    virtual_path: "/Relay Folder 2/Resources".into(),
                    entry_type: "folder".into(),
                    id: "f-res".into(),
                    folder_idx: 1,
                },
                VirtualEntry {
                    virtual_path: "/Relay Folder 2/Resources/Links.md".into(),
                    entry_type: "markdown".into(),
                    id: "L".into(),
                    folder_idx: 1,
                },
            ]
        }

        // === From [W] — /Relay Folder 1/Welcome.md ===
        const W: &str = "/Relay Folder 1/Welcome.md";

        #[test]
        fn w_getting_started() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("Getting Started", Some(W), &e).map(|e| e.id.as_str()),
                Some("GS")
            );
        }
        #[test]
        fn w_notes_ideas() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("Notes/Ideas", Some(W), &e).map(|e| e.id.as_str()),
                Some("I")
            );
        }
        #[test]
        fn w_ideas_no_match() {
            let e = spec_entries();
            assert!(resolve_in_virtual_tree("Ideas", Some(W), &e).is_none());
        }
        #[test]
        fn w_nonexistent() {
            let e = spec_entries();
            assert!(resolve_in_virtual_tree("Nonexistent", Some(W), &e).is_none());
        }
        #[test]
        fn w_cross_folder_absolute() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("Relay Folder 2/Syllabus", Some(W), &e)
                    .map(|e| e.id.as_str()),
                Some("S")
            );
        }
        #[test]
        fn w_cross_folder_relative() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("../Relay Folder 2/Syllabus", Some(W), &e)
                    .map(|e| e.id.as_str()),
                Some("S")
            );
        }

        // === From [I] — /Relay Folder 1/Notes/Ideas.md ===
        const I_PATH: &str = "/Relay Folder 1/Notes/Ideas.md";

        #[test]
        fn i_parent_welcome() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("../Welcome", Some(I_PATH), &e).map(|e| e.id.as_str()),
                Some("W")
            );
        }
        #[test]
        fn i_cousin_roadmap() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("../Projects/Roadmap", Some(I_PATH), &e)
                    .map(|e| e.id.as_str()),
                Some("R")
            );
        }
        #[test]
        fn i_parent_getting_started() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("../Getting Started", Some(I_PATH), &e)
                    .map(|e| e.id.as_str()),
                Some("GS")
            );
        }
        #[test]
        fn i_welcome_no_match() {
            let e = spec_entries();
            assert!(resolve_in_virtual_tree("Welcome", Some(I_PATH), &e).is_none());
        }
        #[test]
        fn i_getting_started_no_match() {
            let e = spec_entries();
            assert!(resolve_in_virtual_tree("Getting Started", Some(I_PATH), &e).is_none());
        }
        #[test]
        fn i_self_link() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("Ideas", Some(I_PATH), &e).map(|e| e.id.as_str()),
                Some("I")
            );
        }
        #[test]
        fn i_absolute_cross_folder() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("Relay Folder 1/Welcome", Some(I_PATH), &e)
                    .map(|e| e.id.as_str()),
                Some("W")
            );
        }

        // === From [R] — /Relay Folder 1/Projects/Roadmap.md ===
        const R_PATH: &str = "/Relay Folder 1/Projects/Roadmap.md";

        #[test]
        fn r_cousin_ideas() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("../Notes/Ideas", Some(R_PATH), &e).map(|e| e.id.as_str()),
                Some("I")
            );
        }
        #[test]
        fn r_parent_welcome() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("../Welcome", Some(R_PATH), &e).map(|e| e.id.as_str()),
                Some("W")
            );
        }
        #[test]
        fn r_notes_ideas_no_match() {
            let e = spec_entries();
            assert!(resolve_in_virtual_tree("Notes/Ideas", Some(R_PATH), &e).is_none());
        }
        #[test]
        fn r_welcome_no_match() {
            let e = spec_entries();
            assert!(resolve_in_virtual_tree("Welcome", Some(R_PATH), &e).is_none());
        }

        // === From [L] — /Relay Folder 2/Resources/Links.md ===
        const L_PATH: &str = "/Relay Folder 2/Resources/Links.md";

        #[test]
        fn l_parent_syllabus() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("../Syllabus", Some(L_PATH), &e).map(|e| e.id.as_str()),
                Some("S")
            );
        }
        #[test]
        fn l_parent_course_notes() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("../Course Notes", Some(L_PATH), &e).map(|e| e.id.as_str()),
                Some("CN")
            );
        }
        #[test]
        fn l_syllabus_no_match() {
            let e = spec_entries();
            assert!(resolve_in_virtual_tree("Syllabus", Some(L_PATH), &e).is_none());
        }
        #[test]
        fn l_cross_folder_relative() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("../../Relay Folder 1/Notes/Ideas", Some(L_PATH), &e)
                    .map(|e| e.id.as_str()),
                Some("I")
            );
        }
        #[test]
        fn l_cross_folder_relative_welcome() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("../../Relay Folder 1/Welcome", Some(L_PATH), &e)
                    .map(|e| e.id.as_str()),
                Some("W")
            );
        }
        #[test]
        fn l_cross_folder_absolute() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("Relay Folder 1/Notes/Ideas", Some(L_PATH), &e)
                    .map(|e| e.id.as_str()),
                Some("I")
            );
        }
        #[test]
        fn l_nonexistent_folder() {
            let e = spec_entries();
            assert!(
                resolve_in_virtual_tree("../../Nonexistent Folder/File", Some(L_PATH), &e)
                    .is_none()
            );
        }

        // === From [CN] — /Relay Folder 2/Course Notes.md ===
        const CN_PATH: &str = "/Relay Folder 2/Course Notes.md";

        #[test]
        fn cn_syllabus() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("Syllabus", Some(CN_PATH), &e).map(|e| e.id.as_str()),
                Some("S")
            );
        }
        #[test]
        fn cn_resources_links() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("Resources/Links", Some(CN_PATH), &e)
                    .map(|e| e.id.as_str()),
                Some("L")
            );
        }
        #[test]
        fn cn_cross_folder_relative() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("../Relay Folder 1/Welcome", Some(CN_PATH), &e)
                    .map(|e| e.id.as_str()),
                Some("W")
            );
        }
        #[test]
        fn cn_cross_folder_absolute() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("Relay Folder 1/Welcome", Some(CN_PATH), &e)
                    .map(|e| e.id.as_str()),
                Some("W")
            );
        }

        // === Type filtering ===
        #[test]
        fn folder_entries_never_resolve() {
            let e = spec_entries();
            assert!(resolve_in_virtual_tree("Notes", Some(W), &e).is_none());
        }

        // === No source path (absolute only) ===
        #[test]
        fn no_source_absolute_works() {
            let e = spec_entries();
            assert_eq!(
                resolve_in_virtual_tree("Relay Folder 1/Welcome", None, &e).map(|e| e.id.as_str()),
                Some("W")
            );
        }
        #[test]
        fn no_source_bare_name_fails() {
            let e = spec_entries();
            assert!(resolve_in_virtual_tree("Welcome", None, &e).is_none());
        }
    }

    mod cross_folder_rename_tests {
        use super::*;

        /// Shared fixture: two folders, each with a file named "Foo"
        fn two_folder_fixture() -> (Doc, Doc) {
            let folder_a =
                create_folder_doc(&[("/Foo.md", "uuid-foo-a"), ("/Notes.md", "uuid-notes-a")]);
            set_folder_name(&folder_a, "Relay Folder 1");

            let folder_b =
                create_folder_doc(&[("/Foo.md", "uuid-foo-b"), ("/Journal.md", "uuid-journal-b")]);
            set_folder_name(&folder_b, "Relay Folder 2");

            (folder_a, folder_b)
        }

        /// Helper: seed indexer cache, rename a file in filemeta, detect renames.
        fn rename_in_folder(
            indexer: &LinkIndexer,
            folder_doc: &Doc,
            folder_cache_key: &str,
            old_path: &str,
            new_path: &str,
            uuid: &str,
        ) -> Vec<RenameEvent> {
            // Seed cache on first call
            indexer.detect_renames(folder_cache_key, folder_doc);

            // Rename: remove old path, add new path with same UUID
            {
                let mut txn = folder_doc.transact_mut();
                let filemeta = txn.get_or_insert_map("filemeta_v0");
                filemeta.remove(&mut txn, old_path);
                let mut map = HashMap::new();
                map.insert("id".to_string(), Any::String(uuid.into()));
                map.insert("type".to_string(), Any::String("markdown".into()));
                map.insert("version".to_string(), Any::Number(0.0));
                filemeta.insert(&mut txn, new_path, Any::Map(map.into()));
            }

            indexer.detect_renames(folder_cache_key, folder_doc)
        }

        /// Helper: apply renames to a content doc using backlinks from a folder doc,
        /// with resolution-aware disambiguation.
        fn apply_renames_to_doc(
            renames: &[RenameEvent],
            folder_docs: &[&Doc],
            folder_names: &[&str],
            rename_folder_doc: &Doc,
            rename_folder_name: &str,
            source_uuid: &str,
            content_doc: &Doc,
        ) {
            let mut entries = build_virtual_entries(folder_docs, folder_names);

            // Patch virtual entries: restore old paths for renamed files so that
            // link resolution works against the pre-rename state.
            for rename in renames {
                let old_virtual_path = format!("/{}{}", rename_folder_name, rename.old_path);
                for entry in entries.iter_mut() {
                    if entry.id == rename.uuid {
                        entry.virtual_path = old_virtual_path.clone();
                    }
                }
            }

            let source_virtual_path: Option<String> = entries
                .iter()
                .find(|e| e.id == source_uuid)
                .map(|e| e.virtual_path.clone());

            for rename in renames {
                let txn = rename_folder_doc.transact();
                if let Some(backlinks) = txn.get_map("backlinks_v0") {
                    let source_uuids = read_backlinks_array(&backlinks, &txn, &rename.uuid);
                    drop(txn);

                    if source_uuids.contains(&source_uuid.to_string()) {
                        let old_virtual_path =
                            format!("/{}{}", rename_folder_name, rename.old_path);
                        update_wikilinks_in_doc_resolved(
                            content_doc,
                            &rename.old_name,
                            &rename.new_name,
                            source_virtual_path.as_deref(),
                            &entries,
                            &old_virtual_path,
                        )
                        .unwrap();
                    }
                }
            }
        }

        // --- Backlink indexing with same-name disambiguation ---

        #[test]
        fn same_name_bare_link_resolves_within_own_folder() {
            let (folder_a, folder_b) = two_folder_fixture();
            // Notes in folder A links to [[Foo]] — should resolve to folder A's Foo (relative)
            let notes_doc = create_content_doc("See [[Foo]]");

            index_content_into_folders("uuid-notes-a", &notes_doc, &[&folder_a, &folder_b])
                .unwrap();

            // Backlink on folder A's Foo (correct — same-folder relative resolution)
            assert_eq!(
                read_backlinks(&folder_a, "uuid-foo-a"),
                vec!["uuid-notes-a"]
            );
            // No backlink on folder B's Foo
            assert!(read_backlinks(&folder_b, "uuid-foo-b").is_empty());
        }

        #[test]
        fn same_name_cross_folder_explicit_link() {
            let (folder_a, folder_b) = two_folder_fixture();
            // Notes in folder A links to [[Relay Folder 2/Foo]] — explicit cross-folder
            let notes_doc = create_content_doc("See [[Relay Folder 2/Foo]]");

            index_content_into_folders("uuid-notes-a", &notes_doc, &[&folder_a, &folder_b])
                .unwrap();

            // Backlink on folder B's Foo
            assert_eq!(
                read_backlinks(&folder_b, "uuid-foo-b"),
                vec!["uuid-notes-a"]
            );
            // No backlink on folder A's Foo
            assert!(read_backlinks(&folder_a, "uuid-foo-a").is_empty());
        }

        #[test]
        fn same_name_both_bare_and_cross_folder_links() {
            let (folder_a, folder_b) = two_folder_fixture();
            // Notes links to BOTH Foos: bare resolves to own folder, explicit to other
            let notes_doc = create_content_doc("[[Foo]] and [[Relay Folder 2/Foo]]");

            index_content_into_folders("uuid-notes-a", &notes_doc, &[&folder_a, &folder_b])
                .unwrap();

            // folder A's Foo: backlinked by notes-a (bare [[Foo]])
            assert_eq!(
                read_backlinks(&folder_a, "uuid-foo-a"),
                vec!["uuid-notes-a"]
            );
            // folder B's Foo: backlinked by notes-a ([[Relay Folder 2/Foo]])
            assert_eq!(
                read_backlinks(&folder_b, "uuid-foo-b"),
                vec!["uuid-notes-a"]
            );
        }

        #[test]
        fn same_name_other_folder_bare_link() {
            let (folder_a, folder_b) = two_folder_fixture();
            // Journal in folder B links to [[Foo]] — resolves to folder B's Foo
            let journal_doc = create_content_doc("See [[Foo]]");

            index_content_into_folders("uuid-journal-b", &journal_doc, &[&folder_a, &folder_b])
                .unwrap();

            assert_eq!(
                read_backlinks(&folder_b, "uuid-foo-b"),
                vec!["uuid-journal-b"]
            );
            assert!(read_backlinks(&folder_a, "uuid-foo-a").is_empty());
        }

        // --- Cross-folder rename: path-qualified links (Bug 1) ---

        #[test]
        fn cross_folder_rename_updates_path_qualified_link() {
            let (folder_a, folder_b) = two_folder_fixture();
            let notes_doc = create_content_doc("See [[Relay Folder 2/Foo]] for details");

            // Index: notes-a links to folder B's Foo
            index_content_into_folders("uuid-notes-a", &notes_doc, &[&folder_a, &folder_b])
                .unwrap();
            assert_eq!(
                read_backlinks(&folder_b, "uuid-foo-b"),
                vec!["uuid-notes-a"]
            );

            // Rename folder B's Foo -> Qux
            let (indexer, _rx) = LinkIndexer::new();
            let renames = rename_in_folder(
                &indexer,
                &folder_b,
                "folder-b",
                "/Foo.md",
                "/Qux.md",
                "uuid-foo-b",
            );
            assert_eq!(renames.len(), 1);
            assert_eq!(renames[0].old_name, "Foo");
            assert_eq!(renames[0].new_name, "Qux");

            // Apply rename to backlinkers
            apply_renames_to_doc(
                &renames,
                &[&folder_a, &folder_b],
                &["Relay Folder 1", "Relay Folder 2"],
                &folder_b,
                "Relay Folder 2",
                "uuid-notes-a",
                &notes_doc,
            );

            // BUG 1: [[Relay Folder 2/Foo]] should become [[Relay Folder 2/Qux]]
            assert_eq!(
                read_contents(&notes_doc),
                "See [[Relay Folder 2/Qux]] for details",
            );
        }

        #[test]
        fn cross_folder_rename_preserves_anchor() {
            let (folder_a, folder_b) = two_folder_fixture();
            let notes_doc = create_content_doc("See [[Relay Folder 2/Foo#Section]]");

            index_content_into_folders("uuid-notes-a", &notes_doc, &[&folder_a, &folder_b])
                .unwrap();

            let (indexer, _rx) = LinkIndexer::new();
            let renames = rename_in_folder(
                &indexer,
                &folder_b,
                "folder-b",
                "/Foo.md",
                "/Qux.md",
                "uuid-foo-b",
            );

            apply_renames_to_doc(
                &renames,
                &[&folder_a, &folder_b],
                &["Relay Folder 1", "Relay Folder 2"],
                &folder_b,
                "Relay Folder 2",
                "uuid-notes-a",
                &notes_doc,
            );

            assert_eq!(
                read_contents(&notes_doc),
                "See [[Relay Folder 2/Qux#Section]]",
            );
        }

        #[test]
        fn cross_folder_rename_preserves_alias() {
            let (folder_a, folder_b) = two_folder_fixture();
            let notes_doc = create_content_doc("See [[Relay Folder 2/Foo|Display]]");

            index_content_into_folders("uuid-notes-a", &notes_doc, &[&folder_a, &folder_b])
                .unwrap();

            let (indexer, _rx) = LinkIndexer::new();
            let renames = rename_in_folder(
                &indexer,
                &folder_b,
                "folder-b",
                "/Foo.md",
                "/Qux.md",
                "uuid-foo-b",
            );

            apply_renames_to_doc(
                &renames,
                &[&folder_a, &folder_b],
                &["Relay Folder 1", "Relay Folder 2"],
                &folder_b,
                "Relay Folder 2",
                "uuid-notes-a",
                &notes_doc,
            );

            assert_eq!(
                read_contents(&notes_doc),
                "See [[Relay Folder 2/Qux|Display]]",
            );
        }

        // --- Same-name disambiguation on rename (Bug 2) ---

        #[test]
        fn rename_same_name_only_updates_correct_links() {
            // notes-a links to BOTH Foos: [[Foo]] (-> foo-a) and [[Relay Folder 2/Foo]] (-> foo-b)
            let (folder_a, folder_b) = two_folder_fixture();
            let notes_doc = create_content_doc("[[Foo]] and [[Relay Folder 2/Foo]]");

            index_content_into_folders("uuid-notes-a", &notes_doc, &[&folder_a, &folder_b])
                .unwrap();

            // Verify both backlinks exist
            assert_eq!(
                read_backlinks(&folder_a, "uuid-foo-a"),
                vec!["uuid-notes-a"]
            );
            assert_eq!(
                read_backlinks(&folder_b, "uuid-foo-b"),
                vec!["uuid-notes-a"]
            );

            // Rename folder B's Foo -> Qux
            let (indexer, _rx) = LinkIndexer::new();
            let renames = rename_in_folder(
                &indexer,
                &folder_b,
                "folder-b",
                "/Foo.md",
                "/Qux.md",
                "uuid-foo-b",
            );
            assert_eq!(renames.len(), 1);

            // Apply rename: only folder B's backlinkers should be updated
            apply_renames_to_doc(
                &renames,
                &[&folder_a, &folder_b],
                &["Relay Folder 1", "Relay Folder 2"],
                &folder_b,
                "Relay Folder 2",
                "uuid-notes-a",
                &notes_doc,
            );

            // Expected: [[Foo]] UNCHANGED (points to folder A's Foo),
            //           [[Relay Folder 2/Foo]] -> [[Relay Folder 2/Qux]]
            assert_eq!(
                read_contents(&notes_doc),
                "[[Foo]] and [[Relay Folder 2/Qux]]",
            );
        }

        #[test]
        fn rename_same_name_from_other_folder_perspective() {
            // journal-b links to [[Foo]] (-> foo-b) and [[Relay Folder 1/Foo]] (-> foo-a)
            let (folder_a, folder_b) = two_folder_fixture();
            let journal_doc = create_content_doc("[[Foo]] and [[Relay Folder 1/Foo]]");

            index_content_into_folders("uuid-journal-b", &journal_doc, &[&folder_a, &folder_b])
                .unwrap();

            assert_eq!(
                read_backlinks(&folder_b, "uuid-foo-b"),
                vec!["uuid-journal-b"]
            );
            assert_eq!(
                read_backlinks(&folder_a, "uuid-foo-a"),
                vec!["uuid-journal-b"]
            );

            // Rename folder A's Foo -> Baz
            let (indexer, _rx) = LinkIndexer::new();
            let renames = rename_in_folder(
                &indexer,
                &folder_a,
                "folder-a",
                "/Foo.md",
                "/Baz.md",
                "uuid-foo-a",
            );
            assert_eq!(renames.len(), 1);

            apply_renames_to_doc(
                &renames,
                &[&folder_a, &folder_b],
                &["Relay Folder 1", "Relay Folder 2"],
                &folder_a,
                "Relay Folder 1",
                "uuid-journal-b",
                &journal_doc,
            );

            // Expected: [[Foo]] UNCHANGED (points to folder B's Foo),
            //           [[Relay Folder 1/Foo]] -> [[Relay Folder 1/Baz]]
            assert_eq!(
                read_contents(&journal_doc),
                "[[Foo]] and [[Relay Folder 1/Baz]]",
            );
        }

        #[test]
        fn rename_same_name_bare_link_updated_in_own_folder() {
            // journal-b links to [[Foo]] which resolves to folder B's Foo (same-folder)
            let (folder_a, folder_b) = two_folder_fixture();
            let journal_doc = create_content_doc("See [[Foo]]");

            index_content_into_folders("uuid-journal-b", &journal_doc, &[&folder_a, &folder_b])
                .unwrap();

            assert_eq!(
                read_backlinks(&folder_b, "uuid-foo-b"),
                vec!["uuid-journal-b"]
            );

            // Rename folder B's Foo -> Qux
            let (indexer, _rx) = LinkIndexer::new();
            let renames = rename_in_folder(
                &indexer,
                &folder_b,
                "folder-b",
                "/Foo.md",
                "/Qux.md",
                "uuid-foo-b",
            );

            apply_renames_to_doc(
                &renames,
                &[&folder_a, &folder_b],
                &["Relay Folder 1", "Relay Folder 2"],
                &folder_b,
                "Relay Folder 2",
                "uuid-journal-b",
                &journal_doc,
            );

            // Bare [[Foo]] in folder B's own doc SHOULD be updated (same-folder rename)
            assert_eq!(read_contents(&journal_doc), "See [[Qux]]");
        }

        // --- Cross-folder rename: relative path links ---

        #[test]
        fn cross_folder_rename_updates_relative_path_link() {
            let (folder_a, folder_b) = two_folder_fixture();
            // Relative cross-folder link: ../Relay Folder 2/Foo
            let notes_doc = create_content_doc("See [[../Relay Folder 2/Foo]]");

            index_content_into_folders("uuid-notes-a", &notes_doc, &[&folder_a, &folder_b])
                .unwrap();
            assert_eq!(
                read_backlinks(&folder_b, "uuid-foo-b"),
                vec!["uuid-notes-a"]
            );

            let (indexer, _rx) = LinkIndexer::new();
            let renames = rename_in_folder(
                &indexer,
                &folder_b,
                "folder-b",
                "/Foo.md",
                "/Qux.md",
                "uuid-foo-b",
            );

            apply_renames_to_doc(
                &renames,
                &[&folder_a, &folder_b],
                &["Relay Folder 1", "Relay Folder 2"],
                &folder_b,
                "Relay Folder 2",
                "uuid-notes-a",
                &notes_doc,
            );

            assert_eq!(read_contents(&notes_doc), "See [[../Relay Folder 2/Qux]]",);
        }
    }

    // === remove_doc_from_backlinks tests ===

    #[test]
    fn remove_doc_from_backlinks_clears_source() {
        // Setup: folder with target_A having backlinks [source_X, source_Y]
        let folder_doc = create_folder_doc(&[
            ("/TargetA.md", "uuid-target-a"),
            ("/SourceX.md", "uuid-source-x"),
            ("/SourceY.md", "uuid-source-y"),
        ]);

        // Manually populate backlinks_v0
        {
            let mut txn = folder_doc.transact_mut_with("link-indexer");
            let backlinks = txn.get_or_insert_map("backlinks_v0");
            let arr = vec![
                Any::String("uuid-source-x".into()),
                Any::String("uuid-source-y".into()),
            ];
            backlinks.insert(&mut txn, "uuid-target-a", arr);
        }

        // Verify precondition
        assert_eq!(
            read_backlinks(&folder_doc, "uuid-target-a"),
            vec!["uuid-source-x", "uuid-source-y"]
        );

        // Act: remove source_X from all backlinks
        let modified = remove_doc_from_backlinks("uuid-source-x", &[&folder_doc]).unwrap();

        // Assert: target_A's backlinks are now just [source_Y]
        assert_eq!(modified, 1);
        assert_eq!(
            read_backlinks(&folder_doc, "uuid-target-a"),
            vec!["uuid-source-y"]
        );
    }

    #[test]
    fn remove_doc_from_backlinks_removes_empty_arrays() {
        // Setup: target_B's backlinks are just [source_X]
        let folder_doc = create_folder_doc(&[
            ("/TargetB.md", "uuid-target-b"),
            ("/SourceX.md", "uuid-source-x"),
        ]);
        {
            let mut txn = folder_doc.transact_mut_with("link-indexer");
            let backlinks = txn.get_or_insert_map("backlinks_v0");
            let arr = vec![Any::String("uuid-source-x".into())];
            backlinks.insert(&mut txn, "uuid-target-b", arr);
        }

        // Act
        let modified = remove_doc_from_backlinks("uuid-source-x", &[&folder_doc]).unwrap();

        // Assert: key removed entirely from backlinks_v0
        assert_eq!(modified, 1);
        assert!(
            read_backlinks(&folder_doc, "uuid-target-b").is_empty(),
            "empty backlinks should be removed entirely"
        );
        // Verify the key itself is gone
        let txn = folder_doc.transact();
        let backlinks = txn.get_map("backlinks_v0").unwrap();
        assert!(
            backlinks.get(&txn, "uuid-target-b").is_none(),
            "key should be removed from backlinks_v0, not just empty"
        );
    }

    #[test]
    fn remove_doc_from_backlinks_idempotent() {
        let folder_doc = create_folder_doc(&[("/TargetA.md", "uuid-target-a")]);
        {
            let mut txn = folder_doc.transact_mut_with("link-indexer");
            let backlinks = txn.get_or_insert_map("backlinks_v0");
            let arr = vec![Any::String("uuid-other".into())];
            backlinks.insert(&mut txn, "uuid-target-a", arr);
        }

        // Removing a UUID that is not in any backlinks array
        let modified = remove_doc_from_backlinks("uuid-nonexistent", &[&folder_doc]).unwrap();
        assert_eq!(modified, 0, "should return 0 when source not found");

        // Backlinks unchanged
        assert_eq!(
            read_backlinks(&folder_doc, "uuid-target-a"),
            vec!["uuid-other"]
        );
    }

    #[test]
    fn remove_doc_from_backlinks_multi_folder() {
        // Source appears in backlinks across 2 folder docs
        let folder_a = create_folder_doc(&[
            ("/TargetA.md", "uuid-target-a"),
            ("/SourceX.md", "uuid-source-x"),
        ]);
        let folder_b = create_folder_doc(&[
            ("/TargetB.md", "uuid-target-b"),
            ("/SourceX.md", "uuid-source-x"),
        ]);

        // Populate backlinks in both folders
        {
            let mut txn = folder_a.transact_mut_with("link-indexer");
            let backlinks = txn.get_or_insert_map("backlinks_v0");
            backlinks.insert(
                &mut txn,
                "uuid-target-a",
                vec![Any::String("uuid-source-x".into())],
            );
        }
        {
            let mut txn = folder_b.transact_mut_with("link-indexer");
            let backlinks = txn.get_or_insert_map("backlinks_v0");
            backlinks.insert(
                &mut txn,
                "uuid-target-b",
                vec![
                    Any::String("uuid-source-x".into()),
                    Any::String("uuid-other".into()),
                ],
            );
        }

        // Act: remove source_X from both folders
        let modified = remove_doc_from_backlinks("uuid-source-x", &[&folder_a, &folder_b]).unwrap();

        // Assert: both folders cleaned
        assert_eq!(modified, 2, "should modify arrays in both folders");
        assert!(read_backlinks(&folder_a, "uuid-target-a").is_empty());
        assert_eq!(
            read_backlinks(&folder_b, "uuid-target-b"),
            vec!["uuid-other"]
        );
    }

    // === move_document tests ===

    mod move_document_tests {
        use super::*;
        use crate::doc_resolver::DocumentResolver;

        const RELAY_ID: &str = "cb696037-0f72-4e93-8717-4e433129d789";

        fn folder0_id() -> String {
            format!("{}-aaaa0000-0000-0000-0000-000000000000", RELAY_ID)
        }

        fn folder1_id() -> String {
            format!("{}-bbbb0000-0000-0000-0000-000000000000", RELAY_ID)
        }

        /// Build a DocumentResolver from bare Y.Docs using the public update_folder_from_doc.
        fn build_resolver(folder_specs: &[(&str, &Doc)]) -> DocumentResolver {
            let resolver = DocumentResolver::new();
            for (doc_id, doc) in folder_specs {
                resolver.update_folder_from_doc(doc_id, doc);
            }
            resolver
        }

        #[test]
        fn move_within_folder_updates_filemeta_path() {
            // Move /Photosynthesis.md -> /Biology/Photosynthesis.md within same folder
            let folder = create_folder_doc(&[
                ("/Photosynthesis.md", "uuid-photo"),
                ("/Other.md", "uuid-other"),
            ]);
            set_folder_name(&folder, "Lens");
            let f0id = folder0_id();

            let resolver = build_resolver(&[(&f0id, &folder)]);
            let content_docs = HashMap::new();

            let result = move_document(
                "uuid-photo",
                "/Biology/Photosynthesis.md",
                &folder,
                &folder, // same folder
                &[&folder],
                &["Lens"],
                &resolver,
                &content_docs,
            )
            .expect("move should succeed");

            assert_eq!(result.old_path, "/Photosynthesis.md");
            assert_eq!(result.new_path, "/Biology/Photosynthesis.md");
            assert_eq!(result.old_folder_name, "Lens");
            assert_eq!(result.new_folder_name, "Lens");

            // Old path removed from filemeta
            let txn = folder.transact();
            let filemeta = txn.get_map("filemeta_v0").unwrap();
            assert!(
                filemeta.get(&txn, "/Photosynthesis.md").is_none(),
                "old path should be removed from filemeta"
            );
            // New path present in filemeta
            let new_entry = filemeta.get(&txn, "/Biology/Photosynthesis.md");
            assert!(new_entry.is_some(), "new path should be in filemeta");
            let new_id = extract_id_from_filemeta_entry(&new_entry.unwrap(), &txn);
            assert_eq!(
                new_id,
                Some("uuid-photo".to_string()),
                "UUID should be preserved"
            );
        }

        #[test]
        fn move_within_folder_rename_rewrites_backlinks() {
            // Rename /Photosynthesis.md -> /Photosynthesis_v2.md
            // Notes.md has [[Photosynthesis]] -> should become [[Photosynthesis_v2]]
            let folder = create_folder_doc(&[
                ("/Photosynthesis.md", "uuid-photo"),
                ("/Notes.md", "uuid-notes"),
            ]);
            set_folder_name(&folder, "Lens");
            let f0id = folder0_id();

            // Create content doc for Notes that links to Photosynthesis
            let notes_doc = create_content_doc("See [[Photosynthesis]] for details");

            // Index Notes -> creates backlinks
            index_content_into_folder("uuid-notes", &notes_doc, &folder).unwrap();
            assert_eq!(read_backlinks(&folder, "uuid-photo"), vec!["uuid-notes"]);

            let resolver = build_resolver(&[(&f0id, &folder)]);
            let mut content_docs = HashMap::new();
            content_docs.insert("uuid-notes".to_string(), &notes_doc as &Doc);

            let result = move_document(
                "uuid-photo",
                "/Photosynthesis_v2.md",
                &folder,
                &folder,
                &[&folder],
                &["Lens"],
                &resolver,
                &content_docs,
            )
            .expect("move should succeed");

            assert_eq!(result.links_rewritten, 1);
            assert_eq!(
                read_contents(&notes_doc),
                "See [[Photosynthesis_v2]] for details"
            );
        }

        #[test]
        fn move_cross_folder_updates_filemeta_in_both() {
            // Move from Lens /Photosynthesis.md -> Lens Edu /Photosynthesis.md
            let folder_a = create_folder_doc(&[("/Photosynthesis.md", "uuid-photo")]);
            set_folder_name(&folder_a, "Lens");
            let folder_b = create_folder_doc(&[("/Welcome.md", "uuid-welcome")]);
            set_folder_name(&folder_b, "Lens Edu");
            let f0id = folder0_id();
            let f1id = folder1_id();

            let resolver = build_resolver(&[(&f0id, &folder_a), (&f1id, &folder_b)]);
            let content_docs = HashMap::new();

            let result = move_document(
                "uuid-photo",
                "/Photosynthesis.md",
                &folder_a,
                &folder_b, // different folder
                &[&folder_a, &folder_b],
                &["Lens", "Lens Edu"],
                &resolver,
                &content_docs,
            )
            .expect("move should succeed");

            assert_eq!(result.old_folder_name, "Lens");
            assert_eq!(result.new_folder_name, "Lens Edu");

            // Source filemeta: no entry
            let txn_a = folder_a.transact();
            let filemeta_a = txn_a.get_map("filemeta_v0").unwrap();
            assert!(
                filemeta_a.get(&txn_a, "/Photosynthesis.md").is_none(),
                "source filemeta should not have entry after cross-folder move"
            );

            // Target filemeta: has entry
            let txn_b = folder_b.transact();
            let filemeta_b = txn_b.get_map("filemeta_v0").unwrap();
            let entry = filemeta_b.get(&txn_b, "/Photosynthesis.md");
            assert!(entry.is_some(), "target filemeta should have the entry");
            let id = extract_id_from_filemeta_entry(&entry.unwrap(), &txn_b);
            assert_eq!(
                id,
                Some("uuid-photo".to_string()),
                "UUID should be preserved"
            );

            // DocumentResolver updated
            let new_resolved = resolver.resolve_path("Lens Edu/Photosynthesis.md");
            assert!(new_resolved.is_some(), "new path should be resolvable");
            assert_eq!(new_resolved.unwrap().uuid, "uuid-photo");
            assert!(
                resolver.resolve_path("Lens/Photosynthesis.md").is_none(),
                "old path should not be resolvable"
            );
        }

        #[test]
        fn move_with_no_backlinkers_succeeds() {
            // Move a document that nobody links to
            let folder =
                create_folder_doc(&[("/Lonely.md", "uuid-lonely"), ("/Other.md", "uuid-other")]);
            set_folder_name(&folder, "Lens");
            let f0id = folder0_id();

            let resolver = build_resolver(&[(&f0id, &folder)]);
            let content_docs = HashMap::new();

            let result = move_document(
                "uuid-lonely",
                "/Archive/Lonely.md",
                &folder,
                &folder,
                &[&folder],
                &["Lens"],
                &resolver,
                &content_docs,
            )
            .expect("move should succeed even with no backlinkers");

            assert_eq!(result.links_rewritten, 0);
        }

        #[test]
        fn move_preserves_uuid_in_resolver() {
            // After move, uuid_to_path returns the new path
            let folder = create_folder_doc(&[("/Photosynthesis.md", "uuid-photo")]);
            set_folder_name(&folder, "Lens");
            let f0id = folder0_id();

            let resolver = build_resolver(&[(&f0id, &folder)]);
            assert_eq!(
                resolver.path_for_uuid("uuid-photo"),
                Some("Lens/Photosynthesis.md".to_string())
            );

            let content_docs = HashMap::new();
            move_document(
                "uuid-photo",
                "/Biology/Photosynthesis.md",
                &folder,
                &folder,
                &[&folder],
                &["Lens"],
                &resolver,
                &content_docs,
            )
            .expect("move should succeed");

            // UUID still maps to a path, but the NEW path
            assert_eq!(
                resolver.path_for_uuid("uuid-photo"),
                Some("Lens/Biology/Photosynthesis.md".to_string())
            );
            // Old path gone
            assert!(resolver.resolve_path("Lens/Photosynthesis.md").is_none());
            // New path resolves
            let info = resolver
                .resolve_path("Lens/Biology/Photosynthesis.md")
                .unwrap();
            assert_eq!(info.uuid, "uuid-photo");
        }

        #[test]
        fn move_rename_preserves_anchors_and_aliases() {
            // [[Photosynthesis#Section]] -> [[NewName#Section]]
            // [[Photosynthesis|Display]] -> [[NewName|Display]]
            let folder = create_folder_doc(&[
                ("/Photosynthesis.md", "uuid-photo"),
                ("/Notes.md", "uuid-notes"),
            ]);
            set_folder_name(&folder, "Lens");
            let f0id = folder0_id();

            let notes_doc =
                create_content_doc("See [[Photosynthesis#Section]] and [[Photosynthesis|Display]]");
            index_content_into_folder("uuid-notes", &notes_doc, &folder).unwrap();
            assert_eq!(read_backlinks(&folder, "uuid-photo"), vec!["uuid-notes"]);

            let resolver = build_resolver(&[(&f0id, &folder)]);
            let mut content_docs = HashMap::new();
            content_docs.insert("uuid-notes".to_string(), &notes_doc as &Doc);

            let result = move_document(
                "uuid-photo",
                "/NewName.md",
                &folder,
                &folder,
                &[&folder],
                &["Lens"],
                &resolver,
                &content_docs,
            )
            .expect("move should succeed");

            assert_eq!(result.links_rewritten, 2);
            assert_eq!(
                read_contents(&notes_doc),
                "See [[NewName#Section]] and [[NewName|Display]]"
            );
        }

        // === Directory move tests (same basename, different path) ===
        // These test the bug: moving a file to a different directory without
        // changing the basename should still rewrite backlinks, because
        // relative wikilinks from sibling docs break.

        #[test]
        fn move_to_subfolder_rewrites_sibling_backlinks() {
            // Move /Welcome.md -> /Archive/Welcome.md (same basename, different dir)
            // /Getting Started.md has [[Welcome]] -> should become [[Archive/Welcome]]
            let folder = create_folder_doc(&[
                ("/Welcome.md", "uuid-welcome"),
                ("/Getting Started.md", "uuid-gs"),
            ]);
            set_folder_name(&folder, "Lens");
            let f0id = folder0_id();

            let gs_doc = create_content_doc("See [[Welcome]] for details");
            index_content_into_folder("uuid-gs", &gs_doc, &folder).unwrap();
            assert_eq!(read_backlinks(&folder, "uuid-welcome"), vec!["uuid-gs"]);

            let resolver = build_resolver(&[(&f0id, &folder)]);
            let mut content_docs = HashMap::new();
            content_docs.insert("uuid-gs".to_string(), &gs_doc as &Doc);

            let result = move_document(
                "uuid-welcome",
                "/Archive/Welcome.md",
                &folder,
                &folder,
                &[&folder],
                &["Lens"],
                &resolver,
                &content_docs,
            )
            .expect("move should succeed");

            assert_eq!(
                result.links_rewritten, 1,
                "sibling backlink should be rewritten when file moves to subfolder"
            );
            assert_eq!(
                read_contents(&gs_doc),
                "See [[Archive/Welcome]] for details",
                "wikilink should be updated with relative path to new location"
            );
        }

        #[test]
        fn move_to_subfolder_rewrites_nested_backlinks() {
            // Move /Welcome.md -> /Archive/Welcome.md
            // /Notes/Ideas.md has [[../Welcome]] -> should become [[../Archive/Welcome]]
            let folder = create_folder_doc(&[
                ("/Welcome.md", "uuid-welcome"),
                ("/Notes/Ideas.md", "uuid-ideas"),
                ("/Notes", "uuid-notes-folder"),
            ]);
            set_folder_name(&folder, "Lens");
            let f0id = folder0_id();

            let ideas_doc = create_content_doc("Check [[../Welcome]] for info");
            index_content_into_folder("uuid-ideas", &ideas_doc, &folder).unwrap();
            assert_eq!(read_backlinks(&folder, "uuid-welcome"), vec!["uuid-ideas"]);

            let resolver = build_resolver(&[(&f0id, &folder)]);
            let mut content_docs = HashMap::new();
            content_docs.insert("uuid-ideas".to_string(), &ideas_doc as &Doc);

            let result = move_document(
                "uuid-welcome",
                "/Archive/Welcome.md",
                &folder,
                &folder,
                &[&folder],
                &["Lens"],
                &resolver,
                &content_docs,
            )
            .expect("move should succeed");

            assert_eq!(
                result.links_rewritten, 1,
                "nested backlink should be rewritten when target moves to subfolder"
            );
            assert_eq!(
                read_contents(&ideas_doc),
                "Check [[../Archive/Welcome]] for info",
                "wikilink should use correct relative path from nested location"
            );
        }

        #[test]
        fn move_from_subfolder_to_root_rewrites_backlinks() {
            // Move /Archive/Welcome.md -> /Welcome.md
            // /Getting Started.md has [[Archive/Welcome]] -> should become [[Welcome]]
            let folder = create_folder_doc(&[
                ("/Archive/Welcome.md", "uuid-welcome"),
                ("/Archive", "uuid-archive-folder"),
                ("/Getting Started.md", "uuid-gs"),
            ]);
            set_folder_name(&folder, "Lens");
            let f0id = folder0_id();

            let gs_doc = create_content_doc("See [[Archive/Welcome]] for details");
            index_content_into_folder("uuid-gs", &gs_doc, &folder).unwrap();
            assert_eq!(read_backlinks(&folder, "uuid-welcome"), vec!["uuid-gs"]);

            let resolver = build_resolver(&[(&f0id, &folder)]);
            let mut content_docs = HashMap::new();
            content_docs.insert("uuid-gs".to_string(), &gs_doc as &Doc);

            let result = move_document(
                "uuid-welcome",
                "/Welcome.md",
                &folder,
                &folder,
                &[&folder],
                &["Lens"],
                &resolver,
                &content_docs,
            )
            .expect("move should succeed");

            assert_eq!(
                result.links_rewritten, 1,
                "backlink should be rewritten when file moves from subfolder to root"
            );
            assert_eq!(
                read_contents(&gs_doc),
                "See [[Welcome]] for details",
                "wikilink should simplify to basename when file moves to same level"
            );
        }

        #[test]
        fn cross_folder_move_same_basename_rewrites_backlinks() {
            // Move /Welcome.md from Lens -> Lens Edu (same basename, different folder)
            // /Getting Started.md in Lens has [[Welcome]] -> should become [[../Lens Edu/Welcome]]
            let folder_a = create_folder_doc(&[
                ("/Welcome.md", "uuid-welcome"),
                ("/Getting Started.md", "uuid-gs"),
            ]);
            set_folder_name(&folder_a, "Lens");
            let folder_b = create_folder_doc(&[("/Course Notes.md", "uuid-cn")]);
            set_folder_name(&folder_b, "Lens Edu");
            let f0id = folder0_id();
            let f1id = folder1_id();

            let gs_doc = create_content_doc("See [[Welcome]] for details");
            index_content_into_folders("uuid-gs", &gs_doc, &[&folder_a, &folder_b]).unwrap();
            assert_eq!(read_backlinks(&folder_a, "uuid-welcome"), vec!["uuid-gs"]);

            let resolver = build_resolver(&[(&f0id, &folder_a), (&f1id, &folder_b)]);
            let mut content_docs = HashMap::new();
            content_docs.insert("uuid-gs".to_string(), &gs_doc as &Doc);

            let result = move_document(
                "uuid-welcome",
                "/Welcome.md",
                &folder_a,
                &folder_b, // cross-folder
                &[&folder_a, &folder_b],
                &["Lens", "Lens Edu"],
                &resolver,
                &content_docs,
            )
            .expect("move should succeed");

            assert_eq!(
                result.links_rewritten, 1,
                "backlink should be rewritten for cross-folder move even with same basename"
            );
            // From /Lens/Getting Started.md, [[../Lens Edu/Welcome]] resolves to /Lens Edu/Welcome.md
            assert_eq!(
                read_contents(&gs_doc),
                "See [[../Lens Edu/Welcome]] for details",
                "wikilink should use cross-folder relative path"
            );
        }

        #[test]
        fn cross_folder_move_transfers_backlink_target_entry() {
            // uuid-gs links to uuid-welcome, creating backlinks_v0[uuid-welcome] = [uuid-gs] on folder_a.
            // Move uuid-welcome from folder_a to folder_b.
            // After: folder_a's backlinks_v0 should NOT have uuid-welcome key.
            //        folder_b's backlinks_v0 SHOULD have uuid-welcome with [uuid-gs].
            let folder_a = create_folder_doc(&[
                ("/Welcome.md", "uuid-welcome"),
                ("/Getting Started.md", "uuid-gs"),
            ]);
            set_folder_name(&folder_a, "Lens");
            let folder_b = create_folder_doc(&[("/Course Notes.md", "uuid-cn")]);
            set_folder_name(&folder_b, "Lens Edu");
            let f0id = folder0_id();
            let f1id = folder1_id();

            let gs_doc = create_content_doc("See [[Welcome]] for details");
            index_content_into_folders("uuid-gs", &gs_doc, &[&folder_a, &folder_b]).unwrap();
            assert_eq!(read_backlinks(&folder_a, "uuid-welcome"), vec!["uuid-gs"]);
            assert!(read_backlinks(&folder_b, "uuid-welcome").is_empty());

            let resolver = build_resolver(&[(&f0id, &folder_a), (&f1id, &folder_b)]);
            let mut content_docs = HashMap::new();
            content_docs.insert("uuid-gs".to_string(), &gs_doc as &Doc);

            let _result = move_document(
                "uuid-welcome",
                "/Welcome.md",
                &folder_a,
                &folder_b,
                &[&folder_a, &folder_b],
                &["Lens", "Lens Edu"],
                &resolver,
                &content_docs,
            )
            .expect("move should succeed");

            // Source folder's backlinks_v0 must NOT have the moved doc's key
            assert!(
                read_backlinks(&folder_a, "uuid-welcome").is_empty(),
                "source folder's backlinks_v0 should not have entry for moved doc"
            );
            // Target folder should have the transferred entry
            assert_eq!(
                read_backlinks(&folder_b, "uuid-welcome"),
                vec!["uuid-gs"],
                "target folder's backlinks_v0 should have the transferred entry"
            );
        }

        #[test]
        fn directory_move_preserves_anchors_and_aliases() {
            // Move /Welcome.md -> /Archive/Welcome.md
            // Notes.md has [[Welcome#Features]] and [[Welcome|Home]] ->
            //   should become [[Archive/Welcome#Features]] and [[Archive/Welcome|Home]]
            let folder =
                create_folder_doc(&[("/Welcome.md", "uuid-welcome"), ("/Notes.md", "uuid-notes")]);
            set_folder_name(&folder, "Lens");
            let f0id = folder0_id();

            let notes_doc = create_content_doc("See [[Welcome#Features]] and [[Welcome|Home]]");
            index_content_into_folder("uuid-notes", &notes_doc, &folder).unwrap();
            assert_eq!(read_backlinks(&folder, "uuid-welcome"), vec!["uuid-notes"]);

            let resolver = build_resolver(&[(&f0id, &folder)]);
            let mut content_docs = HashMap::new();
            content_docs.insert("uuid-notes".to_string(), &notes_doc as &Doc);

            let result = move_document(
                "uuid-welcome",
                "/Archive/Welcome.md",
                &folder,
                &folder,
                &[&folder],
                &["Lens"],
                &resolver,
                &content_docs,
            )
            .expect("move should succeed");

            assert_eq!(
                result.links_rewritten, 2,
                "both anchor and alias links should be rewritten"
            );
            assert_eq!(
                read_contents(&notes_doc),
                "See [[Archive/Welcome#Features]] and [[Archive/Welcome|Home]]",
                "anchors and aliases should be preserved with updated path"
            );
        }

        #[test]
        fn directory_move_only_rewrites_links_to_moved_file() {
            // Two files with same basename: /Welcome.md and /Notes/Welcome.md
            // /Getting Started.md has [[Welcome]] pointing to /Welcome.md (relative sibling)
            // /Notes/Ideas.md has [[Welcome]] pointing to /Notes/Welcome.md (relative sibling)
            // Move /Welcome.md -> /Archive/Welcome.md
            // Only /Getting Started.md should be rewritten (its link breaks)
            // /Notes/Ideas.md should NOT be rewritten (its link still points to /Notes/Welcome.md)
            let folder = create_folder_doc(&[
                ("/Welcome.md", "uuid-welcome"),
                ("/Notes/Welcome.md", "uuid-notes-welcome"),
                ("/Getting Started.md", "uuid-gs"),
                ("/Notes/Ideas.md", "uuid-ideas"),
                ("/Notes", "uuid-notes-folder"),
            ]);
            set_folder_name(&folder, "Lens");
            let f0id = folder0_id();

            let gs_doc = create_content_doc("See [[Welcome]] for details");
            let ideas_doc = create_content_doc("Check [[Welcome]] for info");
            index_content_into_folder("uuid-gs", &gs_doc, &folder).unwrap();
            index_content_into_folder("uuid-ideas", &ideas_doc, &folder).unwrap();

            // uuid-welcome should have uuid-gs as backlinker (relative sibling)
            assert_eq!(read_backlinks(&folder, "uuid-welcome"), vec!["uuid-gs"]);
            // uuid-notes-welcome should have uuid-ideas as backlinker (relative sibling)
            assert_eq!(
                read_backlinks(&folder, "uuid-notes-welcome"),
                vec!["uuid-ideas"]
            );

            let resolver = build_resolver(&[(&f0id, &folder)]);
            let mut content_docs = HashMap::new();
            content_docs.insert("uuid-gs".to_string(), &gs_doc as &Doc);
            content_docs.insert("uuid-ideas".to_string(), &ideas_doc as &Doc);

            let result = move_document(
                "uuid-welcome",
                "/Archive/Welcome.md",
                &folder,
                &folder,
                &[&folder],
                &["Lens"],
                &resolver,
                &content_docs,
            )
            .expect("move should succeed");

            // Only Getting Started.md's link should be rewritten
            assert_eq!(
                result.links_rewritten, 1,
                "only the link that pointed to the moved file should be rewritten"
            );
            assert_eq!(
                read_contents(&gs_doc),
                "See [[Archive/Welcome]] for details",
                "link to moved file should be updated"
            );
            // Ideas.md's link points to /Notes/Welcome.md (unchanged)
            assert_eq!(
                read_contents(&ideas_doc),
                "Check [[Welcome]] for info",
                "link to unmoved file should be unchanged"
            );
        }

        // === Outgoing link rewriting tests ===
        // When a document is moved, its own wikilinks (outgoing) that use
        // path-qualified names may break because they resolve relative to
        // the source's directory. These tests verify that move_document()
        // rewrites outgoing links in the moved document itself.

        #[test]
        fn move_rewrites_outgoing_path_qualified_link() {
            // /Welcome.md has [[Notes/Ideas]] (resolves to /Notes/Ideas.md from /)
            // Move /Welcome.md -> /Archive/Welcome.md
            // From /Archive/, [[Notes/Ideas]] resolves to /Archive/Notes/Ideas.md (wrong)
            // Should become [[../Notes/Ideas]] (resolves to /Notes/Ideas.md from /Archive/)
            let folder = create_folder_doc(&[
                ("/Welcome.md", "uuid-welcome"),
                ("/Notes/Ideas.md", "uuid-ideas"),
                ("/Notes", "uuid-notes-folder"),
                ("/Archive", "uuid-archive-folder"),
            ]);
            set_folder_name(&folder, "Lens");
            let f0id = folder0_id();

            let welcome_doc = create_content_doc("Check out [[Notes/Ideas]] for inspiration");
            index_content_into_folder("uuid-welcome", &welcome_doc, &folder).unwrap();
            assert_eq!(read_backlinks(&folder, "uuid-ideas"), vec!["uuid-welcome"]);

            let resolver = build_resolver(&[(&f0id, &folder)]);
            let mut content_docs = HashMap::new();
            content_docs.insert("uuid-welcome".to_string(), &welcome_doc as &Doc);

            let result = move_document(
                "uuid-welcome",
                "/Archive/Welcome.md",
                &folder,
                &folder,
                &[&folder],
                &["Lens"],
                &resolver,
                &content_docs,
            )
            .expect("move should succeed");

            assert!(
                result.links_rewritten > 0,
                "outgoing path-qualified link should be rewritten"
            );
            assert_eq!(
                read_contents(&welcome_doc),
                "Check out [[../Notes/Ideas]] for inspiration",
                "path-qualified outgoing link should be updated for new location"
            );
        }

        #[test]
        fn move_does_not_rewrite_unresolvable_outgoing_link() {
            // /Welcome.md has [[Nonexistent Page]] (doesn't resolve to any entry)
            // Move /Welcome.md -> /Archive/Welcome.md
            // Since the link doesn't resolve, it should be left unchanged
            let folder = create_folder_doc(&[
                ("/Welcome.md", "uuid-welcome"),
                ("/Archive", "uuid-archive-folder"),
            ]);
            set_folder_name(&folder, "Lens");
            let f0id = folder0_id();

            let welcome_doc = create_content_doc("See [[Nonexistent Page]] first");
            index_content_into_folder("uuid-welcome", &welcome_doc, &folder).unwrap();

            let resolver = build_resolver(&[(&f0id, &folder)]);
            let mut content_docs = HashMap::new();
            content_docs.insert("uuid-welcome".to_string(), &welcome_doc as &Doc);

            let _result = move_document(
                "uuid-welcome",
                "/Archive/Welcome.md",
                &folder,
                &folder,
                &[&folder],
                &["Lens"],
                &resolver,
                &content_docs,
            )
            .expect("move should succeed");

            assert_eq!(
                read_contents(&welcome_doc),
                "See [[Nonexistent Page]] first",
                "unresolvable link should not be rewritten"
            );
        }

        #[test]
        fn move_rewrites_outgoing_relative_parent_link() {
            // /Notes/Ideas.md has [[../Welcome]] (resolves to /Welcome.md)
            // Move /Notes/Ideas.md -> /Ideas.md (to root)
            // From /, [[../Welcome]] would resolve to /../Welcome.md (broken)
            // Should become [[Welcome]] (resolves to /Welcome.md from /)
            let folder = create_folder_doc(&[
                ("/Notes/Ideas.md", "uuid-ideas"),
                ("/Notes", "uuid-notes-folder"),
                ("/Welcome.md", "uuid-welcome"),
            ]);
            set_folder_name(&folder, "Lens");
            let f0id = folder0_id();

            let ideas_doc = create_content_doc("Return to [[../Welcome]]");
            index_content_into_folder("uuid-ideas", &ideas_doc, &folder).unwrap();
            assert_eq!(read_backlinks(&folder, "uuid-welcome"), vec!["uuid-ideas"]);

            let resolver = build_resolver(&[(&f0id, &folder)]);
            let mut content_docs = HashMap::new();
            content_docs.insert("uuid-ideas".to_string(), &ideas_doc as &Doc);

            let result = move_document(
                "uuid-ideas",
                "/Ideas.md",
                &folder,
                &folder,
                &[&folder],
                &["Lens"],
                &resolver,
                &content_docs,
            )
            .expect("move should succeed");

            assert!(
                result.links_rewritten > 0,
                "outgoing relative parent link should be rewritten"
            );
            assert_eq!(
                read_contents(&ideas_doc),
                "Return to [[Welcome]]",
                "relative parent link should simplify when moved to same level as target"
            );
        }

        #[test]
        fn move_rewrites_outgoing_and_incoming_links() {
            // Setup: /Welcome.md links to [[Notes/Ideas]] (outgoing)
            //        /Getting Started.md links to [[Welcome]] (incoming backlink)
            // Move /Welcome.md -> /Archive/Welcome.md
            // Both should be rewritten:
            //   Welcome's [[Notes/Ideas]] -> [[../Notes/Ideas]] (outgoing)
            //   Getting Started's [[Welcome]] -> [[Archive/Welcome]] (incoming)
            let folder = create_folder_doc(&[
                ("/Welcome.md", "uuid-welcome"),
                ("/Notes/Ideas.md", "uuid-ideas"),
                ("/Notes", "uuid-notes-folder"),
                ("/Getting Started.md", "uuid-gs"),
                ("/Archive", "uuid-archive-folder"),
            ]);
            set_folder_name(&folder, "Lens");
            let f0id = folder0_id();

            let welcome_doc = create_content_doc("Check [[Notes/Ideas]]");
            let gs_doc = create_content_doc("See [[Welcome]] for details");
            index_content_into_folder("uuid-welcome", &welcome_doc, &folder).unwrap();
            index_content_into_folder("uuid-gs", &gs_doc, &folder).unwrap();
            assert_eq!(read_backlinks(&folder, "uuid-ideas"), vec!["uuid-welcome"]);
            assert_eq!(read_backlinks(&folder, "uuid-welcome"), vec!["uuid-gs"]);

            let resolver = build_resolver(&[(&f0id, &folder)]);
            let mut content_docs = HashMap::new();
            content_docs.insert("uuid-welcome".to_string(), &welcome_doc as &Doc);
            content_docs.insert("uuid-gs".to_string(), &gs_doc as &Doc);

            let result = move_document(
                "uuid-welcome",
                "/Archive/Welcome.md",
                &folder,
                &folder,
                &[&folder],
                &["Lens"],
                &resolver,
                &content_docs,
            )
            .expect("move should succeed");

            // 1 incoming (Getting Started) + 1 outgoing (Welcome's own link)
            assert_eq!(
                result.links_rewritten, 2,
                "should rewrite both incoming backlinks and outgoing links"
            );
            assert_eq!(
                read_contents(&welcome_doc),
                "Check [[../Notes/Ideas]]",
                "outgoing link should be updated for new location"
            );
            assert_eq!(
                read_contents(&gs_doc),
                "See [[Archive/Welcome]] for details",
                "incoming backlink should be updated"
            );
        }
    }
}
