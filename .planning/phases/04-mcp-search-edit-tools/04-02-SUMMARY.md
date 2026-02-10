---
phase: 04-mcp-search-edit-tools
plan: 02
subsystem: api
tags: [mcp, criticmarkup, yrs, edit, crdt]

# Dependency graph
requires:
  - phase: 04-01
    provides: "dispatch_tool with session_id, read_docs HashSet, test helpers"
provides:
  - "edit MCP tool with CriticMarkup wrapping"
  - "Read-before-edit enforcement"
  - "5 MCP tools total (read, glob, get_links, grep, edit)"
affects: [05-integration-testing]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "CriticMarkup {--old--}{++new++} for AI edit suggestions"
    - "Read-before-edit gate via session.read_docs"
    - "TOCTOU re-verify in write transaction"

key-files:
  created:
    - "crates/relay/src/mcp/tools/edit.rs"
  modified:
    - "crates/relay/src/mcp/tools/mod.rs"
    - "crates/relay/src/mcp/router.rs"

key-decisions:
  - "CriticMarkup format: {--old--}{++new++} (deletion+insertion, not substitution syntax)"
  - "No replace_all for v1 -- single unique match required"
  - "Empty new_string produces valid CriticMarkup: {--old--}{++++}"
  - "TOCTOU re-verify in write transaction before applying edit"

patterns-established:
  - "Read-before-edit: tools that modify documents check session.read_docs first"
  - "Uniqueness enforcement: ambiguous matches rejected with occurrence count"

# Metrics
duration: 7min
completed: 2026-02-10
---

# Phase 4 Plan 2: Edit Tool with CriticMarkup Summary

**CriticMarkup-wrapped edit tool with read-before-edit enforcement, uniqueness checking, and TOCTOU protection**

## Performance

- **Duration:** 7 min
- **Started:** 2026-02-10T15:07:52Z
- **Completed:** 2026-02-10T15:14:58Z
- **Tasks:** 1 (TDD: RED + GREEN)
- **Files modified:** 3

## Accomplishments
- Edit tool wraps AI changes in CriticMarkup `{--old--}{++new++}` for human review
- Read-before-edit gate prevents edits on documents the AI hasn't read
- Error handling: not found, not unique (with count), missing params, unread doc
- 80 total tests pass (72 lib + 5 main + 3 integration), 10 new edit tests

## Task Commits

Each task was committed atomically:

1. **Task 1 RED: Failing edit tests** - `pvsqmykzlyso` (test)
2. **Task 1 GREEN: Edit implementation** - `wuoxormpqrul` (feat)

_TDD task: RED wrote 10 failing tests, GREEN implemented to pass all._

## Files Created/Modified
- `crates/relay/src/mcp/tools/edit.rs` - Edit tool with CriticMarkup wrapping, read-before-edit, uniqueness check, TOCTOU re-verify
- `crates/relay/src/mcp/tools/mod.rs` - Added edit module, tool definition (schema), dispatch case
- `crates/relay/src/mcp/router.rs` - Updated tools_list test to expect 5 tools

## Decisions Made
- CriticMarkup format `{--old--}{++new++}` uses deletion+insertion syntax (not `{~~old~>new~~}` substitution) -- cleaner for reviewing proposed changes
- No `replace_all` parameter for v1 -- single unique match required, encouraging precise edits
- Empty `new_string` is valid (deletion) and produces `{--old--}{++++}`
- TOCTOU protection: re-read content in write transaction to verify old_string hasn't moved

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- All 5 MCP tools complete: read, glob, get_links, grep, edit
- Phase 4 complete -- ready for Phase 5 (integration testing)
- 80 tests cover all MCP tool functionality

---
*Phase: 04-mcp-search-edit-tools*
*Completed: 2026-02-10*
