use crate::server::Server;
use serde_json::Value;
use std::sync::Arc;
use y_sweet_core::link_indexer;
use y_sweet_core::link_parser;
use yrs::{GetString, Map, ReadTxn, Transact};

/// Execute the `get_links` tool: return backlinks and forward links for a document.
pub fn execute(server: &Arc<Server>, arguments: &Value) -> Result<String, String> {
    let file_path = arguments
        .get("file_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: file_path".to_string())?;

    let doc_info = server
        .doc_resolver()
        .resolve_path(file_path)
        .ok_or_else(|| format!("Error: Document not found: {}", file_path))?;

    // --- Backlinks ---
    let backlink_paths = read_backlinks(server, &doc_info.folder_doc_id, &doc_info.uuid);

    // --- Forward links ---
    let forward_link_paths = read_forward_links(server, &doc_info.doc_id);

    // Format output
    let mut output = String::new();
    output.push_str("Backlinks (documents linking to this):\n");
    if backlink_paths.is_empty() {
        output.push_str("- (none)\n");
    } else {
        for path in &backlink_paths {
            output.push_str(&format!("- {}\n", path));
        }
    }

    output.push_str("\nForward links (documents this links to):\n");
    if forward_link_paths.is_empty() {
        output.push_str("- (none)\n");
    } else {
        for path in &forward_link_paths {
            output.push_str(&format!("- {}\n", path));
        }
    }

    Ok(output)
}

/// Read backlinks for a document UUID from the folder doc's backlinks_v0 map.
fn read_backlinks(server: &Arc<Server>, folder_doc_id: &str, uuid: &str) -> Vec<String> {
    // Read backlink UUIDs into owned Vec, then drop all guards
    let backlink_uuids: Vec<String> = {
        let Some(doc_ref) = server.docs().get(folder_doc_id) else {
            return Vec::new();
        };
        let awareness = doc_ref.awareness();
        let guard = awareness.read().unwrap();
        let txn = guard.doc.transact();
        let Some(backlinks_map) = txn.get_map("backlinks_v0") else {
            return Vec::new();
        };
        link_indexer::read_backlinks_array(&backlinks_map, &txn, uuid)
        // guard, awareness, doc_ref all dropped here
    };

    // Resolve UUIDs to paths
    let resolver = server.doc_resolver();
    let mut paths: Vec<String> = backlink_uuids
        .iter()
        .filter_map(|uuid| resolver.path_for_uuid(uuid))
        .collect();
    paths.sort();
    paths
}

/// Read forward links by extracting wikilinks from content and resolving them to paths.
fn read_forward_links(server: &Arc<Server>, doc_id: &str) -> Vec<String> {
    // Read content into owned String, then drop all guards
    let content: String = {
        let Some(doc_ref) = server.docs().get(doc_id) else {
            return Vec::new();
        };
        let awareness = doc_ref.awareness();
        let guard = awareness.read().unwrap();
        let txn = guard.doc.transact();
        match txn.get_text("contents") {
            Some(text) => text.get_string(&txn),
            None => return Vec::new(),
        }
        // guard, awareness, doc_ref all dropped here
    };

    // Extract wikilink names from content
    let link_names = link_parser::extract_wikilinks(&content);

    // Get all document paths for resolution
    let all_paths = server.doc_resolver().all_paths();

    // Resolve each link name to a document path
    let mut forward_links: Vec<String> = Vec::new();
    for link_name in &link_names {
        let normalized = link_name.to_lowercase();

        for path in &all_paths {
            // Extract basename without extension
            let basename = path.rsplit('/').next().unwrap_or(path);
            let basename_no_ext = basename.strip_suffix(".md").unwrap_or(basename);

            if basename_no_ext.to_lowercase() == normalized {
                forward_links.push(path.clone());
                break; // First match wins
            }
        }
    }

    // Deduplicate
    forward_links.sort();
    forward_links.dedup();
    forward_links
}
