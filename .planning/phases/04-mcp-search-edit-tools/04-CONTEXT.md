# Phase 4 Context: MCP Search & Edit Tools

## Decisions

These are locked. Do not revisit or suggest alternatives.

### Tool design principle: Mirror Claude Code tools exactly

All MCP tools must match the interface, parameter names, and behavior of their Claude Code equivalents as closely as possible. The knowledge base is the "filesystem" — an AI assistant using these tools should feel like it's using Claude Code's native Read, Edit, Grep, and Glob tools but against relay documents instead of local files.

Specifically for Phase 4:

- **search_documents** should behave like Claude Code's **Grep** tool:
  - Parameter names and semantics should match Grep where applicable (pattern, path for scoping, output_mode, etc.)
  - Consider naming it `grep` instead of `search_documents` to match the Claude Code convention
  - Returns matching lines with context, file paths, line numbers — same output format as Grep

- **edit** should behave like Claude Code's **Edit** tool:
  - Uses `file_path`, `old_string`, `new_string` parameters (exact same names)
  - Exact string replacement semantics (old_string must be unique in the document)
  - The CriticMarkup wrapping is transparent to the AI — the tool accepts the edit as if it's a direct replacement, but the server wraps it in CriticMarkup (`{--old--}{++new++}`) so humans can review
  - Error messages should match Claude Code's Edit error patterns (e.g., "old_string not found", "old_string is not unique")

### Tools already built (Phase 3) follow this pattern

- `read` mirrors Claude Code's **Read** tool (file_path, offset, limit, cat -n format)
- `glob` mirrors Claude Code's **Glob** tool (pattern, path)
- `get_links` is knowledge-base-specific (no Claude Code equivalent)

### Read-before-edit enforcement

The server tracks which documents each MCP session has read. Edits on unread documents are rejected with a clear error. This prevents blind edits and encourages the AI to read context first.

## Claude's Discretion

These are areas where the planner/researcher can make implementation choices:

- Whether search uses the existing tantivy index or a simpler grep-like approach (or both)
- How to track "read" state per session (could be a HashSet in SessionState, or similar)
- CriticMarkup insertion strategy (Y.Doc text manipulation details)
- Whether to support `replace_all` parameter like Claude Code's Edit tool
- Test strategy and plan decomposition

## Deferred Ideas

Do NOT include these in Phase 4:

- Write tool (creating new documents) — future phase
- Delete tool — future phase
- Rename/move tool — future phase
- Multi-document edits / batch operations — future phase
