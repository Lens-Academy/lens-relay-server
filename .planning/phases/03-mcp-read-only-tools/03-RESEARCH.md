# Phase 3: MCP Read-Only Tools - Research

**Researched:** 2026-02-10
**Domain:** MCP tools/call implementation for document read, glob, and link navigation
**Confidence:** HIGH

## Summary

This phase implements three MCP tools (`read`, `glob`, `get_links`) in the existing relay server's MCP transport (Phase 2). The tools operate on documents stored as Y.Docs in the server's in-memory DashMap, translating between the AI-friendly `Folder/Name.md` path format and the internal relay-id/UUID-based document identifiers. The key design constraint is that `read` and `glob` must mirror Claude Code's built-in tool schemas exactly, so AI assistants experience zero learning curve.

The implementation is straightforward: all three tools are pure reads against already-loaded Y.Docs and the existing backlinks_v0 Y.Map structure. No new data structures or background workers are needed. The Phase 2 MCP transport already has the `tools/list` and `tools/call` dispatch points stubbed out -- this phase fills them in. The only new dependency is `glob-match` (a zero-dependency, single-function glob matcher) for the `glob` tool.

The central architectural challenge is the path translation layer: converting between `Folder/Name.md` paths (e.g., `Lens/Photosynthesis.md`) and the internal `relay_id-doc_uuid` DashMap keys. This mapping already exists implicitly in the `filemeta_v0` Y.Map of each folder doc, and is already traversed by `search_find_title_and_folder()` and `link_indexer::find_all_folder_docs()`. Phase 3 needs a reusable bidirectional path resolver.

**Primary recommendation:** Add a `DocumentResolver` struct to `y-sweet-core` that builds and caches a bidirectional map between `Folder/Name.md` paths and UUIDs at startup (piggybacking on the existing `startup_reindex` flow), kept current via the same folder-doc update hook the link indexer uses. All three tools use this resolver. Add `glob-match = "0.2"` to `relay/Cargo.toml` for glob pattern matching.

## Standard Stack

### Core

| Library | Version | Purpose | Already in Cargo.toml? |
|---------|---------|---------|------------------------|
| glob-match | 0.2.1 | Glob pattern matching against document paths | **No -- add to relay** |
| serde_json | 1.0.103 | Tool parameter parsing and response building | Yes |
| dashmap | 6.0.1 | Document storage, already used everywhere | Yes |
| yrs | 0.19.1 | Y.Doc reading (filemeta_v0, contents, backlinks_v0) | Yes |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| tracing | 0.1.37 | Structured logging for tool calls | Already in deps |
| tokio | 1.29.1 | `spawn_blocking` if any tool does heavy computation | Already in deps |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| glob-match | globset (BurntSushi) | globset pulls in the regex crate; glob-match is zero-dep, ~180ns per match, supports all standard glob syntax |
| glob-match | Custom matching | Glob patterns have edge cases (**, brace expansion, character classes). Don't hand-roll. |
| In-memory path map | Scan filemeta on every request | Too slow -- filemeta scan iterates all folder docs and all entries. Cache once, update incrementally. |

**Installation (add to `crates/relay/Cargo.toml`):**
```toml
glob-match = "0.2"
```

## Architecture Patterns

### Recommended Module Structure

```
crates/relay/src/
  mcp/
    mod.rs              # Add `pub mod tools;`
    router.rs           # Update dispatch: tools/list returns tool defs, tools/call dispatches
    tools/
      mod.rs            # Tool registry, dispatch by name, tool definition JSON
      read.rs           # read tool implementation
      glob.rs           # glob tool implementation
      get_links.rs      # get_links tool implementation
  server.rs             # Wire DocumentResolver into Server

crates/y-sweet-core/src/
  doc_resolver.rs       # DocumentResolver: bidirectional path <-> UUID mapping
  lib.rs                # Add `pub mod doc_resolver;`
```

### Pattern 1: DocumentResolver (Bidirectional Path Map)

**What:** A struct that maintains a bidirectional mapping between `Folder/Name.md` paths and internal doc identifiers (relay_id, UUID). Built at startup, updated incrementally on folder doc changes.

**When to use:** Every MCP tool call that needs to translate a user-facing path to a Y.Doc.

**Design:**

```rust
use dashmap::DashMap;
use std::sync::Arc;

/// Maps between user-facing paths ("Lens/Photosynthesis.md") and internal UUIDs.
pub struct DocumentResolver {
    /// Forward map: "Lens/Photosynthesis.md" -> DocInfo { uuid, relay_id, folder_doc_id }
    path_to_doc: DashMap<String, DocInfo>,
    /// Reverse map: uuid -> "Lens/Photosynthesis.md"
    uuid_to_path: DashMap<String, String>,
}

pub struct DocInfo {
    pub uuid: String,
    pub relay_id: String,
    pub folder_doc_id: String,
    pub folder_name: String,
    /// Full internal doc_id: "{relay_id}-{uuid}"
    pub doc_id: String,
}

impl DocumentResolver {
    pub fn new() -> Self { /* ... */ }

    /// Rebuild maps from all folder docs. Called at startup.
    pub fn rebuild(&self, docs: &DashMap<String, DocWithSyncKv>) { /* ... */ }

    /// Update maps for a single folder doc. Called on folder doc changes.
    pub fn update_folder(&self, folder_doc_id: &str, docs: &DashMap<String, DocWithSyncKv>) { /* ... */ }

    /// Resolve a user-facing path to a DocInfo.
    pub fn resolve_path(&self, path: &str) -> Option<DocInfo> { /* ... */ }

    /// Get the user-facing path for a UUID.
    pub fn path_for_uuid(&self, uuid: &str) -> Option<String> { /* ... */ }

    /// Get all document paths (for glob matching).
    pub fn all_paths(&self) -> Vec<String> { /* ... */ }
}
```

**Key design decisions:**
- Uses the SAME folder name derivation logic as `search_find_title_and_folder()`: first folder doc = "Lens", second = "Lens Edu"
- Path format: `{FolderName}/{filename}.md` where filename is extracted from filemeta_v0 path (strip leading `/` and trailing `.md`, keep subdirectory structure)
- Example: filemeta path `/Notes/Ideas.md` in folder "Lens" becomes `Lens/Notes/Ideas.md`
- Example: filemeta path `/Welcome.md` in folder "Lens" becomes `Lens/Welcome.md`
- Folder names have spaces: "Lens Edu", not "Lens_Edu" (matches production folder names)

### Pattern 2: Tool Definition JSON (MCP tools/list)

**What:** Each tool provides its definition as a JSON value matching the MCP tool schema.

**Example for `tools/list` response:**

```rust
fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "read",
            "description": "Reads a document from the knowledge base. Returns content with line numbers (cat -n format). Supports partial reads via offset and limit.",
            "inputSchema": {
                "type": "object",
                "required": ["file_path"],
                "additionalProperties": false,
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to the document (e.g. 'Lens/Photosynthesis.md')"
                    },
                    "offset": {
                        "type": "number",
                        "description": "The line number to start reading from. Only provide if the document is too large to read at once"
                    },
                    "limit": {
                        "type": "number",
                        "description": "The number of lines to read. Only provide if the document is too large to read at once."
                    }
                }
            }
        }),
        json!({
            "name": "glob",
            "description": "Fast document pattern matching. Returns matching document paths sorted by modification time. Use to discover documents in the knowledge base.",
            "inputSchema": {
                "type": "object",
                "required": ["pattern"],
                "additionalProperties": false,
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "The glob pattern to match documents against (e.g. '**/*.md', 'Lens/*.md', 'Lens Edu/**')"
                    },
                    "path": {
                        "type": "string",
                        "description": "Folder to scope the search to (e.g. 'Lens', 'Lens Edu'). If not specified, searches all folders."
                    }
                }
            }
        }),
        json!({
            "name": "get_links",
            "description": "Get backlinks and forward links for a document. Returns document paths that link TO this document (backlinks) and paths this document links TO (forward links).",
            "inputSchema": {
                "type": "object",
                "required": ["file_path"],
                "additionalProperties": false,
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to the document (e.g. 'Lens/Photosynthesis.md')"
                    }
                }
            }
        }),
    ]
}
```

### Pattern 3: Tool Call Response Format (MCP CallToolResult)

**What:** All tool responses use the MCP `CallToolResult` format with `content` array and `isError` flag.

**Success response:**
```json
{
    "content": [
        {
            "type": "text",
            "text": "... tool output ..."
        }
    ],
    "isError": false
}
```

**Error response (tool-level, NOT protocol-level):**
```json
{
    "content": [
        {
            "type": "text",
            "text": "Error: Document not found: Lens/NonExistent.md"
        }
    ],
    "isError": true
}
```

**Critical distinction:** Tool execution errors go in `result.isError: true` (HTTP 200 with JSON-RPC success). Protocol errors (unknown tool, missing params) go in `error` (JSON-RPC error). The Phase 2 research already established this pattern.

### Pattern 4: Read Tool Output (cat -n Format)

**What:** The `read` tool returns document content with line numbers in `cat -n` format, identical to Claude Code's Read tool.

**Example output for a document with 3 lines:**
```
     1	# Photosynthesis
     2
     3	Plants convert sunlight into energy through photosynthesis.
```

**Format details:**
- Right-aligned line numbers with 6-character width (padded with spaces)
- Tab character separating line number from content
- 1-indexed line numbers
- Each line truncated at 2000 characters (matching Claude Code's behavior)
- Default: return first 2000 lines (but our documents are all small markdown files, typically <100 lines)

**Rust implementation:**
```rust
fn format_cat_n(content: &str, offset: usize, limit: usize) -> String {
    content
        .lines()
        .enumerate()
        .skip(offset)
        .take(limit)
        .map(|(i, line)| {
            let line_num = i + 1; // 1-indexed
            let truncated = if line.len() > 2000 { &line[..2000] } else { line };
            format!("{:>6}\t{}", line_num, truncated)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
```

### Pattern 5: Glob Tool Output (Path List)

**What:** The `glob` tool returns matching document paths, one per line, sorted by modification time (most recent first).

**Example output:**
```
Lens/Photosynthesis.md
Lens/Biology 101.md
Lens Edu/Welcome.md
```

**Modification time note:** Y.Docs do not have a built-in "last modified" timestamp. Options:
1. Use the doc's `last_activity` timestamp from the SyncKv layer (if available)
2. Return alphabetically sorted (simpler, deterministic)
3. Use the tantivy index's update time

**Recommendation:** Sort alphabetically for v1. Claude Code sorts by filesystem mtime, but our documents are in-memory Y.Docs without a native mtime. Alphabetical is deterministic and useful. The path-prefix scoping via the `path` parameter is more important than sort order.

### Pattern 6: Get Links Tool Output

**What:** Returns backlinks and forward links as structured text.

**Example output:**
```
Backlinks (documents linking to this):
- Lens/Biology 101.md
- Lens Edu/Syllabus.md

Forward links (documents this links to):
- Lens/Cell Theory.md
- Lens/Chloroplast.md
```

**Implementation approach:**
1. Resolve the input path to a UUID via DocumentResolver
2. **Backlinks:** Read `backlinks_v0` Y.Map from the folder doc for this UUID's key. Each entry is an array of source UUIDs. Resolve each UUID back to a path via DocumentResolver.
3. **Forward links:** Read the content doc's Y.Text("contents"), extract wikilinks using the existing `extract_wikilinks()` function, then resolve each link name to a UUID using the existing `resolve_links_to_uuids()` function, then resolve UUIDs to paths via DocumentResolver.

### Pattern 7: tools/call Dispatch in Router

**What:** Update the existing `handle_tools_call` stub in `router.rs` to parse `params.name` and `params.arguments`, then dispatch to the correct tool handler.

**Example:**
```rust
fn handle_tools_call(
    server: &Arc<Server>,
    session_id: Option<&str>,
    id: Value,
    params: Option<&Value>,
) -> JsonRpcResponse {
    let tool_name = params
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("");
    let arguments = params
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or(json!({}));

    let result = match tool_name {
        "read" => tools::read::execute(server, &arguments),
        "glob" => tools::glob::execute(server, &arguments),
        "get_links" => tools::get_links::execute(server, &arguments),
        _ => Err(format!("Unknown tool: {}", tool_name)),
    };

    match result {
        Ok(text) => success_response(id, json!({
            "content": [{"type": "text", "text": text}],
            "isError": false
        })),
        Err(msg) => success_response(id, json!({
            "content": [{"type": "text", "text": msg}],
            "isError": true
        })),
    }
}
```

**Note:** Tool execution errors return `isError: true` inside a *successful* JSON-RPC response (HTTP 200). Only protocol-level errors (unknown method, missing params) return JSON-RPC error objects.

### Anti-Patterns to Avoid

- **Scanning all docs on every tool call:** Build the path resolver once, update incrementally. Don't iterate the entire DashMap for each `read` or `glob` call.
- **Returning JSON from read/glob:** Claude Code's Read and Glob return plain text, not JSON. Our tools should too. The MCP `content[].text` field contains the plain text output.
- **Using protocol errors for tool failures:** A missing document is NOT a JSON-RPC error. It's a tool execution error (`isError: true` in the result). Protocol errors are for malformed requests.
- **Holding DashMap guards across await points:** DashMap refs are `!Send`. Read the Y.Doc content, drop the guard, then format the response. All tool execution is synchronous (no await needed for Y.Doc reads).

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Glob pattern matching | Custom glob parser | `glob_match::glob_match()` | Supports `**`, `*`, `?`, `[a-z]`, `{a,b}`. Zero deps. 180ns/match. Battle-tested. |
| Wikilink extraction | New parser | `link_parser::extract_wikilinks()` | Already exists in codebase, handles code blocks, aliases, anchors |
| Wikilink resolution | New resolver | `link_indexer::resolve_links_to_uuids()` | Already handles exact match, case-insensitive, basename match |
| Path-to-UUID lookup | Full scan per request | `DocumentResolver` (new, but reusable) | Amortize the scan cost across many tool calls |
| cat -n formatting | Complex formatting | Simple `format!("{:>6}\t{}", num, line)` | Exact match to standard cat -n output |

**Key insight:** The hard parts (wikilink parsing, link resolution, backlink storage) are already implemented. The MCP tools are thin wrappers that translate between the MCP tool interface and existing Y.Doc data access patterns.

## Common Pitfalls

### Pitfall 1: DashMap Guard Lifetime Across Computations

**What goes wrong:** Holding a DashMap `Ref<>` while doing expensive formatting causes other requests to block on that key.
**Why it happens:** Y.Doc content reads require holding the DashMap guard, awareness RwLock, and transact the doc.
**How to avoid:** Read all needed data (body text, filemeta entries, backlinks arrays) into owned `String`/`Vec<String>` values, drop all guards, THEN format the response.
**Warning signs:** Slow tool responses when multiple tools run concurrently.

### Pitfall 2: Folder Name Derivation Hardcoded

**What goes wrong:** The folder name logic (`folder_idx == 0` -> "Lens", else "Lens Edu") is duplicated in `search_find_title_and_folder()`, `startup_reindex()`, and now the DocumentResolver.
**Why it happens:** The relay server doesn't store folder names as metadata -- the names are derived from folder doc ordering.
**How to avoid:** Centralize folder name derivation in the DocumentResolver. Provide a method like `derive_folder_name(folder_idx: usize) -> &str` or read it from config. All callers use the resolver.
**Warning signs:** Inconsistent folder names between search results and MCP tool outputs.

### Pitfall 3: Path Format Inconsistency

**What goes wrong:** The `read` tool accepts "Lens/Photosynthesis.md" but the `glob` tool returns "Lens/Photosynthesis.md" without the leading folder name, or with different casing.
**Why it happens:** Paths constructed differently in different code paths.
**How to avoid:** All path construction goes through DocumentResolver. The canonical format is `{FolderName}/{subpath}` where subpath comes from filemeta_v0 with leading `/` stripped. Example: filemeta `/Notes/Ideas.md` in folder "Lens" becomes `Lens/Notes/Ideas.md`.
**Warning signs:** `read` fails on paths returned by `glob` or `get_links`.

### Pitfall 4: Missing Documents in Resolver

**What goes wrong:** A document exists in the DashMap but the resolver doesn't know about it, causing "not found" errors.
**Why it happens:** The resolver was built at startup but a new document was created after. Or a folder doc update added a new filemeta entry.
**How to avoid:** Hook the resolver update into the same folder-doc update notification the link indexer and search indexer use. When a folder doc changes, call `resolver.update_folder()`.
**Warning signs:** Newly created documents not found by `read` or `glob` until server restart.

### Pitfall 5: Unicode in Document Names

**What goes wrong:** Document names with non-ASCII characters (accented characters, CJK, emoji) cause path matching failures or display issues.
**Why it happens:** Glob matching, string comparison, or path construction doesn't account for Unicode.
**How to avoid:** `glob-match` handles Unicode correctly. Use `str` (UTF-8) throughout. The filemeta_v0 paths are already UTF-8 strings from Y.js clients.
**Warning signs:** Documents with non-ASCII names not found by `read` but visible in `glob`.

### Pitfall 6: Returning Protocol Errors for Tool Failures

**What goes wrong:** When a document isn't found, the implementation returns a JSON-RPC error response (HTTP error code) instead of a tool result with `isError: true`.
**Why it happens:** Natural instinct to return "error" for failures.
**How to avoid:** ONLY return JSON-RPC errors for protocol violations (unknown tool name, malformed arguments). For ALL domain-level failures (document not found, invalid path, empty results), return a successful JSON-RPC response with `isError: true` and a descriptive text message. This lets the LLM see and react to the error.
**Warning signs:** AI assistant gets confused by error responses it can't parse.

## Code Examples

### Reading Document Content from Y.Doc

```rust
// Source: Existing pattern in link_indexer.rs and server.rs
fn read_document_content(
    doc_id: &str,
    docs: &DashMap<String, DocWithSyncKv>,
) -> Option<String> {
    let doc_ref = docs.get(doc_id)?;
    let awareness = doc_ref.awareness();
    let guard = awareness.read().unwrap();
    let txn = guard.doc.transact();
    let contents = txn.get_text("contents")?;
    Some(contents.get_string(&txn))
}
```

### Glob Matching Against Document Paths

```rust
// Source: glob-match crate (https://github.com/devongovett/glob-match)
use glob_match::glob_match;

fn match_documents(
    pattern: &str,
    scoped_path: Option<&str>,
    resolver: &DocumentResolver,
) -> Vec<String> {
    let all_paths = resolver.all_paths();
    let mut matched: Vec<String> = all_paths
        .into_iter()
        .filter(|path| {
            // If scoped to a folder, only match within that folder
            if let Some(scope) = scoped_path {
                if !path.starts_with(scope) {
                    return false;
                }
            }
            glob_match(pattern, path)
        })
        .collect();
    matched.sort(); // Alphabetical for v1
    matched
}
```

### Reading Backlinks from Folder Doc

```rust
// Source: Existing pattern in link_indexer.rs (read_backlinks_array)
fn get_backlinks_for_uuid(
    uuid: &str,
    folder_doc_id: &str,
    docs: &DashMap<String, DocWithSyncKv>,
) -> Vec<String> {
    let doc_ref = docs.get(folder_doc_id)?;
    let awareness = doc_ref.awareness();
    let guard = awareness.read().unwrap();
    let txn = guard.doc.transact();
    let backlinks = txn.get_map("backlinks_v0")?;
    // Read the array of source UUIDs for this target
    link_indexer::read_backlinks_array(&backlinks, &txn, uuid)
}
```

Note: `read_backlinks_array` is currently a private function in `link_indexer.rs`. It will need to be made `pub` or a public wrapper added.

### Extracting Forward Links from Content

```rust
// Source: Existing functions in link_indexer.rs and link_parser.rs
fn get_forward_links(
    content_doc_id: &str,
    docs: &DashMap<String, DocWithSyncKv>,
) -> Vec<String> {
    // 1. Read content
    let doc_ref = docs.get(content_doc_id)?;
    let awareness = doc_ref.awareness();
    let guard = awareness.read().unwrap();
    let txn = guard.doc.transact();
    let contents = txn.get_text("contents")?;
    let markdown = contents.get_string(&txn);
    drop(txn);
    drop(guard);

    // 2. Extract wikilinks
    let link_names = link_parser::extract_wikilinks(&markdown);

    // 3. Resolve to UUIDs across all folder docs
    // (reuse existing resolve_links_to_uuids pattern)
    // ...
    link_names
}
```

### MCP Tool Result Helpers

```rust
fn tool_success(text: &str) -> Value {
    json!({
        "content": [{"type": "text", "text": text}],
        "isError": false
    })
}

fn tool_error(message: &str) -> Value {
    json!({
        "content": [{"type": "text", "text": message}],
        "isError": true
    })
}
```

## Claude Code Tool Schemas (Verified)

The user's core requirement is that `read` and `glob` schemas match Claude Code's built-in tools exactly. Here are the verified schemas:

### Claude Code Read Tool Schema

```json
{
    "type": "object",
    "required": ["file_path"],
    "additionalProperties": false,
    "properties": {
        "file_path": {
            "type": "string",
            "description": "The absolute path to the file to read"
        },
        "offset": {
            "type": "number",
            "description": "The line number to start reading from. Only provide if the file is too large to read at once"
        },
        "limit": {
            "type": "number",
            "description": "The number of lines to read. Only provide if the file is too large to read at once."
        }
    }
}
```

Source: Extracted from Claude Code internal tool implementation (gist by bgauryy, verified against system prompt)

**Our adaptation:** Same structure. `file_path` description changes to reference knowledge base paths instead of absolute filesystem paths. The `pages` parameter (added in Claude Code v2.1.30 for PDF support) is omitted since our documents are all markdown.

### Claude Code Glob Tool Schema

```json
{
    "type": "object",
    "required": ["pattern"],
    "additionalProperties": false,
    "properties": {
        "pattern": {
            "type": "string",
            "description": "The glob pattern to match files against"
        },
        "path": {
            "type": "string",
            "description": "The directory to search in. If not specified, the current working directory will be used."
        }
    }
}
```

Source: Extracted from Claude Code internal tool implementation (gist by bgauryy, verified against system prompt)

**Our adaptation:** Same structure. `path` description changes to reference knowledge base folders instead of filesystem directories. "Current working directory" becomes "all folders".

### Response Format Details

| Tool | Claude Code Format | Our Format |
|------|-------------------|------------|
| Read | `cat -n` with 6-char right-aligned line numbers, tab separator, 1-indexed | Identical |
| Glob | File paths, one per line, sorted by mtime (most recent first) | Same format, sorted alphabetically (no mtime available) |
| get_links | N/A (custom tool) | Structured text with "Backlinks" and "Forward links" sections |

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| MCP tools return JSON objects | MCP tools return `content[].text` plain text | MCP spec 2025-03-26 | Tool output is plain text in a content wrapper, not arbitrary JSON |
| Tool errors as protocol errors | Tool errors as `isError: true` in result | MCP best practice | LLM can see and react to tool execution failures |
| Custom tool schemas | Mirror familiar tool schemas | Our design decision | Zero learning curve for AI assistants |

## Open Questions

1. **Modification time for glob sort order**
   - What we know: Claude Code's Glob sorts by filesystem mtime. Our documents are Y.Docs in memory without a native mtime.
   - What's unclear: Whether the SyncKv layer exposes a last-modified timestamp. The `DocWithSyncKv` has an `awareness` but no explicit mtime field.
   - Recommendation: Sort alphabetically for v1. This is deterministic and useful. If mtime is needed later, we can add a timestamp to the DocumentResolver entries, updated on document changes.

2. **read_backlinks_array visibility**
   - What we know: The function `read_backlinks_array` in `link_indexer.rs` is private (`fn`, not `pub fn`). The `get_links` tool needs to read backlinks.
   - What's unclear: Whether to make it public or add a public wrapper.
   - Recommendation: Make it `pub fn` -- it's a pure read function with no side effects. Or add a `pub fn get_backlinks(folder_doc: &Doc, uuid: &str) -> Vec<String>` wrapper.

3. **Folder name configuration**
   - What we know: Folder names are hardcoded as "Lens" (first folder) and "Lens Edu" (second folder). This works for the current production deployment.
   - What's unclear: Whether this should be configurable for other deployments.
   - Recommendation: Keep hardcoded for now but centralize in DocumentResolver. Add configuration later if needed.

4. **How tools/call reaches the Server's DashMap**
   - What we know: The current `router.rs` functions take `&SessionManager` but NOT `&Server` or `&DashMap`. The transport handler has `State<Arc<Server>>` but currently only passes `&server.mcp_sessions` to the router.
   - What's unclear: Best way to pass the Server (or its docs/resolver) to tool handlers.
   - Recommendation: Change `handle_tools_call` to accept `&Arc<Server>` instead of just `&SessionManager`. The router dispatch for `tools/call` needs access to `server.docs`, `server.doc_resolver`, etc. The session validation can use `&server.mcp_sessions` internally.

## Sources

### Primary (HIGH confidence)
- **Claude Code Read tool schema:** Extracted from internal implementation gist (https://gist.github.com/bgauryy/0cdb9aa337d01ae5bd0c803943aa36bd) and verified against Claude Code system prompt tool definitions
- **Claude Code Glob tool schema:** Same source as above
- **MCP tools specification 2025-03-26:** https://modelcontextprotocol.io/specification/2025-03-26/server/tools -- CallToolResult format, isError handling, tool definition structure
- **glob-match crate:** https://github.com/devongovett/glob-match -- API, pattern syntax, zero-dep design
- **Codebase: link_indexer.rs** -- `read_backlinks_array()`, `extract_id_from_filemeta_entry()`, `resolve_links_to_uuids()`, `find_all_folder_docs()`, `parse_doc_id()`
- **Codebase: link_parser.rs** -- `extract_wikilinks()`
- **Codebase: server.rs** -- `search_find_title_and_folder()`, `startup_reindex()`, DashMap patterns, Y.Doc access via awareness
- **Codebase: mcp/router.rs** -- Existing `handle_tools_call` stub, `dispatch_request` pattern
- **Codebase: mcp/jsonrpc.rs** -- `success_response`, `error_response` helpers

### Secondary (MEDIUM confidence)
- **Claude Code tool search guide:** https://www.aifreeapi.com/en/posts/claude-code-tool-search -- Confirmed Read output format (cat -n), Glob sorted by mtime
- **MCP error handling guide:** https://alpic.ai/blog/better-mcp-tool-call-error-responses-ai-recover-gracefully -- Confirmed isError pattern for tool failures

### Tertiary (LOW confidence)
- None -- all findings verified with primary sources

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- glob-match verified via official docs, all other deps already in codebase
- Architecture: HIGH -- Based on direct analysis of existing codebase patterns (link_indexer, search_index, mcp transport)
- Tool schemas: HIGH -- Verified against multiple sources including Claude Code internal tool gist and system prompt
- Pitfalls: HIGH -- Derived from codebase-specific patterns (DashMap guards, folder name derivation, path format)

**Research date:** 2026-02-10
**Valid until:** 2026-03-10 (MCP spec stable, Claude Code tool schemas stable)
