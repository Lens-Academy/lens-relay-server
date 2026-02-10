# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-08)

**Core value:** AI assistants can find and work with the right documents across the knowledge base
**Current focus:** Phase 3 - MCP Read-Only Tools -- IN PROGRESS

## Current Position

Phase: 3 of 5 (MCP Read-Only Tools)
Plan: 1 of 2 in current phase (done)
Status: In progress
Last activity: 2026-02-10 -- Completed 03-01-PLAN.md (DocumentResolver)

Progress: [#####.....] 50%

## Performance Metrics

**Velocity:**
- Total plans completed: 5
- Average duration: 11m
- Total execution time: 0.9 hours

**By Phase:**

| Phase | Plans | Total | Avg/Plan |
|-------|-------|-------|----------|
| 01-search-index | 2/2 | 36m | 18m |
| 02-mcp-transport | 2/2 | 12m | 6m |
| 03-mcp-read-only-tools | 1/2 | 6m | 6m |

**Recent Trend:**
- Last 5 plans: 6m, 5m, 7m, 6m, 6m
- Trend: consistent ~6m for focused plans

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

### Pending Todos

None.

### Blockers/Concerns

None.

## Session Continuity

Last session: 2026-02-10 11:13 UTC
Stopped at: Completed 03-01-PLAN.md (DocumentResolver)
Resume file: None
