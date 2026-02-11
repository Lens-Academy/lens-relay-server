# Phase 4: MCP Search & Edit Tools - Research

**Researched:** 2026-02-10
**Domain:** MCP grep/edit tool implementation with CriticMarkup wrapping and read-before-edit enforcement
**Confidence:** HIGH

## Summary

This phase adds two new MCP tools (`grep` and `edit`) and a session-level read-tracking mechanism. The `grep` tool mirrors Claude Code's Grep interface, performing regex-based full-text search across Y.Doc contents. The `edit` tool mirrors Claude Code's Edit interface (`old_string`/`new_string` replacement) but transparently wraps changes in CriticMarkup (`{--old--}{++new++}`) so human collaborators can review suggestions before accepting them. A read-before-edit enforcement gate prevents AI assistants from editing documents they have not first read in the current session.

The key architectural decisions are: (1) `grep` operates directly on Y.Doc text content using the `regex` crate (already a transitive dependency), not the tantivy search index -- because Claude Code's Grep does exact regex matching on file contents (not BM25 ranked search), and the AI expects line numbers and context windows; (2) `edit` modifies Y.Doc text via `TextRef::remove_range` + `TextRef::insert` using the same pattern already proven in `link_indexer::update_wikilinks_in_doc`; (3) read-tracking is a `HashSet<String>` of doc_ids per session, added to `McpSession`. The session ID must be threaded from the router through `dispatch_tool` so the edit tool can check it.

**Primary recommendation:** Add `regex = "1"` to relay's Cargo.toml (already a transitive dep via tantivy, just making it direct). Implement grep as a pure content scan with regex matching. Implement edit as a Y.Doc text mutation that replaces `old_string` with `{--old_string--}{++new_string++}`. Add `read_docs: HashSet<String>` to `McpSession` and record doc_ids on successful `read` calls. Thread session_id through to `dispatch_tool` for edit to check read state.

## Standard Stack

### Core

| Library | Version | Purpose | Already in Cargo.toml? |
|---------|---------|---------|------------------------|
| regex | 1.11.2 | Regex pattern matching for grep tool | **No -- add to relay** (transitive dep via tantivy) |
| yrs | 0.19.1 | Y.Doc text reading (grep) and mutation (edit) | Yes |
| serde_json | 1.0.103 | Tool parameter parsing and response building | Yes |
| dashmap | 6.0.1 | Session storage, document storage | Yes |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| tracing | 0.1.37 | Structured logging for tool calls and edit operations | Already in deps |
| nanoid | 0.4.0 | Session ID generation (already used) | Already in deps |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| regex crate for grep | tantivy search index | Tantivy provides BM25-ranked keyword search, NOT regex matching with line numbers. Claude Code Grep does exact regex search. Use regex for fidelity. |
| regex crate for grep | Custom string matching | Regex patterns are complex; don't hand-roll a regex engine. |
| `{--old--}{++new++}` (deletion+insertion) | `{~~old~>new~~}` (substitution) | Context decision locked: use separate deletion+insertion. More widely supported and already parsed by lens-editor. |

**Installation (add to `crates/relay/Cargo.toml`):**
```toml
regex = "1"
```

## Architecture Patterns

### Recommended Module Structure

```
crates/relay/src/
  mcp/
    mod.rs              # No changes needed
    router.rs           # Thread session_id through to dispatch_tool
    session.rs          # Add read_docs: HashSet<String> to McpSession
    tools/
      mod.rs            # Add grep + edit to dispatch, tool_definitions
      read.rs           # Record doc_id in session's read_docs on success
      glob.rs           # No changes
      get_links.rs      # No changes
      grep.rs           # NEW: regex search across documents
      edit.rs           # NEW: CriticMarkup-wrapped edit
  server.rs             # Add pub fn search_index() accessor
```

### Pattern 1: Session-Scoped Read Tracking

**What:** Each MCP session tracks which documents have been successfully read. The edit tool checks this set before allowing modifications.

**Design:**

```rust
// In session.rs
use std::collections::HashSet;

pub struct McpSession {
    pub session_id: String,
    pub protocol_version: String,
    pub client_info: Option<Value>,
    pub initialized: bool,
    pub created_at: Instant,
    pub last_activity: Instant,
    pub read_docs: HashSet<String>,  // NEW: doc_ids that have been read
}
```

**How read tracking works:**
1. When `read` tool succeeds, the tool records the doc_id in the session's `read_docs`
2. When `edit` tool is called, it checks if the target doc_id is in `read_docs`
3. If not, it returns a tool error: "You must read this document before editing it. Call the read tool first."

**Threading session_id to tools:**

Currently `dispatch_tool` signature is:
```rust
pub fn dispatch_tool(server: &Arc<Server>, name: &str, arguments: &Value) -> Value
```

It needs to become:
```rust
pub fn dispatch_tool(
    server: &Arc<Server>,
    session_id: &str,
    name: &str,
    arguments: &Value,
) -> Value
```

The `session_id` is already available in `handle_tools_call` (passed from the router's validation step). The `read` tool uses it to record reads; the `edit` tool uses it to check reads.

### Pattern 2: Grep Tool (Regex Content Search)

**What:** Search Y.Doc text content using regex patterns, returning matching lines with context. Mirrors Claude Code's Grep tool interface.

**Why not use tantivy:** Claude Code's Grep does exact regex matching on raw file content and returns line-numbered results with context windows (before/after lines). Tantivy provides BM25-ranked keyword search with snippets -- a fundamentally different operation. The MCP tool should behave like Grep, not like a search engine. However, for users who want ranked keyword search, a separate `search` tool could use tantivy in the future.

**Design:**

```rust
// In tools/grep.rs
pub fn execute(
    server: &Arc<Server>,
    arguments: &Value,
) -> Result<String, String> {
    let pattern = arguments.get("pattern").and_then(|v| v.as_str())
        .ok_or("Missing required parameter: pattern")?;

    let path_scope = arguments.get("path").and_then(|v| v.as_str());
    let output_mode = arguments.get("output_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("files_with_matches");
    let case_insensitive = arguments.get("-i")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let context_lines = arguments.get("-C")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let after_context = arguments.get("-A")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(context_lines);
    let before_context = arguments.get("-B")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(context_lines);
    let head_limit = arguments.get("head_limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    // Build regex (case insensitive if requested)
    let re = regex::RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|e| format!("Invalid regex pattern: {}", e))?;

    // Iterate all documents (or scoped to path)
    let resolver = server.doc_resolver();
    let all_paths = resolver.all_paths();
    // ... filter by path scope, match regex against content lines
}
```

**Output format (content mode):** Matches the ripgrep output format Claude Code produces:
```
Lens/Photosynthesis.md:3:Plants convert sunlight into energy through photosynthesis.
Lens/Photosynthesis.md:7:The process of photosynthesis involves chloroplasts.
```

Format: `{path}:{line_number}:{line_content}`

With context lines, a `--` separator between groups (matching ripgrep convention).

**Output format (files_with_matches mode):** Just file paths, one per line:
```
Lens/Photosynthesis.md
Lens/Biology 101.md
```

**Output format (count mode):** File path and count:
```
Lens/Photosynthesis.md:3
Lens/Biology 101.md:1
```

### Pattern 3: Edit Tool (CriticMarkup Wrapping)

**What:** Accept `old_string`/`new_string` parameters, find `old_string` in the document, replace it with `{--old_string--}{++new_string++}` in the Y.Doc.

**Design:**

```rust
// In tools/edit.rs
pub fn execute(
    server: &Arc<Server>,
    session_id: &str,
    arguments: &Value,
) -> Result<String, String> {
    let file_path = arguments.get("file_path").and_then(|v| v.as_str())
        .ok_or("Missing required parameter: file_path")?;
    let old_string = arguments.get("old_string").and_then(|v| v.as_str())
        .ok_or("Missing required parameter: old_string")?;
    let new_string = arguments.get("new_string").and_then(|v| v.as_str())
        .ok_or("Missing required parameter: new_string")?;

    // 1. Resolve path to doc_id
    let doc_info = server.doc_resolver().resolve_path(file_path)
        .ok_or_else(|| format!("Error: Document not found: {}", file_path))?;

    // 2. Check read-before-edit
    let session = server.mcp_sessions.get_session(session_id)
        .ok_or("Session not found")?;
    if !session.read_docs.contains(&doc_info.doc_id) {
        return Err(format!(
            "You must read this document before editing it. Call the read tool with file_path: \"{}\" first.",
            file_path
        ));
    }
    drop(session); // Release DashMap guard before modifying doc

    // 3. Read content, find old_string, verify uniqueness
    // 4. Apply CriticMarkup replacement in Y.Doc
    // (detailed in Code Examples section below)
}
```

**CriticMarkup format:** The locked decision specifies `{--old--}{++new++}`:
- Deletion marker: `{--` + old_string + `--}`
- Insertion marker: `{++` + new_string + `++}`
- Combined: `{--old_string--}{++new_string++}`
- The AI sees a success message; humans see the markup in Obsidian (via CriticMarkup plugin) and lens-editor (native support)

**Y.Doc text manipulation pattern:**
```rust
// Read content
let content = {
    let doc_ref = server.docs().get(&doc_info.doc_id)
        .ok_or_else(|| format!("Error: Document data not loaded: {}", file_path))?;
    let awareness = doc_ref.awareness();
    let guard = awareness.read().unwrap();
    let txn = guard.doc.transact();
    match txn.get_text("contents") {
        Some(text) => text.get_string(&txn),
        None => return Err("Document has no content".to_string()),
    }
};

// Find old_string (byte offset)
let matches: Vec<usize> = content.match_indices(old_string)
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
        file_path, matches.len()
    ));
}

let offset = matches[0] as u32;
let old_len = old_string.len() as u32;
let replacement = format!("{{--{}--}}{{++{}++}}", old_string, new_string);

// Apply edit
{
    let doc_ref = server.docs().get(&doc_info.doc_id)
        .ok_or_else(|| format!("Error: Document data not loaded: {}", file_path))?;
    let awareness = doc_ref.awareness();
    let guard = awareness.read().unwrap();
    let mut txn = guard.doc.transact_mut();
    let text = txn.get_or_insert_text("contents");
    text.remove_range(&mut txn, offset, old_len);
    text.insert(&mut txn, offset, &replacement);
}
```

**Key insight on offsets:** yrs defaults to `OffsetKind::Bytes` (UTF-8 byte offsets), and Rust's `str::match_indices` returns byte offsets. These are compatible -- no conversion needed. This is the same pattern used by `link_indexer::update_wikilinks_in_doc`.

### Pattern 4: Tool Signature Bifurcation

**What:** Tools that need session access (`edit`, `read`) have a different signature from tools that don't (`glob`, `grep`, `get_links`). Handle this cleanly in `dispatch_tool`.

**Design:**
```rust
pub fn dispatch_tool(
    server: &Arc<Server>,
    session_id: &str,
    name: &str,
    arguments: &Value,
) -> Value {
    // All tools receive server; session-aware tools also get session_id
    match name {
        "read" => match read::execute(server, session_id, arguments) { ... },
        "glob" => match glob::execute(server, arguments) { ... },
        "get_links" => match get_links::execute(server, arguments) { ... },
        "grep" => match grep::execute(server, arguments) { ... },
        "edit" => match edit::execute(server, session_id, arguments) { ... },
        _ => tool_error(&format!("Unknown tool: {}", name)),
    }
}
```

The `read` tool's signature changes to accept `session_id` so it can record reads. The `glob`, `grep`, and `get_links` tools ignore the session_id (it is not passed to them).

### Anti-Patterns to Avoid

- **Using tantivy for the grep tool:** Tantivy does BM25 keyword search, not regex content search. Claude Code Grep does exact regex matching. These are different operations.
- **Forgetting to record reads in the read tool:** The read-before-edit gate only works if reads are tracked. Update `McpSession.read_docs` in the read tool, not in the router.
- **Holding DashMap guards across Y.Doc mutations:** Read content into owned String, drop guard, find match, re-acquire guard for mutation. The Y.Doc content could theoretically change between read and write, but in practice MCP edits are the only writer and CriticMarkup wrapping is safe even if content changed (the find will fail on the re-read).
- **Escaping CriticMarkup delimiters in content:** If `old_string` or `new_string` contains CriticMarkup syntax (e.g., `{--`, `++}`), the wrapping could create malformed markup. For v1, document this as a known limitation; in practice, knowledge base content rarely contains CriticMarkup syntax.
- **Applying edit to CriticMarkup-wrapped text:** If the document already has `{--old--}{++new++}` from a previous edit, a naive search for `old` would find it inside the CriticMarkup. The `old_string` match should be exact (matching Claude Code behavior), so this is only a problem if the AI specifically targets text that happens to be inside CriticMarkup delimiters. Low risk for v1.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Regex matching | Custom pattern matcher | `regex::Regex` / `regex::RegexBuilder` | Full regex syntax, case-insensitive flag, Unicode support, battle-tested |
| Y.Doc text mutation | Custom CRDT operations | `TextRef::remove_range` + `TextRef::insert` | Already proven in link_indexer, handles CRDT correctly |
| CriticMarkup parsing (lens-editor) | New parser | Existing `criticmarkup-parser.ts` | Already handles all 5 CriticMarkup types, metadata extraction |
| Session state concurrency | Custom locking | DashMap (already used for sessions) | Thread-safe, lock-free reads, proven in codebase |

**Key insight:** The Y.Doc text mutation pattern is already implemented and proven in `link_indexer::update_wikilinks_in_doc()`. The edit tool uses the exact same `remove_range` + `insert` pattern but with CriticMarkup wrapping instead of wikilink renaming.

## Common Pitfalls

### Pitfall 1: Byte Offset Mismatch Between Content Read and Y.Doc Mutation

**What goes wrong:** The content is read as a Rust `String` (UTF-8), but the Y.Doc `TextRef::remove_range` uses yrs internal offsets. If offset kinds don't match, the wrong text gets replaced.
**Why it happens:** yrs supports multiple offset kinds (Bytes, Utf16, Utf32). Using the wrong kind shifts the offset for multi-byte characters.
**How to avoid:** yrs defaults to `OffsetKind::Bytes` (UTF-8 byte offsets). Rust's `str::match_indices` returns UTF-8 byte offsets. These are compatible by default. Do NOT change the Doc's offset kind. The link_indexer comment confirms this: "yrs defaults to `OffsetKind::Bytes` (UTF-8 byte offsets), which matches the byte offsets... directly -- no conversion needed."
**Warning signs:** Edits on documents with non-ASCII characters (accented letters, CJK, emoji) corrupt text or edit the wrong position.

### Pitfall 2: TOCTOU Between Content Read and Y.Doc Edit

**What goes wrong:** Between reading the content (to find `old_string`) and writing the CriticMarkup replacement, another client could modify the document, shifting offsets.
**Why it happens:** The DashMap guard must be dropped between read and write to avoid holding locks. In that window, a concurrent edit could change the document.
**How to avoid:** For v1, accept this race condition. It is very unlikely (MCP edit operations take microseconds, concurrent human edits at the exact same position are rare). If the race occurs, the `remove_range` will remove wrong text, resulting in a mangled document. To fully prevent this, re-read and re-verify `old_string` within the write transaction. The implementation should read content once with a read transaction, then acquire a write transaction, re-read the text, and verify the match is still at the expected offset before applying.
**Warning signs:** Corrupted documents after concurrent edits from both MCP and human users.

**Recommended safe pattern:**
```rust
// 1. Read content in a read txn (to find offset)
// 2. Re-acquire for write txn
// 3. Re-read content in the write txn
// 4. Verify old_string still at expected offset
// 5. If match moved, re-search; if gone, error
// 6. Apply edit
```

### Pitfall 3: Session ID Not Reaching Tools

**What goes wrong:** The edit tool needs to check `read_docs` on the session, but the session_id is not passed through the dispatch chain.
**Why it happens:** Phase 3 tools don't need session access, so `dispatch_tool` doesn't accept session_id.
**How to avoid:** Add `session_id: &str` parameter to `dispatch_tool`. Pass it from `handle_tools_call` in router.rs (where it's already available from the validation step).
**Warning signs:** Compile error if you try to access session in a tool without the parameter.

### Pitfall 4: DashMap Guard Across Mutable Transaction

**What goes wrong:** Holding a DashMap `Ref<>` while creating a mutable transaction on the Y.Doc causes potential deadlocks or panics.
**Why it happens:** The DashMap guard and Y.Doc awareness lock are independent lock hierarchies. Acquiring both in the wrong order can deadlock.
**How to avoid:** Always read content into an owned String with read guards, drop all guards, then re-acquire for the write operation. The link_indexer already demonstrates this pattern -- it reads plain text in one block, then applies edits in a separate block.
**Warning signs:** Tool calls hanging indefinitely under concurrent access.

### Pitfall 5: Regex Denial of Service

**What goes wrong:** A malicious or pathological regex pattern causes exponential backtracking, freezing the server.
**Why it happens:** Certain regex patterns (e.g., `(a+)+$`) can cause catastrophic backtracking on specific inputs.
**How to avoid:** The Rust `regex` crate is designed to be safe against this -- it uses a finite automaton engine that guarantees O(n) time complexity. Unlike PCRE or Python's `re` module, Rust's `regex` does not support backtracking features (backreferences, lookahead) that cause exponential blowup. This is a non-issue.
**Warning signs:** None -- the regex crate is inherently safe.

### Pitfall 6: Recording Read State for the Wrong Document

**What goes wrong:** The `read` tool records a doc_id in `read_docs`, but the edit tool checks a different doc_id for the same path.
**Why it happens:** If the DocumentResolver returns different `doc_id` values for the same path between the read and edit calls (e.g., because the resolver was rebuilt).
**How to avoid:** Both `read` and `edit` resolve the path through the same `DocumentResolver` at call time. The `doc_id` format is deterministic (`{relay_id}-{uuid}`). As long as both tools use `resolver.resolve_path(file_path).doc_id`, they'll match.
**Warning signs:** Edit rejected even after a successful read of the same path.

## Code Examples

### CriticMarkup Wrapping in Y.Doc

```rust
// Source: Pattern from link_indexer::update_wikilinks_in_doc (crates/y-sweet-core/src/link_indexer.rs:362-396)
// Adapted for CriticMarkup wrapping

fn apply_criticmarkup_edit(
    doc_ref: &DocWithSyncKv,
    old_string: &str,
    new_string: &str,
) -> Result<String, String> {
    // 1. Read content to find offset
    let (content, text_exists) = {
        let awareness = doc_ref.awareness();
        let guard = awareness.read().unwrap();
        let txn = guard.doc.transact();
        match txn.get_text("contents") {
            Some(text) => (text.get_string(&txn), true),
            None => (String::new(), false),
        }
    };

    if !text_exists {
        return Err("Document has no content".to_string());
    }

    // 2. Find old_string and verify uniqueness
    let matches: Vec<usize> = content.match_indices(old_string)
        .map(|(idx, _)| idx)
        .collect();

    if matches.is_empty() {
        return Err("old_string not found in document. Make sure it matches exactly.".to_string());
    }
    if matches.len() > 1 {
        return Err(format!(
            "old_string is not unique in document ({} occurrences found). Include more surrounding context to make it unique.",
            matches.len()
        ));
    }

    let byte_offset = matches[0] as u32;
    let old_len = old_string.len() as u32;
    let replacement = format!("{{--{}--}}{{++{}++}}", old_string, new_string);

    // 3. Apply edit in write transaction (re-verify in same txn)
    {
        let awareness = doc_ref.awareness();
        let guard = awareness.read().unwrap();
        let mut txn = guard.doc.transact_mut();
        let text = txn.get_or_insert_text("contents");

        // Re-verify: check that the text at the expected offset still matches
        let current_content = text.get_string(&txn);
        let actual_slice = current_content.get(byte_offset as usize..(byte_offset + old_len) as usize);
        if actual_slice != Some(old_string) {
            return Err("Document changed since last read. Please re-read and try again.".to_string());
        }

        text.remove_range(&mut txn, byte_offset, old_len);
        text.insert(&mut txn, byte_offset, &replacement);
    }

    Ok(format!(
        "Applied edit to document. The change is wrapped in CriticMarkup for human review:\n{--old--}{++new++}",
    ))
}
```

### Grep Content Search

```rust
// Regex-based content search across Y.Doc documents
fn grep_documents(
    server: &Arc<Server>,
    pattern: &regex::Regex,
    path_scope: Option<&str>,
    output_mode: &str,
    before_ctx: usize,
    after_ctx: usize,
    head_limit: usize,
) -> Result<String, String> {
    let resolver = server.doc_resolver();
    let mut all_paths = resolver.all_paths();
    all_paths.sort(); // Deterministic order

    let mut output = String::new();
    let mut total_matches = 0;
    let mut files_matched = 0;

    for path in &all_paths {
        // Apply path scope filter
        if let Some(scope) = path_scope {
            let prefix = if scope.ends_with('/') {
                scope.to_string()
            } else {
                format!("{}/", scope)
            };
            if !path.starts_with(&prefix) && path != scope {
                continue;
            }
        }

        // Resolve and read document
        let doc_info = match resolver.resolve_path(path) {
            Some(info) => info,
            None => continue,
        };

        let content = match read_doc_content(server, &doc_info.doc_id) {
            Some(c) => c,
            None => continue,
        };

        let lines: Vec<&str> = content.lines().collect();
        let mut file_matches = 0;
        let mut matched_line_indices: Vec<usize> = Vec::new();

        for (i, line) in lines.iter().enumerate() {
            if pattern.is_match(line) {
                matched_line_indices.push(i);
                file_matches += 1;
            }
        }

        if file_matches == 0 {
            continue;
        }

        files_matched += 1;
        total_matches += file_matches;

        match output_mode {
            "files_with_matches" => {
                output.push_str(path);
                output.push('\n');
            }
            "count" => {
                output.push_str(&format!("{}:{}\n", path, file_matches));
            }
            "content" | _ => {
                // Output matching lines with optional context
                // Group adjacent matches to avoid duplicating context
                for &line_idx in &matched_line_indices {
                    let start = line_idx.saturating_sub(before_ctx);
                    let end = (line_idx + after_ctx + 1).min(lines.len());
                    for i in start..end {
                        let line_num = i + 1;
                        output.push_str(&format!("{}:{}:{}\n", path, line_num, lines[i]));
                    }
                }
            }
        }

        if head_limit > 0 {
            let entry_count = match output_mode {
                "files_with_matches" => files_matched,
                "count" => files_matched,
                _ => total_matches,
            };
            if entry_count >= head_limit {
                break;
            }
        }
    }

    if output.is_empty() {
        Ok("No matches found.".to_string())
    } else {
        Ok(output.trim_end().to_string())
    }
}

fn read_doc_content(server: &Arc<Server>, doc_id: &str) -> Option<String> {
    let doc_ref = server.docs().get(doc_id)?;
    let awareness = doc_ref.awareness();
    let guard = awareness.read().unwrap();
    let txn = guard.doc.transact();
    let text = txn.get_text("contents")?;
    Some(text.get_string(&txn))
}
```

### Recording Reads in Session

```rust
// In tools/read.rs -- modified execute to accept session_id
pub fn execute(
    server: &Arc<Server>,
    session_id: &str,
    arguments: &Value,
) -> Result<String, String> {
    // ... existing parameter parsing and document reading ...

    let doc_info = server.doc_resolver().resolve_path(file_path)
        .ok_or_else(|| format!("Error: Document not found: {}", file_path))?;

    // Record successful read in session
    if let Some(mut session) = server.mcp_sessions.get_session_mut(session_id) {
        session.read_docs.insert(doc_info.doc_id.clone());
    }

    // ... rest of read implementation ...
}
```

## Claude Code Tool Schemas (Verified)

### Claude Code Edit Tool Schema

```json
{
    "type": "object",
    "required": ["file_path", "old_string", "new_string"],
    "additionalProperties": false,
    "properties": {
        "file_path": {
            "type": "string",
            "description": "The absolute path to the file to modify"
        },
        "old_string": {
            "type": "string",
            "description": "The text to replace"
        },
        "new_string": {
            "type": "string",
            "description": "The text to replace it with (must be different from old_string)"
        },
        "replace_all": {
            "type": "boolean",
            "default": false,
            "description": "Replace all occurences of old_string (default false)"
        }
    }
}
```

Source: [Claude Code internal tools gist](https://gist.github.com/bgauryy/0cdb9aa337d01ae5bd0c803943aa36bd)

**Our adaptation:** Same required parameters (`file_path`, `old_string`, `new_string`). Descriptions adapted for knowledge base paths. `replace_all` is a discretionary parameter -- recommend omitting for v1 since CriticMarkup wrapping of multiple replacements is complex and the AI can call edit multiple times.

### Claude Code Grep Tool Schema

```json
{
    "type": "object",
    "required": ["pattern"],
    "additionalProperties": false,
    "properties": {
        "pattern": {
            "type": "string",
            "description": "The regular expression pattern to search for in file contents"
        },
        "path": {
            "type": "string",
            "description": "File or directory to search in (rg PATH). Defaults to current working directory."
        },
        "output_mode": {
            "type": "string",
            "enum": ["content", "files_with_matches", "count"],
            "description": "Output mode"
        },
        "-i": {"type": "boolean", "description": "Case insensitive search"},
        "-n": {"type": "boolean", "description": "Show line numbers in output"},
        "-A": {"type": "number", "description": "Number of lines to show after each match"},
        "-B": {"type": "number", "description": "Number of lines to show before each match"},
        "-C": {"type": "number", "description": "Number of lines before and after each match"},
        "multiline": {"type": "boolean", "description": "Enable multiline mode (default: false)"},
        "head_limit": {"type": "number", "description": "Limit output to first N lines/entries"},
        "glob": {"type": "string", "description": "Glob pattern to filter files"},
        "type": {"type": "string", "description": "File type to search"}
    }
}
```

Source: [Claude Code internal tools gist](https://gist.github.com/bgauryy/0cdb9aa337d01ae5bd0c803943aa36bd)

**Our adaptation:** Include: `pattern` (required), `path`, `output_mode`, `-i`, `-C`, `-A`, `-B`, `head_limit`. Omit: `glob` (all our docs are .md -- the `path` parameter provides scoping), `type` (all markdown), `-n` (always show line numbers in content mode), `multiline` (can add later if needed). This keeps the schema focused while maintaining Claude Code familiarity.

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Direct document edits from AI | CriticMarkup-wrapped suggestions | This phase | Human review step before changes take effect |
| No access control on edits | Read-before-edit enforcement | This phase | AI must understand document context before modifying |
| BM25 search only | BM25 (HTTP) + regex grep (MCP) | This phase | AI gets exact pattern matching, not just keyword ranking |

## Design Decisions (Claude's Discretion)

### Decision: Grep uses regex on raw content, NOT tantivy

**Rationale:** Claude Code's Grep tool does exact regex matching on file content. An AI assistant calling `grep` with a regex pattern expects to find literal matches with line numbers and context. Tantivy provides BM25-ranked keyword search -- a search engine, not a grep tool. Using tantivy would break the interface contract. If ranked keyword search is wanted, it can be exposed as a separate `search` tool in a future phase.

### Decision: No `replace_all` parameter in v1

**Rationale:** Claude Code's Edit tool supports `replace_all` as a boolean flag. However, CriticMarkup wrapping of multiple replacements is complex: wrapping the first occurrence shifts byte offsets for subsequent occurrences. While solvable (apply in reverse offset order, as link_indexer does), this adds complexity for a feature the AI rarely uses. The AI can call `edit` multiple times for multiple replacements. Recommend adding `replace_all` in a follow-up if needed.

### Decision: `read_docs` HashSet on McpSession

**Rationale:** A `HashSet<String>` of doc_ids on the session struct is the simplest correct approach. Alternatives considered:
- Separate DashMap keyed by session_id: adds another data structure to manage and clean up on session deletion.
- Timestamp-based tracking (record when read, expire after N minutes): over-engineering for v1.
- The HashSet is cleaned up automatically when the session is removed from the DashMap.

### Decision: CriticMarkup format is `{--old--}{++new++}` (not `{~~old~>new~~}`)

**Rationale:** This is a locked decision from CONTEXT.md. The deletion+insertion format is simpler to implement, more widely supported by CriticMarkup renderers, and already correctly parsed by the lens-editor's `criticmarkup-parser.ts` (parses as two separate ranges: one deletion, one addition). The substitution syntax would parse as a single range and might be harder for the lens-editor's accept/reject UI to handle.

### Decision: Edit success message is simple, not detailed

**Rationale:** Claude Code's Edit tool returns a success message showing the file content after editing. For our use case, showing the full document after CriticMarkup insertion would be noisy (the AI already knows what it submitted). Return a short confirmation: "Edited {file_path}: replaced N characters. The change is wrapped in CriticMarkup for human review."

## Open Questions

1. **Obsidian CriticMarkup visibility**
   - What we know: Obsidian does NOT natively support CriticMarkup rendering. It requires the community plugin [obsidian-criticmarkup](https://github.com/Fevol/obsidian-criticmarkup) by Fevol, which is in beta.
   - What's unclear: Whether users of the Lens Relay (using the Relay.md plugin) will also install the CriticMarkup plugin. Without it, CriticMarkup appears as raw text in Obsidian.
   - Recommendation: This is acceptable for v1. The raw CriticMarkup syntax (`{--old--}{++new++}`) is human-readable even without rendering. The lens-editor already has full CriticMarkup support. Document the Obsidian plugin recommendation in user-facing docs. Success criterion 4 is met by lens-editor; Obsidian support is "nice to have" via plugin.

2. **Concurrent edit safety**
   - What we know: The TOCTOU window between reading content (to find offset) and writing the replacement is tiny but real.
   - What's unclear: How often concurrent edits will occur in practice (MCP sessions vs human Obsidian editing).
   - Recommendation: Implement the re-verify pattern (re-read content within the write transaction, check offset still matches). This is cheap and eliminates the race condition entirely.

3. **Search index accessor**
   - What we know: `server.search_index` is `Option<Arc<SearchIndex>>` but has no public accessor. The HTTP search handler accesses it directly from `server_state.search_index`.
   - What's unclear: Whether we'll need it for the grep tool (we decided not to use tantivy for grep).
   - Recommendation: Add `pub fn search_index(&self) -> Option<&Arc<SearchIndex>>` to Server anyway for future use, even though the grep tool won't use it.

## Sources

### Primary (HIGH confidence)
- **Claude Code Edit tool schema:** [Internal tools gist](https://gist.github.com/bgauryy/0cdb9aa337d01ae5bd0c803943aa36bd) -- Verified Edit and Grep JSON schemas
- **Claude Code Grep tool schema:** Same source as above
- **CriticMarkup specification:** [CriticMarkup toolkit README](https://github.com/CriticMarkup/CriticMarkup-toolkit/blob/master/README.md) -- Syntax for all 5 markup types
- **CriticMarkup in lens-editor:** `lens-editor/src/lib/criticmarkup-parser.ts` -- Existing parser handles all types, metadata, substitution
- **Y.Doc text mutation pattern:** `crates/y-sweet-core/src/link_indexer.rs:362-396` -- `update_wikilinks_in_doc()` demonstrates `remove_range` + `insert` with byte offsets
- **yrs TextRef API:** [docs.rs/yrs TextRef](https://docs.rs/yrs/latest/yrs/types/text/struct.TextRef.html) -- `insert(txn, index: u32, chunk: &str)`, `remove_range(txn, index: u32, len: u32)`, `get_string(txn) -> String`
- **yrs OffsetKind:** Confirmed via codebase comment in link_indexer.rs:360 -- "yrs defaults to `OffsetKind::Bytes` (UTF-8 byte offsets)"
- **regex crate:** Already in dependency tree via tantivy; v1.11.2, guarantees O(n) time (no catastrophic backtracking)
- **Codebase: mcp/session.rs** -- Current McpSession struct, DashMap-based session storage
- **Codebase: mcp/router.rs** -- Current dispatch_request flow, session_id available but not passed to tools
- **Codebase: mcp/tools/mod.rs** -- Current dispatch_tool signature, tool_definitions, tool_success/tool_error helpers

### Secondary (MEDIUM confidence)
- **Claude Code tool search guide:** [AI Free API guide](https://www.aifreeapi.com/en/posts/claude-code-tool-search) -- Confirmed Grep output format, parameters
- **Claude Code tools reference:** [vtrivedy.com](https://www.vtrivedy.com/posts/claudecode-tools-reference) -- Confirmed Edit tool error patterns
- **Obsidian CriticMarkup plugin:** [GitHub](https://github.com/Fevol/obsidian-criticmarkup) -- Beta plugin, community-maintained, not native Obsidian

### Tertiary (LOW confidence)
- None -- all findings verified with primary sources

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- regex already in dep tree, yrs API verified via docs.rs, patterns proven in codebase
- Architecture: HIGH -- Based on direct analysis of existing MCP infrastructure (Phase 3), Y.Doc mutation patterns (link_indexer), session management
- Tool schemas: HIGH -- Verified against Claude Code internal tools gist
- CriticMarkup: HIGH -- Syntax verified against official spec AND existing lens-editor parser
- Pitfalls: HIGH -- Derived from codebase-specific patterns (byte offsets, DashMap guards, TOCTOU)

**Research date:** 2026-02-10
**Valid until:** 2026-03-10 (MCP spec stable, Claude Code tool schemas stable, CriticMarkup spec unchanged since inception)
