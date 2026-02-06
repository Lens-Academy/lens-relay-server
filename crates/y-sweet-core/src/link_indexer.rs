use crate::link_parser::extract_wikilinks;
use std::cell::RefCell;
use std::collections::HashSet;
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

        if let Some(Out::YMap(meta_map)) = filemeta.get(txn, &*path) {
            if let Some(Out::Any(Any::String(ref id))) = meta_map.get(txn, "id") {
                uuids.push(id.to_string());
                continue;
            }
        }

        // Try case-insensitive match by iterating all entries
        let mut found = false;
        for (entry_path, entry_value) in filemeta.iter(txn) {
            let entry_name = entry_path
                .strip_prefix('/')
                .and_then(|s| s.strip_suffix(".md"))
                .unwrap_or(entry_path);

            if entry_name.to_lowercase() == name.to_lowercase() {
                if let Out::YMap(meta_map) = entry_value {
                    if let Some(Out::Any(Any::String(ref id))) = meta_map.get(txn, "id") {
                        uuids.push(id.to_string());
                        found = true;
                        break;
                    }
                }
            }
        }
        // If not found, the link is unresolvable — silently skip
        let _ = found;
    }

    uuids
}

// ---------------------------------------------------------------------------
// Core indexing function (testable with bare Y.Docs)
// ---------------------------------------------------------------------------

/// Core indexing logic — operates on bare Y.Docs for testability.
///
/// This is the function that unit+1 tests exercise directly.
/// The server worker unwraps DocWithSyncKv -> Doc before calling this.
pub fn index_content_into_folder(
    source_uuid: &str,
    content_doc: &Doc,
    folder_doc: &Doc,
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

    // 3. Resolve link names to UUIDs using filemeta_v0
    let target_uuids = {
        let txn = folder_doc.transact();
        let filemeta = txn
            .get_map("filemeta_v0")
            .ok_or_else(|| anyhow::anyhow!("No filemeta_v0 in folder doc"))?;
        resolve_links_to_uuids(&link_names, &filemeta, &txn)
    };

    // 4. Diff-update backlinks_v0 (add new, remove stale)
    let _guard = IndexingGuard::new();
    let mut txn = folder_doc.transact_mut_with("link-indexer");
    let backlinks = txn.get_or_insert_map("backlinks_v0");

    let new_targets: HashSet<&str> = target_uuids.iter().map(|s| s.as_str()).collect();

    // Add source to each target's backlinks
    for target_uuid in &target_uuids {
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

    // Remove source from targets it no longer links to (stale cleanup)
    let all_keys: Vec<String> = backlinks.keys(&txn).map(|k| k.to_string()).collect();
    for key in all_keys {
        if new_targets.contains(key.as_str()) {
            continue; // Still linked, skip
        }
        let current: Vec<String> = read_backlinks_array(&backlinks, &txn, &key);
        if current.contains(&source_uuid.to_string()) {
            let updated: Vec<String> = current.into_iter().filter(|s| s != source_uuid).collect();
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

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::{Doc, GetString, Map, MapPrelim, Text, Transact, WriteTxn, In, Any};

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
    fn create_folder_doc(entries: &[(&str, &str)]) -> Doc {
        let doc = Doc::new();
        {
            let mut txn = doc.transact_mut();
            let filemeta = txn.get_or_insert_map("filemeta_v0");
            for (path, uuid) in entries {
                // filemeta_v0 values are nested Y.Maps: { id, type, version }
                // MapPrelim needs all values to be the same Into<In> type,
                // so we use In explicitly for mixed types.
                let mut prelim = MapPrelim::default();
                prelim.insert(
                    "id".into(),
                    In::Any(Any::String((*uuid).into())),
                );
                prelim.insert(
                    "type".into(),
                    In::Any(Any::String("markdown".into())),
                );
                prelim.insert(
                    "version".into(),
                    In::Any(Any::Number(0.0)),
                );
                filemeta.insert(&mut txn, *path, prelim);
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
}
