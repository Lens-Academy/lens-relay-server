use serde::{Deserialize, Serialize};
use serde_json::Value;

// JSON-RPC 2.0 error codes
pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug)]
pub enum JsonRpcMessage {
    Request(JsonRpcRequest),
    Notification(JsonRpcNotification),
}

#[derive(Debug, Serialize, Clone)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize, Clone)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Parse a JSON value into a JsonRpcMessage.
/// Presence of "id" field distinguishes Request from Notification.
pub fn parse_message(body: &Value) -> Result<JsonRpcMessage, JsonRpcError> {
    let obj = body.as_object().ok_or_else(|| JsonRpcError {
        code: INVALID_REQUEST,
        message: "Invalid Request: expected JSON object".into(),
        data: None,
    })?;

    // Check that "method" field exists and is a string
    if !obj.contains_key("method") {
        return Err(JsonRpcError {
            code: INVALID_REQUEST,
            message: "Invalid Request: missing method field".into(),
            data: None,
        });
    }

    // Presence of "id" field (even if null) distinguishes Request from Notification
    if obj.contains_key("id") {
        serde_json::from_value::<JsonRpcRequest>(body.clone())
            .map(JsonRpcMessage::Request)
            .map_err(|e| JsonRpcError {
                code: INVALID_REQUEST,
                message: format!("Invalid Request: {}", e),
                data: None,
            })
    } else {
        serde_json::from_value::<JsonRpcNotification>(body.clone())
            .map(JsonRpcMessage::Notification)
            .map_err(|e| JsonRpcError {
                code: INVALID_REQUEST,
                message: format!("Invalid Request: {}", e),
                data: None,
            })
    }
}

/// Create a success response with the given id and result.
pub fn success_response(id: Value, result: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: Some(result),
        error: None,
    }
}

/// Create an error response with the given id, code, and message.
pub fn error_response(id: Value, code: i64, message: impl Into<String>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.into(),
            data: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_request_with_numeric_id() {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
        let msg = parse_message(&body).expect("should parse");
        match msg {
            JsonRpcMessage::Request(req) => {
                assert_eq!(req.id, json!(1));
                assert_eq!(req.method, "ping");
                assert_eq!(req.jsonrpc, "2.0");
            }
            _ => panic!("expected Request"),
        }
    }

    #[test]
    fn parse_request_with_string_id() {
        let body = json!({"jsonrpc": "2.0", "id": "abc", "method": "tools/list"});
        let msg = parse_message(&body).expect("should parse");
        match msg {
            JsonRpcMessage::Request(req) => {
                assert_eq!(req.id, json!("abc"));
                assert_eq!(req.method, "tools/list");
            }
            _ => panic!("expected Request"),
        }
    }

    #[test]
    fn parse_request_with_null_id() {
        // id field present but null -> still a Request (not a Notification)
        let body = json!({"jsonrpc": "2.0", "id": null, "method": "ping"});
        let msg = parse_message(&body).expect("should parse");
        match msg {
            JsonRpcMessage::Request(req) => {
                assert_eq!(req.id, Value::Null);
                assert_eq!(req.method, "ping");
            }
            _ => panic!("expected Request, got Notification"),
        }
    }

    #[test]
    fn parse_notification_no_id() {
        let body = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let msg = parse_message(&body).expect("should parse");
        match msg {
            JsonRpcMessage::Notification(notif) => {
                assert_eq!(notif.method, "notifications/initialized");
                assert_eq!(notif.jsonrpc, "2.0");
            }
            _ => panic!("expected Notification"),
        }
    }

    #[test]
    fn parse_request_with_params() {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "tools/call",
            "params": {"name": "search", "arguments": {"query": "test"}}
        });
        let msg = parse_message(&body).expect("should parse");
        match msg {
            JsonRpcMessage::Request(req) => {
                assert_eq!(req.method, "tools/call");
                assert!(req.params.is_some());
                let params = req.params.unwrap();
                assert_eq!(params["name"], "search");
            }
            _ => panic!("expected Request"),
        }
    }

    #[test]
    fn parse_missing_method_returns_error() {
        let body = json!({"jsonrpc": "2.0", "id": 1});
        let err = parse_message(&body).unwrap_err();
        assert_eq!(err.code, INVALID_REQUEST);
    }

    #[test]
    fn parse_malformed_json_value_returns_error() {
        // A JSON value that is not an object at all
        let body = json!("just a string");
        let err = parse_message(&body).unwrap_err();
        assert_eq!(err.code, INVALID_REQUEST);
    }

    #[test]
    fn success_response_has_correct_structure() {
        let resp = success_response(json!(1), json!({"status": "ok"}));
        assert_eq!(resp.jsonrpc, "2.0");
        assert_eq!(resp.id, json!(1));
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
        assert_eq!(resp.result.unwrap()["status"], "ok");
    }

    #[test]
    fn error_response_has_correct_structure() {
        let resp = error_response(json!(5), METHOD_NOT_FOUND, "Method not found: foo");
        assert_eq!(resp.jsonrpc, "2.0");
        assert_eq!(resp.id, json!(5));
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
        let err = resp.error.unwrap();
        assert_eq!(err.code, METHOD_NOT_FOUND);
        assert_eq!(err.message, "Method not found: foo");
    }
}
