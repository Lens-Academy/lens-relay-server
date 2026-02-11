# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-08)

**Core value:** AI assistants can find and work with the right documents across the knowledge base
**Current focus:** Phase 5 - Search UI (plan 1 of 2 complete)
**Next:** Phase 5 Plan 02 - Sidebar integration

## Current Position

Phase: 5 of 5 (Search UI)
Plan: 1 of 2 in current phase
Status: In progress
Last activity: 2026-02-11 -- Completed 05-01-PLAN.md

Progress: [#########.] 90%

## Performance Metrics

**Velocity:**
- Total plans completed: 9
- Average duration: 10m
- Total execution time: 1.5 hours

**By Phase:**

| Phase | Plans | Total | Avg/Plan |
|-------|-------|-------|----------|
| 01-search-index | 2/2 | 36m | 18m |
| 02-mcp-transport | 2/2 | 12m | 6m |
| 03-mcp-read-only-tools | 2/2 | 21m | 10.5m |
| 04-mcp-search-edit-tools | 2/2 | 20m | 10m |
| 05-search-ui | 1/2 | 5m | 5m |

**Recent Trend:**
- Last 5 plans: 6m, 15m, 13m, 7m, 5m
- Trend: consistent ~5-7m for focused plans, 13-15m for complex TDD plans

*Updated after each plan completion*

## Accumulated Context

### Decisions

Decisions are logged in PROJECT.md Key Decisions table.
Recent decisions affecting current work:

- Custom MCP transport (no rmcp) -- avoids Axum upgrade, gives control over session state
- Search index uses tantivy with MmapDirectory -- memory-safe for 4GB VPS
- MCP endpoint embedded in relay server (/mcp) -- direct access to Y.Docs and search index
- Parse JSON-RPC Request vs Notification by id field presence (not serde untagged)
- Always negotiate MCP protocol version 2025-03-26
- Require initialized session only for tools/call, not ping or tools/list
- HTTP 400 for missing session ID, HTTP 404 for unknown session ID (MCP spec)
- JSON parse errors return HTTP 200 with JSON-RPC error body (protocol-level, not transport-level)
- Test DocumentResolver against bare Y.Docs to avoid DocWithSyncKv async dependency
- Dual update API: update_folder (server) + update_folder_from_doc (testable)
- derive_folder_name centralizes folder naming convention
- Lazy resolver rebuild on first tool call for in-memory mode
- Router dispatch_request takes &Arc<Server> for tool access to docs/resolver
- Forward links resolved via case-insensitive basename matching
- Grep uses regex crate directly on Y.Doc text content -- precise ripgrep-compatible output
- Test DocWithSyncKv creation via tokio block_on with None store -- avoids modifying y-sweet-core
- session_id threaded through dispatch_tool for tool-level session awareness
- CriticMarkup format: {--old--}{++new++} (deletion+insertion, not substitution syntax)
- No replace_all for v1 -- single unique match required
- TOCTOU re-verify in write transaction before applying edit

### Pending Todos

None.

### Blockers/Concerns

None.

## Session Continuity

Last session: 2026-02-11 08:21 UTC
Stopped at: Completed 05-01-PLAN.md -- ready for 05-02 (sidebar integration)
Resume file: None
