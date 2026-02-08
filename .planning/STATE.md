# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-08)

**Core value:** AI assistants can find and work with the right documents across the knowledge base
**Current focus:** Phase 2 - MCP Transport

## Current Position

Phase: 2 of 5 (MCP Transport)
Plan: 1 of 2 in current phase
Status: In progress
Last activity: 2026-02-08 -- Completed 02-01-PLAN.md (MCP protocol engine)

Progress: [###.......] 30%

## Performance Metrics

**Velocity:**
- Total plans completed: 3
- Average duration: 14m
- Total execution time: 0.7 hours

**By Phase:**

| Phase | Plans | Total | Avg/Plan |
|-------|-------|-------|----------|
| 01-search-index | 2/2 | 36m | 18m |
| 02-mcp-transport | 1/2 | 7m | 7m |

**Recent Trend:**
- Last 5 plans: 6m, 30m, 7m
- Trend: fast (protocol engine was pure TDD, no external deps)

*Updated after each plan completion*

## Accumulated Context

### Decisions

Decisions are logged in PROJECT.md Key Decisions table.
Recent decisions affecting current work:

- Custom MCP transport (no rmcp) -- avoids Axum upgrade, gives control over session state
- Search index uses tantivy with MmapDirectory -- memory-safe for 4GB VPS
- MCP endpoint embedded in relay server (/mcp) -- direct access to Y.Docs and search index
- SearchIndex schema: doc_id (STRING|STORED), title (TEXT|STORED, 2x boost), body (TEXT|STORED), folder (STORED only)
- AND query semantics by default (conjunction_by_default) for precise knowledge base search
- Lenient query parsing (parse_query_lenient) for better search box UX
- Custom <mark> tags for snippet highlighting (semantic HTML)
- Parse JSON-RPC Request vs Notification by id field presence (not serde untagged)
- Always negotiate MCP protocol version 2025-03-26
- Require initialized session only for tools/call, not ping or tools/list

### Pending Todos

None.

### Blockers/Concerns

None.

## Session Continuity

Last session: 2026-02-08 21:33 UTC
Stopped at: Completed 02-01-PLAN.md (MCP protocol engine)
Resume file: None
