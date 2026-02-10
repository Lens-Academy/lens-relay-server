# Phase 5: Search UI - Research

**Researched:** 2026-02-10
**Domain:** React search UI (fetch + render results list with navigation)
**Confidence:** HIGH

## Summary

This phase adds a full-text search UI to lens-editor that calls the existing search HTTP API endpoint (`GET /search?q=...&limit=N`) built in Phase 1. The frontend is React 19.2 + TailwindCSS 4 + Vite 7. The search API returns results with `doc_id` (UUID), `title`, `folder`, `snippet` (HTML with `<mark>` tags), and `score`. The UI needs a search bar, a results list with snippets, and click-to-navigate functionality.

The codebase already has established patterns for everything needed: `useDeferredValue` for input debouncing (used in Sidebar), `NavigationContext.onNavigate()` for document navigation, `RELAY_ID` prefix for compound doc IDs, `/api/relay` Vite proxy for API calls in development, and TailwindCSS utility classes for all styling. No new libraries are needed.

The approach is straightforward: add a search API client function to `relay-api.ts`, create a `useSearch` hook that debounces queries and fetches results, build a `SearchPanel` component that renders in the sidebar, and wire up keyboard shortcut (Ctrl+K / Cmd+K) to focus the search input. The existing `SearchInput` component in the sidebar currently does client-side filename filtering; the new full-text search will be a separate mode or panel that replaces the file tree with search results when active.

**Primary recommendation:** Build the search UI using existing codebase patterns -- no new dependencies required. Place the search results panel in the left sidebar, toggling between file tree and search results based on whether the user has an active search query.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| React | 19.2.0 | UI framework (already installed) | Already the app framework |
| TailwindCSS | 4.1.18 | Styling (already installed) | Already used for all styling in the app |
| Vite | 7.2.4 | Dev server with proxy (already installed) | Already proxies `/api/relay` to relay server |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| vitest | 4.0.18 | Unit/integration tests (already installed) | Testing the search hook and component |
| @testing-library/react | 16.3.2 | Component testing (already installed) | Testing SearchPanel rendering and interactions |
| happy-dom | 20.4.0 | Test DOM environment (already installed) | Vitest environment for component tests |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Custom fetch + useDeferredValue | React Query / TanStack Query | Overkill -- the app has no other data-fetching patterns, and the search is a simple GET. Adding a caching layer adds complexity for no real benefit with a single endpoint. |
| Inline search in sidebar | Modal/overlay (cmdk, kbar) | The sidebar approach matches the existing UX pattern. A modal is better for command palettes but the requirements call for a visible search bar, not a modal. |
| dangerouslySetInnerHTML for snippets | Parse and render manually | The snippets contain only `<mark>` tags from our own server -- no user-generated HTML. Sanitization via DOMPurify is unnecessary overhead. dangerouslySetInnerHTML is safe here. |

**Installation:**
```bash
# No new packages needed -- everything is already installed
```

## Architecture Patterns

### Recommended File Structure
```
lens-editor/src/
  lib/
    relay-api.ts            # ADD: searchDocuments() function
  hooks/
    useSearch.ts            # NEW: debounced search hook
    useSearch.test.ts       # NEW: unit tests
  components/
    Sidebar/
      SearchPanel.tsx       # NEW: search results list
      SearchPanel.test.tsx  # NEW: component tests
      Sidebar.tsx           # MODIFY: integrate SearchPanel
      SearchInput.tsx       # MODIFY: add onSubmit/onFocus callbacks
```

### Pattern 1: Search API Client (relay-api.ts)
**What:** A function that calls `GET /search?q=...&limit=N` via the existing `/api/relay` proxy.
**When to use:** Called by the `useSearch` hook.
**Example:**
```typescript
// In relay-api.ts -- follows the same API_BASE pattern as createDocumentOnServer
export interface SearchResult {
  doc_id: string;   // UUID (no RELAY_ID prefix)
  title: string;
  folder: string;
  snippet: string;  // HTML with <mark> tags
  score: number;
}

export interface SearchResponse {
  results: SearchResult[];
  total_hits: number;
  query: string;
}

export async function searchDocuments(
  query: string,
  limit: number = 20
): Promise<SearchResponse> {
  const params = new URLSearchParams({ q: query, limit: String(limit) });
  const response = await fetch(`${API_BASE}/search?${params}`);
  if (!response.ok) {
    throw new Error(`Search failed: ${response.status}`);
  }
  return response.json();
}
```

**Key detail:** The search endpoint does NOT require authentication (no `Authorization` header needed). The `handle_search` function in `server.rs` does not call `check_auth`. The `/api/relay` Vite proxy handles routing to the relay server.

### Pattern 2: useSearch Hook (debounced fetch)
**What:** A custom hook that takes a query string, debounces it, fetches results from the API, and returns results + loading state.
**When to use:** In the Sidebar component.

**Design choice -- setTimeout debounce vs useDeferredValue:**
The existing sidebar uses `useDeferredValue` for client-side filtering (instant, no network). For server-side search, a traditional `setTimeout` debounce (300ms) is more appropriate because:
1. We want to avoid firing HTTP requests on every keystroke
2. `useDeferredValue` defers rendering, not network requests -- it would still fire a fetch per keystroke
3. The 300ms debounce is the standard UX pattern for search-as-you-type

**Example:**
```typescript
import { useState, useEffect, useRef } from 'react';
import { searchDocuments, type SearchResult } from '../lib/relay-api';

interface UseSearchReturn {
  results: SearchResult[];
  loading: boolean;
  error: string | null;
}

export function useSearch(query: string, debounceMs = 300): UseSearchReturn {
  const [results, setResults] = useState<SearchResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    const trimmed = query.trim();
    if (!trimmed) {
      setResults([]);
      setLoading(false);
      setError(null);
      return;
    }

    setLoading(true);

    const timer = setTimeout(async () => {
      // Abort any in-flight request
      abortRef.current?.abort();
      const controller = new AbortController();
      abortRef.current = controller;

      try {
        const response = await searchDocuments(trimmed);
        if (!controller.signal.aborted) {
          setResults(response.results);
          setError(null);
        }
      } catch (err) {
        if (!controller.signal.aborted) {
          setError(err instanceof Error ? err.message : 'Search failed');
          setResults([]);
        }
      } finally {
        if (!controller.signal.aborted) {
          setLoading(false);
        }
      }
    }, debounceMs);

    return () => {
      clearTimeout(timer);
      abortRef.current?.abort();
    };
  }, [query, debounceMs]);

  return { results, loading, error };
}
```

### Pattern 3: SearchPanel Component (results list)
**What:** A component that displays search results as a clickable list with document names and text snippets.
**When to use:** Shown in the sidebar when the search input has a non-empty query.

**Key design decisions:**
- Snippets use `dangerouslySetInnerHTML` because they contain `<mark>` tags from our own server (not user-generated HTML). This is safe and the standard approach.
- Each result shows: title, folder badge, and snippet with highlights.
- Clicking a result calls `onNavigate(RELAY_ID + '-' + result.doc_id)` -- same compound ID pattern used everywhere.
- The panel replaces the file tree when search is active (same space, different content).

**Example:**
```typescript
import { RELAY_ID } from '../../App';
import type { SearchResult } from '../../lib/relay-api';

interface SearchPanelProps {
  results: SearchResult[];
  loading: boolean;
  error: string | null;
  query: string;
  onNavigate: (docId: string) => void;
}

export function SearchPanel({ results, loading, error, query, onNavigate }: SearchPanelProps) {
  if (loading) {
    return <div className="p-4 text-sm text-gray-500">Searching...</div>;
  }
  if (error) {
    return <div className="p-4 text-sm text-red-500">{error}</div>;
  }
  if (query && results.length === 0) {
    return <div className="p-4 text-sm text-gray-500">No results found</div>;
  }

  return (
    <ul className="divide-y divide-gray-100">
      {results.map((result) => (
        <li key={result.doc_id}>
          <button
            onClick={() => onNavigate(`${RELAY_ID}-${result.doc_id}`)}
            className="w-full text-left px-3 py-2 hover:bg-gray-50 transition-colors"
          >
            <div className="text-sm font-medium text-gray-900 truncate">
              {result.title}
            </div>
            {result.folder && (
              <span className="text-xs text-gray-400">{result.folder}</span>
            )}
            <div
              className="text-xs text-gray-600 mt-0.5 line-clamp-2"
              dangerouslySetInnerHTML={{ __html: result.snippet }}
            />
          </button>
        </li>
      ))}
    </ul>
  );
}
```

### Pattern 4: Sidebar Integration (toggle between tree and search)
**What:** The Sidebar conditionally renders either the FileTree or SearchPanel based on whether there is an active search query.
**When to use:** Always -- this is the integration pattern.

**Approach:** The current `SearchInput` component does client-side filename filtering. For Phase 5, there are two options:
1. **Repurpose the existing SearchInput** -- When the user types, it triggers server-side full-text search instead of (or in addition to) client-side filename filtering.
2. **Separate mode** -- Add a toggle or dedicate the search input to full-text search, replacing the file tree with results.

**Recommended approach:** Option 1 is simpler and matches user expectations. When the search input has text, show full-text search results from the API. When cleared, show the file tree. The existing `SearchInput` component already has the right UI (input with clear button).

**Current sidebar structure:**
```
aside (w-64)
  div (header: New Doc button + SearchInput)
  div (flex-1: FileTree OR SearchPanel)
```

The toggle is simple: if `searchTerm` is non-empty, render `<SearchPanel>`. Otherwise, render `<FileTree>`.

### Pattern 5: Keyboard Shortcut (Ctrl+K / Cmd+K)
**What:** Global keyboard shortcut to focus the search input.
**When to use:** Standard UX pattern for search.

**Example:**
```typescript
// In Sidebar.tsx or App.tsx
useEffect(() => {
  const handler = (e: KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
      e.preventDefault();
      searchInputRef.current?.focus();
    }
  };
  document.addEventListener('keydown', handler);
  return () => document.removeEventListener('keydown', handler);
}, []);
```

**Important:** The `SearchInput` component needs a `ref` prop (using `forwardRef`) or an `inputRef` callback to expose the underlying `<input>` element for programmatic focus. Currently it does not expose a ref.

### Pattern 6: Mark Tag Styling
**What:** Style the `<mark>` tags in search snippets for visual highlighting.
**When to use:** The search API returns snippets with `<mark>` tags wrapping matched terms.

**Example (TailwindCSS):**
```css
/* In index.css or as a Tailwind utility */
/* The browser default for <mark> is yellow background, which works well */
/* Optionally refine: */
.search-snippet mark {
  background-color: #fef08a; /* yellow-200 */
  color: inherit;
  border-radius: 2px;
  padding: 0 1px;
}
```

### Anti-Patterns to Avoid
- **Fetching on every keystroke:** Always debounce. 300ms is standard. Without debounce, typing "hello" sends 5 requests.
- **Not aborting stale requests:** If the user types "hel" then quickly "hello", the "hel" request might return after "hello" and overwrite correct results. Use AbortController.
- **Using a heavy library for simple fetch:** React Query, SWR, etc. are excellent but overkill here. The app has no other data-fetching patterns. A custom hook with `useState` + `useEffect` + `setTimeout` is simpler and sufficient.
- **Sanitizing snippets with DOMPurify:** The snippets come from our own Rust server (tantivy SnippetGenerator), not from user input. The only HTML is `<mark>` tags. Adding DOMPurify would be security theater adding a dependency for no real threat.
- **Separate search page or modal:** The requirements say "search bar visible in lens-editor" -- this means inline in the sidebar, not a separate route or overlay.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| API proxy in development | Custom proxy middleware | Vite's built-in proxy (`/api/relay`) | Already configured, handles CORS correctly |
| Input debouncing | Custom debounce utility function | `setTimeout` + `clearTimeout` in `useEffect` | 5 lines of code, no utility needed. React's effect cleanup handles cancellation natively. |
| Compound doc ID construction | Custom ID builder | `${RELAY_ID}-${result.doc_id}` | Same pattern used in Sidebar.tsx line 50, BacklinksPanel.tsx line 65, Editor.tsx line 145 |
| Search result navigation | Custom routing | `NavigationContext.onNavigate(compoundDocId)` | Already wired up -- same as clicking a file in the tree or a backlink |
| Loading/error states | Custom state machine | Simple `useState` for `loading` and `error` | The pattern is used throughout the codebase (useFolderMetadata.ts lines 30-32) |

**Key insight:** This phase requires zero new dependencies. Everything needed is already in the codebase: React, TailwindCSS, fetch API, Vite proxy, NavigationContext. The entire phase is glue code connecting an existing API to existing UI patterns.

## Common Pitfalls

### Pitfall 1: Race Condition with Stale Search Results
**What goes wrong:** User types "hello", pauses, types "world". The "hello" response arrives after "world" response and overwrites it. UI shows results for "hello" even though the input says "world".
**Why it happens:** HTTP responses can arrive out of order. The slower request completes last.
**How to avoid:** Use `AbortController` to cancel the previous request when a new one starts. The `useSearch` hook pattern above demonstrates this.
**Warning signs:** Search results don't match the current query text.

### Pitfall 2: Search Firing During Server Startup
**What goes wrong:** The search API returns 503 during initial indexing (when the relay server is building the search index at startup).
**Why it happens:** The server sets `search_ready = false` until initial indexing completes and returns 503 for any search requests.
**How to avoid:** Handle 503 gracefully in the UI -- show "Search is initializing..." instead of an error. This is an expected transient state.
**Warning signs:** Error message on first load that goes away after a few seconds.

### Pitfall 3: SearchInput Ref Not Exposed
**What goes wrong:** Ctrl+K shortcut can't focus the search input because `SearchInput` doesn't forward refs.
**Why it happens:** `SearchInput` is a plain function component without `forwardRef`. There's no way to get a reference to the underlying `<input>` element.
**How to avoid:** Either add `forwardRef` to `SearchInput`, or add an `inputRef` callback prop, or use an `id` attribute with `document.getElementById`. The `forwardRef` approach is cleanest.
**Warning signs:** Keyboard shortcut does nothing.

### Pitfall 4: XSS Concern with dangerouslySetInnerHTML
**What goes wrong:** Code reviewers flag `dangerouslySetInnerHTML` as a security risk.
**Why it happens:** The name is deliberately scary. It IS dangerous when rendering user-generated HTML.
**How to avoid:** Document clearly that snippets come from tantivy's `SnippetGenerator` which only adds `<mark>` tags around matched terms from the index. The content indexed is Y.Doc text content from the relay server -- not arbitrary user HTML. The render_snippet_with_mark function in `search_index.rs` produces only `<mark>` tags from string fragments.
**Warning signs:** None -- this is a false alarm, but document the reasoning.

### Pitfall 5: Empty Query Sends Request
**What goes wrong:** An empty or whitespace-only query triggers an API call.
**Why it happens:** The debounce timer fires even for empty strings if not guarded.
**How to avoid:** Check `query.trim()` before making the API call. The server also handles this (returns empty results for empty queries), but avoiding the request is better.
**Warning signs:** Unnecessary network requests visible in DevTools.

### Pitfall 6: Replacing File Tree Loses Scroll Position
**What goes wrong:** User scrolls to a document in the file tree, searches, then clears search. The file tree resets to top.
**Why it happens:** Unmounting and remounting the FileTree component (react-arborist) resets its internal scroll state.
**How to avoid:** Keep both FileTree and SearchPanel mounted, use CSS `display: none` / `display: block` to toggle visibility instead of conditional rendering. Or accept the tradeoff -- scroll reset on clear is minor.
**Warning signs:** File tree always starts at top after clearing search.

## Code Examples

### Example 1: Search API Response Shape (from actual server code)
```json
{
  "results": [
    {
      "doc_id": "c0000001-0000-4000-8000-000000000001",
      "title": "Welcome",
      "folder": "Lens",
      "snippet": "Welcome to <mark>Lens</mark> Relay! This is a collaborative...",
      "score": 2.45
    }
  ],
  "total_hits": 1,
  "query": "Lens"
}
```
Source: `crates/relay/src/server.rs` lines 1763-1807, `crates/y-sweet-core/src/search_index.rs` lines 14-20

### Example 2: Compound Doc ID Construction (existing codebase pattern)
```typescript
// From Sidebar.tsx line 50
const handleSelect = useCallback((docId: string) => {
  const compoundDocId = `${RELAY_ID}-${docId}`;
  onSelectDocument(compoundDocId);
}, [onSelectDocument]);

// Search results use the same pattern:
const handleSearchResultClick = (resultDocId: string) => {
  onNavigate(`${RELAY_ID}-${resultDocId}`);
};
```
Source: `lens-editor/src/components/Sidebar/Sidebar.tsx` line 49-52

### Example 3: API Call via Proxy (existing codebase pattern)
```typescript
// From relay-api.ts -- all API calls go through the same base
const API_BASE = import.meta.env?.DEV ? '/api/relay' : RELAY_URL;

// Search follows the same pattern:
const response = await fetch(`${API_BASE}/search?q=${encodeURIComponent(query)}&limit=20`);
```
Source: `lens-editor/src/lib/relay-api.ts` line 16

### Example 4: Conditional Sidebar Content (proposed pattern)
```typescript
// In Sidebar.tsx -- toggle between file tree and search results
<div className="flex-1 overflow-y-auto">
  {searchTerm.trim() ? (
    <SearchPanel
      results={searchResults}
      loading={searchLoading}
      error={searchError}
      query={searchTerm}
      onNavigate={handleSearchNavigate}
    />
  ) : (
    <>
      {/* Existing FileTree rendering */}
      {filteredTree.length > 0 && (
        <FileTreeProvider value={/* ... */}>
          <FileTree data={filteredTree} onSelect={handleSelect} openAll={!!deferredSearch} />
        </FileTreeProvider>
      )}
    </>
  )}
</div>
```

### Example 5: Test Pattern (from existing test)
```typescript
// From Sidebar.test.tsx -- mock NavigationContext, render, assert
import { render, screen } from '@testing-library/react';
import { NavigationContext } from '../../contexts/NavigationContext';

// Same pattern for SearchPanel tests:
render(
  <SearchPanel
    results={[{
      doc_id: 'test-uuid',
      title: 'Test Document',
      folder: 'Lens',
      snippet: 'Some <mark>highlighted</mark> text',
      score: 1.5,
    }]}
    loading={false}
    error={null}
    query="highlighted"
    onNavigate={vi.fn()}
  />
);

expect(screen.getByText('Test Document')).toBeInTheDocument();
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| useDeferredValue for search debounce | setTimeout for network, useDeferredValue for rendering | React 18+ | useDeferredValue defers rendering, not network requests. Use setTimeout for debouncing API calls. |
| External state management (Redux) for async data | Local state with hooks | React 16.8+ | For a single endpoint, useState + useEffect is simpler and sufficient. |
| DOMPurify for all HTML rendering | dangerouslySetInnerHTML for trusted server content | Always | When HTML comes from your own server and contains only known-safe tags, DOMPurify adds weight without benefit. |

**Deprecated/outdated:**
- None relevant to this phase.

## Open Questions

1. **Search input dual-purpose or dedicated?**
   - What we know: The existing `SearchInput` in the sidebar currently does client-side filename filtering via `filterTree()`. Phase 5 adds server-side full-text search.
   - What's unclear: Should the same input do both? Or should they be separate inputs?
   - Recommendation: **Repurpose the single input.** When the user types, switch from filename filtering to full-text search. This avoids confusion about which search to use. The filename filtering can be seen as a subset of full-text search (titles are indexed). If the user wants quick filename navigation, they can still use the file tree. If full-text search is too slow for filename-only queries, we can add a threshold (e.g., only search server-side after 2+ characters).

2. **Minimum query length?**
   - What we know: The server handles empty queries (returns empty results). Single-character queries will return many results.
   - What's unclear: Should we enforce a minimum length client-side?
   - Recommendation: Require at least 2 characters before sending a request. Show the file tree for 0-1 characters (existing filter behavior). This reduces noise and unnecessary API calls.

3. **Scroll to match in document after navigation?**
   - What we know: The requirements say "clicking a search result opens that document." They don't mention scrolling to the matched text.
   - What's unclear: Whether the user expects to land at the matching section of the document.
   - Recommendation: Out of scope for v1. Just navigate to the document. Scroll-to-match would require passing the search query to the editor and implementing CodeMirror search decoration, which is a separate feature.

## Sources

### Primary (HIGH confidence)
- Codebase: `crates/relay/src/server.rs` lines 169-177 (SearchQuery struct), 1763-1807 (handle_search endpoint)
- Codebase: `crates/y-sweet-core/src/search_index.rs` lines 14-20 (SearchResult struct), 148-197 (search method), 200-215 (snippet rendering)
- Codebase: `lens-editor/src/components/Sidebar/Sidebar.tsx` (sidebar structure, existing SearchInput usage, navigation pattern)
- Codebase: `lens-editor/src/components/Sidebar/SearchInput.tsx` (existing search input component)
- Codebase: `lens-editor/src/lib/relay-api.ts` (API_BASE pattern, authentication handling)
- Codebase: `lens-editor/src/contexts/NavigationContext.tsx` (onNavigate interface)
- Codebase: `lens-editor/src/components/BacklinksPanel/BacklinksPanel.tsx` (list rendering + navigation pattern)
- Codebase: `lens-editor/src/App.tsx` (RELAY_ID export, compound doc ID construction)
- Codebase: `lens-editor/vite.config.ts` (proxy configuration)
- [React useDeferredValue docs](https://react.dev/reference/react/useDeferredValue) -- Confirmed useDeferredValue defers rendering, not network requests

### Secondary (MEDIUM confidence)
- [React debounce patterns](https://www.developerway.com/posts/debouncing-in-react) -- Confirmed setTimeout + useEffect as standard for network debouncing
- [dangerouslySetInnerHTML best practices](https://refine.dev/blog/use-react-dangerouslysetinnerhtml/) -- Confirmed safe for trusted server-generated HTML

### Tertiary (LOW confidence)
- None -- all findings verified with primary codebase analysis

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- No new libraries needed. All patterns directly from the existing codebase.
- Architecture: HIGH -- Based on direct codebase analysis of existing components (Sidebar, BacklinksPanel, SearchInput) and the actual search API implementation.
- Pitfalls: HIGH -- Derived from standard React async patterns (race conditions, debouncing) and codebase-specific concerns (compound doc IDs, 503 during startup).

**Research date:** 2026-02-10
**Valid until:** 2026-03-10 (stable stack, no breaking changes expected)
