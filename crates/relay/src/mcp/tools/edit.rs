use crate::server::Server;
use serde_json::Value;
use std::sync::Arc;
use yrs::{GetString, ReadTxn, Text, Transact, WriteTxn};

/// Execute the `edit` tool: replace old_string with CriticMarkup-wrapped suggestion.
///
/// The edit is wrapped in CriticMarkup format `{--old--}{++new++}` so human
/// collaborators can review and accept/reject the AI's proposed change.
pub fn execute(
    server: &Arc<Server>,
    arguments: &Value,
) -> Result<String, String> {
    // 1. Parse parameters
    let file_path = arguments
        .get("file_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: file_path".to_string())?;

    let old_string = arguments
        .get("old_string")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: old_string".to_string())?;

    let new_string = arguments
        .get("new_string")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: new_string".to_string())?;

    let session_id = arguments
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: session_id. Pass the session value from the read tool's response.".to_string())?;

    // 2. Resolve document path to doc_id
    let doc_info = server
        .doc_resolver()
        .resolve_path(file_path)
        .ok_or_else(|| format!("Error: Document not found: {}", file_path))?;

    // 3. Check read-before-edit: session must have read this document first
    {
        let session = server
            .mcp_sessions
            .get_session(session_id)
            .ok_or_else(|| "Error: Session not found".to_string())?;
        if !session.read_docs.contains(&doc_info.doc_id) {
            return Err(format!(
                "You must read this document before editing it. Call the read tool with file_path: \"{}\" first.",
                file_path
            ));
        }
        // Drop session guard before accessing Y.Doc
    }

    // 4. Read content and find old_string
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
            None => return Err("Document has no content".to_string()),
        }
    };

    // 5. Find old_string and check uniqueness
    let matches: Vec<usize> = content
        .match_indices(old_string)
        .map(|(idx, _)| idx)
        .collect();

    if matches.is_empty() {
        return Err(format!(
            "Error: old_string not found in {}. Make sure it matches exactly.",
            file_path
        ));
    }

    if matches.len() > 1 {
        return Err(format!(
            "Error: old_string is not unique in {} ({} occurrences found). Include more surrounding context to make it unique.",
            file_path,
            matches.len()
        ));
    }

    // 6. Build CriticMarkup replacement with metadata
    let byte_offset = matches[0] as u32;
    let old_len = old_string.len() as u32;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let meta_prefix = format!(
        r#"{{"author":"AI","timestamp":{}}}@@"#,
        timestamp
    );
    let replacement =
        super::critic_diff::smart_critic_markup(old_string, new_string, Some(&meta_prefix));

    // 7. Apply edit in write transaction with TOCTOU re-verify
    {
        let doc_ref = server
            .docs()
            .get(&doc_info.doc_id)
            .ok_or_else(|| format!("Error: Document data not loaded: {}", file_path))?;
        let awareness = doc_ref.awareness();
        let mut guard = awareness.write().unwrap_or_else(|e| e.into_inner());
        let mut txn = guard.doc.transact_mut();
        let text = txn.get_or_insert_text("contents");

        // Re-verify: check old_string still at expected offset
        let current_content = text.get_string(&txn);
        let actual_slice =
            current_content.get(byte_offset as usize..(byte_offset + old_len) as usize);
        if actual_slice != Some(old_string) {
            return Err(
                "Document changed since last read. Please re-read and try again.".to_string(),
            );
        }

        text.remove_range(&mut txn, byte_offset, old_len);
        text.insert(&mut txn, byte_offset, &replacement);
    }

    // 8. Return success
    Ok(format!(
        "Edited {}: replaced {} characters with CriticMarkup suggestion for human review.",
        file_path,
        old_string.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use y_sweet_core::doc_resolver::DocumentResolver;
    use yrs::{Any, Doc, Map, Text, Transact, WriteTxn};

    // === Test Helpers ===

    const RELAY_ID: &str = "cb696037-0f72-4e93-8717-4e433129d789";
    const FOLDER0_UUID: &str = "aaaa0000-0000-0000-0000-000000000000";

    fn folder0_id() -> String {
        format!("{}-{}", RELAY_ID, FOLDER0_UUID)
    }

    /// Create a folder Y.Doc with filemeta_v0 populated.
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

    /// Create a test server with docs and a session with the doc marked as read.
    fn build_test_server(entries: &[(&str, &str, &str)]) -> Arc<Server> {
        let server = Server::new_for_test();

        let filemeta_entries: Vec<(&str, &str)> =
            entries.iter().map(|(path, uuid, _)| (*path, *uuid)).collect();
        let folder_doc = create_folder_doc(&filemeta_entries);

        let resolver = server.doc_resolver();
        resolver.update_folder_from_doc(&folder0_id(), 0, &folder_doc);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        for (_, uuid, content) in entries {
            let doc_id = format!("{}-{}", RELAY_ID, uuid);
            let content_owned = content.to_string();
            let dwskv = rt.block_on(async {
                y_sweet_core::doc_sync::DocWithSyncKv::new(&doc_id, None, || (), None)
                    .await
                    .expect("Failed to create test DocWithSyncKv")
            });

            {
                let awareness = dwskv.awareness();
                let mut guard = awareness.write().unwrap();
                let mut txn = guard.doc.transact_mut();
                let text = txn.get_or_insert_text("contents");
                text.insert(&mut txn, 0, &content_owned);
            }

            server.docs().insert(doc_id, dwskv);
        }

        server
    }

    /// Create a session with a doc marked as already read.
    fn setup_session_with_read(server: &Arc<Server>, doc_id: &str) -> String {
        let sid = server.mcp_sessions.create_session("2025-03-26".into(), None);
        server.mcp_sessions.mark_initialized(&sid);
        if let Some(mut session) = server.mcp_sessions.get_session_mut(&sid) {
            session.read_docs.insert(doc_id.to_string());
        }
        sid
    }

    /// Create a session WITHOUT any docs marked as read.
    fn setup_session_no_reads(server: &Arc<Server>) -> String {
        let sid = server.mcp_sessions.create_session("2025-03-26".into(), None);
        server.mcp_sessions.mark_initialized(&sid);
        sid
    }

    /// Read the Y.Doc content back for verification.
    fn read_doc_content(server: &Arc<Server>, doc_id: &str) -> String {
        let doc_ref = server.docs().get(doc_id).expect("doc should exist");
        let awareness = doc_ref.awareness();
        let guard = awareness.read().unwrap();
        let txn = guard.doc.transact();
        txn.get_text("contents")
            .map(|text| text.get_string(&txn))
            .unwrap_or_default()
    }

    // === Edit Tests ===

    #[test]
    fn edit_basic_replacement() {
        let server = build_test_server(&[
            ("/Hello.md", "uuid-hello", "say hello to all"),
        ]);
        let doc_id = format!("{}-{}", RELAY_ID, "uuid-hello");
        let sid = setup_session_with_read(&server, &doc_id);

        let result = execute(
            &server,
            &json!({"file_path": "Lens/Hello.md", "old_string": "hello", "new_string": "world", "session_id": sid}),
        );

        assert!(result.is_ok(), "edit should succeed, got: {:?}", result);

        // Verify the Y.Doc content was actually modified with CriticMarkup + metadata
        let content = read_doc_content(&server, &doc_id);
        // Metadata is dynamic (timestamp), so check structure not exact string
        assert!(
            content.contains("{--") && content.contains("--}"),
            "Should contain deletion markup: {}", content
        );
        assert!(
            content.contains("{++") && content.contains("++}"),
            "Should contain insertion markup: {}", content
        );
        assert!(
            content.contains(r#""author":"AI""#),
            "Should contain author metadata: {}", content
        );
        assert!(
            content.contains("@@hello--}"),
            "Deletion should contain old text after @@: {}", content
        );
        assert!(
            content.contains("@@world++}"),
            "Insertion should contain new text after @@: {}", content
        );
        assert!(
            content.starts_with("say ") && content.ends_with(" to all"),
            "Surrounding text should be preserved: {}", content
        );
    }

    #[test]
    fn edit_read_before_edit_enforced() {
        let server = build_test_server(&[
            ("/Doc.md", "uuid-doc", "some content"),
        ]);
        // Session WITHOUT the doc in read_docs
        let sid = setup_session_no_reads(&server);

        let result = execute(
            &server,
            &json!({"file_path": "Lens/Doc.md", "old_string": "some", "new_string": "any", "session_id": sid}),
        );

        assert!(result.is_err(), "should reject edit on unread doc");
        let err = result.unwrap_err();
        assert!(
            err.to_lowercase().contains("must read") || err.to_lowercase().contains("read"),
            "Error should mention reading first: {}",
            err
        );
    }

    #[test]
    fn edit_old_string_not_found() {
        let server = build_test_server(&[
            ("/Doc.md", "uuid-doc", "actual content here"),
        ]);
        let doc_id = format!("{}-{}", RELAY_ID, "uuid-doc");
        let sid = setup_session_with_read(&server, &doc_id);

        let result = execute(
            &server,
            &json!({"file_path": "Lens/Doc.md", "old_string": "nonexistent", "new_string": "replacement", "session_id": sid}),
        );

        assert!(result.is_err(), "should reject when old_string not found");
        let err = result.unwrap_err();
        assert!(
            err.to_lowercase().contains("not found"),
            "Error should mention 'not found': {}",
            err
        );
    }

    #[test]
    fn edit_old_string_not_unique() {
        let server = build_test_server(&[
            ("/Cats.md", "uuid-cats", "the cat sat on the cat"),
        ]);
        let doc_id = format!("{}-{}", RELAY_ID, "uuid-cats");
        let sid = setup_session_with_read(&server, &doc_id);

        let result = execute(
            &server,
            &json!({"file_path": "Lens/Cats.md", "old_string": "the cat", "new_string": "a dog", "session_id": sid}),
        );

        assert!(result.is_err(), "should reject when old_string is not unique");
        let err = result.unwrap_err();
        assert!(
            err.to_lowercase().contains("not unique") || err.contains("2"),
            "Error should mention not unique or count 2: {}",
            err
        );
    }

    #[test]
    fn edit_document_not_found() {
        let server = build_test_server(&[]);
        let sid = setup_session_no_reads(&server);

        let result = execute(
            &server,
            &json!({"file_path": "Nonexistent/Doc.md", "old_string": "hello", "new_string": "world", "session_id": sid}),
        );

        assert!(result.is_err(), "should reject when document not found");
        let err = result.unwrap_err();
        assert!(
            err.contains("not found") || err.contains("Not found"),
            "Error should mention document not found: {}",
            err
        );
    }

    #[test]
    fn edit_missing_parameters() {
        let server = build_test_server(&[
            ("/Doc.md", "uuid-doc", "content"),
        ]);
        let doc_id = format!("{}-{}", RELAY_ID, "uuid-doc");
        let sid = setup_session_with_read(&server, &doc_id);

        // Missing old_string
        let result = execute(
            &server,
            &json!({"file_path": "Lens/Doc.md", "new_string": "world", "session_id": sid}),
        );
        assert!(result.is_err(), "missing old_string should error");
        assert!(
            result.unwrap_err().contains("old_string"),
            "Error should mention old_string"
        );

        // Missing new_string
        let result = execute(
            &server,
            &json!({"file_path": "Lens/Doc.md", "old_string": "content", "session_id": sid}),
        );
        assert!(result.is_err(), "missing new_string should error");
        assert!(
            result.unwrap_err().contains("new_string"),
            "Error should mention new_string"
        );

        // Missing file_path
        let result = execute(
            &server,
            &json!({"old_string": "content", "new_string": "replacement", "session_id": sid}),
        );
        assert!(result.is_err(), "missing file_path should error");
        assert!(
            result.unwrap_err().contains("file_path"),
            "Error should mention file_path"
        );
    }

    #[test]
    fn edit_preserves_surrounding_content() {
        let server = build_test_server(&[
            ("/Lines.md", "uuid-lines", "line 1\nline 2\nline 3"),
        ]);
        let doc_id = format!("{}-{}", RELAY_ID, "uuid-lines");
        let sid = setup_session_with_read(&server, &doc_id);

        let result = execute(
            &server,
            &json!({"file_path": "Lens/Lines.md", "old_string": "line 2", "new_string": "modified line 2", "session_id": sid}),
        );

        assert!(result.is_ok(), "edit should succeed, got: {:?}", result);

        let content = read_doc_content(&server, &doc_id);
        assert!(
            content.starts_with("line 1\n{++"),
            "Should start with line 1 then insertion markup: {}", content
        );
        assert!(
            content.contains("@@modified "),
            "Insertion should contain 'modified ' after @@: {}", content
        );
        assert!(
            content.ends_with("line 2\nline 3"),
            "Should preserve surrounding content: {}", content
        );
    }

    #[test]
    fn edit_multiline_old_string() {
        let server = build_test_server(&[
            ("/Multi.md", "uuid-multi", "line 1\nline 2\nline 3\nline 4"),
        ]);
        let doc_id = format!("{}-{}", RELAY_ID, "uuid-multi");
        let sid = setup_session_with_read(&server, &doc_id);

        let result = execute(
            &server,
            &json!({"file_path": "Lens/Multi.md", "old_string": "line 2\nline 3", "new_string": "replaced lines", "session_id": sid}),
        );

        assert!(result.is_ok(), "multiline edit should succeed, got: {:?}", result);

        let content = read_doc_content(&server, &doc_id);
        assert!(
            content.starts_with("line 1\n{--"),
            "Should start with line 1 then deletion markup: {}", content
        );
        assert!(
            content.contains("@@line 2\nline 3--}"),
            "Deletion should wrap multiline old text: {}", content
        );
        assert!(
            content.contains("@@replaced lines++}"),
            "Insertion should contain new text: {}", content
        );
        assert!(
            content.ends_with("\nline 4"),
            "Should preserve trailing content: {}", content
        );
    }

    #[test]
    fn edit_empty_new_string() {
        let server = build_test_server(&[
            ("/Del.md", "uuid-del", "keep delete me keep"),
        ]);
        let doc_id = format!("{}-{}", RELAY_ID, "uuid-del");
        let sid = setup_session_with_read(&server, &doc_id);

        let result = execute(
            &server,
            &json!({"file_path": "Lens/Del.md", "old_string": "delete me", "new_string": "", "session_id": sid}),
        );

        assert!(result.is_ok(), "deletion edit should succeed, got: {:?}", result);

        let content = read_doc_content(&server, &doc_id);
        assert!(
            content.starts_with("keep {--") && content.ends_with("--} keep"),
            "Should wrap deletion with surrounding text preserved: {}", content
        );
        assert!(
            content.contains("@@delete me--}"),
            "Deletion should contain old text after @@: {}", content
        );
        assert!(
            !content.contains("{++"),
            "Pure deletion should not have insertion markup: {}", content
        );
    }

    #[test]
    fn edit_success_message() {
        let server = build_test_server(&[
            ("/Msg.md", "uuid-msg", "hello world"),
        ]);
        let doc_id = format!("{}-{}", RELAY_ID, "uuid-msg");
        let sid = setup_session_with_read(&server, &doc_id);

        let result = execute(
            &server,
            &json!({"file_path": "Lens/Msg.md", "old_string": "hello", "new_string": "goodbye", "session_id": sid}),
        );

        assert!(result.is_ok(), "edit should succeed");
        let msg = result.unwrap();
        assert!(
            msg.contains("Lens/Msg.md"),
            "Success message should mention file_path: {}",
            msg
        );
        assert!(
            msg.to_lowercase().contains("criticmarkup") || msg.to_lowercase().contains("critic"),
            "Success message should mention CriticMarkup: {}",
            msg
        );
    }

    #[test]
    fn edit_missing_session_id_returns_error() {
        let server = build_test_server(&[
            ("/Doc.md", "uuid-doc", "content"),
        ]);

        let result = execute(
            &server,
            &json!({"file_path": "Lens/Doc.md", "old_string": "content", "new_string": "new"}),
        );

        assert!(result.is_err(), "missing session_id should error");
        assert!(
            result.unwrap_err().contains("session_id"),
            "Error should mention session_id"
        );
    }

    #[test]
    fn edit_invalid_session_id_returns_error() {
        let server = build_test_server(&[
            ("/Doc.md", "uuid-doc", "content"),
        ]);

        let result = execute(
            &server,
            &json!({
                "file_path": "Lens/Doc.md",
                "old_string": "content",
                "new_string": "new",
                "session_id": "nonexistent-session-id"
            }),
        );

        assert!(result.is_err(), "invalid session_id should error");
        let err = result.unwrap_err();
        assert!(
            err.to_lowercase().contains("session"),
            "Error should mention session: {}",
            err
        );
    }
}
