use serde_json::{json, Value};
use std::sync::Arc;
use tracing::debug;

use super::jsonrpc::{
    error_response, success_response, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse,
    INTERNAL_ERROR, METHOD_NOT_FOUND,
};
use super::session::SessionManager;
use super::tools;
use crate::server::Server;

/// Dispatch a JSON-RPC request to the appropriate handler.
/// Returns the response and an optional new session ID (set only for initialize).
pub fn dispatch_request(
    server: &Arc<Server>,
    session_id: Option<&str>,
    request: &JsonRpcRequest,
) -> (JsonRpcResponse, Option<String>) {
    let sessions = &server.mcp_sessions;
    match request.method.as_str() {
        "initialize" => {
            let (resp, sid) =
                handle_initialize(sessions, request.id.clone(), request.params.as_ref());
            (resp, Some(sid))
        }
        "ping" => (handle_ping(request.id.clone()), None),
        "tools/list" => (handle_tools_list(request.id.clone()), None),
        "tools/call" => {
            if let Err(err_resp) = validate_session(sessions, session_id, &request.id) {
                return (err_resp, None);
            }
            // session_id is Some(&str) here since validate_session passed
            (
                handle_tools_call(server, session_id.unwrap(), request.id.clone(), request.params.as_ref()),
                None,
            )
        }
        _ => (
            error_response(
                request.id.clone(),
                METHOD_NOT_FOUND,
                format!("Method not found: {}", request.method),
            ),
            None,
        ),
    }
}

/// Handle a JSON-RPC notification (no response expected).
pub fn handle_notification(
    sessions: &SessionManager,
    session_id: Option<&str>,
    notification: &JsonRpcNotification,
) {
    match notification.method.as_str() {
        "notifications/initialized" => {
            if let Some(sid) = session_id {
                if sessions.mark_initialized(sid) {
                    debug!(session_id = sid, "Session marked as initialized");
                } else {
                    debug!(session_id = sid, "Session not found for initialized notification");
                }
            }
        }
        "notifications/cancelled" => {
            debug!(method = "notifications/cancelled", "Cancellation notification received (no-op)");
        }
        other => {
            debug!(method = other, "Unknown notification received");
        }
    }
}

fn handle_initialize(
    sessions: &SessionManager,
    id: Value,
    params: Option<&Value>,
) -> (JsonRpcResponse, String) {
    let protocol_version = params
        .and_then(|p| p.get("protocolVersion"))
        .and_then(|v| v.as_str())
        .unwrap_or("2025-03-26")
        .to_string();

    let client_info = params.and_then(|p| p.get("clientInfo")).cloned();

    // Version negotiation: we always respond with our supported version
    let negotiated_version = "2025-03-26".to_string();

    debug!(
        client_version = %protocol_version,
        negotiated_version = %negotiated_version,
        "MCP initialize request"
    );

    let session_id = sessions.create_session(negotiated_version.clone(), client_info);

    let response = success_response(
        id,
        json!({
            "protocolVersion": negotiated_version,
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "lens-relay",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
    );

    (response, session_id)
}

fn handle_ping(id: Value) -> JsonRpcResponse {
    success_response(id, json!({}))
}

fn handle_tools_list(id: Value) -> JsonRpcResponse {
    let definitions = tools::tool_definitions();
    success_response(id, json!({ "tools": definitions }))
}

fn handle_tools_call(
    server: &Arc<Server>,
    session_id: &str,
    id: Value,
    params: Option<&Value>,
) -> JsonRpcResponse {
    let (name, arguments) = match params {
        Some(p) => {
            let name = p
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let arguments = p
                .get("arguments")
                .cloned()
                .unwrap_or(json!({}));
            (name.to_string(), arguments)
        }
        None => {
            return success_response(
                id,
                tools::dispatch_tool(server, session_id, "", &json!({})),
            );
        }
    };

    let result = tools::dispatch_tool(server, session_id, &name, &arguments);
    success_response(id, result)
}

fn validate_session(
    sessions: &SessionManager,
    session_id: Option<&str>,
    id: &Value,
) -> Result<(), JsonRpcResponse> {
    let sid = session_id.ok_or_else(|| {
        error_response(
            id.clone(),
            INTERNAL_ERROR,
            "No session ID provided. Send an initialize request first.",
        )
    })?;

    let session = sessions.get_session(sid).ok_or_else(|| {
        error_response(
            id.clone(),
            INTERNAL_ERROR,
            "Session not found. Send an initialize request first.",
        )
    })?;

    if !session.initialized {
        return Err(error_response(
            id.clone(),
            INTERNAL_ERROR,
            "Session not initialized. Send notifications/initialized first.",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_request(id: Value, method: &str, params: Option<Value>) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        }
    }

    fn make_notification(method: &str, params: Option<Value>) -> JsonRpcNotification {
        JsonRpcNotification {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        }
    }

    /// Create a minimal Server for testing (no store, no auth, no docs).
    fn test_server() -> Arc<Server> {
        Server::new_for_test()
    }

    #[test]
    fn initialize_creates_session_and_returns_capabilities() {
        let server = test_server();
        let req = make_request(
            json!(1),
            "initialize",
            Some(json!({
                "protocolVersion": "2025-03-26",
                "clientInfo": {"name": "test-client", "version": "1.0"}
            })),
        );

        let (resp, new_session_id) = dispatch_request(&server, None, &req);

        // Should return a new session ID
        let sid = new_session_id.expect("initialize should return session ID");
        assert_eq!(sid.len(), 32);

        // Response should have correct structure
        assert_eq!(resp.jsonrpc, "2.0");
        assert_eq!(resp.id, json!(1));
        assert!(resp.error.is_none());

        let result = resp.result.expect("should have result");
        assert_eq!(result["protocolVersion"], "2025-03-26");
        assert!(result["capabilities"]["tools"].is_object());
        assert_eq!(result["serverInfo"]["name"], "lens-relay");
        assert!(result["serverInfo"]["version"].is_string());

        // Session should exist in manager
        assert!(server.mcp_sessions.get_session(&sid).is_some());
    }

    #[test]
    fn ping_returns_empty_result() {
        let server = test_server();
        let req = make_request(json!(2), "ping", None);

        let (resp, new_session_id) = dispatch_request(&server, None, &req);

        assert!(new_session_id.is_none());
        assert_eq!(resp.id, json!(2));
        assert!(resp.error.is_none());
        assert_eq!(resp.result.unwrap(), json!({}));
    }

    #[test]
    fn tools_list_returns_five_tools() {
        let server = test_server();
        let req = make_request(json!(3), "tools/list", None);

        // Create and initialize a session
        let sid = server.mcp_sessions.create_session("2025-03-26".into(), None);
        server.mcp_sessions.mark_initialized(&sid);

        let (resp, new_session_id) = dispatch_request(&server, Some(&sid), &req);

        assert!(new_session_id.is_none());
        assert_eq!(resp.id, json!(3));
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        assert!(result["tools"].is_array());
        let tools_arr = result["tools"].as_array().unwrap();
        assert_eq!(tools_arr.len(), 5);

        // Verify tool names
        let names: Vec<&str> = tools_arr
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"read"));
        assert!(names.contains(&"glob"));
        assert!(names.contains(&"get_links"));
        assert!(names.contains(&"grep"));
        assert!(names.contains(&"edit"));
    }

    #[test]
    fn tools_call_without_session_returns_error() {
        let server = test_server();
        let req = make_request(
            json!(4),
            "tools/call",
            Some(json!({"name": "read", "arguments": {"file_path": "test"}})),
        );

        let (resp, _) = dispatch_request(&server, None, &req);

        assert!(resp.result.is_none());
        let err = resp.error.expect("should have error");
        // Error should mention session
        assert!(
            err.message.to_lowercase().contains("session"),
            "error message should mention session: {}",
            err.message
        );
    }

    #[test]
    fn tools_call_unknown_tool_returns_tool_error() {
        let server = test_server();
        let sid = server.mcp_sessions.create_session("2025-03-26".into(), None);
        server.mcp_sessions.mark_initialized(&sid);

        let req = make_request(
            json!(5),
            "tools/call",
            Some(json!({"name": "nonexistent_tool", "arguments": {}})),
        );

        let (resp, _) = dispatch_request(&server, Some(&sid), &req);

        // Should be a successful JSON-RPC response with isError in the result
        assert!(resp.error.is_none());
        let result = resp.result.expect("should have result");
        assert_eq!(result["isError"], true);
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Unknown tool"));
    }

    #[test]
    fn unknown_method_returns_method_not_found() {
        let server = test_server();
        let req = make_request(json!(6), "foo/bar", None);

        let (resp, new_session_id) = dispatch_request(&server, None, &req);

        assert!(new_session_id.is_none());
        assert!(resp.result.is_none());
        let err = resp.error.expect("should have error");
        assert_eq!(err.code, METHOD_NOT_FOUND);
        assert!(err.message.contains("foo/bar"));
    }

    #[test]
    fn notifications_initialized_marks_session() {
        let server = test_server();
        let sessions = &server.mcp_sessions;
        let sid = sessions.create_session("2025-03-26".into(), None);
        assert!(!sessions.get_session(&sid).unwrap().initialized);

        let notif = make_notification("notifications/initialized", None);
        handle_notification(sessions, Some(&sid), &notif);

        assert!(sessions.get_session(&sid).unwrap().initialized);
    }

    #[test]
    fn notifications_cancelled_is_noop() {
        let server = test_server();
        let sessions = &server.mcp_sessions;
        let notif = make_notification("notifications/cancelled", Some(json!({"requestId": 1})));
        // Should not panic
        handle_notification(sessions, None, &notif);
    }

    #[test]
    fn tools_call_with_uninitialized_session_returns_error() {
        let server = test_server();
        let sid = server.mcp_sessions.create_session("2025-03-26".into(), None);
        // Not calling mark_initialized -- session exists but is not initialized

        let req = make_request(
            json!(7),
            "tools/call",
            Some(json!({"name": "read", "arguments": {}})),
        );

        let (resp, _) = dispatch_request(&server, Some(&sid), &req);

        assert!(resp.result.is_none());
        let err = resp.error.expect("should have error");
        assert!(
            err.message.to_lowercase().contains("initialized")
                || err.message.to_lowercase().contains("session"),
            "error should mention initialization: {}",
            err.message
        );
    }

    #[test]
    fn read_records_doc_in_session() {
        use std::collections::HashMap;
        use yrs::{Any, Doc, Map, Text, Transact, WriteTxn};

        let server = test_server();

        // Set up a doc with content
        let relay_id = "cb696037-0f72-4e93-8717-4e433129d789";
        let folder_uuid = "aaaa0000-0000-0000-0000-000000000000";
        let content_uuid = "uuid-test-read";
        let folder_doc_id = format!("{}-{}", relay_id, folder_uuid);
        let content_doc_id = format!("{}-{}", relay_id, content_uuid);

        // Create folder doc with filemeta
        let folder_doc = Doc::new();
        {
            let mut txn = folder_doc.transact_mut();
            let filemeta = txn.get_or_insert_map("filemeta_v0");
            let mut map = HashMap::new();
            map.insert("id".to_string(), Any::String(content_uuid.into()));
            map.insert("type".to_string(), Any::String("markdown".into()));
            map.insert("version".to_string(), Any::Number(0.0));
            filemeta.insert(&mut txn, "/TestDoc.md", Any::Map(map.into()));
        }

        // Register in resolver
        server
            .doc_resolver()
            .update_folder_from_doc(&folder_doc_id, 0, &folder_doc);

        // Create content DocWithSyncKv
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dwskv = rt.block_on(async {
            y_sweet_core::doc_sync::DocWithSyncKv::new(&content_doc_id, None, || (), None)
                .await
                .unwrap()
        });
        {
            let awareness = dwskv.awareness();
            let mut guard = awareness.write().unwrap();
            let mut txn = guard.doc.transact_mut();
            let text = txn.get_or_insert_text("contents");
            text.insert(&mut txn, 0, "test content");
        }
        server.docs().insert(content_doc_id.clone(), dwskv);

        // Create and initialize a session
        let sid = server
            .mcp_sessions
            .create_session("2025-03-26".into(), None);
        server.mcp_sessions.mark_initialized(&sid);

        // Verify read_docs is empty before read
        {
            let session = server.mcp_sessions.get_session(&sid).unwrap();
            assert!(session.read_docs.is_empty(), "read_docs should start empty");
        }

        // Call read tool via dispatch
        let req = make_request(
            json!(10),
            "tools/call",
            Some(json!({"name": "read", "arguments": {"file_path": "Lens/TestDoc.md"}})),
        );
        let (resp, _) = dispatch_request(&server, Some(&sid), &req);
        assert!(resp.error.is_none(), "read should succeed");

        // Verify read_docs now contains the doc_id
        {
            let session = server.mcp_sessions.get_session(&sid).unwrap();
            assert!(
                session.read_docs.contains(&content_doc_id),
                "read_docs should contain {} after read, got: {:?}",
                content_doc_id,
                session.read_docs
            );
        }
    }
}
