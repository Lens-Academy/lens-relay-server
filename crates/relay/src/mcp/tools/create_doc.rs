use crate::server::{search_handle_content_update, Server};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use y_sweet_core::doc_resolver::{read_folder_name, DocInfo};
use y_sweet_core::link_indexer;
use yrs::{Any, Map, ReadTxn, Text, Transact, WriteTxn};

/// Execute the `create` tool: create a new document at the specified path.
pub async fn execute(server: &Arc<Server>, arguments: &Value) -> Result<String, String> {
    let file_path = arguments
        .get("file_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: file_path".to_string())?;

    let content = arguments
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("_");

    // Reject if AI included CriticMarkup in content
    super::critic_markup::reject_if_contains_markup(content, "content")?;

    // Validate: must end with .md
    if !file_path.ends_with(".md") {
        return Err("file_path must end with '.md'".to_string());
    }

    // Split at first '/' into folder name + in-folder path
    let slash_pos = file_path
        .find('/')
        .ok_or_else(|| "file_path must include a folder name (e.g. 'Lens/Doc.md')".to_string())?;

    let folder_name_input = &file_path[..slash_pos];
    let in_folder_path = format!("/{}", &file_path[slash_pos + 1..]);

    // Find folder docs and match folder name
    let docs = server.docs();
    let folder_doc_ids = link_indexer::find_all_folder_docs(docs);
    if folder_doc_ids.is_empty() {
        return Err("No folder documents found".to_string());
    }

    let mut folder_match: Option<String> = None;
    let mut available_folders: Vec<String> = Vec::new();

    for folder_doc_id in &folder_doc_ids {
        let Some(doc_ref) = docs.get(folder_doc_id) else {
            continue;
        };
        let awareness = doc_ref.awareness();
        let guard = awareness.read().unwrap_or_else(|e| e.into_inner());
        let name = read_folder_name(&guard.doc, folder_doc_id);
        if name == folder_name_input {
            folder_match = Some(folder_doc_id.clone());
        }
        available_folders.push(name);
    }

    let folder_doc_id = folder_match.ok_or_else(|| {
        format!(
            "Unknown folder '{}'. The path must start with a folder name. Available folders: {}",
            folder_name_input,
            available_folders.join(", ")
        )
    })?;

    // Check path doesn't already exist in filemeta_v0
    {
        let Some(doc_ref) = docs.get(&folder_doc_id) else {
            return Err("Folder doc not loaded".to_string());
        };
        let awareness = doc_ref.awareness();
        let guard = awareness.read().unwrap_or_else(|e| e.into_inner());
        let txn = guard.doc.transact();
        if let Some(filemeta) = txn.get_map("filemeta_v0") {
            if filemeta.get(&txn, &*in_folder_path).is_some() {
                return Err(format!(
                    "Path '{}' already exists in folder '{}'",
                    in_folder_path, folder_name_input
                ));
            }
        }
    }

    // Generate UUID v4
    let uuid = uuid::Uuid::new_v4().to_string();

    // Compute full_doc_id = "{relay_id}-{uuid}"
    let relay_id = link_indexer::parse_doc_id(&folder_doc_id)
        .map(|(r, _)| r.to_string())
        .unwrap_or_default();

    let full_doc_id = if relay_id.is_empty() {
        uuid.clone()
    } else {
        format!("{}-{}", relay_id, uuid)
    };

    // Create content doc on server
    server
        .get_or_create_doc(&full_doc_id)
        .await
        .map_err(|e| format!("Failed to create content doc: {}", e))?;

    // Write initial content to content doc's "contents" Y.Text, wrapped in CriticMarkup
    {
        let doc_ref = docs
            .get(&full_doc_id)
            .ok_or_else(|| "Content doc not loaded after creation".to_string())?;
        let awareness = doc_ref.awareness();
        let guard = awareness.write().unwrap_or_else(|e| e.into_inner());
        let mut txn = guard.doc.transact_mut();
        let text = txn.get_or_insert_text("contents");
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let wrapped = format!(
            "{{++{{\"author\":\"AI\",\"timestamp\":{}}}@@{}++}}",
            timestamp, content
        );
        text.insert(&mut txn, 0, &wrapped);
    }

    // Write to folder doc: filemeta_v0 and legacy docs map
    {
        let doc_ref = docs
            .get(&folder_doc_id)
            .ok_or_else(|| "Folder doc not loaded".to_string())?;
        let awareness = doc_ref.awareness();
        let guard = awareness.write().unwrap_or_else(|e| e.into_inner());
        let mut txn = guard.doc.transact_mut_with("mcp");

        // filemeta_v0
        let filemeta = txn.get_or_insert_map("filemeta_v0");
        let mut map = HashMap::new();
        map.insert("id".to_string(), Any::String(uuid.clone().into()));
        map.insert("type".to_string(), Any::String("markdown".into()));
        map.insert("version".to_string(), Any::Number(0.0));
        filemeta.insert(&mut txn, &*in_folder_path, Any::Map(map.into()));

        // legacy docs map
        let docs_map = txn.get_or_insert_map("docs");
        docs_map.insert(&mut txn, &*in_folder_path, Any::String(uuid.clone().into()));
    }

    // Update doc_resolver
    server.doc_resolver().upsert_doc(
        &uuid,
        file_path,
        DocInfo {
            uuid: uuid.clone(),
            relay_id: relay_id.clone(),
            folder_doc_id: folder_doc_id.clone(),
            folder_name: folder_name_input.to_string(),
            doc_id: full_doc_id.clone(),
        },
    );

    // Update search index
    if let Some(ref search_index) = server.search_index() {
        search_handle_content_update(&full_doc_id, server.docs(), search_index);
    }

    Ok(format!("Created {}", file_path))
}
