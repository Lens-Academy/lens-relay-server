# Stack Research

**Domain:** MCP server + keyword search index for CRDT document collaboration
**Researched:** 2026-02-08
**Confidence:** HIGH (core decisions verified with official docs + crates.io)

## Executive Decision: Language Choice

**Recommendation: Build both the MCP server and search index in Rust, as a new crate in the existing workspace.**

Rationale:

1. **The relay server already runs Rust/Axum/tokio/yrs.** The MCP server needs to read Y.Doc content (via yrs) and the search index needs to receive document updates. Building in-process avoids serialization overhead and network hops between services.

2. **rmcp (the official Rust MCP SDK) natively integrates with Axum.** The relay already uses Axum 0.7 (upgrading to 0.8 is straightforward). rmcp provides `StreamableHttpService` that nests directly into an Axum router -- the MCP endpoint can live alongside the existing relay HTTP routes.

3. **4GB RAM constraint eliminates separate-process architectures.** Running a Python MCP server + a search engine as separate processes alongside the relay would consume too much memory. An in-process Rust solution shares the relay's memory space.

4. **The Python SDK exists but is thin.** The `python/src/relay_sdk/` code is a simple HTTP client wrapper (requests + pycrdt). It does not provide in-process access to Y.Docs -- it fetches them over HTTP. For search indexing, we need the doc content as it changes, which the Rust relay already has in memory via `DashMap<String, DocWithSyncKv>`.

Why NOT Python:
- Would require running a separate process (memory overhead on 4GB VPS)
- Python MCP SDK (FastMCP) is more mature for quick prototyping, but we need deep integration with the relay's in-memory doc state
- pycrdt can read Y.Docs, but only via HTTP round-trips through the relay API -- adds latency and complexity
- Two runtimes on a 4GB VPS is wasteful

Why NOT TypeScript:
- Same separate-process problem as Python
- No access to the relay's in-memory Y.Doc state
- Would need to connect via WebSocket to read documents (another client consuming relay resources)
- The lens-editor is TypeScript, but it's a frontend -- the MCP server is backend infrastructure

## Recommended Stack

### Core Technologies

| Technology | Version | Purpose | Why Recommended |
|------------|---------|---------|-----------------|
| **rmcp** | 0.14.0 (crates.io) | MCP server SDK | Official Rust SDK, 3K+ GitHub stars, 130+ contributors. Native Axum integration via `StreamableHttpService`. Shares tokio runtime with relay. **Confidence: HIGH** (verified via docs.rs and GitHub) |
| **tantivy** | 0.25.0 | Full-text search engine | Lucene-equivalent in Rust. Memory-mapped index means near-zero resident RAM when using MmapDirectory. 2x faster than Lucene. BM25 scoring, phrase queries, snippet generation. Perfect for 4GB VPS. **Confidence: HIGH** (verified via docs.rs and GitHub) |
| **rusqlite** | 0.38.0 | Search metadata store | Optional -- for storing doc metadata (uuid-to-path mappings, index timestamps) alongside tantivy. The `bundled` feature auto-enables FTS5 as a backup/simpler search option. **Confidence: HIGH** (verified via docs.rs and GitHub build.rs) |
| **Axum** | 0.8.x | HTTP framework | rmcp requires Axum 0.8 for StreamableHttpService. Relay currently uses 0.7.4 -- upgrade required but straightforward (breaking changes are minimal: handler signatures, state extraction). **Confidence: HIGH** |
| **yrs** | 0.19.1 | Y.Doc access | Already in workspace. Used to read `getText("contents")` for indexing and `getMap("filemeta_v0")` for doc metadata. No additional dependency needed. **Confidence: HIGH** |

### Supporting Libraries

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| **schemars** | 1.0 | JSON Schema generation | Required by rmcp for tool parameter schemas. rmcp uses schemars 1.0 (2020-12 draft). |
| **serde** | 1.0 | Serialization | Already in workspace. Used for MCP message serialization and search result formatting. |
| **tokio** | 1.x | Async runtime | Already in workspace. Shared between relay, MCP server, and search indexer. |
| **tracing** | 0.1 | Structured logging | Already in workspace. MCP and search operations log through the same tracing subscriber. |

### Development Tools

| Tool | Purpose | Notes |
|------|---------|-------|
| `mcp-inspector` | MCP server testing | Official CLI tool for testing MCP servers interactively. Install via `npx @anthropic-ai/mcp-inspector`. |
| `curl` / `httpie` | HTTP testing | StreamableHTTP transport is just HTTP POST -- testable with curl. |

## Installation

```toml
# In the new crate's Cargo.toml (e.g., crates/mcp-search/Cargo.toml)

[dependencies]
# MCP server
rmcp = { version = "0.14", features = ["server", "transport-streamable-http-server", "macros"] }

# Full-text search
tantivy = "0.25"

# Metadata store (optional, start without it)
# rusqlite = { version = "0.38", features = ["bundled"] }

# Shared with relay workspace
axum = { version = "0.8", features = ["ws"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread", "signal"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tracing = "0.1"
anyhow = "1.0"

# Y.Doc access (shared with relay)
yrs = "0.19.1"

# JSON Schema for MCP tool parameters
schemars = "1.0"
```

```bash
# For MCP testing (one-time install)
npx @anthropic-ai/mcp-inspector
```

## Alternatives Considered

### MCP SDK

| Recommended | Alternative | When to Use Alternative |
|-------------|-------------|-------------------------|
| **rmcp** (Rust, official) | **mcp-python-sdk** (Python, official) | If building a standalone MCP server that does NOT need in-process access to relay Y.Doc state. Python FastMCP is easier for quick prototyping, but requires a separate process and HTTP round-trips to read documents. |
| **rmcp** (Rust, official) | **mcpkit** (Rust, community) | If rmcp's API is too verbose. mcpkit provides a `#[mcp_server]` macro that reduces boilerplate. However, mcpkit is community-maintained (lower bus factor) and may lag protocol updates. Use if rmcp's `#[tool_router]` macro proves insufficient. |
| **rmcp** (Rust, official) | **TypeScript SDK** (official) | If building a separate MCP server that connects to the relay as a WebSocket client. Only makes sense if the MCP server is deployed on a different machine. On same 4GB VPS, adds unnecessary overhead. |

### Search Engine

| Recommended | Alternative | When to Use Alternative |
|-------------|-------------|-------------------------|
| **tantivy** (Rust library) | **SQLite FTS5** (via rusqlite) | If the document corpus is very small (<100 docs) and you want zero additional dependencies. FTS5 is simpler but lacks: snippet generation with context, BM25 tuning, faceted search, and incremental merge policies. Good fallback if tantivy proves overkill. |
| **tantivy** (Rust library) | **Meilisearch** (separate service) | Never on a 4GB VPS. Meilisearch uses 6-8x more storage than SQLite FTS5 and runs as a separate process consuming 100-500MB RAM. Only if you need typo-tolerance and instant search UX, which is unnecessary for an MCP tool interface. |
| **tantivy** (Rust library) | **Sonic** (separate service) | Lightweight alternative to Meilisearch. Still a separate process. Only if you need sub-millisecond search and your corpus grows to millions of docs. Overkill for this use case. |

### MCP Transport

| Recommended | Alternative | When to Use Alternative |
|-------------|-------------|-------------------------|
| **Streamable HTTP** | **stdio** | If the MCP server runs as a subprocess spawned by Claude Desktop. Not applicable here -- the server runs on a remote VPS, so HTTP is required. |
| **Streamable HTTP** | **SSE** (legacy) | Never. SSE was deprecated in MCP spec 2025-03-26. Streamable HTTP is the replacement. |

## What NOT to Use

| Avoid | Why | Use Instead |
|-------|-----|-------------|
| **Meilisearch / Elasticsearch** | Separate process, 200-500MB+ RAM. On a 4GB VPS already running the relay, leaves too little headroom. Storage is 6-8x larger than alternatives. | tantivy (in-process, mmap-based, near-zero resident RAM) |
| **Python MCP server** (separate process) | Adds ~100MB+ for Python runtime + pycrdt. Requires HTTP round-trips to read Y.Doc content from relay. Two processes on 4GB VPS is tight. | rmcp in-process with the relay server |
| **SSE transport** | Deprecated in MCP spec (2025-03-26). Two-endpoint architecture replaced by single-endpoint Streamable HTTP. | Streamable HTTP transport |
| **MCP over stdio** | Requires local subprocess spawning. Not compatible with remote VPS deployment. | Streamable HTTP (works over network) |
| **mcp_rust_sdk** (community crate) | Multiple community SDKs exist (mcp_rust_sdk, mcp-sdk-rs, mcp-protocol-sdk). All are lower adoption than the official rmcp. Risk of protocol version drift. | rmcp (official, actively maintained, 3K+ stars) |

## Stack Patterns by Architecture

**Pattern A: MCP as separate binary (NOT recommended)**
- Separate `crates/mcp-server/` binary
- Connects to relay via HTTP API to read docs
- Runs its own tantivy instance
- Two processes on VPS (relay + mcp)
- Simpler isolation but wastes RAM and adds latency

**Pattern B: MCP integrated into relay binary (RECOMMENDED)**
- New `crates/mcp-search/` library crate
- Relay binary imports and mounts MCP routes alongside existing Axum routes
- Shares DashMap access to in-memory Y.Docs
- Tantivy index managed by the search module
- Single process, single tokio runtime, minimal overhead

Why Pattern B:
- The relay already has all Y.Doc content in memory
- The link indexer (`link_indexer.rs`) already demonstrates the pattern of background workers processing doc updates via mpsc channels
- Search indexing follows the exact same pattern: debounced doc updates -> extract text -> update tantivy index
- MCP tools can query tantivy directly without network hops

**If deploying MCP on a separate machine later:**
- Extract the MCP crate to its own binary
- Connect to relay via HTTP API (using the existing Python SDK pattern, ported to Rust)
- This is a future concern, not a present one

## Version Compatibility

| Package A | Compatible With | Notes |
|-----------|-----------------|-------|
| rmcp 0.14 | axum 0.8 | rmcp's `transport-streamable-http-server` feature depends on axum 0.8. Relay currently uses axum 0.7.4 -- **must upgrade**. |
| rmcp 0.14 | schemars 1.0 | rmcp uses schemars 1.0 with `chrono04` feature for JSON Schema generation (2020-12 draft). This is a different major version from schemars 0.8. |
| rmcp 0.14 | tokio 1.x | Compatible with the relay's existing tokio 1.29+. |
| tantivy 0.25 | tokio 1.x | tantivy's async operations use tokio 1.x. |
| tantivy 0.25 | serde 1.0 | Compatible for serializing search results. |
| axum 0.8 | axum-extra 0.9 | Relay uses axum-extra 0.9.2 -- check if it needs updating to match axum 0.8. |

**Critical upgrade note:** The Axum 0.7 -> 0.8 upgrade is the only breaking change. Key differences:
- `State` extraction changes
- Handler trait signature updates
- Should be a focused PR before MCP work begins

## Memory Budget (4GB VPS)

| Component | Estimated Memory | Notes |
|-----------|------------------|-------|
| Linux OS + services | ~500MB | Baseline |
| relay-server (current) | ~200-400MB | Y.Docs in memory, WebSocket connections |
| tantivy index (mmap) | ~10-50MB resident | MmapDirectory means only hot pages resident. Index size on disk may be larger but OS manages page cache. |
| MCP server (in-process) | ~0MB additional | Shares relay process. Per-request allocations are transient. |
| Docker overhead | ~100MB | Container runtime |
| **Total** | **~810-1050MB** | Comfortable within 4GB, leaves 3GB for page cache and spikes |

tantivy's mmap approach is critical here: the index can be larger on disk, but only accessed pages consume RAM. For a corpus of hundreds to low thousands of markdown documents, the index will be small (tens of MB on disk, single-digit MB resident).

## Sources

- [rmcp on docs.rs (v0.14.0)](https://docs.rs/rmcp/latest/rmcp/) -- verified version, API, features (HIGH confidence)
- [rmcp GitHub (official Rust SDK)](https://github.com/modelcontextprotocol/rust-sdk) -- verified stars, contributors, release count (HIGH confidence)
- [tantivy on docs.rs (v0.25.0)](https://docs.rs/tantivy/latest/tantivy/) -- verified version, modules (HIGH confidence)
- [tantivy GitHub](https://github.com/quickwit-oss/tantivy) -- verified memory model, MmapDirectory, benchmarks (HIGH confidence)
- [rusqlite on docs.rs (v0.38.0)](https://docs.rs/rusqlite/latest/rusqlite/) -- verified version (HIGH confidence)
- [rusqlite build.rs](https://github.com/rusqlite/rusqlite/blob/master/libsqlite3-sys/build.rs) -- verified bundled feature enables SQLITE_ENABLE_FTS5 (HIGH confidence)
- [MCP official SDKs page](https://modelcontextprotocol.io/docs/sdk) -- verified all official SDKs listed (HIGH confidence)
- [MCP transport spec](https://modelcontextprotocol.io/specification/2025-03-26/basic/transports) -- verified SSE deprecation, Streamable HTTP as standard (HIGH confidence)
- [Shuttle blog: Streamable HTTP MCP in Rust](https://www.shuttle.dev/blog/2025/10/29/stream-http-mcp) -- rmcp + axum integration pattern (MEDIUM confidence)
- [tantivy memory model blog](https://fulmicoton.com/posts/behold-tantivy/) -- MmapDirectory and memory characteristics (MEDIUM confidence)
- [CriticMarkup spec](https://fletcher.github.io/MultiMarkdown-6/syntax/critic.html) -- CriticMarkup syntax for MCP edit suggestions (HIGH confidence)
- Existing codebase: `crates/relay/Cargo.toml`, `crates/y-sweet-core/src/link_indexer.rs`, `python/src/relay_sdk/` -- verified current dependencies and patterns (HIGH confidence)

---
*Stack research for: MCP server + keyword search index in CRDT document collaboration system*
*Researched: 2026-02-08*
