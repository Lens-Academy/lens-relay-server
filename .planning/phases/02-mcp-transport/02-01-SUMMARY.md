---
phase: 02-mcp-transport
plan: 01
subsystem: mcp-protocol
tags: [json-rpc, mcp, session-management, tdd]
dependency_graph:
  requires: []
  provides: [jsonrpc-types, session-manager, method-dispatch]
  affects: [02-02, 03-01]
tech_stack:
  added: []
  patterns: [dashmap-session-store, json-rpc-dispatch, nanoid-session-ids]
key_files:
  created:
    - crates/relay/src/mcp/mod.rs
    - crates/relay/src/mcp/jsonrpc.rs
    - crates/relay/src/mcp/session.rs
    - crates/relay/src/mcp/router.rs
  modified:
    - crates/relay/src/lib.rs
decisions:
  - id: jsonrpc-parse-by-id-field
    decision: "Distinguish Request from Notification by checking for 'id' field presence (not serde untagged enum)"
    reason: "Clearer error messages, explicit handling of null id case per JSON-RPC spec"
  - id: validate-session-before-tools
    decision: "Require initialized session for tools/call but not for ping or tools/list"
    reason: "Ping is a utility, tools/list is discovery -- both should work pre-init. Tool execution needs session context."
  - id: always-negotiate-2025-03-26
    decision: "Always respond with protocolVersion 2025-03-26 regardless of client request"
    reason: "2025-03-26 is confirmed working with Claude Code; forward-compatible structure"
metrics:
  duration: 7m
  completed: 2026-02-08
---

# Phase 2 Plan 1: MCP Protocol Engine Summary

JSON-RPC 2.0 message types, DashMap-backed session manager, and method dispatch router with 26 passing tests covering all MCP protocol behavior.

## Performance

- **Duration:** 7 minutes
- **Start:** 2026-02-08T21:24:49Z
- **End:** 2026-02-08T21:32:40Z
- **Tasks:** 2 (RED + GREEN; no REFACTOR needed)
- **Tests written:** 26
- **Files created:** 4
- **Files modified:** 1

## Accomplishments

1. **JSON-RPC 2.0 message parsing** - `parse_message()` correctly distinguishes Request (has `id` field, even if null) from Notification (no `id` field), with proper error handling for malformed input and missing method fields.

2. **Session management** - `SessionManager` backed by `DashMap` provides concurrent-safe session lifecycle: create (with nanoid 32-char IDs), get, mark_initialized, and remove. Tracks protocol version, client info, and activity timestamps.

3. **Method dispatch** - `dispatch_request()` routes to handlers for `initialize`, `ping`, `tools/list`, and `tools/call`. Unknown methods return `-32601`. `handle_notification()` processes `notifications/initialized` (marks session) and `notifications/cancelled` (no-op). Session validation enforces initialized state before tool execution.

4. **Module structure** - Clean `mod.rs` re-exports key types (`SessionManager`, `JsonRpcMessage`, `JsonRpcResponse`, `dispatch_request`, `handle_notification`) for use by the transport layer in Plan 02.

## Task Commits

| # | Type | Commit | Description |
|---|------|--------|-------------|
| 1 | RED | `563ae8bf` | Failing tests for all 3 modules (26 tests) |
| 2 | GREEN | `79dd5f60` | Full implementation, all 26 tests pass |

## Files Created

| File | Purpose |
|------|---------|
| `crates/relay/src/mcp/mod.rs` | Module declarations and key type re-exports |
| `crates/relay/src/mcp/jsonrpc.rs` | JSON-RPC 2.0 types, parse_message, response helpers, error code constants |
| `crates/relay/src/mcp/session.rs` | McpSession struct, SessionManager with DashMap |
| `crates/relay/src/mcp/router.rs` | dispatch_request, handle_notification, method handlers, session validation |

## Files Modified

| File | Change |
|------|--------|
| `crates/relay/src/lib.rs` | Added `pub mod mcp;` declaration |

## Decisions Made

1. **Parse by id field presence** - Used explicit `body.get("id").is_some()` check rather than `serde(untagged)` enum. This correctly handles the JSON-RPC spec case where `id: null` is still a Request, and produces better error messages on parse failure.

2. **Session validation scope** - `ping` and `tools/list` do not require an initialized session. Only `tools/call` validates session existence and initialization. This matches MCP spec behavior where ping is a utility and tools/list is discovery.

3. **Protocol version negotiation** - Always respond with `2025-03-26` regardless of what the client requests. Logged the client's requested version for diagnostics. This avoids complexity while maintaining compatibility.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added `pub mod mcp;` to lib.rs**

- **Found during:** Task 1 (RED phase)
- **Issue:** Plan said "Do NOT modify lib.rs" but without the module declaration, the mcp module cannot be compiled or tested by cargo
- **Fix:** Added single line `pub mod mcp;` to lib.rs
- **Files modified:** `crates/relay/src/lib.rs`
- **Commit:** `563ae8bf`

## Issues Encountered

None.

## Next Phase Readiness

Plan 02-02 (HTTP transport handler + server integration) can proceed immediately. All types it needs are exported from `mcp::`:
- `SessionManager` for adding to Server struct
- `JsonRpcMessage` and `parse_message` for POST body parsing
- `dispatch_request` and `handle_notification` for routing
- `JsonRpcResponse` for serializing responses
- Error code constants for HTTP-level error responses
