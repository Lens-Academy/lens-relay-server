# Architecture Research: MCP Server + Search Index Integration

**Domain:** MCP server and keyword search for CRDT document system
**Researched:** 2026-02-08
**Confidence:** HIGH (based on deep codebase analysis + verified external research)

## Existing System Architecture

Before recommending new component boundaries, here is a precise map of what exists.

```
                    Cloudflare R2
                    (lens-relay-storage)
                         |
                    S3Store (rusty-s3)
                         |
  Clients ---------- relay-server (Rust, Axum, port 8080) ---------- Webhooks
  (Obsidian,         |           |           |                       (relay-git-sync)
   lens-editor,      |           |           |
   Python SDK)       |           |           |
                     |           |           |
              DashMap<String,    LinkIndexer  EventDispatcher
              DocWithSyncKv>     (backlinks)  (webhook + sync-proto)
                     |
              DocWithSyncKv
              |-- Awareness(Y.Doc)    <-- in-memory CRDT
              |-- SyncKv              <-- persistence layer
              |-- Subscription        <-- update observer
```

### Key Data Structures in Y.Docs

Each document has a Y.Doc containing:
- **Content docs:** `Y.Text("contents")` -- markdown content
- **Folder docs:** `Y.Map("filemeta_v0")` -- path-to-UUID mapping, `Y.Map("backlinks_v0")` -- UUID-to-backlinker-UUIDs

### Existing Access Patterns

| Access Method | How It Works | Auth |
|---------------|-------------|------|
| WebSocket (yjs sync) | `GET /doc/ws/:doc_id` or `/d/:doc_id/ws/:doc_id2` | Doc token in query param |
| HTTP get-as-update | `GET /d/:doc_id/as-update` | Bearer doc token |
| HTTP update | `POST /d/:doc_id/update` | Bearer doc token (Full auth) |
| Server API (create, auth) | `POST /doc/new`, `POST /doc/:doc_id/auth` | Bearer server token |
| Python SDK | Uses HTTP API: `get_as_update()` + `update_doc()` via `pycrdt` | Server token -> doc token |

### Memory Model (Critical Constraint)

The relay server holds **all loaded documents in memory** as Y.Docs inside `DashMap<String, DocWithSyncKv>`. On a 4GB VPS:

- Each Y.Doc occupies memory proportional to document content + edit history
- `load_all_docs()` loads every document from R2 on startup for backlink reindexing
- Typical corpus: ~100-200 markdown documents across 2 shared folders
- Estimated memory per doc: 10-100KB (small markdown files with moderate history)
- Total doc memory: ~10-20MB (manageable)
- Server process + tokio runtime + HTTP: ~50-100MB
- **Available for search index: ~3.5GB** (plenty for a corpus this small)

## Recommended Architecture

### Decision: Separate MCP Server, Search Index Inside Relay

```
                         Cloudflare R2
                              |
                         relay-server (Rust, Axum)
                         |    |    |    |
                   DashMap  LinkIdx  SearchIdx  HTTP API
                   (docs)  (backlinks) (tantivy) (new endpoints)
                                                  |
                                           /search
                                           /docs (list)
                                           /docs/:id/content
                                           /docs/:id/backlinks
                                                  |
                         MCP Server (Python, stdio)
                         |         |
                   relay_sdk    mcp SDK
                   (existing)  (FastMCP)
                         |
                    Claude Code / Claude Desktop
```

### Component Boundaries

| Component | Language | Responsibility | Communicates With |
|-----------|----------|----------------|-------------------|
| **relay-server** | Rust | Document storage, sync, auth, search index, HTTP API | R2, clients (WS), MCP server (HTTP) |
| **Search index** | Rust (tantivy, embedded) | Full-text indexing of document contents | Embedded in relay-server, reads Y.Docs |
| **MCP server** | Python | MCP protocol, tool definitions, LLM interface | relay-server (HTTP API), Claude (stdio) |
| **Python SDK** | Python | HTTP client for relay API | relay-server (HTTP) |

### Rationale for Key Decisions

#### Why embed search in relay-server (not standalone)

1. **The relay already holds all Y.Docs in memory.** The search indexer needs to read document content. If search runs as a separate service, it would need to either (a) maintain its own copy of every document via WebSocket connections, or (b) call HTTP to fetch each document's content. Both waste resources on a 4GB VPS.

2. **The LinkIndexer already demonstrates the pattern.** The existing `LinkIndexer` watches document updates via the event callback, debounces, reads Y.Text("contents"), and writes to Y.Map("backlinks_v0"). A search indexer follows the exact same lifecycle: watch updates, debounce, read content, update index.

3. **Memory efficiency.** Tantivy with MmapDirectory has near-zero resident memory for a small corpus. Embedding it adds negligible overhead versus a separate process that would duplicate the tokio runtime, HTTP client, and document loading.

4. **Operational simplicity.** One Docker container, one process. No inter-service coordination, no service discovery, no additional port management.

#### Why MCP server is Python (not Rust, not embedded)

1. **The Python SDK already exists** with `DocumentManager`, `DocConnection`, and `UpdateContext`. The MCP server's document operations (list, read, edit) are thin wrappers around these existing primitives.

2. **The MCP Python SDK is the most mature** with `FastMCP` providing decorator-based tool registration. The Rust MCP SDK (`rmcp`) exists but is less mature and would require significant boilerplate.

3. **MCP servers are thin.** The MCP server does not hold state. It translates tool calls to HTTP requests against the relay server. CPU/memory overhead of a Python process for this workload is negligible.

4. **stdio transport is standard for Claude Code integration.** Claude Code spawns MCP servers as child processes communicating via stdin/stdout. Python with `FastMCP("lens-relay").run(transport="stdio")` is the simplest path.

#### Why MCP connects via HTTP (not WebSocket, not native)

1. **MCP tools are request/response**, not streaming. "Read document X" and "Search for Y" are HTTP GET equivalents. WebSocket connections are for long-lived sync sessions.

2. **The relay already has HTTP endpoints** for `get_as_update` and `update`. Adding `/search` and `/docs` endpoints extends the existing pattern.

3. **The Python SDK already implements HTTP auth flow.** `DocumentManager._do_request()` handles server token auth, and `DocConnection` handles doc-level operations. The MCP server reuses this.

4. **No auth complexity.** The MCP server uses the server token (from connection string) for all operations, same as the Python SDK does today. No need for per-document WebSocket token negotiation.

## Data Flow

### Document Indexing Flow

```
User edits document (Obsidian / lens-editor)
    |
    v
WebSocket sync -> Y.Doc updated in memory
    |
    v
observe_update_v1 callback fires
    |
    +-- SyncKv persistence (existing)
    +-- Event dispatch (existing webhooks)
    +-- LinkIndexer.on_document_update() (existing backlinks)
    +-- SearchIndexer.on_document_update() (NEW -- same pattern)
            |
            v
        Debounce (2s, same as LinkIndexer)
            |
            v
        Read Y.Text("contents") from in-memory Y.Doc
            |
            v
        Extract plain text from markdown
            |
            v
        tantivy index.writer().add_document() / delete + re-add
            |
            v
        index.writer().commit()
```

### Search Query Flow

```
Claude Code: "Search for documents about 'machine learning'"
    |
    v
MCP Server receives tool call: search(query="machine learning")
    |
    v
HTTP GET relay-server/search?q=machine+learning&limit=10
    |
    v
relay-server: tantivy searcher.search(query, limit)
    |
    v
Returns: [{doc_id, title, snippet, score}, ...]
    |
    v
MCP Server formats results as tool response
    |
    v
Claude Code displays to user
```

### Document Read Flow (MCP)

```
Claude Code: "Read the document titled 'Syllabus'"
    |
    v
MCP Server: list_docs() -> find doc_id for "Syllabus"
    |
    v
HTTP GET relay-server/d/{doc_id}/as-update (existing endpoint)
    |
    v
MCP Server: pycrdt.Doc().apply_update(bytes)
    |
    v
MCP Server: doc.get("contents", type=pycrdt.Text).to_string()
    |
    v
Returns markdown text to Claude
```

### Document Edit Flow (MCP with CriticMarkup)

```
Claude Code: "Add a comment to paragraph 3"
    |
    v
MCP Server: edit_doc(doc_id, edits=[{type: "insert", ...}])
    |
    v
MCP Server: conn.for_update() context manager
    |-- GET as-update -> pycrdt.Doc
    |-- Apply CriticMarkup insertions to Y.Text
    |-- POST update (diff)
    |
    v
relay-server applies update, triggers re-index
```

## Recommended Project Structure

```
crates/
  relay/
    src/
      server.rs          # Add new HTTP endpoints here
      search_indexer.rs   # NEW: tantivy integration, mirrors link_indexer pattern
  y-sweet-core/
    src/
      link_indexer.rs     # Existing pattern to follow
      search_indexer.rs   # NEW: core search logic (schema, indexing, querying)

mcp-server/              # NEW: Python MCP server
  pyproject.toml          # uv managed, deps: mcp, relay-sdk (local)
  src/
    lens_mcp/
      __init__.py
      server.py           # FastMCP server definition
      tools/
        documents.py      # list_docs, read_doc, edit_doc tools
        search.py          # search tool
        links.py           # get_backlinks, get_outlinks tools
      relay_client.py     # Wraps relay_sdk.DocumentManager with search

python/                   # EXISTING: Python SDK
  src/
    relay_sdk/
      __init__.py         # DocumentManager (existing)
      connection.py       # DocConnection (existing)
      update.py           # UpdateContext (existing)
```

### Structure Rationale

- **`crates/y-sweet-core/src/search_indexer.rs`**: Core indexing and query logic lives in y-sweet-core (not relay) because it operates on bare Y.Docs, making it testable without a running server -- same pattern as `link_indexer.rs` where `index_content_into_folder()` is a free function that takes `&Doc` references.

- **`crates/relay/src/search_indexer.rs`**: Server-level glue that owns the tantivy `Index`, spawns the background worker, and registers HTTP endpoints. Mirrors how `link_indexer.rs` has `LinkIndexer` struct with `run_worker()`.

- **`mcp-server/`**: Separate directory (not inside `python/`) because the MCP server is a deployable service with its own entry point, not a library. It depends on `relay_sdk` as a local path dependency.

## Architectural Patterns

### Pattern 1: Piggyback on Existing Update Observer

**What:** The relay server already has an `observe_update_v1` callback on every Y.Doc that fires on every mutation. The LinkIndexer, webhook dispatcher, and search indexer all attach to this same callback chain.

**When to use:** Any time you need to react to document changes.

**Trade-offs:** Simple, zero-latency notification. But the callback runs synchronously in the yrs update path, so it must be non-blocking (just send to a channel, like LinkIndexer does).

**Example (existing pattern in server.rs):**
```rust
// Inside the event_callback closure in load_doc_with_user():
if let Some(ref indexer) = link_indexer_for_callback {
    if y_sweet_core::link_indexer::should_index() {
        let indexer = indexer.clone();
        let doc_key = doc_key_for_indexer.clone();
        tokio::spawn(async move {
            indexer.on_document_update(&doc_key).await;
        });
    }
}
// NEW: Same pattern for search indexer
if let Some(ref search_idx) = search_indexer_for_callback {
    let search_idx = search_idx.clone();
    let doc_key = doc_key_for_search.clone();
    tokio::spawn(async move {
        search_idx.on_document_update(&doc_key).await;
    });
}
```

### Pattern 2: Debounced Background Worker

**What:** Instead of indexing on every keystroke, batch updates with a debounce timer. The LinkIndexer uses a 2-second debounce: the first update sends a message to an mpsc channel, subsequent updates just reset a timestamp, and the worker only processes after 2 seconds of silence.

**When to use:** Any indexing or processing that reads full document content. Typing produces 10+ updates per second; indexing once after a pause is sufficient.

**Trade-offs:** Adds 2-second latency to index freshness. For search, this is acceptable. The alternative (index every keystroke) would thrash the tantivy writer and waste CPU.

### Pattern 3: HTTP Thin Client for MCP

**What:** The MCP server is a stateless HTTP client. It holds no document state, no search index, no persistent connections. Every tool call translates to one or more HTTP requests against the relay server.

**When to use:** When the backend already has the data and the client just needs to expose it through a different protocol.

**Trade-offs:** Adds HTTP round-trip latency (localhost: <1ms). But avoids all the complexity of maintaining synchronized state in two processes. If the relay server restarts, the MCP server does not need to rebuild anything.

## Anti-Patterns to Avoid

### Anti-Pattern 1: MCP Server as yjs Client

**What people do:** Have the MCP server connect to the relay via WebSocket and maintain its own Y.Doc copies, then build the search index inside the MCP process.

**Why it's wrong:** Doubles memory usage (every doc in both relay and MCP). Adds complexity of managing WebSocket reconnection, state vector sync, and index rebuilding. The relay already has all docs in memory -- why duplicate?

**Do this instead:** Embed the search index in the relay server and expose it via HTTP. The MCP server stays stateless.

### Anti-Pattern 2: Separate Search Service

**What people do:** Run Elasticsearch/Meilisearch/Typesense as a separate Docker container for search.

**Why it's wrong:** For ~200 markdown documents on a 4GB VPS, a separate search engine is massive overkill. Elasticsearch alone wants 1-2GB of heap. Meilisearch is lighter but still a separate process with its own persistence, its own document loading, and its own update pipeline.

**Do this instead:** Use tantivy as an embedded library inside the relay server. For this corpus size, the search index is likely <5MB. Tantivy with MmapDirectory has near-zero resident memory overhead.

### Anti-Pattern 3: Synchronous Search Index Updates

**What people do:** Update the search index inside the `observe_update_v1` callback synchronously, blocking the yjs sync loop.

**Why it's wrong:** Tantivy `index.writer().commit()` can take milliseconds. The `observe_update_v1` callback must be non-blocking -- it runs while holding the Y.Doc write lock.

**Do this instead:** Follow the LinkIndexer pattern: send a message to an async channel, process in a background task with debouncing.

### Anti-Pattern 4: Exposing Search via WebSocket

**What people do:** Create a custom WebSocket message type for search queries and responses, extending the yjs sync protocol.

**Why it's wrong:** Search is request/response, not streaming. The yjs sync protocol is for document state synchronization. Mixing concerns creates protocol complexity and makes the search feature dependent on having a WebSocket connection.

**Do this instead:** Add a standard HTTP endpoint (`GET /search?q=...`). Both the MCP server and the lens-editor can call it independently.

## New HTTP Endpoints on Relay Server

These endpoints are needed by the MCP server and are also useful for the lens-editor.

| Endpoint | Method | Auth | Purpose |
|----------|--------|------|---------|
| `GET /search` | GET | Server token | Full-text search across all documents |
| `GET /docs` | GET | Server token | List all documents with metadata |
| `GET /docs/:doc_id/content` | GET | Server/Doc token | Get document as plain text (not yjs binary) |
| `GET /docs/:doc_id/backlinks` | GET | Server/Doc token | Get backlinks for a document |
| `GET /docs/:doc_id/outlinks` | GET | Server/Doc token | Get outgoing links from a document |

**Why server token auth:** These endpoints expose data across all documents (search, list) or provide read-only views. Server-level auth is appropriate, matching how the Python SDK already authenticates.

**Why `/docs/:doc_id/content` is new:** The existing `/d/:doc_id/as-update` returns yjs binary that requires a CRDT library to decode. The MCP server needs plain text. Rather than decode yjs binary in Python on every read, the relay server can extract `Y.Text("contents").get_string()` and return plain text directly. This avoids the pycrdt dependency in the hot path and reduces latency.

## Search Index Design (Tantivy)

### Schema

```rust
let mut schema_builder = Schema::builder();
schema_builder.add_text_field("doc_id", STRING | STORED);       // exact match, stored for retrieval
schema_builder.add_text_field("title", TEXT | STORED);           // tokenized, for search + display
schema_builder.add_text_field("path", STRING | STORED);          // e.g., "/Notes/Ideas.md"
schema_builder.add_text_field("folder", STRING | STORED);        // which shared folder
schema_builder.add_text_field("content", TEXT);                  // tokenized, for search (not stored -- too large)
schema_builder.add_u64_field("updated_at", INDEXED | STORED);   // for sorting by recency
```

### Index Lifecycle

1. **Startup:** After `load_all_docs()` and `reindex_all_backlinks()`, do `reindex_all_search()` -- iterate all content docs, extract text, build full index. This mirrors the existing `reindex_all_backlinks()` pattern.

2. **Runtime:** On document update (via debounced worker), delete the old document entry by `doc_id` and re-add with current content. Tantivy handles this efficiently with `delete_term()` + `add_document()`.

3. **Persistence:** Use `MmapDirectory` pointing to a local directory (e.g., `/data/search-index/`). The index survives relay restarts without full rebuild from R2. But since docs are loaded from R2 anyway, a full rebuild on startup is also acceptable for ~200 docs.

### Memory Impact

For ~200 documents of ~5KB average content:
- Raw content: ~1MB
- Tantivy index (inverted index + FSTs): ~500KB-2MB
- MmapDirectory: near-zero resident memory (OS pages in/out as needed)
- Index writer buffer: 15MB during indexing (configurable, can lower to 5MB)
- **Total additional memory: <20MB** -- negligible on 4GB VPS

## Build Order (Dependencies)

The components have clear build-order dependencies:

```
Phase 1: HTTP Read API on relay server
  - GET /docs (list documents with metadata from filemeta_v0)
  - GET /docs/:doc_id/content (plain text extraction from Y.Text)
  - GET /docs/:doc_id/backlinks (read from backlinks_v0)
  No new dependencies. Uses existing DashMap<docs>, existing filemeta/backlinks Y.Maps.

Phase 2: MCP Server (basic tools)
  - Python project with FastMCP
  - list_docs, read_doc tools (calls Phase 1 endpoints)
  - get_backlinks tool
  Depends on: Phase 1 endpoints existing.

Phase 3: Search index in relay server
  - Add tantivy dependency to y-sweet-core/relay
  - SearchIndexer struct (mirrors LinkIndexer)
  - GET /search endpoint
  - Startup reindexing
  Depends on: Nothing from Phase 1/2 (parallel-safe), but logically follows.

Phase 4: Search tool in MCP server
  - search() tool in MCP server (calls GET /search)
  Depends on: Phase 2 (MCP server exists) + Phase 3 (search endpoint exists).

Phase 5: Edit capabilities
  - edit_doc tool in MCP server (CriticMarkup insertions)
  - Uses existing Python SDK UpdateContext
  Depends on: Phase 2 (MCP server exists).
```

**Key insight:** Phases 1 and 3 are independent (HTTP API and search index). Phase 2 depends on Phase 1. Phase 4 depends on both 2 and 3. This allows parallel work on the relay (Rust) and MCP (Python) tracks.

## Scaling Considerations

| Scale | Architecture Adjustments |
|-------|--------------------------|
| Current (~200 docs, 1-5 users) | Monolith is perfect. All docs in memory, search index embedded, single process. |
| ~1000 docs, 10 users | Still fine. Tantivy handles millions of docs. Memory might reach 200-300MB for docs. |
| ~10,000 docs, 100 users | Consider lazy doc loading (don't `load_all_docs` on startup). Search index becomes the primary discovery mechanism. May need GC tuning. |
| Hypothetical large scale | Not a concern for this project. The architecture doesn't preclude migration to external search if ever needed. |

### First Bottleneck

**Memory from loaded docs.** If all documents are loaded on startup (current behavior for backlink reindexing), memory scales linearly with corpus size. For search-only needs, tantivy can index from stored data and doesn't require docs to be in memory -- but backlink indexing currently does. This is a future concern, not a current one.

### Second Bottleneck

**Startup time.** Loading all docs from R2 and reindexing takes time. Currently ~200 docs loads in seconds. At 1000+ docs, this could take 30+ seconds. Mitigation: persist the search index to disk so it doesn't need full rebuild, and lazy-load docs for backlinks.

## Integration Points

### External Services

| Service | Integration Pattern | Notes |
|---------|---------------------|-------|
| Cloudflare R2 | S3-compatible presigned URLs via rusty-s3 | Existing, no changes needed |
| relay-git-sync | HTTP webhooks on document update | Existing, no changes needed |
| Claude Code | stdio MCP protocol | New, MCP server provides this |

### Internal Boundaries

| Boundary | Communication | Notes |
|----------|---------------|-------|
| relay-server <-> search index | In-process (embedded tantivy) | No IPC, shared memory via `Arc<DashMap>` |
| relay-server <-> MCP server | HTTP (localhost) | MCP server uses Python relay_sdk |
| MCP server <-> Claude Code | stdio (JSON-RPC) | Spawned as child process |
| search indexer <-> link indexer | Independent workers | Both watch same update events, don't interact |

## Sources

- Codebase analysis: `server.rs`, `link_indexer.rs`, `doc_sync.rs`, `doc_connection.rs`, `event.rs` (HIGH confidence -- primary source)
- [MCP Official Documentation - Build Server](https://modelcontextprotocol.io/docs/develop/build-server) (HIGH confidence)
- [MCP Python SDK](https://github.com/modelcontextprotocol/python-sdk) (HIGH confidence)
- [Tantivy GitHub](https://github.com/quickwit-oss/tantivy) (HIGH confidence)
- [Tantivy memory architecture](https://fulmicoton.com/posts/behold-tantivy/) (MEDIUM confidence -- blog post from tantivy author)
- [Yjs search indexer architecture discussion](https://discuss.yjs.dev/t/search-indexer-architecture/520) (MEDIUM confidence -- community discussion)
- [MCP Transport Protocols comparison](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports) (HIGH confidence -- official spec)

---
*Architecture research for: MCP server + search index integration with relay server*
*Researched: 2026-02-08*
