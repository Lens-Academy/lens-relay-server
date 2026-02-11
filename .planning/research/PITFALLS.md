# Pitfalls Research

**Domain:** MCP server + keyword search for CRDT collaborative document system
**Researched:** 2026-02-08
**Confidence:** MEDIUM-HIGH (MCP pitfalls well-documented; CRDT indexing pitfalls from codebase analysis + community sources; memory pitfalls from production experience with this exact server)

## Critical Pitfalls

### Pitfall 1: Treating the MCP Server Like a REST API Wrapper

**What goes wrong:**
Developers expose internal relay server HTTP endpoints (create doc, get token, list docs) as individual MCP tools. The AI assistant receives 15-20 tool descriptions, consuming thousands of context tokens. The LLM struggles to choose the right tool, makes unnecessary round-trips, and frequently calls tools in the wrong order. Query results blow past context window limits.

**Why it happens:**
MCP looks like "tools = endpoints" so the instinct is to mirror the REST API. But MCP is a user interface for AI agents, not a programmatic API. What works for human developers (composable endpoints) creates problems for LLMs (decision fatigue, context bloat).

**How to avoid:**
- Design 5-10 outcome-oriented tools, not operation-oriented ones. Instead of `search_documents` + `get_document` + `get_backlinks`, create `find_relevant_content(query, include_backlinks=true)` that returns a curated summary.
- Each tool should produce a self-contained, actionable result. The LLM should never need to call tool A to get an ID for tool B.
- Flatten all arguments to top-level primitives. No nested objects, no complex config. Use `Literal` enums for constrained choices.
- Keep tool output compact: return summaries, not full documents. Offer a `read_document(id)` tool for when the LLM needs the full text.
- Apply the "5-15 tools" rule. If you have more, split into separate MCP servers or consolidate.

**Warning signs:**
- More than 15 tools registered on the MCP server
- Tool output regularly exceeds 2000 tokens
- LLM calls the same tool multiple times to assemble a single answer
- LLM frequently selects the wrong tool or hallucinates parameters

**Phase to address:**
MCP server design phase -- tool API must be designed before implementation begins. Changing tool boundaries after deployment breaks all existing agent prompts.

---

### Pitfall 2: Search Index Grows Unbounded on 4GB VPS

**What goes wrong:**
The search index, the relay server's in-memory document map (`DashMap<String, DocWithSyncKv>`), the link indexer's data structures, and the MCP server's session state all compete for 4GB of RAM. Under load or after extended uptime, the process hits OOM and gets killed. The existing WebSocket FD leak (CLOSE-WAIT accumulation) compounds the problem by holding memory in leaked socket buffers (~50KB per leaked connection based on tokio-tungstenite issue #195).

**Why it happens:**
Multiple memory-hungry subsystems coexist in a single process:
1. Yrs `Doc` instances stay in memory while connections are active (CRDT state can be large for documents with long edit histories)
2. Tantivy's in-memory segments accumulate between commits
3. Link indexer's `DashMap` caches grow with document count
4. Leaked WebSocket connections hold ~50KB each and accumulate over weeks
5. MCP session state adds per-connection overhead

On a 4GB VPS running Docker with other containers (relay-git-sync, cloudflared), realistic available memory is ~2-2.5GB.

**How to avoid:**
- Use Tantivy's `MmapDirectory` instead of `RamDirectory`. MmapDirectory has near-zero resident memory because the OS manages page cache. This is the single most important decision for memory.
- Set Tantivy's `IndexWriter` heap budget to 15-50MB explicitly (default is 100MB which is too much for this VPS).
- Implement the search index as a separate process (not embedded in the relay server). This allows independent OOM-kill recovery -- if the search indexer dies, the relay keeps serving documents.
- Monitor RSS memory with a periodic check (every 60s log current RSS). Set alerts at 70% of available memory.
- Address the WebSocket FD leak -- even the current `ulimit` workaround just extends time-to-failure from days to ~39 days.

**Warning signs:**
- RSS memory climbing steadily over days without new documents
- OOM kills in `dmesg` or Docker logs
- Tantivy commit operations becoming slow (sign of memory pressure forcing page eviction)
- System swap usage increasing (4GB VPS likely has limited or no swap)

**Phase to address:**
Architecture decisions phase (process separation) and search implementation phase (Tantivy configuration). Must be decided before any search code is written.

---

### Pitfall 3: Indexing Stale CRDT State (Race Between Edits and Index)

**What goes wrong:**
The search index shows text that was deleted seconds ago, or misses text that was just added. Users search for text they just typed and get no results. Worse: the indexer reads the Y.Doc at the exact moment a transaction is being applied, getting a partially-applied state. The existing link indexer already demonstrates this challenge -- it uses debouncing (2-second delay) and an `IndexingGuard` to prevent infinite loops when its own writes trigger observers.

**Why it happens:**
CRDT documents change continuously from multiple sources (WebSocket connections, the link indexer itself, the relay-git-sync service). The search indexer must:
1. Detect that a document changed
2. Wait for edits to settle (debounce)
3. Read the Y.Text content
4. Update the search index

Steps 2-4 are not atomic. New edits can arrive between "read content" and "update index." The existing `observe_update_v1` callback fires on every keystroke, and naive handling would re-index on every character typed.

**How to avoid:**
- Reuse the existing debounce pattern from `LinkIndexer` (2-second debounce, `DashMap<String, Instant>` for pending updates). The link indexer already handles this correctly -- follow the same pattern for search indexing.
- Accept eventual consistency: the search index will lag 2-5 seconds behind live edits. This is acceptable for keyword search. Document it as expected behavior.
- Read the Y.Text content inside a single `doc.transact()` -- yrs transactions are atomic reads. Never read content outside a transaction.
- Do NOT try to apply incremental updates to the search index from CRDT deltas. The delta format from yrs is complex (insert/delete/retain operations relative to CRDT positions, not text positions). Full re-index of the document text on each change is simpler and correct.
- Batch index commits: update Tantivy's in-memory segment per-document, but only call `index_writer.commit()` every 5-10 seconds or after N documents, not after every document update.

**Warning signs:**
- Search results returning deleted content
- CPU spikes during rapid typing (indexing too frequently)
- "Document not found" errors when indexing (doc was unloaded between notification and index attempt)
- Index growing without bound (indexing the same document repeatedly without deleting old version)

**Phase to address:**
Search indexer implementation phase. The debounce pattern already exists in the codebase (`link_indexer.rs` lines 402-450) and should be reused, not reinvented.

---

### Pitfall 4: MCP Transport Choice Locks You Into Wrong Architecture

**What goes wrong:**
Starting with STDIO transport because it is simpler to implement, then discovering it cannot handle multiple concurrent AI assistant connections. STDIO is one-process-per-client -- you would need to spawn a new relay-aware process for each Claude Code / Cursor session. Alternatively, starting with SSE transport only to find it deprecated in favor of Streamable HTTP.

**Why it happens:**
The MCP spec has evolved rapidly. SSE was the original remote transport (2024-11-05 spec), then Streamable HTTP replaced it in the 2025-03-26 spec. Many tutorials and examples still show SSE. STDIO is simpler for local tools but fundamentally cannot support concurrent remote clients.

For this project, the MCP server must:
- Run alongside the relay server (same VPS)
- Accept connections from multiple AI assistants (Claude Code, Cursor, etc.)
- Access the relay server's document data (either via HTTP API or shared memory)

**How to avoid:**
- Use Streamable HTTP transport from the start. It supports multiple concurrent clients, works behind reverse proxies/Cloudflare, and is the current spec recommendation.
- Use the `rust-mcp-sdk` crate with its `HyperServer` (Axum-based, supports Streamable HTTP + backward-compatible SSE). Alternatively, `rmcp` crate works but requires Rust Edition 2024 (nightly) which adds build complexity.
- The MCP server can be a separate Axum service on a different port (e.g., 8091), or mounted as a sub-router on the existing relay server's Axum instance. Separate port is safer for process isolation.
- Do NOT embed MCP directly in the relay server binary at first. Run it as a sidecar that talks to the relay server via HTTP. This allows independent deployment and avoids coupling MCP session lifecycle with relay server lifecycle.

**Warning signs:**
- MCP server only works with one client at a time
- Connection timeout errors when second client connects
- SSE connections dropping behind Cloudflare (Cloudflare buffers SSE aggressively)
- "Session not found" errors after client reconnects

**Phase to address:**
MCP foundation phase -- transport must be chosen first, as it determines the entire server architecture (process model, session management, state sharing).

---

### Pitfall 5: Stdout Corruption in STDIO Transport (If Used for Local Dev)

**What goes wrong:**
Any `println!`, `tracing` output to stdout, or library that writes to stdout corrupts the MCP STDIO protocol stream. The client receives malformed JSON-RPC and the connection breaks silently. Debug logging becomes impossible through normal channels.

**Why it happens:**
MCP's STDIO transport uses stdout exclusively for JSON-RPC messages. The Rust ecosystem commonly uses `tracing` with stdout subscribers, and many libraries print warnings to stdout. A single stray `println!` or `eprintln!` to the wrong stream breaks everything.

**How to avoid:**
- If using STDIO for local development/testing: route ALL logging to stderr using `tracing_subscriber` with `.with_writer(std::io::stderr)`.
- Better: skip STDIO entirely. Use Streamable HTTP even for local development. The MCP Inspector tool works with HTTP transport.
- In the relay server codebase, the existing `tracing` setup uses stdout -- this MUST be changed if the MCP server runs in the same process with STDIO transport.
- Test with the MCP Inspector tool before connecting a real LLM client.

**Warning signs:**
- MCP client silently disconnects
- "Parse error" or "Invalid JSON" in MCP client logs
- MCP server appears to work in testing but fails with real clients
- Intermittent failures that correlate with log output volume

**Phase to address:**
MCP implementation phase. Trivial to prevent if you know about it, catastrophic if you don't.

---

### Pitfall 6: CriticMarkup Injection Through Search Results

**What goes wrong:**
The MCP server returns search results to an AI assistant. The assistant uses those results to generate CriticMarkup edits (`{++insert++}`, `{--delete--}`, `{~~old~>new~~}`). But if the search results themselves contain CriticMarkup syntax (either from previous AI edits or from user content), the assistant may misinterpret existing markup as instructions, or produce nested CriticMarkup that the editor cannot parse.

**Why it happens:**
CriticMarkup is plain text with special delimiters. There is no escaping mechanism in the CriticMarkup spec. If a document contains `{++already inserted text++}` and the AI sees this in search results, it may:
1. Try to "apply" it again
2. Wrap it in additional CriticMarkup: `{++{++already inserted text++}++}` (invalid nesting)
3. Confuse it with the document structure

**How to avoid:**
- Strip existing CriticMarkup from search result snippets returned to the AI. The AI should see clean text, not pending edits.
- In MCP tool descriptions, explicitly document that CriticMarkup in the document content is existing markup and should not be re-applied.
- Validate CriticMarkup output from the AI before writing to Y.Doc: reject nested delimiters, malformed syntax.
- Consider a "resolved text" view for search: show the document as it would appear with all CriticMarkup accepted, so the AI works with the "current" state.

**Warning signs:**
- Nested CriticMarkup appearing in documents (`{++{++text++}++}`)
- AI assistant attempting to "undo" existing CriticMarkup
- Search results showing CriticMarkup delimiters in snippets
- Editor rendering errors on AI-edited documents

**Phase to address:**
MCP tool implementation phase (when designing the edit/write tools). Must be addressed before AI assistants are given write access.

---

## Technical Debt Patterns

Shortcuts that seem reasonable but create long-term problems.

| Shortcut | Immediate Benefit | Long-term Cost | When Acceptable |
|----------|-------------------|----------------|-----------------|
| Embedding search index in relay server process | Simpler deployment, shared memory access to Y.Docs | OOM kills take down entire relay server; cannot restart search without disconnecting all users | Never for production; acceptable for local dev prototyping only |
| Full document re-index on every keystroke | Simple implementation, always up-to-date | CPU spike on rapid typing, wastes resources re-indexing unchanged docs | Never -- always debounce (2-5 seconds minimum) |
| Using Tantivy `RamDirectory` | Faster index operations, no disk I/O | Unbounded memory growth, index lost on restart | Only for tests; production must use `MmapDirectory` |
| Skipping MCP session isolation | Simpler state management | One user's search context bleeds into another's; security failure | Never -- session IDs must be generated and validated from day one |
| Hardcoding relay server URL in MCP server | Quick setup | Cannot test locally, breaks in different environments | MVP only; must be configurable by Phase 2 |
| Returning full document text in MCP tool output | AI has all context | Context window overflow at ~30 documents; token costs explode | Only for single-document reads, never for search results |

## Integration Gotchas

Common mistakes when connecting MCP server to the relay system.

| Integration | Common Mistake | Correct Approach |
|-------------|----------------|------------------|
| MCP server reading Y.Docs | Holding `RwLock<Awareness>` read lock while doing slow operations (search, network) | Acquire lock, extract text via `doc.transact()`, release lock immediately. Never hold across await points. |
| Tantivy + relay server | Calling `index_writer.commit()` synchronously in the document update callback | Batch commits in a background task. The `observe_update_v1` callback runs synchronously and blocks the yrs event loop. |
| MCP tool calling relay HTTP API | Using the same auth token for all MCP sessions (shared service account) | Generate per-session tokens with appropriate scoping, or use the existing HMAC auto-detect auth flow. |
| Search index on server restart | Expecting the search index to survive restarts without rebuilding | On startup, walk all documents and rebuild the index. Tantivy `MmapDirectory` persists to disk, but CRDT content may have changed while the server was down (relay-git-sync pushes updates). |
| Link indexer + search indexer | Both observing document updates, creating duplicate work or race conditions | Unify the notification channel: both indexers receive updates from the same `mpsc::Sender`, or use a fan-out pattern. The existing link indexer uses `mpsc::channel(1000)` -- extend this pattern. |
| MCP server + Cloudflare Tunnel | SSE streams dropping after 100 seconds (Cloudflare's default timeout for idle connections) | Use Streamable HTTP with periodic heartbeats, or configure Cloudflare tunnel `--proxy-keepalive-timeout`. |

## Performance Traps

Patterns that work at small scale but fail as usage grows.

| Trap | Symptoms | Prevention | When It Breaks |
|------|----------|------------|----------------|
| Linear scan of `filemeta_v0` for link resolution | Works fine, slightly slow | Build a reverse index (basename -> uuid HashMap) on startup and maintain it | Over 500 documents per folder (current code iterates filemeta for every wikilink in every document) |
| Re-indexing all documents on startup via `reindex_all_backlinks()` | Takes seconds for 50 docs | Persist backlink state; only re-index changed docs on startup | Over 200 documents; startup takes >30 seconds |
| Search index rebuild on every server restart | Clean index every time | Persist Tantivy index to disk with `MmapDirectory`; only re-index docs changed since last shutdown | Over 100 documents; startup takes >60 seconds |
| Unbounded MCP tool output | Works with 5 search results | Implement `limit` parameter (default 10-20) with `has_more` / `next_offset` pagination metadata | Over 30 matching documents; context window overflow |
| Loading all Y.Docs into memory for search | Direct access to CRDT state | Extract text and index it; unload docs that have no active WebSocket connections | Over 200 documents; each Doc can be 10KB-1MB depending on edit history |
| Single-threaded Tantivy `IndexWriter` blocking on commit | Commits are fast at small scale | Use `commit()` in a dedicated background task with batching; never block the main event loop | Commit frequency > 1/second with >50 documents |

## Security Mistakes

Domain-specific security issues beyond general web security.

| Mistake | Risk | Prevention |
|---------|------|------------|
| MCP server exposes document deletion tools | AI assistant could be prompt-injected to delete documents | CriticMarkup approach eliminates this: AI can only suggest edits, never directly write or delete. Keep it this way. |
| Returning document UUIDs in MCP search results without validation | UUIDs could be used to access documents outside the user's folder scope | MCP tools should scope all operations to configured folder(s). Validate that requested doc UUIDs belong to allowed folders. |
| MCP server runs without authentication | Any AI client on the network can read all documents | Require API key or token for MCP connections. Even though CriticMarkup prevents direct writes, read access to all documents is still sensitive. |
| Search index stored world-readable on disk | Other processes/users on VPS can read indexed content | Set file permissions on Tantivy index directory to 600/700. Run MCP server as same user as relay server. |
| Logging full document content in MCP server traces | Sensitive content appears in log files | Log document IDs and search queries only, never full content. Redact content in trace output. |

## UX Pitfalls

Common user experience mistakes in this domain.

| Pitfall | User Impact | Better Approach |
|---------|-------------|-----------------|
| Search returns CRDT-internal content (tombstones, metadata) | Users see garbled results with invisible characters or deleted text fragments | Always extract plain text via `Y.Text.get_string()` / `getText('contents').toString()` which returns the current visible state, excluding deleted content |
| Search results show raw markdown without context | Results are hard to scan; headers and lists look cluttered | Return structured results: `{ title, snippet, path, backlink_count }` with snippet showing matched text with surrounding context |
| No indication of search index freshness | Users search for recently-typed text and get confused when it is not found | Return `last_indexed` timestamp with search results; show "results may lag 2-5 seconds behind live edits" in UI |
| MCP tool errors are opaque | AI assistant says "the tool failed" with no actionable information | Return structured error messages: `{ error: "reason", suggestion: "try X instead" }`. The LLM can use suggestions to self-correct. |
| Backlink results lack path/folder context | "3 backlinks" is less useful than knowing which documents link here | Always include document path (from filemeta_v0) alongside UUID in backlink results |

## "Looks Done But Isn't" Checklist

Things that appear complete but are missing critical pieces.

- [ ] **Search indexing:** Often missing delete handling -- verify that when a document is deleted from filemeta_v0, its content is removed from the search index too (not just added/updated)
- [ ] **MCP tool descriptions:** Often missing edge case documentation -- verify that tool descriptions explain what happens with empty queries, documents without content, and unresolvable links
- [ ] **Tantivy index:** Often missing schema migration -- verify that if you add a new field to the search schema, existing indexes can be rebuilt without data loss
- [ ] **Session cleanup:** Often missing timeout handling -- verify that abandoned MCP sessions (client disconnected without closing) are cleaned up after a timeout, not leaked
- [ ] **Startup reindexing:** Often missing progress indication -- verify that the server reports health as "degraded" or "indexing" during the startup rebuild, not "ready" before the index is populated
- [ ] **Cross-folder search:** Often missing scope validation -- verify that search results are scoped to the user's allowed folders, not returning results from all folders
- [ ] **CriticMarkup validation:** Often missing malformed input handling -- verify that the system rejects CriticMarkup with unclosed delimiters or nested markup gracefully

## Recovery Strategies

When pitfalls occur despite prevention, how to recover.

| Pitfall | Recovery Cost | Recovery Steps |
|---------|---------------|----------------|
| Search index corruption (Tantivy) | LOW | Delete index directory, restart server. Startup reindexing rebuilds from Y.Docs. Data loss: zero (source of truth is Y.Docs, not the index). |
| OOM kill of relay server | MEDIUM | Docker `--restart unless-stopped` auto-recovers. But all WebSocket connections drop and clients must reconnect. CRDT state in Y.Docs is persisted to R2, so no data loss. |
| MCP session state lost | LOW | AI assistants reconnect and re-establish context. No persistent state to lose (search is stateless query). |
| Stale search index (debounce too aggressive) | LOW | Reduce debounce duration. Or trigger manual reindex via admin endpoint. |
| CriticMarkup corruption in document | HIGH | Must manually edit the Y.Doc to fix malformed CriticMarkup. CRDTs make "undo the last AI edit" difficult because operations are merged, not stacked. Consider keeping a pre-AI-edit snapshot. |
| WebSocket FD leak exhausting file descriptors | MEDIUM | Restart relay server. The `ulimit` workaround extends time-to-failure but doesn't fix the root cause. Long-term: investigate and fix the leak in the connection cleanup code. |

## Pitfall-to-Phase Mapping

How roadmap phases should address these pitfalls.

| Pitfall | Prevention Phase | Verification |
|---------|------------------|--------------|
| REST-wrapper MCP design | MCP design (before implementation) | Review tool count (<15), test with LLM inspector, measure token usage per tool call |
| Memory exhaustion on VPS | Architecture decisions (process separation, Tantivy config) | Monitor RSS over 48h test; verify MmapDirectory is used; verify IndexWriter heap budget is set |
| Stale CRDT search index | Search indexer implementation | Automated test: write to Y.Doc, wait 5s, verify search finds new content; verify deleted content not found |
| Wrong MCP transport | MCP foundation (first implementation task) | Verify two concurrent MCP clients can connect and get independent results |
| Stdout corruption | MCP implementation (logging config) | Test with MCP Inspector; verify no stdout output besides JSON-RPC |
| CriticMarkup injection | MCP tool implementation (edit tools) | Test: create doc with existing CriticMarkup, search it, verify clean results; test nested markup rejection |
| Unbounded tool output | MCP tool implementation | Test: search query matching 100+ docs, verify paginated response under 2000 tokens |
| Search index on restart | Startup sequence | Measure startup time with 200 docs; verify search works within 30s of server start |
| Session isolation | MCP session management | Test: two concurrent sessions searching different content, verify no cross-contamination |

## Sources

- [How Not to Write an MCP Server - Towards Data Science](https://towardsdatascience.com/how-not-to-write-an-mcp-server/) (MCP anti-patterns) - MEDIUM confidence
- [MCP Best Practices - Phil Schmid](https://www.philschmid.de/mcp-best-practices) (tool design patterns) - MEDIUM confidence
- [MCP Implementation Tips and Pitfalls - Nearform](https://nearform.com/digital-community/implementing-model-context-protocol-mcp-tips-tricks-and-pitfalls/) (protocol gotchas) - MEDIUM confidence
- [MCP Specification 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25) (authoritative protocol spec) - HIGH confidence
- [Configuring MCP Servers for Multiple Connections - MCPcat](https://mcpcat.io/guides/configuring-mcp-servers-multiple-simultaneous-connections/) (concurrency patterns) - MEDIUM confidence
- [Search Indexer Architecture - Yjs Community](https://discuss.yjs.dev/t/search-indexer-architecture/520) (CRDT indexing challenges) - MEDIUM confidence
- [Tantivy GitHub - MmapDirectory documentation](https://github.com/quickwit-oss/tantivy) (memory management) - HIGH confidence
- [tokio-tungstenite Issue #195](https://github.com/snapview/tokio-tungstenite/issues/195) (WebSocket memory leak) - HIGH confidence
- [rust-mcp-sdk on Lib.rs](https://lib.rs/crates/rust-mcp-sdk) (Rust MCP SDK) - MEDIUM confidence
- Codebase analysis of `link_indexer.rs`, `doc_connection.rs`, `doc_sync.rs`, `server.rs` (existing patterns) - HIGH confidence
- [Handling Large Text Output from MCP Server - GitHub Discussion](https://github.com/orgs/community/discussions/169224) (context window limits) - MEDIUM confidence

---
*Pitfalls research for: MCP server + keyword search for CRDT collaborative document system*
*Researched: 2026-02-08*
