# Project Research Summary

**Project:** MCP Server + Keyword Search for CRDT Document Collaboration
**Domain:** AI Assistant Integration for Collaborative Knowledge Base
**Researched:** 2026-02-08
**Confidence:** MEDIUM-HIGH

## Executive Summary

This project adds AI assistant integration to the lens-relay CRDT collaboration system via the Model Context Protocol (MCP). Expert implementations combine MCP servers with full-text search indexes to enable AI assistants to navigate and work with knowledge bases. The research reveals a critical architectural tension: whether to build the MCP server in Rust (embedded in the relay process) or Python (as a separate service).

The recommended approach has two parts: (1) embed the search index in the relay server using Tantivy with MmapDirectory to minimize memory overhead on the 4GB VPS, and (2) build a separate Python MCP server that connects to the relay via HTTP API. This separates concerns cleanly—the Rust relay handles document storage and indexing (what it's already good at), while the Python MCP server handles LLM integration (where Python's FastMCP SDK is most mature). The MCP server uses Streamable HTTP transport to support multiple concurrent AI clients.

Key risks are memory exhaustion (mitigated via Tantivy MmapDirectory + process separation), stale search results from CRDT race conditions (mitigated by copying the existing LinkIndexer debounce pattern), and poor MCP tool design creating context window overflow (mitigated by designing 5-10 outcome-oriented tools, not 15+ operation-oriented ones). The constraint of running on a 4GB VPS alongside the existing relay-git-sync service dictates an in-process search index but a separate MCP server process.

## Key Findings

### Recommended Stack

**CRITICAL TENSION:** The Stack and Architecture researchers disagree on MCP server implementation language.

**Stack researcher recommends:**
- Rust MCP server using rmcp SDK (0.14.0)
- Embedded in relay server Axum instance via `StreamableHttpService`
- In-process access to Y.Docs via shared `DashMap`
- Single process on 4GB VPS (minimal memory overhead)

**Architecture researcher recommends:**
- Python MCP server using FastMCP (python-sdk)
- Separate process connecting to relay via HTTP API
- Reuses existing Python SDK (`relay_sdk`)
- Better separation of concerns, independent deployment

**Pitfalls researcher notes:**
- Streamable HTTP is the only viable transport (not stdio) for multi-client support
- rmcp may require Rust Edition 2024 (nightly compiler)
- Python FastMCP is more mature and simpler for tool registration
- Memory pressure on 4GB VPS is real (existing WebSocket FD leak compounds this)

**Resolution for roadmap:** Present both options to user during Phase 1 planning. The Python approach is lower risk (mature SDK, existing relay_sdk integration, process isolation) but adds ~100MB memory overhead. The Rust approach is more efficient but requires Axum 0.7→0.8 upgrade and less mature SDK. Both are viable.

**Core technologies (consensus across researchers):**

- **Tantivy 0.25**: Full-text search engine (embedded library, not service). Memory-mapped index means near-zero resident RAM. BM25 scoring, snippet generation. Perfect for 4GB VPS. HIGH confidence.
- **Streamable HTTP transport**: MCP transport for multi-client support. Required for remote VPS deployment. SSE is deprecated. HIGH confidence.
- **FastMCP (Python) OR rmcp (Rust)**: Official MCP SDKs. FastMCP is more mature; rmcp integrates natively with Axum. MEDIUM-HIGH confidence.
- **Axum 0.8**: HTTP framework. rmcp requires 0.8; relay currently uses 0.7.4. Upgrade required if using Rust MCP approach. HIGH confidence.
- **Existing relay_sdk (Python)**: HTTP client for relay API. Already implements doc reading, writing, auth flow. Reusable for MCP server. HIGH confidence.

**Supporting decisions:**
- Search index: Tantivy with MmapDirectory (embedded in relay server)
- Metadata store: Defer rusqlite until needed (filemeta_v0 + backlinks_v0 already provide UUID→path mappings)
- JSON Schema: schemars 1.0 for MCP tool parameters (rmcp requirement)

### Expected Features

**Must have (table stakes):**
- `list_documents` — AI needs to discover what exists
- `read_document` — AI needs to read markdown content
- `search_documents` — AI needs keyword search to find relevant documents
- `get_backlinks` — Expose existing backlinks_v0 data (already indexed)
- `get_forward_links` — Complement to backlinks for bidirectional graph navigation
- Search API endpoint — HTTP endpoint consumed by both MCP and lens-editor
- Search UI in lens-editor — Users expect search in the web editor

**Should have (competitive differentiators):**
- `edit_document` with CriticMarkup — AI suggests edits as reviewable markup, not destructive writes. Unique to this system. No other MCP server offers this.
- `traverse_links` (N-degree graph) — BFS traversal to discover related documents multiple hops away (like GraphThulhu)
- `get_document_context` (bundle) — Returns document + backlinks + forward links in one call (reduces LLM round-trips)
- Cross-folder link awareness — Surface connections between Lens and Lens Edu folders
- MCP Prompts — Pre-built workflows like "find related documents" and "summarize with context"

**Defer (v2+):**
- Semantic/vector search — Adds embedding infrastructure, GPU/API costs. Keyword search covers 80% of use cases.
- MCP Resources (vs Tools) — Resources are for application-managed data with subscriptions. MCP client support is nascent.
- Document creation via MCP — Requires writing to filemeta_v0 + docs maps (tricky Obsidian compatibility). Defer until AuthZ exists.
- Real-time subscriptions — MCP async Tasks are new; client support is limited. Polling via tools is sufficient.

**Anti-features (explicitly avoid):**
- Direct document writes without CriticMarkup — No AuthZ system yet; unreviewed AI edits to shared knowledge base are dangerous
- Bulk operations (edit all matching docs) — Too dangerous without AuthZ and rate limiting
- Admin tools via MCP — System administration should require human action through dedicated interface

### Architecture Approach

The recommended architecture separates concerns by component responsibility: the relay server owns document storage, sync, and search indexing; the MCP server owns LLM protocol and tool definitions; they communicate via HTTP. Search indexing is embedded in the relay (same process) because the relay already holds all Y.Docs in memory and the LinkIndexer demonstrates the pattern. The MCP server is a separate process (likely Python) because it's a thin stateless client that just translates tool calls to HTTP requests.

**Major components:**

1. **relay-server (Rust, extended)** — Owns document storage (existing), CRDT sync (existing), backlinks indexing (existing), plus NEW: search index (Tantivy embedded), NEW: HTTP API endpoints for search/list/content
2. **Search index (Tantivy, embedded)** — Watches document updates via `observe_update_v1` callback (same pattern as LinkIndexer), debounces (2s like LinkIndexer), extracts Y.Text("contents"), updates inverted index. MmapDirectory for near-zero resident memory.
3. **MCP server (Python OR Rust, separate process)** — Exposes MCP tools, handles JSON-RPC protocol, translates to HTTP calls against relay server. Stateless. Uses server token auth (from connection string).
4. **HTTP API (new endpoints on relay)** — `GET /search`, `GET /docs`, `GET /docs/:id/content`, `GET /docs/:id/backlinks`. Consumed by both MCP server and lens-editor.

**Key architectural patterns from research:**
- **Piggyback on existing update observer** — The relay's `observe_update_v1` callback already fires on every Y.Doc mutation. LinkIndexer, webhook dispatcher, and search indexer all attach to this same callback. Add search indexer via tokio::spawn in the same pattern.
- **Debounced background worker** — Don't index on every keystroke. Use LinkIndexer's 2-second debounce pattern: send message to mpsc channel, reset timestamp on subsequent updates, process after silence. Prevents CPU thrashing.
- **HTTP thin client for MCP** — MCP server holds no state, no document copies, no search index. Every tool call is one or more HTTP requests to relay server. If relay restarts, MCP server needs no rebuild.

**Data flow (search indexing):**
```
User edits doc (Obsidian/lens-editor)
  → WebSocket sync → Y.Doc updated in memory
  → observe_update_v1 callback fires
  → SearchIndexer.on_document_update() (debounced, same as LinkIndexer)
  → Read Y.Text("contents") from in-memory Y.Doc
  → tantivy index.writer().add_document()
  → Batch commit every 5-10s
```

**Data flow (MCP query):**
```
Claude Code: "Search for 'machine learning'"
  → MCP Server: search(query="machine learning")
  → HTTP GET relay-server/search?q=machine+learning&limit=10
  → relay-server: tantivy searcher.search(query, limit)
  → Returns: [{doc_id, title, snippet, score}, ...]
  → MCP formats as tool response → Claude displays
```

### Critical Pitfalls

1. **Treating the MCP Server Like a REST API Wrapper** — Exposing 15-20 low-level operations as individual tools overwhelms the LLM with choices and consumes thousands of context tokens. Design 5-10 outcome-oriented tools instead (e.g., `find_relevant_content(query, include_backlinks=true)` instead of `search` + `get_document` + `get_backlinks` separately). Keep tool output under 2000 tokens. This must be decided during MCP design phase before any implementation.

2. **Memory Exhaustion on 4GB VPS** — Multiple subsystems compete for RAM: relay Y.Docs in memory (~10-20MB for 200 docs), Tantivy index, link indexer caches, MCP session state, PLUS the existing WebSocket FD leak (CLOSE-WAIT accumulation at ~50KB per leaked connection). Mitigation: use Tantivy MmapDirectory (not RamDirectory), set IndexWriter heap budget to 15-50MB (not default 100MB), consider separate MCP process (allows independent OOM recovery). Address during architecture decisions phase.

3. **Indexing Stale CRDT State (Race Conditions)** — Search index shows deleted text or misses just-added text because updates arrive continuously from multiple sources. The observe_update_v1 callback fires on every keystroke. Mitigation: reuse the existing LinkIndexer debounce pattern (2-second delay, DashMap<String, Instant> for pending updates). Accept eventual consistency (2-5 second lag). Read Y.Text content inside a single doc.transact() for atomic reads. Do NOT try to apply incremental CRDT deltas to search index—full re-index per document is simpler and correct. Address during search indexer implementation.

4. **MCP Transport Choice Locks Architecture** — STDIO transport cannot handle multiple concurrent clients (one process per client). SSE is deprecated (replaced by Streamable HTTP in 2025-03-26 spec). Use Streamable HTTP from the start. The MCP server must support multiple concurrent AI assistants (Claude Code, Cursor, etc.). Address during MCP foundation phase—transport determines entire server architecture (process model, session management).

5. **Stdout Corruption in STDIO Transport** — Any println!, tracing to stdout, or library warning corrupts the JSON-RPC protocol stream. Debugging becomes impossible. If STDIO is used: route ALL logging to stderr. Better: skip STDIO entirely, use Streamable HTTP even for local dev (works with MCP Inspector tool). Address during MCP implementation phase.

6. **CriticMarkup Injection Through Search Results** — If search results contain existing CriticMarkup syntax (from previous AI edits or user content), the AI may misinterpret it as instructions or produce nested/invalid markup like `{++{++text++}++}`. Mitigation: strip existing CriticMarkup from search snippets returned to AI. Validate CriticMarkup output before writing to Y.Doc (reject nested delimiters). Address during MCP tool implementation for edit capabilities.

## Implications for Roadmap

Based on research, suggested phase structure:

### Phase 1: HTTP API + Search Index Foundation
**Rationale:** The search index is foundational—both MCP server and lens-editor depend on it. The HTTP API is needed before any MCP work can begin. This phase has no MCP-specific code, so it can proceed with zero SDK risk. Building search indexing first validates the memory model on the 4GB VPS before adding MCP overhead.

**Delivers:**
- New HTTP endpoints: `GET /search`, `GET /docs`, `GET /docs/:id/content`, `GET /docs/:id/backlinks`
- Search index service embedded in relay (Tantivy with MmapDirectory)
- SearchIndexer following LinkIndexer pattern (debounced background worker)
- Startup reindexing (mirrors existing `reindex_all_backlinks()`)

**Addresses features:**
- Search API endpoint (table stakes)
- Infrastructure for search_documents tool (table stakes)
- Infrastructure for list_documents tool (table stakes)

**Avoids pitfalls:**
- Memory exhaustion (Tantivy MmapDirectory + IndexWriter heap budget configuration)
- Stale CRDT index (debounce pattern from LinkIndexer)

**Tech notes:**
- No Axum upgrade needed if Python MCP is chosen
- Axum 0.7→0.8 upgrade required if Rust MCP is chosen (do before this phase)
- Test on production-like environment (4GB Docker container with relay-git-sync running)

### Phase 2: MCP Server Foundation (Read-Only)
**Rationale:** Start with read-only tools to validate MCP SDK choice, transport, and tool design before adding write capabilities. Read-only tools are lower risk (no data corruption potential) and let us test the architecture under load. Defers the Rust-vs-Python decision to planning time when user preference is known.

**Delivers:**
- MCP server project structure (Python FastMCP OR Rust rmcp, TBD)
- Streamable HTTP transport configured for multiple clients
- Tools: `list_documents`, `read_document`, `search_documents`, `get_backlinks`, `get_forward_links`
- MCP testing with Inspector tool + real Claude Code session

**Addresses features:**
- All table stakes read-only features
- MCP server foundation

**Avoids pitfalls:**
- Wrong transport (Streamable HTTP from day 1)
- REST wrapper anti-pattern (design tools before implementation—get user feedback on tool API)
- Stdout corruption (proper logging configuration, or skip STDIO entirely)

**Tech notes:**
- If Python: reuse existing relay_sdk, add FastMCP dependency
- If Rust: new mcp-search crate, upgrade Axum to 0.8, use rmcp 0.14
- Test with 2-3 concurrent Claude Code clients

### Phase 3: Search UI in lens-editor
**Rationale:** This phase is independent of MCP (shares the search API but doesn't interact with MCP server). Can be built in parallel with Phase 2. Delivers value to human users before AI users.

**Delivers:**
- Search input component in React (lens-editor)
- Results panel showing snippets + backlink counts
- Click-to-open document from search results
- "Last indexed" timestamp display

**Addresses features:**
- Search UI (table stakes for users)
- Search with link context (differentiator)

**Avoids pitfalls:**
- UX pitfall: raw markdown in results (extract plain text from Y.Text)
- UX pitfall: no indication of index freshness (show last_indexed timestamp)

**Tech notes:**
- Purely frontend work (no relay server changes)
- Can start while Phase 2 is in progress

### Phase 4: MCP Edit Capabilities (CriticMarkup)
**Rationale:** Once read-only tools are validated, add write capabilities via CriticMarkup. This is the key differentiator—no other MCP server offers suggestion-based editing. Requires careful testing of yrs write operations and CriticMarkup validation.

**Delivers:**
- `edit_document` tool with CriticMarkup support
- Validation: reject nested/malformed CriticMarkup
- Strip existing CriticMarkup from search results to prevent injection
- Tool description documenting review workflow

**Addresses features:**
- edit_document with CriticMarkup (key differentiator)
- Reviewable AI suggestions (safety without AuthZ)

**Avoids pitfalls:**
- CriticMarkup injection (strip from search results, validate output)
- Direct writes without AuthZ (by design: only CriticMarkup suggestions)

**Tech notes:**
- Uses existing Python SDK UpdateContext (if Python MCP)
- Requires Y.Text write access via yrs (if Rust MCP)
- Test: nested markup rejection, existing markup in search results

### Phase 5: Advanced Graph Navigation
**Rationale:** After core tools are working, add advanced graph features that differentiate this MCP server from generic filesystem access. These are nice-to-have enhancements, not MVP requirements.

**Delivers:**
- `traverse_links` tool (N-degree BFS graph traversal)
- `get_document_context` bundle tool (doc + links in one call)
- MCP Prompts for common workflows ("find related", "summarize with context")
- Cross-folder link awareness in results

**Addresses features:**
- Graph traversal (differentiator)
- Document context bundle (differentiator, reduces LLM round-trips)
- MCP Prompts (differentiator, improves UX)

**Avoids pitfalls:**
- Tool output overflow (implement limit parameter with pagination metadata)

**Tech notes:**
- Leverage existing backlinks_v0 + forward links from link indexer
- BFS implementation with configurable max depth
- Return compact summaries, not full document text

### Phase Ordering Rationale

**Why HTTP API + Search first:** MCP server depends on it. Testing search indexing under load validates memory model before adding MCP overhead. Delivers value (HTTP API for lens-editor) even if MCP work is delayed.

**Why read-only MCP second:** Lower risk than starting with edits. Validates SDK choice, transport, and tool design. Allows testing under concurrent load before data-modifying operations are enabled.

**Why search UI in parallel:** Independent work stream (frontend only). Delivers value to human users. Can proceed while MCP tools are being built/tested.

**Why edit capabilities fourth:** Requires validated MCP foundation. Write operations need careful testing. CriticMarkup is novel—needs user feedback before advanced features are added.

**Why advanced graph features last:** Not MVP requirements. Build after core tools prove useful and users request enhancements.

**How this avoids pitfalls:**
- Memory issues addressed early (Phase 1 validates Tantivy configuration)
- MCP design mistakes caught early (Phase 2 tests read-only tools, gets feedback before edits)
- CRDT race conditions addressed via existing pattern (Phase 1 follows LinkIndexer)
- Transport choice locked in at foundation (Phase 2, before significant code investment)

### Research Flags

**Phases needing deeper research during planning:**

- **Phase 2 (MCP Foundation):** Needs SDK comparison research if user wants to evaluate both options. Rust rmcp requires nightly compiler investigation. Python FastMCP tool registration patterns need examples.
- **Phase 4 (Edit Capabilities):** CriticMarkup syntax validation needs detailed spec research. Yrs write operation patterns need codebase investigation. UpdateContext usage in Python SDK needs documentation review.

**Phases with standard patterns (skip research-phase):**

- **Phase 1 (HTTP API + Search):** Well-documented patterns. Tantivy documentation is excellent. LinkIndexer already demonstrates the exact pattern for search indexing.
- **Phase 3 (Search UI):** Standard React component work. No novel patterns.
- **Phase 5 (Graph Navigation):** BFS is textbook algorithm. Backlinks data structure already exists.

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | MEDIUM-HIGH | Core technologies verified (Tantivy, MCP SDKs). Tension on Rust-vs-Python MCP unresolved—both viable, needs user input. Axum upgrade required for Rust approach. |
| Features | MEDIUM | MCP ecosystem well-surveyed (multiple implementations analyzed). Feature expectations grounded in competitor analysis. CriticMarkup editing is novel—no prior art to reference. |
| Architecture | HIGH | Based on deep codebase analysis. LinkIndexer pattern is proven and directly applicable. HTTP API pattern already used in relay. Process separation vs in-process tradeoffs well understood. |
| Pitfalls | MEDIUM-HIGH | MCP pitfalls well-documented in multiple sources. CRDT indexing pitfalls validated via codebase analysis + community discussion. Memory constraints are production reality for this specific VPS. |

**Overall confidence:** MEDIUM-HIGH

Research provides strong foundation for roadmap decisions. The Rust-vs-Python MCP tension is the main open question—it's a legitimate architectural choice, not a gap in research. Both paths are viable.

### Gaps to Address

**Unresolved architectural choice (Rust vs Python MCP):**
- **Gap:** Stack researcher recommends Rust rmcp (in-process, minimal overhead). Architecture researcher recommends Python FastMCP (separate process, mature SDK). Both have merit.
- **How to handle:** Present both options with tradeoffs during Phase 2 planning. Let user decide based on preference for memory efficiency (Rust) vs development speed (Python). Roadmap should support either path.
- **Tradeoffs documented:**
  - Python: +100MB memory overhead, mature SDK, existing relay_sdk integration, simpler tooling, process isolation
  - Rust: near-zero overhead, requires Axum 0.8 upgrade, less mature SDK, may need nightly compiler, shares relay process (OOM kills both)

**CriticMarkup validation details:**
- **Gap:** Research identifies CriticMarkup injection as a pitfall but doesn't provide detailed validation rules.
- **How to handle:** During Phase 4 planning, research CriticMarkup spec for parsing rules and edge cases. Implement simple regex-based validation initially; iterate based on real usage patterns.

**Rust rmcp nightly compiler requirement:**
- **Gap:** Pitfalls researcher noted rmcp may require Rust Edition 2024 (nightly). Stack researcher didn't investigate compiler requirements.
- **How to handle:** If Rust MCP is chosen, verify rmcp 0.14 compiler requirements during Phase 2 planning. Check if stable Rust 1.82+ is sufficient or if nightly is actually needed. This informs feasibility.

**Production memory budget validation:**
- **Gap:** Memory estimates are based on calculations, not real measurements under load with concurrent MCP clients + search indexing + relay sync.
- **How to handle:** During Phase 1 implementation, instrument RSS memory monitoring (log every 60s). Run load test: 3-5 concurrent WebSocket clients + search queries for 24-48 hours. Validate that Tantivy MmapDirectory + IndexWriter heap budget keeps total RSS under 1.5GB (leaving 2.5GB headroom on 4GB VPS).

**WebSocket FD leak root cause:**
- **Gap:** Known issue (CLOSE-WAIT accumulation, ~39 day time-to-failure with ulimit workaround). Root cause not investigated during research.
- **How to handle:** Out of scope for this project (MCP + search). Document as known constraint. The MCP server doesn't use WebSocket (HTTP only), so it doesn't worsen the leak. Consider separate investigation/bugfix later.

## Sources

### Primary (HIGH confidence)
- Codebase analysis: `server.rs`, `link_indexer.rs`, `doc_sync.rs`, `doc_connection.rs`, Python `relay_sdk/` — existing patterns and constraints
- [MCP Official Specification 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25) — protocol requirements
- [MCP Official SDKs](https://modelcontextprotocol.io/docs/sdk) — Python FastMCP, Rust rmcp
- [Tantivy GitHub](https://github.com/quickwit-oss/tantivy) — memory model (MmapDirectory)
- [Tantivy docs.rs 0.25.0](https://docs.rs/tantivy/latest/tantivy/) — API and features
- [rmcp docs.rs 0.14.0](https://docs.rs/rmcp/latest/rmcp/) — Rust MCP SDK API
- [rusqlite docs.rs 0.38.0](https://docs.rs/rusqlite/latest/rusqlite/) — SQLite FTS5 alternative

### Secondary (MEDIUM confidence)
- [Obsidian MCP Server (cyanheads)](https://github.com/cyanheads/obsidian-mcp-server) — competitor feature analysis
- [GraphThulhu](https://github.com/skridlevsky/graphthulhu) — graph traversal patterns
- [MCP Filesystem Server (Anthropic)](https://github.com/modelcontextprotocol/servers/tree/main/src/filesystem) — tool design patterns
- [Shuttle blog: Streamable HTTP MCP in Rust](https://www.shuttle.dev/blog/2025/10/29/stream-http-mcp) — rmcp + Axum integration
- [Tantivy memory model blog](https://fulmicoton.com/posts/behold-tantivy/) — MmapDirectory explanation
- [Yjs search indexer discussion](https://discuss.yjs.dev/t/search-indexer-architecture/520) — CRDT indexing challenges
- [tokio-tungstenite Issue #195](https://github.com/snapview/tokio-tungstenite/issues/195) — WebSocket FD leak documentation
- [How Not to Write an MCP Server](https://towardsdatascience.com/how-not-to-write-an-mcp-server/) — MCP anti-patterns
- [MCP Best Practices](https://www.philschmid.de/mcp-best-practices) — tool design guidelines
- [Configuring MCP for Multiple Connections](https://mcpcat.io/guides/configuring-mcp-servers-multiple-simultaneous-connections/) — concurrency patterns

### Tertiary (LOW confidence)
- [Best MCP Servers for Knowledge Bases 2026](https://desktopcommander.app/blog/2026/02/06/best-mcp-servers-for-knowledge-bases-in-2026/) — ecosystem survey
- [MCP Best Practices 2026](https://www.cdata.com/blog/mcp-server-best-practices-2026) — production patterns (blog post)

---
*Research completed: 2026-02-08*
*Ready for roadmap: yes*
