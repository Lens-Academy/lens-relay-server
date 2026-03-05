use crate::server::{search_handle_content_update, Server};
use serde_json::Value;
use std::sync::Arc;
use y_sweet_core::link_indexer;
use yrs::{Array, Map, ReadTxn, Transact};

/// Execute the `move_document` tool: move a document to a new path within or across folders.
pub async fn execute(server: &Arc<Server>, arguments: &Value) -> Result<String, String> {
    let file_path = arguments
        .get("file_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: file_path".to_string())?;

    let new_path = arguments
        .get("new_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: new_path".to_string())?;

    let target_folder = arguments.get("target_folder").and_then(|v| v.as_str());

    // Validate new_path format
    if !new_path.starts_with('/') {
        return Err("new_path must start with '/' and end with '.md'".to_string());
    }
    if !new_path.ends_with(".md") {
        return Err("new_path must start with '/' and end with '.md'".to_string());
    }

    // Resolve file_path to a UUID via doc_resolver
    let doc_info = server
        .doc_resolver()
        .resolve_path(file_path)
        .ok_or_else(|| format!("Document not found: {}", file_path))?;

    // Reload from storage if GC evicted the doc
    server
        .ensure_doc_loaded(&doc_info.doc_id)
        .await
        .map_err(|e| format!("Error: Failed to load document {}: {}", file_path, e))?;

    let uuid = &doc_info.uuid;
    let docs = server.docs();

    // Synchronous block: all DashMap guards and awareness locks stay within scope
    let (result, content_doc_id) = {
        // 1. Find all folder doc IDs
        let folder_doc_ids = link_indexer::find_all_folder_docs(docs);
        if folder_doc_ids.is_empty() {
            return Err("No folder documents found".to_string());
        }

        // 2. Find source folder and read folder names
        let source_folder_doc_id = &doc_info.folder_doc_id;
        let mut folder_names: Vec<(String, String)> = Vec::new(); // (doc_id, folder_name)

        for folder_doc_id in &folder_doc_ids {
            let Some(doc_ref) = docs.get(folder_doc_id) else {
                continue;
            };
            let awareness = doc_ref.awareness();
            let guard = awareness.read().unwrap_or_else(|e| e.into_inner());
            let folder_name =
                y_sweet_core::doc_resolver::read_folder_name(&guard.doc, folder_doc_id);
            folder_names.push((folder_doc_id.clone(), folder_name));
        }

        // 3. Determine target folder doc ID
        let target_folder_doc_id = if let Some(target_name) = target_folder {
            let found = folder_names
                .iter()
                .find(|(_, name)| name == target_name)
                .map(|(id, _)| id.clone());
            found.ok_or_else(|| {
                let available: Vec<&str> =
                    folder_names.iter().map(|(_, name)| name.as_str()).collect();
                format!(
                    "Unknown target folder: {}. Available: {}",
                    target_name,
                    available.join(", ")
                )
            })?
        } else {
            source_folder_doc_id.clone()
        };

        // 4. Check if new_path already exists in target folder doc
        {
            let Some(doc_ref) = docs.get(&target_folder_doc_id) else {
                return Err("Target folder doc not loaded".to_string());
            };
            let awareness = doc_ref.awareness();
            let guard = awareness.read().unwrap_or_else(|e| e.into_inner());
            let txn = guard.doc.transact();
            if let Some(filemeta) = txn.get_map("filemeta_v0") {
                if filemeta.get(&txn, new_path).is_some() {
                    return Err(format!(
                        "Path '{}' already exists in target folder",
                        new_path
                    ));
                }
            }
        }

        // 5. Collect all content doc UUIDs (from filemeta + backlinks)
        let mut all_content_uuids: Vec<String> = Vec::new();
        for folder_doc_id in &folder_doc_ids {
            let Some(doc_ref) = docs.get(folder_doc_id) else {
                continue;
            };
            let awareness = doc_ref.awareness();
            let guard = awareness.read().unwrap_or_else(|e| e.into_inner());
            let txn = guard.doc.transact();

            if let Some(filemeta) = txn.get_map("filemeta_v0") {
                for (_path, value) in filemeta.iter(&txn) {
                    if let Some(id) = link_indexer::extract_id_from_filemeta_entry(&value, &txn) {
                        if !all_content_uuids.contains(&id) {
                            all_content_uuids.push(id);
                        }
                    }
                }
            }

            if let Some(backlinks) = txn.get_map("backlinks_v0") {
                if let Some(bl_array) = backlinks.get(&txn, uuid.as_str()) {
                    if let yrs::Out::YArray(arr) = bl_array {
                        for item in arr.iter(&txn) {
                            if let yrs::Out::Any(yrs::Any::String(s)) = item {
                                let s = s.to_string();
                                if !all_content_uuids.contains(&s) {
                                    all_content_uuids.push(s);
                                }
                            }
                        }
                    }
                }
            }
        }

        // 6. Find the relay_id prefix from any folder doc
        let relay_id = folder_doc_ids
            .first()
            .and_then(|id| link_indexer::parse_doc_id(id).map(|(r, _)| r.to_string()))
            .unwrap_or_default();

        // 7. Acquire awareness write locks on all folder docs and content docs
        let doc_resolver = server.doc_resolver().clone();

        let folder_refs: Vec<_> = folder_doc_ids
            .iter()
            .filter_map(|id| docs.get(id))
            .collect();
        let folder_awareness: Vec<_> = folder_refs.iter().map(|r| r.awareness()).collect();
        let folder_guards: Vec<_> = folder_awareness
            .iter()
            .map(|a| a.write().unwrap_or_else(|e| e.into_inner()))
            .collect();

        let folder_doc_refs: Vec<&yrs::Doc> = folder_guards.iter().map(|g| &g.doc).collect();
        let folder_name_strings: Vec<String> = folder_doc_ids
            .iter()
            .zip(folder_guards.iter())
            .map(|(id, g)| y_sweet_core::doc_resolver::read_folder_name(&g.doc, id))
            .collect();
        let folder_name_refs: Vec<&str> = folder_name_strings.iter().map(|s| s.as_str()).collect();

        let source_idx = folder_doc_ids
            .iter()
            .position(|id| id == source_folder_doc_id)
            .ok_or_else(|| "Source folder doc not in folder list".to_string())?;
        let target_idx = folder_doc_ids
            .iter()
            .position(|id| id == &target_folder_doc_id)
            .ok_or_else(|| "Target folder doc not in folder list".to_string())?;

        let content_doc_ids: Vec<String> = all_content_uuids
            .iter()
            .map(|u| {
                if relay_id.is_empty() {
                    u.clone()
                } else {
                    format!("{}-{}", relay_id, u)
                }
            })
            .collect();
        let content_refs: Vec<_> = content_doc_ids
            .iter()
            .filter_map(|id| docs.get(id))
            .collect();
        let content_awareness: Vec<_> = content_refs.iter().map(|r| r.awareness()).collect();
        let content_guards: Vec<_> = content_awareness
            .iter()
            .map(|a| a.write().unwrap_or_else(|e| e.into_inner()))
            .collect();

        let mut content_docs: std::collections::HashMap<String, &yrs::Doc> =
            std::collections::HashMap::new();
        for (i, guard) in content_guards.iter().enumerate() {
            let doc_id = content_refs[i].key();
            if let Some((_r, u)) = link_indexer::parse_doc_id(doc_id) {
                content_docs.insert(u.to_string(), &guard.doc);
            }
        }

        // 8. Call move_document
        let result = link_indexer::move_document(
            uuid,
            new_path,
            folder_doc_refs[source_idx],
            folder_doc_refs[target_idx],
            &folder_doc_refs,
            &folder_name_refs,
            &doc_resolver,
            &content_docs,
        )
        .map_err(|e| e.to_string())?;

        // Compute content_doc_id for search update
        let content_doc_id = if relay_id.is_empty() {
            uuid.to_string()
        } else {
            format!("{}-{}", relay_id, uuid)
        };

        (result, content_doc_id)
    }; // All DashMap refs, awareness guards, and write guards dropped here

    // 9. Update search index (synchronous, no .await needed)
    if let Some(ref search_index) = server.search_index() {
        search_handle_content_update(&content_doc_id, server.docs(), search_index);
    }

    Ok(format!(
        "Moved {}{} -> {}{} ({} links rewritten)",
        result.old_folder_name,
        result.old_path,
        result.new_folder_name,
        result.new_path,
        result.links_rewritten,
    ))
}
