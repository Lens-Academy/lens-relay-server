# Session ID as Tool Argument

## Problem

Claude.ai (and ChatGPT) create a fresh MCP session for every tool call. The `read` tool records the document in the session's `read_docs` set, but by the time `edit` runs, it's a new session with an empty `read_docs`. The edit guard always rejects with "must read first."

Confirmed by: Playwright MCP issue #1045, LiteLLM issue #20242, OpenAI community reports.

## Solution

Move session tracking from the transport layer (mcp-session-id header) to tool arguments. The `read` tool returns the session ID in its response text. The `edit` tool requires a `session_id` parameter. The LLM's context window becomes the session persistence layer.

Validated by: OpenAI Codex issue #2434 / PR #2449 (same pattern), MCP spec discussion #102 (Vercel endorsement).

## Changes

### 1. Read tool response (`tools/read.rs`)

Append the session ID to the response after the content:

```
     1  line one
     2  line two
     3  line three

[session: abc123def456...]
```

The `[session: ...]` line appears after a blank line separator so it's visually distinct from document content.

### 2. Edit tool input schema (`tools/mod.rs`)

Add `session_id` as a required parameter:

```json
{
  "name": "edit",
  "description": "Edit a document by replacing old_string with new_string. The change is wrapped in CriticMarkup ({--old--}{++new++}) for human review. You must read the document first.",
  "inputSchema": {
    "required": ["file_path", "old_string", "new_string", "session_id"],
    "properties": {
      "session_id": {
        "type": "string",
        "description": "The session value from the read tool's response. Required to verify the document was read before editing."
      }
    }
  }
}
```

### 3. Edit guard logic (`tools/edit.rs`)

Replace the current guard (which checks the transport session) with a lookup of the `session_id` argument:

```
Current: server.mcp_sessions.get_session(session_id)  // transport session
New:     server.mcp_sessions.get_session(arg_session_id)  // from tool argument
```

The edit function signature changes — it no longer needs the transport `session_id` parameter. Instead it reads `session_id` from `arguments`.

### 4. Read tool description update (`tools/mod.rs`)

Update the read tool's description to mention the session value:

```
"Reads a document from the knowledge base. Returns content with line numbers (cat -n format).
The response includes a [session: ...] value — pass this to the edit tool's session_id parameter when editing."
```

### 5. Dispatch changes (`tools/mod.rs`)

The `edit` dispatch no longer passes the transport `session_id`:

```rust
// Current:
"edit" => edit::execute(server, session_id, arguments)

// New:
"edit" => edit::execute(server, arguments)
```

The `read` dispatch still receives `session_id` (to record the read in the session).

### 6. Session TTL cleanup

Sessions accumulate because Claude.ai never sends `DELETE /mcp`. Add a TTL sweep:

- `SessionManager` gets a `cleanup_stale(&self, max_age: Duration)` method that removes sessions older than `max_age`
- Called lazily at the start of `create_session()` (no background task needed)
- TTL: 1 hour (hardcoded constant)

## Test changes

- `edit_read_before_edit_enforced`: Pass a session ID from a different session (no reads) as the argument
- `edit_basic_replacement` and all other edit tests: Include `session_id` in the arguments JSON
- New test: `edit_with_valid_session_id_from_read` — call read, extract session from response, pass to edit
- New test: `edit_with_invalid_session_id` — bogus session ID returns error
- New test: `session_cleanup_removes_stale` — verify TTL sweep

## What doesn't change

- Transport-level `mcp-session-id` header still used for `initialize`, `notifications/initialized`, request routing, and `DELETE /mcp`
- CriticMarkup wrapping
- `old_string` exact-match uniqueness check
- TOCTOU re-verify
- `glob`, `grep`, `get_links` tools (no session tracking needed)
