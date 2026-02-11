---
phase: 01-search-index
plan: 02
subsystem: search
tags: [server-integration, http-endpoint, search-worker, startup-indexing]
dependency-graph:
  requires: [01-01]
  provides: [GET-/search, startup-indexing, live-search-updates]
  affects: [04-mcp-search-tools, 05-search-ui]
tech-stack:
  added: []
  patterns: [mpsc-channel-worker, debounced-reindex, spawn-blocking, filemeta-cache-diffing]
key-files:
  created: []
  modified:
    - crates/relay/src/server.rs
    - crates/y-sweet-core/src/link_indexer.rs
decisions:
  - id: search-worker-pattern
    description: "Search worker follows LinkIndexer pattern: mpsc channel, DashMap pending, debounced processing"
    rationale: "Consistency with existing codebase patterns, proven debounce approach"
  - id: temp-dir-index
    description: "SearchIndex stored at /tmp/lens-relay-search-index, cleaned on startup"
    rationale: "Memory-mapped index survives process lifetime but fresh on restart (matches in-memory relay semantics)"
  - id: folder-name-iteration-order
    description: "Folder names derived from iteration order of find_all_folder_docs (first = 'Lens', second = 'Lens Edu')"
    rationale: "Simple approach for v1 with known two-folder setup"
  - id: search-ready-gate
    description: "AtomicBool search_ready gates /search endpoint with 503 during initial indexing"
    rationale: "Prevents partial results during startup reindex"
  - id: pub-link-indexer-utils
    description: "Made find_all_folder_docs, is_folder_doc, extract_id_from_filemeta_entry pub"
    rationale: "General-purpose utilities needed by search worker, no reason to keep private"
metrics:
  duration: ~30m
  completed: 2026-02-08
---

# Phase 01 Plan 02: Server Integration + HTTP Search Endpoint Summary

Wired SearchIndex into relay server with startup indexing, debounced live updates, and GET /search HTTP endpoint. Verified with real data (6 documents across 2 folders).

## What Was Built

### Server Integration (`crates/relay/src/server.rs`)

**Server struct additions:**
- `search_index: Option<Arc<SearchIndex>>` — shared search index
- `search_ready: Arc<AtomicBool>` — gates /search with 503 during initial indexing
- `search_tx: mpsc::Sender<String>` — channel to search worker

**Server::new():**
- Creates SearchIndex at `/tmp/lens-relay-search-index` (cleaned on startup)
- Spawns `search_worker` background task following LinkIndexer pattern
- Logs "SearchIndex created" and "Search index worker started"

**startup_reindex():**
- After existing backlink reindex, scans all folder docs' filemeta_v0
- Builds uuid-to-(title, folder) mapping
- Reads Y.Text("contents") for each content doc
- Calls search_index.add_document() for each
- Sets search_ready = true after completion

**Event callback:**
- Sends doc_id to search worker channel on document update (guarded by should_index())

**search_worker():**
- Debounced background worker (2s for content docs, immediate for folder docs)
- Maintains filemeta cache for detecting adds/removes/renames
- Content docs: reads body + looks up title from folder metadata
- Folder docs: diffs filemeta to detect changes, adds/removes from index

### HTTP Endpoint

- `GET /search?q=<query>&limit=20` — BM25-ranked JSON results
- Returns 503 while initial indexing in progress
- Empty query returns `{"results": [], "total_hits": 0}`
- Limit capped at 100
- Each result: `{ doc_id, title, folder, snippet, score }`

### link_indexer.rs changes

Made 3 functions `pub`: `find_all_folder_docs`, `is_folder_doc`, `extract_id_from_filemeta_entry`

## Live Verification Results

Tested with relay on port 8290 (memory-only) + setup script (6 documents, 2 folders):

| Query | Results | Top Hit | Score | Cross-folder |
|-------|---------|---------|-------|-------------|
| "welcome" | 3 | Welcome (Lens) | 3.98 | No |
| "getting started" | 4 | Getting Started (Lens) | 5.96 | Yes (Links in Lens Edu) |
| "syllabus" | 3 | Syllabus (Lens Edu) | 4.22 | No |
| "" (empty) | 0 | - | - | - |
| "nonexistent xyzzy" | 0 | - | - | - |

- Title boost ranking confirmed (title matches score ~4x higher)
- `<mark>` tags present in all snippets, no `<b>` leaking
- AND semantics working (multi-term queries require all terms)
- Folder attribution correct ("Lens" vs "Lens Edu")

## Deviations from Plan

None. All tasks completed as specified.

## Commits

| Hash | Type | Description |
|------|------|-------------|
| 9a3256b7a223 | feat | Integrate SearchIndex into Server with startup indexing and live updates |
| 0b82a180f8e0 | feat | Add GET /search HTTP endpoint with BM25-ranked results |

## Phase 1 Complete

Both plans delivered. The relay server now has:
- **Plan 01**: SearchIndex core module (tantivy, BM25, snippets, 15 unit tests)
- **Plan 02**: Server integration (startup indexing, live updates, GET /search endpoint)

Phase 2 (MCP Transport) can proceed independently.
