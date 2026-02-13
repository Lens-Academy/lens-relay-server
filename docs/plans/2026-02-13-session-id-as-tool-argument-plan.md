# Session ID as Tool Argument — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the MCP edit tool work on Claude.ai by passing session ID as a tool argument instead of relying on the transport-level mcp-session-id header (which Claude.ai discards between tool calls).

**Architecture:** The `read` tool appends `[session: <id>]` to its response. The `edit` tool requires a `session_id` argument and looks up that session's `read_docs` set. Sessions get TTL cleanup since Claude.ai never sends DELETE.

**Tech Stack:** Rust, serde_json, DashMap, yrs (Y.Doc CRDT)

**Design doc:** `docs/plans/2026-02-13-session-id-as-tool-argument.md`

---

### Task 1: Session TTL cleanup

**Files:**
- Modify: `crates/relay/src/mcp/session.rs`

**Context:** Sessions accumulate because Claude.ai creates a new one per tool call and never deletes them. `SessionManager` uses `DashMap<String, McpSession>`. Each `McpSession` already has a `created_at: Instant` field.

**Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `crates/relay/src/mcp/session.rs`:

```rust
#[test]
fn cleanup_stale_removes_old_sessions() {
    let mgr = SessionManager::new();
    let id = mgr.create_session("2025-03-26".into(), None);

    // Session exists
    assert!(mgr.get_session(&id).is_some());

    // Cleanup with 0 duration removes everything
    mgr.cleanup_stale(std::time::Duration::from_secs(0));

    assert!(mgr.get_session(&id).is_none());
}

#[test]
fn cleanup_stale_keeps_fresh_sessions() {
    let mgr = SessionManager::new();
    let id = mgr.create_session("2025-03-26".into(), None);

    // Cleanup with 1 hour keeps the just-created session
    mgr.cleanup_stale(std::time::Duration::from_secs(3600));

    assert!(mgr.get_session(&id).is_some());
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path crates/Cargo.toml -p relay --lib mcp::session::tests::cleanup_stale 2>&1 | tail -20`
Expected: FAIL — `cleanup_stale` method does not exist.

**Step 3: Implement cleanup_stale**

Add to the `impl SessionManager` block in `crates/relay/src/mcp/session.rs`, after the `remove_session` method:

```rust
/// Remove sessions older than `max_age`.
pub fn cleanup_stale(&self, max_age: std::time::Duration) {
    let cutoff = Instant::now() - max_age;
    self.sessions.retain(|_, session| session.created_at > cutoff);
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path crates/Cargo.toml -p relay --lib mcp::session::tests::cleanup_stale 2>&1 | tail -10`
Expected: 2 tests PASS.

**Step 5: Add lazy cleanup call in create_session**

In `crates/relay/src/mcp/session.rs`, add a constant at the top of the file (after the `use` block):

```rust
/// Maximum session age before cleanup. Sessions from clients that never
/// send DELETE (e.g. Claude.ai) are purged after this duration.
const SESSION_TTL: std::time::Duration = std::time::Duration::from_secs(3600);
```

Then add `self.cleanup_stale(SESSION_TTL);` as the first line of `create_session()`.

**Step 6: Run all session tests**

Run: `cargo test --manifest-path crates/Cargo.toml -p relay --lib mcp::session::tests 2>&1 | tail -15`
Expected: All tests PASS (existing + new).

**Step 7: Commit**

```
jj describe -m "feat(mcp): add session TTL cleanup"
jj new
```

---

### Task 2: Read tool appends session ID to response

**Files:**
- Modify: `crates/relay/src/mcp/tools/read.rs`

**Context:** The `read::execute` function returns `Ok(format_cat_n(&content, offset, limit))`. It receives `session_id: &str` (the transport session ID). After recording the read, it should append the session ID to the response so the LLM can pass it to the edit tool.

**Step 1: Write the failing test**

Add to `#[cfg(test)] mod tests` in `crates/relay/src/mcp/tools/read.rs`:

```rust
#[test]
fn format_cat_n_with_session_appends_session_line() {
    let content = "hello\nworld";
    let result = format_cat_n_with_session(content, 0, 2000, "test-session-abc");
    assert!(result.ends_with("\n\n[session: test-session-abc]"));
    assert!(result.starts_with("     1\thello\n     2\tworld"));
}

#[test]
fn format_cat_n_with_session_empty_content() {
    let result = format_cat_n_with_session("", 0, 2000, "sid-123");
    assert_eq!(result, "\n\n[session: sid-123]");
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path crates/Cargo.toml -p relay --lib mcp::tools::read::tests::format_cat_n_with_session 2>&1 | tail -20`
Expected: FAIL — `format_cat_n_with_session` does not exist.

**Step 3: Implement format_cat_n_with_session**

Add a new function in `crates/relay/src/mcp/tools/read.rs` (after `format_cat_n`):

```rust
/// Format content as cat -n output with session ID appended.
fn format_cat_n_with_session(content: &str, offset: usize, limit: usize, session_id: &str) -> String {
    let formatted = format_cat_n(content, offset, limit);
    format!("{}\n\n[session: {}]", formatted, session_id)
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path crates/Cargo.toml -p relay --lib mcp::tools::read::tests::format_cat_n_with_session 2>&1 | tail -10`
Expected: 2 tests PASS.

**Step 5: Wire up in execute()**

In `crates/relay/src/mcp/tools/read.rs`, change the return statement from:

```rust
Ok(format_cat_n(&content, offset, limit))
```

to:

```rust
Ok(format_cat_n_with_session(&content, offset, limit, session_id))
```

**Step 6: Run all read tests**

Run: `cargo test --manifest-path crates/Cargo.toml -p relay --lib mcp::tools::read::tests 2>&1 | tail -15`
Expected: All tests PASS.

**Step 7: Commit**

```
jj describe -m "feat(mcp): read tool appends session ID to response"
jj new
```

---

### Task 3: Edit tool reads session_id from arguments

**Files:**
- Modify: `crates/relay/src/mcp/tools/edit.rs`

**Context:** Currently `edit::execute` takes `session_id: &str` from the transport layer and checks `session.read_docs`. Change it to read `session_id` from `arguments` instead.

**Step 1: Update the function signature and guard**

In `crates/relay/src/mcp/tools/edit.rs`, change the function signature from:

```rust
pub fn execute(
    server: &Arc<Server>,
    session_id: &str,
    arguments: &Value,
) -> Result<String, String> {
```

to:

```rust
pub fn execute(
    server: &Arc<Server>,
    arguments: &Value,
) -> Result<String, String> {
```

Then add `session_id` parsing right after the `new_string` parameter extraction (before the `// 2. Resolve document path` comment):

```rust
let session_id = arguments
    .get("session_id")
    .and_then(|v| v.as_str())
    .ok_or_else(|| "Missing required parameter: session_id. Pass the session value from the read tool's response.".to_string())?;
```

The rest of the function uses `session_id` unchanged — it's now a local variable instead of a parameter.

**Step 2: Update all existing test helpers and tests**

In the test module, the helper `setup_session_with_read` returns a session ID. All tests currently pass it as the `session_id` parameter to `execute()`. Update them to pass it inside the `arguments` JSON instead.

In every test that calls `execute()`, change from:

```rust
let result = execute(&server, &sid, &json!({...}));
```

to:

```rust
let result = execute(&server, &json!({..., "session_id": sid}));
```

Specifically, update these tests:
- `edit_basic_replacement`: Add `"session_id": sid` to the json
- `edit_read_before_edit_enforced`: Add `"session_id": sid` to the json
- `edit_old_string_not_found`: Add `"session_id": sid` to the json
- `edit_old_string_not_unique`: Add `"session_id": sid` to the json
- `edit_document_not_found`: Add `"session_id": sid` to the json
- `edit_missing_parameters`: Add `"session_id": sid` to the json
- `edit_preserves_surrounding_content`: Add `"session_id": sid` to the json
- `edit_multiline_old_string`: Add `"session_id": sid` to the json
- `edit_empty_new_string`: Add `"session_id": sid` to the json
- `edit_success_message`: Add `"session_id": sid` to the json

**Step 3: Add test for missing session_id argument**

Add to the test module:

```rust
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
```

**Step 4: Add test for invalid session_id**

```rust
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
```

**Step 5: Run all edit tests**

Run: `cargo test --manifest-path crates/Cargo.toml -p relay --lib mcp::tools::edit::tests 2>&1 | tail -20`
Expected: All tests PASS (10 existing + 2 new).

**Step 6: Commit**

```
jj describe -m "feat(mcp): edit tool reads session_id from arguments instead of transport"
jj new
```

---

### Task 4: Update dispatch and tool definitions

**Files:**
- Modify: `crates/relay/src/mcp/tools/mod.rs`

**Context:** The `dispatch_tool` function currently passes `session_id` to `edit::execute`. The tool definitions JSON needs `session_id` added to edit's schema, and the read description updated.

**Step 1: Update dispatch_tool**

In `crates/relay/src/mcp/tools/mod.rs`, change the edit dispatch from:

```rust
"edit" => match edit::execute(server, session_id, arguments) {
```

to:

```rust
"edit" => match edit::execute(server, arguments) {
```

**Step 2: Update edit tool definition**

In `tool_definitions()`, replace the edit tool definition with:

```rust
json!({
    "name": "edit",
    "description": "Edit a document by replacing old_string with new_string. The change is wrapped in CriticMarkup ({--old--}{++new++}) for human review. You must read the document first.",
    "inputSchema": {
        "type": "object",
        "required": ["file_path", "old_string", "new_string", "session_id"],
        "additionalProperties": false,
        "properties": {
            "file_path": {
                "type": "string",
                "description": "Path to the document (e.g. 'Lens/Photosynthesis.md')"
            },
            "old_string": {
                "type": "string",
                "description": "The exact text to find and replace. Must match exactly and be unique in the document."
            },
            "new_string": {
                "type": "string",
                "description": "The replacement text. Empty string for deletion."
            },
            "session_id": {
                "type": "string",
                "description": "The session value from the read tool's response. Required to verify the document was read before editing."
            }
        }
    }
}),
```

**Step 3: Update read tool description**

In `tool_definitions()`, change the read tool's description from:

```
"Reads a document from the knowledge base. Returns content with line numbers (cat -n format). Supports partial reads via offset and limit."
```

to:

```
"Reads a document from the knowledge base. Returns content with line numbers (cat -n format). Supports partial reads via offset and limit. The response includes a [session: ...] value — pass this to the edit tool's session_id parameter when editing."
```

**Step 4: Build to check compilation**

Run: `cargo build --manifest-path crates/Cargo.toml -p relay 2>&1 | tail -20`
Expected: Compiles without errors.

**Step 5: Run router test that checks tool count**

Run: `cargo test --manifest-path crates/Cargo.toml -p relay --lib mcp::router::tests::tools_list_returns_five_tools 2>&1 | tail -10`
Expected: PASS (still 5 tools, just different schemas).

**Step 6: Commit**

```
jj describe -m "feat(mcp): update tool definitions and dispatch for session_id argument"
jj new
```

---

### Task 5: Integration test — read then edit via dispatch

**Files:**
- Modify: `crates/relay/src/mcp/router.rs` (add test in existing test module)

**Context:** The router test `read_records_doc_in_session` already sets up a full server with docs. Add a test that calls read via dispatch, extracts the session ID from the response, then calls edit with it.

**Step 1: Write the integration test**

Add to `#[cfg(test)] mod tests` in `crates/relay/src/mcp/router.rs`:

```rust
#[test]
fn read_then_edit_via_session_id_argument() {
    use std::collections::HashMap;
    use yrs::{Any, Doc, Map, Text, Transact, WriteTxn};

    let server = test_server();

    // Set up a doc with content
    let relay_id = "cb696037-0f72-4e93-8717-4e433129d789";
    let folder_uuid = "aaaa0000-0000-0000-0000-000000000000";
    let content_uuid = "uuid-rte";
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
        filemeta.insert(&mut txn, "/EditTest.md", Any::Map(map.into()));
    }

    server
        .doc_resolver()
        .update_folder_from_doc(&folder_doc_id, 0, &folder_doc);

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
        text.insert(&mut txn, 0, "hello world");
    }
    server.docs().insert(content_doc_id.clone(), dwskv);

    // Create and initialize a session
    let sid = server
        .mcp_sessions
        .create_session("2025-03-26".into(), None);
    server.mcp_sessions.mark_initialized(&sid);

    // Step 1: Call read tool
    let read_req = make_request(
        json!(20),
        "tools/call",
        Some(json!({"name": "read", "arguments": {"file_path": "Lens/EditTest.md"}})),
    );
    let (read_resp, _) = dispatch_request(&server, Some(&sid), &read_req);
    assert!(read_resp.error.is_none(), "read should succeed");

    // Extract session ID from response text
    let read_result = read_resp.result.unwrap();
    let read_text = read_result["content"][0]["text"].as_str().unwrap();
    assert!(
        read_text.contains("[session: "),
        "read response should contain session ID: {}",
        read_text
    );
    let session_token = read_text
        .rsplit("[session: ")
        .next()
        .unwrap()
        .trim_end_matches(']');

    // Step 2: Call edit tool with extracted session_id
    let edit_req = make_request(
        json!(21),
        "tools/call",
        Some(json!({
            "name": "edit",
            "arguments": {
                "file_path": "Lens/EditTest.md",
                "old_string": "hello",
                "new_string": "goodbye",
                "session_id": session_token
            }
        })),
    );
    // Use a DIFFERENT transport session to simulate Claude.ai behavior
    let sid2 = server
        .mcp_sessions
        .create_session("2025-03-26".into(), None);
    server.mcp_sessions.mark_initialized(&sid2);

    let (edit_resp, _) = dispatch_request(&server, Some(&sid2), &edit_req);
    assert!(edit_resp.error.is_none(), "edit should succeed at protocol level");

    let edit_result = edit_resp.result.unwrap();
    assert_eq!(
        edit_result["isError"], false,
        "edit tool should succeed: {}",
        edit_result["content"][0]["text"]
    );
}
```

**Step 2: Run the integration test**

Run: `cargo test --manifest-path crates/Cargo.toml -p relay --lib mcp::router::tests::read_then_edit_via_session_id_argument 2>&1 | tail -20`
Expected: PASS.

**Step 3: Run the full test suite**

Run: `cargo test --manifest-path crates/Cargo.toml -p relay 2>&1 | tail -20`
Expected: All tests PASS.

**Step 4: Commit**

```
jj describe -m "test(mcp): integration test for read-then-edit via session_id argument"
jj new
```

---

### Task 6: Final verification and squash

**Step 1: Run full relay test suite one more time**

Run: `cargo test --manifest-path crates/Cargo.toml -p relay 2>&1 | tail -30`
Expected: All tests PASS.

**Step 2: Run cargo clippy**

Run: `cargo clippy --manifest-path crates/Cargo.toml -p relay 2>&1 | tail -20`
Expected: No warnings on changed code.

**Step 3: Squash into a single commit**

Squash all changes into one commit:

```
jj squash --from <first-change>::@ --into <first-change>
jj describe -m "feat(mcp): pass session_id as edit tool argument for Claude.ai compatibility

Claude.ai creates a fresh MCP session per tool call, breaking the
read-before-edit guard. Fix by returning the session ID in the read
tool's response and requiring it as an edit tool argument.

- Read tool appends [session: <id>] to response
- Edit tool requires session_id argument (from read response)
- Session TTL cleanup (1h) for orphaned sessions
- Integration test: read → extract session → edit with different transport session"
```
