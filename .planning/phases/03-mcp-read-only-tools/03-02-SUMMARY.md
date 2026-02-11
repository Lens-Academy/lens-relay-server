---
phase: 03-mcp-read-only-tools
plan: 02
subsystem: api
tags: [mcp, tools, glob-match, doc-resolver, backlinks, wikilinks]

# Dependency graph
requires:
  - phase: 03-mcp-read-only-tools
    plan: 01
    provides: "DocumentResolver with resolve_path, path_for_uuid, all_paths"
  - phase: 02-mcp-transport
    provides: "MCP transport with router dispatch, session management"
provides:
  - "Three MCP tools: read, glob, get_links"
  - "Tool registry with definitions and dispatch"
  - "Server.doc_resolver field built at startup"
  - "Server.docs() public accessor"
affects: [04-xx (search and edit tools extend this tool infrastructure)]

# Tech tracking
tech-stack:
  added: ["glob-match 0.2"]
  patterns: ["Lazy resolver rebuild on first tool call", "Tool dispatch via match on name string"]

key-files:
  created:
    - "crates/relay/src/mcp/tools/mod.rs"
    - "crates/relay/src/mcp/tools/read.rs"
    - "crates/relay/src/mcp/tools/glob.rs"
    - "crates/relay/src/mcp/tools/get_links.rs"
  modified:
    - "crates/relay/src/mcp/router.rs"
    - "crates/relay/src/mcp/transport.rs"
    - "crates/relay/src/mcp/mod.rs"
    - "crates/relay/src/server.rs"
    - "crates/relay/Cargo.toml"

key-decisions:
  - "Lazy resolver rebuild: if resolver is empty when a tool is called, trigger rebuild from docs (handles in-memory mode where startup_reindex doesn't run)"
  - "Sorted folder_doc_ids in DocumentResolver::rebuild() for deterministic folder naming since DashMap iteration order is arbitrary"
  - "Router dispatch_request takes &Arc<Server> instead of &SessionManager for tool access"
  - "Server::new_for_test() constructor for router unit tests"
  - "Forward links resolved via case-insensitive basename matching against all_paths"

patterns-established:
  - "Tool module pattern: each tool in separate file with pub fn execute(server, arguments) -> Result<String, String>"
  - "Tool result wrapping: tool_success/tool_error helpers for MCP CallToolResult format"

# Metrics
duration: ~15min
completed: 2026-02-10
---

# Phase 3 Plan 2: MCP Tool Implementations Summary

**Three MCP tools (read, glob, get_links) wired into the relay server, verified live via MCP Inspector**

## Performance

- **Duration:** ~15 min (code) + interactive testing
- **Started:** 2026-02-10T11:15:00Z
- **Completed:** 2026-02-10T13:00:00Z (including live testing)
- **Tasks:** 2 auto + 1 checkpoint (approved via MCP Inspector testing)
- **Files modified:** 9

## Accomplishments

- Tool registry: tool_definitions() returns read, glob, get_links with JSON schemas
- dispatch_tool() routes by name, wraps results in MCP CallToolResult format
- read tool: document content in cat -n format with offset/limit support
- glob tool: pattern matching against all document paths using glob-match crate
- get_links tool: backlinks from backlinks_v0 Y.Map + forward links via wikilink extraction
- Router updated to accept &Arc<Server> for tool access
- Transport passes &server to router
- Server holds doc_resolver field, built during startup_reindex
- All 47+ unit tests passing, live verification via MCP Inspector

## Task Commits

1. **Tool modules + router wiring + server integration** - `0d68b19cd016` (feat)
2. **Lazy resolver rebuild + deterministic folder ordering** - `ee98ebd1859e` (fix)

## Files Created/Modified

- `crates/relay/src/mcp/tools/mod.rs` - Tool registry, definitions, dispatch
- `crates/relay/src/mcp/tools/read.rs` - Read tool: cat -n format with offset/limit
- `crates/relay/src/mcp/tools/glob.rs` - Glob tool: pattern matching via glob-match
- `crates/relay/src/mcp/tools/get_links.rs` - Links tool: backlinks + forward wikilinks
- `crates/relay/src/mcp/router.rs` - dispatch_request takes &Arc<Server>, tools/list and tools/call wired
- `crates/relay/src/mcp/transport.rs` - Passes &server to router
- `crates/relay/src/mcp/mod.rs` - Added pub mod tools
- `crates/relay/src/server.rs` - doc_resolver field, docs() accessor, rebuild in startup_reindex
- `crates/relay/Cargo.toml` - Added glob-match 0.2

## Deviations from Plan

- **Lazy resolver rebuild:** Added fallback in dispatch_tool() to rebuild resolver if empty, handling in-memory mode where startup_reindex doesn't run
- **Sorted folder_doc_ids:** DashMap iteration order is nondeterministic, so folder_doc_ids sorted before derive_folder_name assignment for deterministic naming

## Live Testing Results (MCP Inspector)

All 3 tools verified interactively via MCP Inspector:

| Tool | Input | Result |
|------|-------|--------|
| glob | `**/*` | Found all 8 docs across both folders |
| read | `Lens/Welcome.md` | 21 lines in cat -n format |
| get_links | `Lens/Welcome.md` | Backlinks: Getting Started.md, Notes/Ideas.md; Forward: Getting Started.md |

## Known Limitations

- Forward link resolution uses basename-only matching; wikilinks with path components (e.g., `[[Notes/Ideas]]`) resolve only by the final segment ("Ideas"), which may miss nested paths. Addressable in a future iteration.

---
*Phase: 03-mcp-read-only-tools*
*Completed: 2026-02-10*
