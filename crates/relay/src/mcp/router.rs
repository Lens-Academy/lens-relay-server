use serde_json::{json, Value};
use tracing::debug;

use super::jsonrpc::{
    error_response, success_response, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse,
    INTERNAL_ERROR, METHOD_NOT_FOUND,
};
use super::session::SessionManager;

/// Dispatch a JSON-RPC request to the appropriate handler.
/// Returns the response and an optional new session ID (set only for initialize).
pub fn dispatch_request(
    sessions: &SessionManager,
    session_id: Option<&str>,
    request: &JsonRpcRequest,
) -> (JsonRpcResponse, Option<String>) {
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
            (
                handle_tools_call(sessions, session_id, request.id.clone(), request.params.as_ref()),
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
    success_response(id, json!({ "tools": [] }))
}

fn handle_tools_call(
    _sessions: &SessionManager,
    _session_id: Option<&str>,
    id: Value,
    _params: Option<&Value>,
) -> JsonRpcResponse {
    // Stub: no tools available yet (will be implemented in Phase 4)
    error_response(id, INTERNAL_ERROR, "No tools available")
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

    #[test]
    fn initialize_creates_session_and_returns_capabilities() {
        let sessions = SessionManager::new();
        let req = make_request(
            json!(1),
            "initialize",
            Some(json!({
                "protocolVersion": "2025-03-26",
                "clientInfo": {"name": "test-client", "version": "1.0"}
            })),
        );

        let (resp, new_session_id) = dispatch_request(&sessions, None, &req);

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
        assert!(sessions.get_session(&sid).is_some());
    }

    #[test]
    fn ping_returns_empty_result() {
        let sessions = SessionManager::new();
        let req = make_request(json!(2), "ping", None);

        let (resp, new_session_id) = dispatch_request(&sessions, None, &req);

        assert!(new_session_id.is_none());
        assert_eq!(resp.id, json!(2));
        assert!(resp.error.is_none());
        assert_eq!(resp.result.unwrap(), json!({}));
    }

    #[test]
    fn tools_list_returns_empty_array() {
        let sessions = SessionManager::new();
        let req = make_request(json!(3), "tools/list", None);

        // Create and initialize a session
        let sid = sessions.create_session("2025-03-26".into(), None);
        sessions.mark_initialized(&sid);

        let (resp, new_session_id) = dispatch_request(&sessions, Some(&sid), &req);

        assert!(new_session_id.is_none());
        assert_eq!(resp.id, json!(3));
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        assert!(result["tools"].is_array());
        assert_eq!(result["tools"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn tools_call_without_session_returns_error() {
        let sessions = SessionManager::new();
        let req = make_request(
            json!(4),
            "tools/call",
            Some(json!({"name": "search", "arguments": {"query": "test"}})),
        );

        let (resp, _) = dispatch_request(&sessions, None, &req);

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
    fn tools_call_with_initialized_session_returns_no_tools_error() {
        let sessions = SessionManager::new();
        let sid = sessions.create_session("2025-03-26".into(), None);
        sessions.mark_initialized(&sid);

        let req = make_request(
            json!(5),
            "tools/call",
            Some(json!({"name": "search", "arguments": {"query": "test"}})),
        );

        let (resp, _) = dispatch_request(&sessions, Some(&sid), &req);

        assert!(resp.result.is_none());
        let err = resp.error.expect("should have error");
        assert!(
            err.message.to_lowercase().contains("no tools")
                || err.message.to_lowercase().contains("not available"),
            "error should mention no tools: {}",
            err.message
        );
    }

    #[test]
    fn unknown_method_returns_method_not_found() {
        let sessions = SessionManager::new();
        let req = make_request(json!(6), "foo/bar", None);

        let (resp, new_session_id) = dispatch_request(&sessions, None, &req);

        assert!(new_session_id.is_none());
        assert!(resp.result.is_none());
        let err = resp.error.expect("should have error");
        assert_eq!(err.code, METHOD_NOT_FOUND);
        assert!(err.message.contains("foo/bar"));
    }

    #[test]
    fn notifications_initialized_marks_session() {
        let sessions = SessionManager::new();
        let sid = sessions.create_session("2025-03-26".into(), None);
        assert!(!sessions.get_session(&sid).unwrap().initialized);

        let notif = make_notification("notifications/initialized", None);
        handle_notification(&sessions, Some(&sid), &notif);

        assert!(sessions.get_session(&sid).unwrap().initialized);
    }

    #[test]
    fn notifications_cancelled_is_noop() {
        let sessions = SessionManager::new();
        let notif = make_notification("notifications/cancelled", Some(json!({"requestId": 1})));
        // Should not panic
        handle_notification(&sessions, None, &notif);
    }

    #[test]
    fn tools_call_with_uninitialized_session_returns_error() {
        let sessions = SessionManager::new();
        let sid = sessions.create_session("2025-03-26".into(), None);
        // Not calling mark_initialized -- session exists but is not initialized

        let req = make_request(
            json!(7),
            "tools/call",
            Some(json!({"name": "search", "arguments": {}})),
        );

        let (resp, _) = dispatch_request(&sessions, Some(&sid), &req);

        assert!(resp.result.is_none());
        let err = resp.error.expect("should have error");
        assert!(
            err.message.to_lowercase().contains("initialized")
                || err.message.to_lowercase().contains("session"),
            "error should mention initialization: {}",
            err.message
        );
    }
}
