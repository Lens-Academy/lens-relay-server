---
phase: 01-search-index
plan: 01
subsystem: search
tags: [tantivy, full-text-search, bm25, search-index]
dependency-graph:
  requires: []
  provides: [SearchIndex, SearchResult, tantivy-integration]
  affects: [01-02, 02-mcp-server]
tech-stack:
  added: [tantivy-0.25]
  patterns: [mutex-writer, ram-directory-testing, custom-snippet-rendering]
key-files:
  created:
    - crates/y-sweet-core/src/search_index.rs
  modified:
    - crates/y-sweet-core/Cargo.toml
    - crates/y-sweet-core/src/lib.rs
    - crates/Cargo.lock
decisions:
  - id: search-schema
    description: "Four-field schema: doc_id (STRING|STORED), title (TEXT|STORED, boosted 2x), body (TEXT|STORED), folder (STORED only)"
    rationale: "STRING for doc_id enables exact-match delete_term. TEXT+STORED on body required for snippet generation. Folder not searchable in v1."
  - id: and-semantics
    description: "QueryParser uses conjunction_by_default (AND semantics)"
    rationale: "AND semantics produce more precise results for knowledge base search -- users expect all terms to appear"
  - id: lenient-parsing
    description: "parse_query_lenient used instead of parse_query"
    rationale: "Tolerates malformed queries (e.g. trailing AND) without errors -- better UX for search boxes"
  - id: custom-snippet-mark
    description: "Custom render_snippet_with_mark uses <mark> tags instead of default <b>"
    rationale: "Semantic HTML -- <mark> represents highlighted/relevant text, <b> is for bold styling"
  - id: ram-directory-tests
    description: "Tests use RamDirectory via new_in_memory() constructor"
    rationale: "Avoids filesystem side effects in tests, faster execution, no cleanup needed"
metrics:
  duration: 6m
  completed: 2026-02-08
---

# Phase 01 Plan 01: SearchIndex Core Module Summary

BM25-ranked full-text search over relay documents using tantivy 0.25, with idempotent upsert, <mark>-tagged snippets, AND query semantics, and title boost ranking.

## What Was Built

### SearchIndex (`crates/y-sweet-core/src/search_index.rs`)

A synchronous full-text search module wrapping tantivy with:

- **Schema**: 4 fields -- doc_id (STRING for exact delete), title (TEXT, 2x boost), body (TEXT, stored for snippets), folder (STORED, not searchable)
- **Construction**: `new(path)` for MmapDirectory, `new_in_memory()` for RamDirectory (tests). Shared `build()` accepts any `Into<Box<dyn Directory>>`
- **Indexing**: `add_document()` with delete_term + add + commit pattern for idempotent upsert. IndexWriter in Mutex for thread safety
- **Removal**: `remove_document()` with delete_term + commit
- **Search**: `search(query, limit)` with BM25 ranking, TopDocs collector, SnippetGenerator with 150-char limit and custom `<mark>` tag rendering
- **Query handling**: AND conjunction by default, lenient parsing, empty/whitespace guard

### Test Coverage (15 tests)

| Test | Behavior Verified |
|------|-------------------|
| empty_index_returns_empty_results | No panic on empty index search |
| search_by_title_finds_document | Title field indexed and searchable |
| search_by_body_finds_document | Body field indexed and searchable |
| title_match_scores_higher_than_body_only_match | 2x title boost produces correct ranking |
| snippet_contains_mark_tags | Custom snippet rendering works |
| snippet_does_not_contain_bold_tags | No default `<b>` tags leak through |
| update_document_replaces_old_content | delete_term + add is idempotent |
| remove_document_makes_it_unsearchable | Document removal works |
| empty_query_returns_empty_results | Empty string guard |
| whitespace_query_returns_empty_results | Whitespace-only guard |
| search_respects_limit | TopDocs limit parameter works |
| phrase_search_works | Quoted phrase queries match correctly |
| and_semantics_by_default | Multi-term query requires all terms |
| lenient_parsing_handles_malformed_query | Malformed queries do not error |
| folder_is_stored_in_results | Folder field stored and returned |

## Decisions Made

1. **Four-field schema** with STRING doc_id for exact-match deletion, TEXT+STORED body for snippet generation, STORED-only folder (not searchable in v1)
2. **AND semantics by default** -- conjunction_by_default produces more precise results for knowledge base search
3. **Lenient parsing** -- parse_query_lenient tolerates syntax errors for better UX
4. **Custom `<mark>` tags** -- semantic HTML instead of default `<b>` for highlighted snippets
5. **RamDirectory for tests** -- avoids filesystem side effects, no cleanup needed

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] tantivy 0.25 API differences from research notes**

- **Found during:** GREEN phase
- **Issue:** `IndexReaderBuilder::try_open()` does not exist in tantivy 0.25; the builder implements `TryInto<IndexReader>` instead. Also, `Value::as_str()` requires explicit trait import.
- **Fix:** Changed `.try_open()` to `.try_into()` with explicit `IndexReader` type annotation. Added `use tantivy::schema::Value` import.
- **Files modified:** `crates/y-sweet-core/src/search_index.rs`

## Commits

| Hash | Type | Description |
|------|------|-------------|
| 380aac2907b4 | test | Add failing tests for SearchIndex (15 tests, todo! stubs) |
| ec4631ddc67f | feat | Implement SearchIndex with tantivy (all 15 tests pass) |

## Next Phase Readiness

Plan 01-02 (wiring SearchIndex into the relay server) can proceed. The SearchIndex module provides:
- `SearchIndex::new(path)` for production with MmapDirectory
- `add_document`, `remove_document`, `search` -- all synchronous, thread-safe via Mutex
- The async boundary (spawn_blocking) will be added in Plan 02
