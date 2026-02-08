# Roadmap: Lens Relay Search & MCP

## Overview

This roadmap delivers full-text keyword search and MCP-based AI assistant integration for the Lens Relay knowledge base. The work splits naturally into five phases: building the search index foundation, establishing the custom MCP transport layer, implementing read-only MCP tools, adding search and edit capabilities, and delivering the search UI in lens-editor. The search index and MCP transport are independent foundations that can be built in parallel; everything else flows from them.

## Phases

**Phase Numbering:**
- Integer phases (1, 2, 3): Planned milestone work
- Decimal phases (2.1, 2.2): Urgent insertions (marked with INSERTED)

Decimal phases appear between their surrounding integers in numeric order.

- [x] **Phase 1: Search Index** - Full-text search with tantivy embedded in relay server
- [x] **Phase 2: MCP Transport** - Custom Streamable HTTP endpoint with session management
- [ ] **Phase 3: MCP Read-Only Tools** - List, read, and link navigation tools
- [ ] **Phase 4: MCP Search & Edit Tools** - Search queries and CriticMarkup editing via MCP
- [ ] **Phase 5: Search UI** - Search bar and results in lens-editor

**Parallelization:** Phases 1 and 2 have no dependencies on each other and can be built in parallel. Phase 5 depends only on Phase 1 and can proceed in parallel with Phases 3-4.

## Phase Details

### Phase 1: Search Index
**Goal**: Users and services can search across all Lens and Lens Edu documents via an HTTP API, with results ranked by relevance and including text snippets
**Depends on**: Nothing (foundation)
**Requirements**: SRCH-01, SRCH-02, SRCH-03, SRCH-04, SRCH-05, SRCH-06
**Success Criteria** (what must be TRUE):
  1. An HTTP request to the search endpoint returns ranked results matching the query across both Lens and Lens Edu folders
  2. Search results include text snippets showing where the query matched in each document
  3. Editing a document in Obsidian or lens-editor causes the search index to reflect the change within a few seconds
  4. The relay server starts up, indexes all existing documents, and serves search queries without exceeding reasonable memory on the 4GB VPS
**Plans**: 2 plans

Plans:
- [x] 01-01-PLAN.md -- SearchIndex core module (TDD: schema, indexing, search, snippets)
- [x] 01-02-PLAN.md -- Server integration + HTTP search endpoint

### Phase 2: MCP Transport
**Goal**: AI assistants can connect to the relay server's /mcp endpoint and exchange JSON-RPC messages over Streamable HTTP with proper session tracking
**Depends on**: Nothing (can be built in parallel with Phase 1)
**Requirements**: MCP-01, MCP-02, MCP-03
**Success Criteria** (what must be TRUE):
  1. An MCP client (e.g., Claude Code, MCP Inspector) can POST to /mcp and receive a valid JSON-RPC initialize response with a server-assigned session ID
  2. Subsequent requests with the Mcp-Session-Id header are routed to the correct session state
  3. Multiple concurrent MCP clients each maintain independent sessions without cross-contamination
**Plans**: 2 plans

Plans:
- [x] 02-01-PLAN.md -- MCP protocol engine (TDD: JSON-RPC types, sessions, method dispatch)
- [x] 02-02-PLAN.md -- HTTP transport handler + server integration + live verification

### Phase 3: MCP Read-Only Tools
**Goal**: AI assistants can discover, read, and navigate links between documents in the knowledge base via MCP tools
**Depends on**: Phase 2
**Requirements**: MCP-05, MCP-06, MCP-08
**Success Criteria** (what must be TRUE):
  1. An AI assistant can call list_documents and receive all documents with name, folder, and last-modified metadata
  2. An AI assistant can call read_document with a document identifier and receive the full markdown content
  3. An AI assistant can call get_links for a document and receive both its backlinks and forward links
**Plans**: TBD

Plans:
- [ ] 03-01: TBD
- [ ] 03-02: TBD

### Phase 4: MCP Search & Edit Tools
**Goal**: AI assistants can find documents by keyword search and propose edits as reviewable CriticMarkup suggestions
**Depends on**: Phase 1 (search index), Phase 3 (tool infrastructure)
**Requirements**: MCP-04, MCP-07, MCP-09
**Success Criteria** (what must be TRUE):
  1. An AI assistant can call search_documents with a query and receive ranked results with snippets (same quality as the HTTP API)
  2. An AI assistant can call edit_document with old_string/new_string and the edit appears in the document wrapped in CriticMarkup (e.g., `{--old--}{++new++}`)
  3. Attempting to edit a document the session has not previously read is rejected with a clear error message
  4. CriticMarkup suggestions are visible to human collaborators in Obsidian and lens-editor for review
**Plans**: TBD

Plans:
- [ ] 04-01: TBD
- [ ] 04-02: TBD

### Phase 5: Search UI
**Goal**: Users of lens-editor can search across all documents and navigate to results without leaving the editor
**Depends on**: Phase 1 (search API)
**Requirements**: UI-01, UI-02, UI-03
**Success Criteria** (what must be TRUE):
  1. A search bar is visible in lens-editor where the user can type a query
  2. Search results appear as a list showing document names and text snippets with matching content
  3. Clicking a search result opens that document in the editor
**Plans**: TBD

Plans:
- [ ] 05-01: TBD
- [ ] 05-02: TBD

## Progress

**Execution Order:**
Phases 1 and 2 can proceed in parallel. Then 3 -> 4. Phase 5 can proceed after Phase 1, in parallel with 3-4.

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Search Index | 2/2 | Complete | 2026-02-08 |
| 2. MCP Transport | 2/2 | Complete | 2026-02-08 |
| 3. MCP Read-Only Tools | 0/2 | Not started | - |
| 4. MCP Search & Edit Tools | 0/2 | Not started | - |
| 5. Search UI | 0/2 | Not started | - |
