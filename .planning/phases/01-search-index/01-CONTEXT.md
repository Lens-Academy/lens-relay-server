# Phase 1: Search Index - Context

**Gathered:** 2026-02-08
**Status:** Ready for planning

<domain>
## Phase Boundary

Full-text keyword search embedded in the relay server. An HTTP API endpoint that indexes Y.Doc content from both Lens and Lens Edu folders, returns BM25-ranked results with text snippets, and stays current as documents are edited. No folder filtering, no semantic search, no pagination — those are v2.

</domain>

<decisions>
## Implementation Decisions

### Indexing scope
- Two tantivy fields per document: **title** (boosted, from filename) and **body** (full markdown from `doc.getText('contents')`)
- Title derived from document path in folder metadata (e.g. `Meeting Notes.md` → `Meeting Notes`)
- Folder membership stored as metadata field (`"Lens"` or `"Lens Edu"`) — not searchable in v1, present for future filtering
- Wikilinks not indexed separately (already in body text)
- No frontmatter or Y.Doc metadata field indexing

### Search API response
- Single endpoint: `GET /search?q=<query>&limit=20`
- Response includes: `doc_id`, `title`, `folder`, `snippet`, `score`, plus `total_hits` and `query` echo
- Snippets: ~150 chars of context around best match, `<mark>` tags around matched terms
- Default limit 20, max 100
- No folder filtering in v1 (flat result list across both folders)
- No pagination in v1 (limit parameter sufficient for MCP and UI consumers)

### Update behavior
- **Startup:** Block and index all existing documents before serving search. Return 503 during initial indexing.
- **Live updates:** Reindex on Y.Doc `contents` text change, debounced at ~2 seconds (mirrors link indexer pattern)
- **Deletions:** Remove from index when document removed from folder doc metadata
- **Renames:** Treat as delete + add (new title, same content)

### Query handling
- tantivy query parser with AND semantics — `meeting notes` matches docs containing both terms
- Phrase search with quotes: `"meeting notes"` matches exact phrase
- Case insensitive (standard tokenizer)
- No fuzzy matching in v1
- No exposed boolean operators — users just type words

### Claude's Discretion
- Tantivy schema details and field options
- Exact debounce timing (proposal: ~2s, adjust if needed)
- Snippet generation approach (tantivy built-in vs custom)
- Error response format for search endpoint
- Index storage path and lifecycle management

</decisions>

<specifics>
## Specific Ideas

No specific requirements — open to standard approaches. The existing link indexer already reads Y.Doc content and observes changes, so the search indexer can follow the same pattern.

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 01-search-index*
*Context gathered: 2026-02-08*
