use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::debug;

use super::jsonrpc::{self, parse_message, JsonRpcMessage, JsonRpcResponse, PARSE_ERROR};
use super::router;
use crate::server::Server;

/// Handle POST /mcp — JSON-RPC messages (requests and notifications).
pub async fn handle_mcp_post(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    // Parse JSON body
    let value: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            let resp = JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: Value::Null,
                result: None,
                error: Some(jsonrpc::JsonRpcError {
                    code: PARSE_ERROR,
                    message: "Parse error".into(),
                    data: None,
                }),
            };
            return (StatusCode::OK, Json(resp)).into_response();
        }
    };

    // Parse the JSON-RPC message (request vs notification)
    let message = match parse_message(&value) {
        Ok(msg) => msg,
        Err(err) => {
            let resp = JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: value.get("id").cloned().unwrap_or(Value::Null),
                result: None,
                error: Some(err),
            };
            return (StatusCode::OK, Json(resp)).into_response();
        }
    };

    let session_id = extract_session_id(&headers);
    let sessions = &server.mcp_sessions;

    match message {
        JsonRpcMessage::Notification(notif) => {
            debug!(method = %notif.method, "MCP notification received");
            router::handle_notification(sessions, session_id.as_deref(), &notif);
            StatusCode::ACCEPTED.into_response()
        }
        JsonRpcMessage::Request(req) => {
            debug!(method = %req.method, id = %req.id, "MCP request received");

            if req.method == "initialize" {
                // Initialize does not require an existing session
                let (resp, new_session_id) =
                    router::dispatch_request(sessions, None, &req);

                let mut response = (StatusCode::OK, Json(resp)).into_response();

                if let Some(sid) = new_session_id {
                    if let Ok(val) = HeaderValue::from_str(&sid) {
                        response.headers_mut().insert("mcp-session-id", val);
                    }
                }

                response
            } else {
                // Non-initialize requests require a session ID
                let sid = match session_id {
                    Some(ref s) => s.clone(),
                    None => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(json!({
                                "jsonrpc": "2.0",
                                "id": req.id,
                                "error": {
                                    "code": -32600,
                                    "message": "Missing mcp-session-id header"
                                }
                            })),
                        )
                            .into_response();
                    }
                };

                // Verify session exists
                if sessions.get_session(&sid).is_none() {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({
                            "jsonrpc": "2.0",
                            "id": req.id,
                            "error": {
                                "code": -32600,
                                "message": "Session not found"
                            }
                        })),
                    )
                        .into_response();
                }

                let (resp, _) =
                    router::dispatch_request(sessions, Some(&sid), &req);

                (StatusCode::OK, Json(resp)).into_response()
            }
        }
    }
}

/// Handle GET /mcp — SSE transport (not yet implemented).
pub async fn handle_mcp_get() -> impl IntoResponse {
    (StatusCode::METHOD_NOT_ALLOWED, "SSE not supported yet")
}

/// Handle DELETE /mcp — session termination.
pub async fn handle_mcp_delete(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> Response {
    let session_id = match extract_session_id(&headers) {
        Some(sid) => sid,
        None => {
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    let sessions = &server.mcp_sessions;
    if sessions.remove_session(&session_id) {
        debug!(session_id = %session_id, "MCP session deleted");
        StatusCode::OK.into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

/// Extract the mcp-session-id from request headers.
fn extract_session_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}
