# Phase 2: MCP Transport - Research

**Researched:** 2026-02-08
**Domain:** MCP Streamable HTTP transport, JSON-RPC 2.0, Axum 0.7 SSE
**Confidence:** HIGH

## Summary

This phase implements the MCP (Model Context Protocol) Streamable HTTP transport layer in the existing Rust/Axum relay server. The transport receives JSON-RPC messages via HTTP POST at `/mcp`, dispatches them to the correct session, and returns responses as either `application/json` or `text/event-stream` (SSE). Sessions are tracked via the `Mcp-Session-Id` header, assigned by the server during the `initialize` handshake.

The implementation is custom (no rmcp SDK) as already decided. This is the right call: the scope is 5 tools, the relay server uses Axum 0.7, and rmcp requires Axum 0.8. A custom implementation gives full control over session state and avoids a framework upgrade.

The key finding is that both MCP protocol versions (2025-03-26 and 2025-11-25) need consideration. The 2025-11-25 spec introduced breaking changes: batching was removed (single messages only), the session header was renamed from `Mcp-Session-Id` to `MCP-Session-Id`, and a new `MCP-Protocol-Version` header is required on all post-init requests. Targeting 2025-03-26 is safest since Claude Code is confirmed to use that version, but the implementation should be structured to accommodate 2025-11-25 easily.

**Primary recommendation:** Implement the 2025-03-26 Streamable HTTP spec (single messages only, no batching -- simpler and forward-compatible with 2025-11-25). Use `Mcp-Session-Id` header. Return `application/json` for all responses (no SSE streaming needed for this phase -- tools return synchronous results). Accept both header casings for robustness.

## Standard Stack

### Core

No new crate dependencies needed. Everything required is already in the relay server's dependency tree.

| Library | Version | Purpose | Already in Cargo.toml? |
|---------|---------|---------|------------------------|
| axum | 0.7.4 | HTTP routing, SSE response types | Yes |
| serde_json | 1.0.103 | JSON-RPC message parsing/serialization | Yes |
| serde | 1.0.171 | Derive Serialize/Deserialize for message types | Yes |
| dashmap | 6.0.1 | Concurrent session storage (same pattern as `docs` map) | Yes |
| nanoid | 0.4.0 | Session ID generation (cryptographically random) | Yes |
| tokio | 1.29.1 | Async runtime, channels for future SSE | Yes |
| tracing | 0.1.37 | Structured logging | Yes |

### Supporting (for future SSE, not needed in Phase 2)

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| axum::response::sse | (part of axum) | SSE streaming responses | When tools need streaming (Phase 2 can skip) |
| tokio_stream | 0.1.14 | Stream adapters for SSE | Already in deps, needed if SSE streaming added |
| futures | 0.3.28 | Stream combinators | Already in deps |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Custom JSON-RPC | rmcp crate | Requires Axum 0.7 -> 0.8 upgrade; overkill for 5 tools |
| Custom JSON-RPC | jsonrpsee | Heavy; designed for websocket, not HTTP POST |
| nanoid for session IDs | uuid crate | uuid would work, but nanoid already in deps and is fine |
| DashMap for sessions | std::sync::RwLock<HashMap> | DashMap already used for docs, proven pattern in codebase |

**No new dependencies needed.** This is a significant advantage of the custom approach.

## Architecture Patterns

### Recommended Module Structure

```
crates/relay/src/
  mcp/
    mod.rs           # Module exports, MCP method constants
    transport.rs     # Axum handlers (POST /mcp, GET /mcp, DELETE /mcp)
    session.rs       # McpSession struct, SessionManager (DashMap-based)
    jsonrpc.rs       # JSON-RPC 2.0 types (Request, Response, Error, Notification)
    router.rs        # Method dispatch (initialize -> handler, tools/list -> handler, etc.)
  lib.rs             # Add `pub mod mcp;`
  server.rs          # Add `/mcp` route to `fn routes()`
```

### Pattern 1: JSON-RPC Message Types

**What:** Strongly-typed JSON-RPC 2.0 request/response/error types using serde.
**When to use:** For all MCP message parsing and serialization.
**Example:**

```rust
// Source: JSON-RPC 2.0 specification (https://www.jsonrpc.org/specification)
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,        // String, Number, or Null
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

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}
```

Note: Use `#[serde(untagged)]` enum to parse either request or notification from the same POST body:

```rust
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    Request(JsonRpcRequest),       // Has "id" field
    Notification(JsonRpcNotification), // No "id" field
}
```

However, `serde(untagged)` can produce confusing error messages. A better approach is to deserialize to a raw `Value` first, then check for `"id"` field presence to distinguish request from notification:

```rust
pub fn parse_message(body: &Value) -> Result<JsonRpcMessage, JsonRpcError> {
    if body.get("id").is_some() {
        // It's a request
        serde_json::from_value(body.clone()).map(JsonRpcMessage::Request)
    } else {
        // It's a notification
        serde_json::from_value(body.clone()).map(JsonRpcMessage::Notification)
    }
    .map_err(|_| JsonRpcError {
        code: -32600,
        message: "Invalid Request".into(),
        data: None,
    })
}
```

### Pattern 2: Session Management with DashMap

**What:** Thread-safe concurrent session store using DashMap (same pattern as the existing `docs` field in Server).
**When to use:** For MCP session lifecycle.
**Example:**

```rust
// Source: Codebase pattern from server.rs (docs: Arc<DashMap<String, DocWithSyncKv>>)
use dashmap::DashMap;
use std::sync::Arc;
use std::time::Instant;

pub struct McpSession {
    pub session_id: String,
    pub protocol_version: String,
    pub client_info: Option<Value>,
    pub initialized: bool,          // true after receiving initialized notification
    pub created_at: Instant,
    pub last_activity: Instant,
    // Future: read_docs tracking for read-before-edit enforcement
}

pub struct SessionManager {
    sessions: DashMap<String, McpSession>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
        }
    }

    pub fn create_session(&self, protocol_version: String, client_info: Option<Value>) -> String {
        let session_id = nanoid::nanoid!(32); // 32-char random ID
        let session = McpSession {
            session_id: session_id.clone(),
            protocol_version,
            client_info,
            initialized: false,
            created_at: Instant::now(),
            last_activity: Instant::now(),
        };
        self.sessions.insert(session_id.clone(), session);
        session_id
    }

    pub fn get_session(&self, session_id: &str) -> Option<dashmap::mapref::one::Ref<'_, String, McpSession>> {
        let session = self.sessions.get(session_id)?;
        // Touch last_activity -- note: requires get_mut for mutation
        Some(session)
    }

    pub fn remove_session(&self, session_id: &str) -> bool {
        self.sessions.remove(session_id).is_some()
    }
}
```

### Pattern 3: POST Handler with Response Type Branching

**What:** Single Axum handler for POST /mcp that returns either JSON or 202 Accepted.
**When to use:** The main MCP endpoint handler.
**Example:**

```rust
// Source: MCP spec (https://modelcontextprotocol.io/specification/2025-03-26/basic/transports)
// + Axum IntoResponse pattern (https://docs.rs/axum/0.7.4/axum/response/trait.IntoResponse.html)
use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response, Json},
};

async fn handle_mcp_post(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    // 1. Parse JSON body
    let value: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return json_rpc_error(None, -32700, "Parse error").into_response(),
    };

    // 2. Determine message type
    let is_request = value.get("id").is_some();
    let method = value.get("method").and_then(|m| m.as_str());

    // 3. Handle based on type
    if !is_request {
        // Notification or response -> 202 Accepted
        // Process notification (e.g., notifications/initialized)
        handle_notification(&server, &headers, &value).await;
        return StatusCode::ACCEPTED.into_response();
    }

    // 4. It's a request -- dispatch to method handler
    let session_id = headers.get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let response = dispatch_request(&server, session_id, &value).await;

    // 5. Return JSON response, include Mcp-Session-Id if this was initialize
    let mut http_response = Json(response).into_response();
    if method == Some("initialize") {
        if let Some(sid) = extract_new_session_id(&value) {
            http_response.headers_mut().insert(
                "mcp-session-id",
                HeaderValue::from_str(&sid).unwrap(),
            );
        }
    }
    http_response
}
```

### Pattern 4: Method Dispatch Table

**What:** Simple match-based routing for JSON-RPC methods.
**When to use:** Mapping MCP method names to handler functions.
**Example:**

```rust
async fn dispatch_request(
    server: &Arc<Server>,
    session_id: Option<String>,
    request: &Value,
) -> JsonRpcResponse {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = request.get("params").cloned();

    match method {
        "initialize" => handle_initialize(server, id, params).await,
        "ping" => handle_ping(id),
        "tools/list" => handle_tools_list(id),
        "tools/call" => {
            let session = match validate_session(server, &session_id) {
                Ok(s) => s,
                Err(e) => return e,
            };
            handle_tools_call(server, &session, id, params).await
        }
        _ => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: format!("Method not found: {}", method),
                data: None,
            }),
        },
    }
}
```

### Anti-Patterns to Avoid

- **Deserializing request body to a specific method type before routing:** Parse to generic `Value` first, route by method name, then deserialize params for the specific method. Avoids complex generic type gymnastics.
- **Using SSE for simple request/response:** The spec allows returning `application/json` directly. For tools that return a single synchronous result (all 5 of our tools), JSON is simpler and sufficient. Reserve SSE for long-running operations.
- **Sharing state between sessions:** Each MCP session is independent. Never share mutable state between sessions. Use the session ID to isolate all session-specific data.
- **Implementing JSON-RPC batching:** The 2025-11-25 spec removed batching. Even the 2025-03-26 spec allows it optionally. Skip it -- no known MCP client sends batches.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| JSON-RPC error codes | Ad-hoc error integers | Constants from the spec | Standard codes: -32700 (parse), -32600 (invalid request), -32601 (method not found), -32602 (invalid params), -32603 (internal error) |
| Session ID generation | Custom random strings | `nanoid::nanoid!(32)` | Already in deps, cryptographically random, URL-safe |
| Concurrent map | RwLock<HashMap> | DashMap (already used) | Proven pattern in this codebase, less contention |
| SSE event formatting | Manual string building | `axum::response::sse::Event` | Handles newline escaping, content-type headers, keep-alive |

**Key insight:** The JSON-RPC 2.0 spec is simple enough to implement with raw serde_json. The MCP protocol layer on top is thin (6-8 methods). The real complexity is in the tool implementations (Phase 4), not the transport.

## Common Pitfalls

### Pitfall 1: Case-Sensitive Header Names

**What goes wrong:** The `Mcp-Session-Id` header name differs between spec versions (2025-03-26 uses `Mcp-Session-Id`, 2025-11-25 uses `MCP-Session-Id`). HTTP headers are case-insensitive per RFC 7230, but some clients or proxies may send them in specific casing.
**Why it happens:** Spec revision changed the casing convention.
**How to avoid:** When reading the session ID from headers, use case-insensitive lookup. Axum's `HeaderMap::get()` is already case-insensitive (it normalizes to lowercase internally). When setting the response header, use lowercase `mcp-session-id` (Axum normalizes this).
**Warning signs:** Sessions not being found for valid clients.

### Pitfall 2: Missing Session ID on Non-Initialize Requests

**What goes wrong:** Client sends a `tools/call` request without the `Mcp-Session-Id` header (e.g., after a reconnect or misconfiguration). Server processes it anyway, leading to undefined behavior.
**Why it happens:** Client bug or network proxy stripping custom headers.
**How to avoid:** Validate that all non-initialize requests include a valid session ID. Return HTTP 400 Bad Request if missing. Return HTTP 404 Not Found if session ID is unknown (per spec, this signals the client to re-initialize).
**Warning signs:** Tools executing without session context.

### Pitfall 3: JSON-RPC id Field Type

**What goes wrong:** The `id` field in JSON-RPC can be a string, number, or null. Treating it as always a number (or always a string) breaks interoperability.
**Why it happens:** Different clients use different types. Claude Code may use integers; other clients may use strings.
**How to avoid:** Use `serde_json::Value` for the `id` field in both requests and responses. Always echo back the exact `id` value in the response.
**Warning signs:** Client receives response but can't match it to its pending request.

### Pitfall 4: Forgetting the initialized Notification

**What goes wrong:** Server starts processing tool calls before the client sends `notifications/initialized`. This violates the MCP lifecycle.
**Why it happens:** The initialize response is sent, and the developer assumes the session is ready.
**How to avoid:** Track `initialized: bool` in the session. After sending the `InitializeResult`, wait for the `notifications/initialized` notification before allowing tool calls. Return an error for premature tool calls.
**Warning signs:** Tool calls work in testing but fail with strict MCP clients.

### Pitfall 5: Blocking the Tokio Runtime

**What goes wrong:** Synchronous operations (like tantivy search) block the async executor.
**Why it happens:** Search index queries are CPU-bound and synchronous.
**How to avoid:** Use `tokio::task::spawn_blocking()` for any tantivy search operations (this pattern is already used in `handle_search`). Keep the MCP transport layer fully async.
**Warning signs:** All MCP clients slow down when one client runs a search.

### Pitfall 6: Not Returning Correct HTTP Status Codes

**What goes wrong:** Returning 200 for everything instead of the MCP-specified status codes.
**Why it happens:** It's easy to always return JSON with an error inside.
**How to avoid:** Follow the spec strictly:
- Initialize request with valid response: 200 + JSON body + `Mcp-Session-Id` header
- Notification/response from client: 202 Accepted (no body)
- Request without required session ID: 400 Bad Request
- Request with unknown session ID: 404 Not Found
- Unknown method: 200 + JSON-RPC error response (method not found is a protocol-level error, not HTTP-level)
**Warning signs:** Clients failing to detect session expiry.

## Code Examples

### Initialize Request/Response

```rust
// Source: MCP spec (https://modelcontextprotocol.io/specification/2025-03-26/basic/lifecycle)

async fn handle_initialize(
    server: &Arc<Server>,
    id: Value,
    params: Option<Value>,
) -> (JsonRpcResponse, Option<String>) {
    // Parse client's protocol version and info
    let protocol_version = params.as_ref()
        .and_then(|p| p.get("protocolVersion"))
        .and_then(|v| v.as_str())
        .unwrap_or("2025-03-26")
        .to_string();

    let client_info = params.as_ref()
        .and_then(|p| p.get("clientInfo"))
        .cloned();

    // Version negotiation: we support 2025-03-26
    let negotiated_version = if protocol_version == "2025-03-26"
        || protocol_version == "2025-11-25" {
        "2025-03-26".to_string()  // Respond with our supported version
    } else {
        "2025-03-26".to_string()  // Always respond with what we support
    };

    // Create session
    let session_id = server.mcp_sessions.create_session(
        negotiated_version.clone(),
        client_info,
    );

    let response = JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: Some(serde_json::json!({
            "protocolVersion": negotiated_version,
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "lens-relay",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),
        error: None,
    };

    (response, Some(session_id))
}
```

### Ping Handler

```rust
// Source: MCP spec (https://modelcontextprotocol.io/specification/2025-03-26/basic/utilities/ping)

fn handle_ping(id: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: Some(serde_json::json!({})),
        error: None,
    }
}
```

### Tools List Response

```rust
// Source: MCP spec (https://modelcontextprotocol.io/specification/2025-03-26/server/tools)

fn handle_tools_list(id: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: Some(serde_json::json!({
            "tools": [
                // Tool definitions will be added in Phase 4
                // Example structure:
                // {
                //     "name": "search",
                //     "description": "Search documents by keyword",
                //     "inputSchema": {
                //         "type": "object",
                //         "properties": {
                //             "query": { "type": "string", "description": "Search query" }
                //         },
                //         "required": ["query"]
                //     }
                // }
            ]
        })),
        error: None,
    }
}
```

### Axum Route Registration

```rust
// Source: Codebase pattern from server.rs fn routes()

// In server.rs, add to fn routes():
pub fn routes(self: &Arc<Self>) -> Router {
    let mut router = Router::new()
        // ... existing routes ...
        .route("/search", get(handle_search))
        // MCP endpoint (POST for messages, GET for SSE, DELETE for session termination)
        .route("/mcp", post(handle_mcp_post).get(handle_mcp_get).delete(handle_mcp_delete));
    // ...
}
```

### SSE Response (for future use)

```rust
// Source: Axum SSE docs (https://docs.rs/axum/0.7.4/axum/response/sse/index.html)
// This is NOT needed for Phase 2, but shows the pattern for future phases.

use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use futures::stream;
use std::convert::Infallible;

async fn handle_mcp_post_with_sse(/* ... */) -> Response {
    // For simple synchronous tools, return JSON directly:
    if !needs_streaming {
        return Json(response).into_response();
    }

    // For streaming responses (future), return SSE:
    let (tx, rx) = tokio::sync::mpsc::channel(32);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let sse_stream = stream.map(|msg: String| -> Result<SseEvent, Infallible> {
        Ok(SseEvent::default()
            .event("message")
            .data(msg))
    });

    Sse::new(sse_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| HTTP+SSE transport (two endpoints) | Streamable HTTP (single endpoint) | MCP 2025-03-26 | Simpler: one URL for everything |
| `Mcp-Session-Id` header | `MCP-Session-Id` header | MCP 2025-11-25 | Case change; HTTP headers are case-insensitive so both work |
| JSON-RPC batching supported | Batching removed | MCP 2025-11-25 | Simplifies implementation; no need to handle arrays |
| No protocol version header | `MCP-Protocol-Version` required | MCP 2025-11-25 | Server should accept absence (assume 2025-03-26) |

**Deprecated/outdated:**
- HTTP+SSE transport (2024-11-05): Replaced by Streamable HTTP. We don't need to support it.
- JSON-RPC batching in MCP: Removed in 2025-11-25. Don't implement it.

## MCP Protocol Quick Reference

### Methods We Must Handle (Phase 2 Transport)

| Method | Type | Phase 2 Action |
|--------|------|----------------|
| `initialize` | Request | Create session, return capabilities + session ID |
| `notifications/initialized` | Notification | Mark session as initialized, return 202 |
| `ping` | Request | Return empty result `{}` |
| `tools/list` | Request | Return empty tools array (tools added in Phase 4) |
| `tools/call` | Request | Return "not implemented" error (tools added in Phase 4) |
| `notifications/cancelled` | Notification | Log and ignore (no long-running ops yet), return 202 |

### HTTP Methods for /mcp

| HTTP Method | Purpose | Phase 2 Action |
|-------------|---------|----------------|
| POST | Send JSON-RPC messages | Full implementation |
| GET | Open SSE stream for server-initiated messages | Return 405 Method Not Allowed (not needed yet) |
| DELETE | Terminate session | Remove session from DashMap, return 200 |

### JSON-RPC Error Codes

| Code | Name | When to Use |
|------|------|-------------|
| -32700 | Parse error | Invalid JSON body |
| -32600 | Invalid Request | Missing jsonrpc/method fields |
| -32601 | Method not found | Unknown method name |
| -32602 | Invalid params | Method params don't match schema |
| -32603 | Internal error | Server-side failure |

## Open Questions

1. **Which protocol version should we negotiate?**
   - What we know: Claude Code likely uses 2025-03-26. The 2025-11-25 spec is newer but adds complexity (Tasks, OAuth, extensions).
   - What's unclear: Whether Claude Code has already upgraded to send 2025-11-25.
   - Recommendation: Respond with `2025-03-26` always. This is backward-compatible. The transport behavior is identical for both versions (the differences are in higher-level features like Tasks which we don't use).

2. **Session expiration/cleanup**
   - What we know: Sessions should be cleaned up when clients disconnect. The spec says servers MAY terminate sessions.
   - What's unclear: Optimal timeout duration. No guidance in spec.
   - Recommendation: Implement a simple TTL (e.g., 30 minutes of inactivity). Use a background task that periodically sweeps expired sessions from the DashMap. Not critical for Phase 2 (can be a simple periodic task).

3. **Thread safety of session state mutation**
   - What we know: DashMap provides thread-safe access. But mutating session fields (like `last_activity` or `initialized`) requires `get_mut()`.
   - What's unclear: Whether contention will be an issue.
   - Recommendation: Use `DashMap::get_mut()` for mutations. With few concurrent MCP sessions (typically 1-3 AI assistants), contention is negligible.

4. **Origin header validation**
   - What we know: MCP spec MUST validate Origin to prevent DNS rebinding. But this server runs behind Cloudflare Tunnel in production.
   - What's unclear: Whether Cloudflare Tunnel strips or modifies Origin headers.
   - Recommendation: For Phase 2, log the Origin header but don't enforce validation. Add strict validation later when auth is in place. The server is behind a tunnel, not exposed to direct internet.

## Sources

### Primary (HIGH confidence)
- MCP Specification 2025-03-26 -- Transports: https://modelcontextprotocol.io/specification/2025-03-26/basic/transports
- MCP Specification 2025-03-26 -- Lifecycle: https://modelcontextprotocol.io/specification/2025-03-26/basic/lifecycle
- MCP Specification 2025-03-26 -- Tools: https://modelcontextprotocol.io/specification/2025-03-26/server/tools
- MCP Specification 2025-03-26 -- Ping: https://modelcontextprotocol.io/specification/2025-03-26/basic/utilities/ping
- MCP Specification 2025-03-26 -- Cancellation: https://modelcontextprotocol.io/specification/2025-03-26/basic/utilities/cancellation
- MCP Specification 2025-11-25 -- Transports: https://modelcontextprotocol.io/specification/2025-11-25/basic/transports
- JSON-RPC 2.0 Specification: https://www.jsonrpc.org/specification
- Axum 0.7 SSE docs: https://docs.rs/axum/0.7.4/axum/response/sse/index.html
- Axum IntoResponse trait: https://docs.rs/axum/latest/axum/response/trait.IntoResponse.html
- Codebase: `crates/relay/src/server.rs` (routes, Server struct, DashMap patterns, AppError)

### Secondary (MEDIUM confidence)
- Claude Code MCP configuration docs: https://code.claude.com/docs/en/mcp (confirms `--transport http` and Streamable HTTP)
- MCP Inspector for testing: https://github.com/modelcontextprotocol/inspector
- Shuttle blog on Streamable HTTP MCP in Rust: https://www.shuttle.dev/blog/2025/10/29/stream-http-mcp (confirms rmcp requires Axum 0.8; validates custom approach)

### Tertiary (LOW confidence)
- Claude Code GitHub issue #5960 on Streamable HTTP: https://github.com/anthropics/claude-code/issues/5960 (mentions potential client-side issues with Streamable HTTP)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- no new dependencies, all patterns verified in existing codebase
- Architecture: HIGH -- module structure follows codebase conventions; JSON-RPC types are straightforward serde
- Pitfalls: HIGH -- sourced directly from MCP spec requirements and JSON-RPC spec
- Code examples: HIGH -- verified against MCP spec and Axum 0.7 docs

**Research date:** 2026-02-08
**Valid until:** 2026-03-08 (MCP spec is stable; 2025-11-25 is latest)
