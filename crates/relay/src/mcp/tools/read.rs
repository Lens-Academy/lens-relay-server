use crate::server::Server;
use serde_json::Value;
use std::sync::Arc;
use yrs::{GetString, ReadTxn, Transact};

/// Execute the `read` tool: read document content in cat -n format.
pub async fn execute(
    server: &Arc<Server>,
    session_id: &str,
    arguments: &Value,
) -> Result<String, String> {
    let file_path = arguments
        .get("file_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: file_path".to_string())?;

    let offset = arguments
        .get("offset")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(0);

    let limit = arguments
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(2000);

    let doc_info = server
        .doc_resolver()
        .resolve_path(file_path)
        .ok_or_else(|| format!("Error: Document not found: {}", file_path))?;

    // Reload from storage if GC evicted the doc
    server
        .ensure_doc_loaded(&doc_info.doc_id)
        .await
        .map_err(|e| format!("Error: Failed to load document {}: {}", file_path, e))?;

    // Read Y.Doc content into an owned String, then drop all guards
    let content = {
        let doc_ref = server
            .docs()
            .get(&doc_info.doc_id)
            .ok_or_else(|| format!("Error: Document data not loaded: {}", file_path))?;
        let awareness = doc_ref.awareness();
        let guard = awareness.read().unwrap_or_else(|e| e.into_inner());
        let txn = guard.doc.transact();
        match txn.get_text("contents") {
            Some(text) => text.get_string(&txn),
            None => String::new(),
        }
        // guard, awareness, doc_ref all dropped here
    };

    // Record this doc as read in the session (for read-before-edit enforcement)
    if let Some(mut session) = server.mcp_sessions.get_session_mut(session_id) {
        session.read_docs.insert(doc_info.doc_id.clone());
    }

    Ok(format_cat_n(&content, offset, limit))
}

/// Format content as cat -n output with 6-char right-aligned line numbers.
fn format_cat_n(content: &str, offset: usize, limit: usize) -> String {
    // offset is 1-indexed (line number to start from), 0 means start from beginning
    let skip = if offset > 0 {
        offset.saturating_sub(1)
    } else {
        0
    };

    content
        .lines()
        .enumerate()
        .skip(skip)
        .take(limit)
        .map(|(i, line)| {
            let line_num = i + 1; // 1-indexed
            let truncated = if line.len() > 2000 {
                &line[..2000]
            } else {
                line
            };
            format!("{:>6}\t{}", line_num, truncated)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_cat_n_basic() {
        let content = "line one\nline two\nline three";
        let result = format_cat_n(content, 0, 2000);
        assert_eq!(
            result,
            "     1\tline one\n     2\tline two\n     3\tline three"
        );
    }

    #[test]
    fn format_cat_n_with_offset() {
        let content = "a\nb\nc\nd";
        let result = format_cat_n(content, 3, 2000);
        // offset=3 means start from line 3
        assert_eq!(result, "     3\tc\n     4\td");
    }

    #[test]
    fn format_cat_n_with_limit() {
        let content = "a\nb\nc\nd";
        let result = format_cat_n(content, 0, 2);
        assert_eq!(result, "     1\ta\n     2\tb");
    }

    #[test]
    fn format_cat_n_with_offset_and_limit() {
        let content = "a\nb\nc\nd\ne";
        let result = format_cat_n(content, 2, 2);
        // offset=2 means start from line 2, limit=2
        assert_eq!(result, "     2\tb\n     3\tc");
    }

    #[test]
    fn format_cat_n_empty_content() {
        let result = format_cat_n("", 0, 2000);
        // Empty string has no lines
        assert_eq!(result, "");
    }

    #[test]
    fn format_cat_n_truncates_long_lines() {
        let long_line = "x".repeat(3000);
        let result = format_cat_n(&long_line, 0, 2000);
        // Should truncate to 2000 chars
        let expected = format!("     1\t{}", "x".repeat(2000));
        assert_eq!(result, expected);
    }

    /// After GC evicts a doc from memory, read should reload it from storage.
    ///
    /// This reproduces the bug where MCP can't read newly created files:
    /// 1. Doc loaded via load_doc() (same path as WebSocket connections)
    /// 2. GC worker spawned with doc_gc=true
    /// 3. No WebSocket connections hold awareness Arc → strong_count == 1
    /// 4. After 2 GC checkpoints, doc evicted from server.docs()
    /// 5. DocumentResolver still has the path mapping
    /// 6. read::execute fails with "Document data not loaded" instead of reloading
    #[tokio::test(start_paused = true)]
    async fn read_reloads_doc_from_store_after_gc_eviction() {
        use async_trait::async_trait;
        use dashmap::DashMap;
        use serde_json::json;
        use std::collections::HashMap;
        use std::sync::Arc;
        use std::time::Duration;
        use tokio_util::sync::CancellationToken;
        use y_sweet_core::store::Result as StoreResult;
        use y_sweet_core::store::Store;
        use yrs::{Any, Doc, Map, Text, Transact, WriteTxn};

        // In-memory store backed by shared DashMap
        struct MemoryStore {
            data: Arc<DashMap<String, Vec<u8>>>,
        }

        #[async_trait]
        impl Store for MemoryStore {
            async fn init(&self) -> StoreResult<()> {
                Ok(())
            }
            async fn get(&self, key: &str) -> StoreResult<Option<Vec<u8>>> {
                Ok(self.data.get(key).map(|v| v.clone()))
            }
            async fn set(&self, key: &str, value: Vec<u8>) -> StoreResult<()> {
                self.data.insert(key.to_owned(), value);
                Ok(())
            }
            async fn remove(&self, key: &str) -> StoreResult<()> {
                self.data.remove(key);
                Ok(())
            }
            async fn exists(&self, key: &str) -> StoreResult<bool> {
                Ok(self.data.contains_key(key))
            }
        }

        let store_data: Arc<DashMap<String, Vec<u8>>> = Arc::new(DashMap::new());
        let checkpoint_freq = Duration::from_secs(10);

        // Server with doc_gc ENABLED — GC workers will be spawned by load_doc()
        let server = Arc::new(
            Server::new_without_workers(
                Some(Box::new(MemoryStore {
                    data: store_data.clone(),
                })),
                checkpoint_freq,
                None,
                None,
                Vec::new(),
                CancellationToken::new(),
                true, // doc_gc enabled — this is the production default
                None,
            )
            .await
            .unwrap(),
        );

        let relay_id = "cb696037-0f72-4e93-8717-4e433129d789";
        let folder_uuid = "aaaa0000-0000-0000-0000-000000000001";
        let content_uuid = "cccc0000-0000-4000-8000-000000000001";
        let folder_doc_id = format!("{}-{}", relay_id, folder_uuid);
        let content_doc_id = format!("{}-{}", relay_id, content_uuid);

        // Build folder doc with filemeta and register in resolver
        let folder_doc = Doc::new();
        {
            let mut txn = folder_doc.transact_mut();
            let filemeta = txn.get_or_insert_map("filemeta_v0");
            let mut map = HashMap::new();
            map.insert("id".to_string(), Any::String(content_uuid.into()));
            map.insert("type".to_string(), Any::String("markdown".into()));
            map.insert("version".to_string(), Any::Number(0.0));
            filemeta.insert(&mut txn, "/TestDoc.md", Any::Map(map.into()));
            let config = txn.get_or_insert_map("folder_config");
            config.insert(&mut txn, "name", Any::String("TestFolder".into()));
        }
        server
            .doc_resolver()
            .update_folder_from_doc(&folder_doc_id, &folder_doc);

        // Load doc through the real code path — same as a WebSocket connection.
        // This creates DocWithSyncKv, persists to store, and spawns GC worker.
        server
            .load_doc(&content_doc_id, None)
            .await
            .expect("load_doc should succeed");

        // Write content and persist (simulates a client writing to the doc)
        {
            let doc_ref = server.docs().get(&content_doc_id).unwrap();
            let awareness = doc_ref.awareness();
            let mut guard = awareness.write().unwrap();
            let mut txn = guard.doc.transact_mut();
            let text = txn.get_or_insert_text("contents");
            text.insert(&mut txn, 0, "Hello from persisted storage");
        }
        // Clone Arc<SyncKv> then drop DashMap guard before awaiting
        let sync_kv = server.docs().get(&content_doc_id).unwrap().sync_kv();
        sync_kv.persist().await.expect("persist should succeed");

        // Advance paused time past 2 GC checkpoints so the GC worker evicts the doc.
        // With no WebSocket connections, awareness strong_count == 1, so GC proceeds.
        for _ in 0..10 {
            tokio::time::advance(checkpoint_freq).await;
            tokio::task::yield_now().await;
            if server.docs().get(&content_doc_id).is_none() {
                break;
            }
        }

        // Verify GC actually happened
        assert!(
            server.docs().get(&content_doc_id).is_none(),
            "doc should have been evicted by GC worker"
        );

        // Sanity: resolver still knows the path (GC doesn't touch the resolver)
        assert!(
            server
                .doc_resolver()
                .resolve_path("TestFolder/TestDoc.md")
                .is_some(),
            "resolver should still have the path after GC"
        );

        // Create MCP session
        let sid = server
            .mcp_sessions
            .create_session("2025-03-26".into(), None);
        server.mcp_sessions.mark_initialized(&sid);

        // THIS IS THE BUG: read should reload from store, not fail
        let result = execute(
            &server,
            &sid,
            &json!({
                "file_path": "TestFolder/TestDoc.md",
                "session_id": sid,
            }),
        )
        .await;

        assert!(
            result.is_ok(),
            "read should reload doc from store after GC eviction, but got: {:?}",
            result.err()
        );

        let content = result.unwrap();
        assert!(
            content.contains("Hello from persisted storage"),
            "read should return the persisted content, got: {}",
            content
        );
    }
}
