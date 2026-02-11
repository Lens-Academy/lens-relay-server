# Phase 5 Plan 01: Search API Client, useSearch Hook, SearchPanel Component

Search API client with debounced hook and results panel -- zero new dependencies, all built on existing codebase patterns.

## What Was Done

### Task 1: searchDocuments API function + useSearch hook (TDD)

**RED phase** (kzoywmyyooor):
- Added `SearchResult`, `SearchResponse` types and `searchDocuments()` function to `relay-api.ts`
- Created 9 tests for useSearch hook covering: empty query, short query (<2 chars), debounce timing, loading state, error state, query change clearing, abort on query change, and 503 graceful handling

**GREEN phase** (rxlzwrtoqztw):
- Implemented `useSearch` hook with 300ms debounce, AbortController for stale request cancellation
- Minimum 2-character query length, 503 "initializing" message
- All 9 tests pass

### Task 2: SearchPanel component with tests (TDD)

**RED phase** (yymmwpokouon):
- Created 9 tests for SearchPanel covering: title rendering, folder labels, HTML snippet with `<mark>` tags, click navigation with compound doc ID, loading/error/empty/no-query states, empty folder string handling

**GREEN phase** (znpmmwrqulsz):
- Implemented SearchPanel component with `dangerouslySetInnerHTML` for snippets
- TailwindCSS arbitrary selector `[&_mark]:bg-yellow-200` for highlight styling
- Compound doc ID construction (`RELAY_ID-doc_uuid`) matching existing codebase pattern

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| setTimeout debounce (not useDeferredValue) | useDeferredValue defers rendering, not network requests -- setTimeout is correct for server-side search |
| dangerouslySetInnerHTML for snippets | HTML comes from tantivy SnippetGenerator (only `<mark>` tags), not user input -- safe and standard |
| 2-character minimum query length | Avoids noisy single-character results and unnecessary API calls |
| AbortController for stale requests | Prevents race condition where slow request overwrites faster later request |

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed test ambiguity for "Lens" text**
- **Found during:** Task 2 GREEN phase
- **Issue:** Test "renders folder labels" used `screen.getByText('Lens')` which matched both the folder `<span>` and the snippet `<mark>Lens</mark>`, causing a TestingLibraryElementError
- **Fix:** Changed to `document.querySelectorAll('span.text-gray-400')` for precise folder label targeting
- **Files modified:** `lens-editor/src/components/Sidebar/SearchPanel.test.tsx`
- **Commit:** znpmmwrqulsz

## Test Results

18 new tests, all passing:
- `useSearch.test.ts`: 9 tests (debounce, abort, loading, error, empty query, short query, query clearing, 503 handling)
- `SearchPanel.test.tsx`: 9 tests (titles, folders, snippets, navigation, loading, error, empty, no-query, empty folder)

Full suite: 374 passed, 5 failed (pre-existing integration test failures requiring running relay server -- not related to this plan)

## Commits

| Change ID | Type | Description |
|-----------|------|-------------|
| kzoywmyyooor | test | Add failing tests for useSearch hook |
| rxlzwrtoqztw | feat | Implement useSearch hook with debounce and abort |
| yymmwpokouon | test | Add failing tests for SearchPanel |
| znpmmwrqulsz | feat | Implement SearchPanel component |

## Key Files

### Created
- `lens-editor/src/hooks/useSearch.ts` -- Debounced search hook with abort
- `lens-editor/src/hooks/useSearch.test.ts` -- 9 unit tests
- `lens-editor/src/components/Sidebar/SearchPanel.tsx` -- Search results panel
- `lens-editor/src/components/Sidebar/SearchPanel.test.tsx` -- 9 unit tests

### Modified
- `lens-editor/src/lib/relay-api.ts` -- Added searchDocuments(), SearchResult, SearchResponse
- `lens-editor/src/hooks/index.ts` -- Added useSearch export

## Duration

~5 minutes (2026-02-11T08:15:36Z to 2026-02-11T08:20:52Z)

## Next Plan Readiness

Plan 05-02 (Sidebar integration) can proceed. All building blocks are ready:
- `searchDocuments()` exported from `relay-api.ts`
- `useSearch` hook exported from `hooks/index.ts`
- `SearchPanel` component ready at `components/Sidebar/SearchPanel.tsx`
