# Phase 3: MCP Read-Only Tools - Context

**Gathered:** 2026-02-10
**Status:** Ready for planning

<domain>
## Phase Boundary

AI assistants can discover, read, and navigate links between documents in the knowledge base via MCP tools. Three tools: `read`, `glob`, and `get_links`. Creating/editing documents and search are separate phases.

</domain>

<decisions>
## Implementation Decisions

### Tool naming and schemas
- Name our tools `read`, `glob`, and `get_links` — NOT `list_documents`, `read_document`, etc.
- `read` and `glob` schemas must be **identical** to Claude Code's built-in Read and Glob tools
- The AI assistant already knows these tool interfaces — zero learning curve
- `get_links` is our custom tool (no Claude Code equivalent)

### Tool: read
- Mirrors Claude Code's Read tool schema exactly
- Accepts file path (in `Folder/Name.md` format), optional offset and limit for line ranges
- Returns document content with line numbers, same as Claude Code's Read
- Supports partial reads for large documents

### Tool: glob
- Mirrors Claude Code's Glob tool schema exactly
- Accepts glob pattern, optional path to scope the search
- Returns matching document paths sorted by modification time
- Paths use `Folder/Name.md` format (e.g., `Lens/Photosynthesis.md`, `Lens Edu/Biology 101.md`)

### Tool: get_links
- Custom tool unique to our server
- Accepts a document path
- Returns both directions: backlinks (docs linking TO this doc) and forward links (docs this doc links TO)
- Names/paths only — no surrounding text snippets

### Research directive
- Deeply investigate Claude Code's actual tool implementations — not just docs but source code
- Use Exa search and web search to find Claude Code's Read and Glob tool schemas
- Our tools should feel native to an AI assistant already familiar with Claude Code

### Claude's Discretion
- Exact parameter names and types (should match Claude Code exactly, but Claude investigates what those are)
- Error message format and verbosity
- How to handle documents that don't exist or can't be read
- Response format details beyond matching Claude Code patterns

</decisions>

<specifics>
## Specific Ideas

- "I want us to name our tools read, glob, and grep" — The user explicitly wants tool names to match Claude Code conventions. (`grep` is Phase 4's search tool, but the naming convention applies across phases.)
- Tool invocations should look like `Lens_MCP:glob(...)`, `Lens_MCP:read(...)` — natural alongside Claude Code's built-in tools
- The goal is that an AI assistant using our MCP server feels like it has an extended version of its own native file tools, but for the knowledge base

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 03-mcp-read-only-tools*
*Context gathered: 2026-02-10*
