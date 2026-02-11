# Phase 1: Search Index - Research

**Researched:** 2026-02-08
**Domain:** Full-text search (tantivy) embedded in a Rust relay server (axum)
**Confidence:** HIGH

## Summary

This phase adds a tantivy-based full-text search index to the existing relay server. The relay server is a Rust/axum application that manages Y.Doc CRDT documents, stores them in S3/filesystem, and already has an in-memory document map (`DashMap<String, DocWithSyncKv>`) plus a link indexer that observes document updates with debounced processing. The search index follows the same architectural pattern.

The standard approach is straightforward: tantivy is the Rust ecosystem's canonical full-text search library (BM25 scoring, mmap-backed indexes, built-in snippet generation). The relay server already loads all documents into memory at startup via `load_all_docs()` and reindexes backlinks via `startup_reindex()`. The search index will hook into the same lifecycle: build the index at startup from loaded docs, observe live updates via the same webhook callback mechanism the link indexer uses, and serve results via a new axum route.

Key technical decisions are well-supported: tantivy's `MmapDirectory` keeps memory usage low (OS page cache manages it), `SnippetGenerator` produces highlighted snippets natively, `QueryParser::set_conjunction_by_default()` gives AND semantics, and `delete_term()` + `add_document()` handles incremental updates. The 4GB VPS constraint is easily met since tantivy was designed for low-memory mmap operation.

**Primary recommendation:** Add tantivy 0.25 to y-sweet-core, create a `SearchIndex` struct following the `LinkIndexer` pattern (DashMap-based pending queue, debounced background worker, mpsc channel), and wire it into Server alongside the link indexer.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| tantivy | 0.25.0 | Full-text search engine (indexing, BM25 scoring, snippets, query parsing) | The canonical Rust search library. 14k+ GitHub stars, used by Quickwit, ParadeDB. Mmap-backed, low memory, <10ms startup. |
| serde / serde_json | 1.0 (already in workspace) | JSON serialization for search API responses | Already a dependency. |
| axum | 0.7.4 (already in workspace) | HTTP endpoint for search API | Already the web framework. |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| tokio | 1.29+ (already in workspace) | Async runtime for debounced indexing worker | Already a dependency. Background worker uses `tokio::spawn`, `mpsc::channel`, `tokio::time::sleep`. |
| dashmap | 6.0.1 (already in workspace) | Thread-safe pending update tracking | Already a dependency. Same pattern as LinkIndexer. |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| tantivy | Custom inverted index | No. BM25 scoring, snippet generation, query parsing are complex. tantivy does this in ~1 dependency. |
| MmapDirectory | RamDirectory | RamDirectory keeps everything in anonymous memory. MmapDirectory lets the OS manage page cache, much safer for 4GB VPS. Only use RamDirectory for tests. |

**Installation (add to `crates/y-sweet-core/Cargo.toml`):**
```toml
tantivy = "0.25"
```

No additional feature flags needed -- the defaults include `mmap`, `lz4-compression`, `stopwords`.

## Architecture Patterns

### Recommended Module Structure
```
crates/y-sweet-core/src/
  search_index.rs      # SearchIndex struct, schema, indexing, querying
  lib.rs               # Add `pub mod search_index;`

crates/relay/src/
  server.rs            # Add search_index field to Server, wire into routes/startup
```

The search index lives in `y-sweet-core` (like `link_indexer.rs`) because it needs access to `DocWithSyncKv` and `DashMap<String, DocWithSyncKv>`. The HTTP endpoint lives in `server.rs` (like all other routes).

### Pattern 1: SearchIndex Struct (mirrors LinkIndexer)

**What:** A `SearchIndex` struct that owns the tantivy `Index`, `IndexWriter`, and `IndexReader`, with a debounced background worker for live updates.

**When to use:** Always -- this is the core pattern.

**Key design:**
```rust
use tantivy::{
    schema::{Schema, Field, TEXT, STORED, STRING},
    Index, IndexWriter, IndexReader,
    query::QueryParser,
    collector::TopDocs,
    snippet::SnippetGenerator,
    directory::MmapDirectory,
    doc,
};
use dashmap::DashMap;
use tokio::sync::mpsc;
use std::sync::{Arc, Mutex};

pub struct SearchIndex {
    index: Index,
    schema: Schema,
    // Fields
    title_field: Field,
    body_field: Field,
    doc_id_field: Field,
    folder_field: Field,
    // Writer wrapped in Mutex (single writer constraint)
    writer: Mutex<IndexWriter>,
    // Reader (clone-safe, auto-reloads on commit)
    reader: IndexReader,
    // Debounce infrastructure (same pattern as LinkIndexer)
    pending: Arc<DashMap<String, tokio::time::Instant>>,
    index_tx: mpsc::Sender<String>,
}
```

**Critical tantivy constraints:**
- Only ONE `IndexWriter` per index (tantivy enforces this via file lock). Wrap in `Mutex`.
- `IndexReader` is clone-safe and lightweight. Create one, keep it for the lifetime of the server.
- Documents become searchable only after `index_writer.commit()`. The reader auto-reloads on `ReloadPolicy::OnCommitWithDelay`.
- Updates = delete old document by term + add new document + commit. tantivy has no in-place update.

### Pattern 2: Schema Design

**What:** Define the tantivy schema with fields matching the CONTEXT.md decisions.

```rust
fn build_schema() -> (Schema, Field, Field, Field, Field) {
    let mut builder = Schema::builder();

    // Title: tokenized + stored + boosted at query time
    let title = builder.add_text_field("title", TEXT | STORED);

    // Body: tokenized + stored (needed for snippet generation)
    let body = builder.add_text_field("body", TEXT | STORED);

    // doc_id: stored + indexed as term (for delete_term)
    // STRING = not tokenized, indexed as single term
    let doc_id = builder.add_text_field("doc_id", STRING | STORED);

    // folder: stored only (not searchable in v1, for future filtering)
    let folder = builder.add_text_field("folder", STORED);

    let schema = builder.build();
    (schema, title, body, doc_id, folder)
}
```

**Key decisions:**
- `body` MUST be `STORED` for snippet generation. `SnippetGenerator` needs the stored text.
- `doc_id` uses `STRING` (not `TEXT`) so it is indexed as a single term, enabling `delete_term()`.
- `title` uses `TEXT | STORED` -- tokenized for search, stored for result display.
- `folder` uses `STORED` only -- present in results but not searchable in v1.
- Title boosting is done at query time via `QueryParser::set_field_boost(title, 2.0)`, not schema time.

### Pattern 3: Document Indexing (from Y.Doc)

**What:** Extract title and body from the Y.Doc ecosystem and add to tantivy.

**How to get content:**
```rust
// Body: read Y.Text("contents") from content doc
let awareness = doc_ref.awareness();
let guard = awareness.read().unwrap();
let txn = guard.doc.transact();
if let Some(contents) = txn.get_text("contents") {
    let body = contents.get_string(&txn);
    // ... index body
}
```

**How to get title:** Iterate filemeta_v0 in folder docs to find the path for a given UUID, then strip `.md` and extract basename:
```rust
// In folder doc:
// filemeta_v0: "/Meeting Notes.md" -> { "id": "uuid-123", ... }
// Title for uuid-123 = "Meeting Notes"
let path = "/Meeting Notes.md";
let title = path
    .strip_prefix('/')
    .and_then(|s| s.strip_suffix(".md"))
    .unwrap_or(path)
    .rsplit('/')
    .next()
    .unwrap_or(path);
```

This is the same pattern used in `detect_renames()` in `link_indexer.rs` (line 469-477).

### Pattern 4: Startup Full Index Build

**What:** At server startup, after `load_all_docs()`, iterate all folder docs to build a (uuid -> title, folder_name) map, then iterate all content docs to index title + body.

**Flow:**
1. `load_all_docs()` -- already exists, loads all Y.Docs from storage
2. `search_index.build_initial_index(&docs)` -- new method:
   a. Find all folder docs (has non-empty filemeta_v0)
   b. Build uuid-to-metadata map: `{ uuid -> (title, folder_name) }`
   c. For each content doc: read body from Y.Text("contents"), look up title from map
   d. Add document to tantivy index writer
   e. Commit once after all documents are added
3. Return 503 for `/search` requests until initial indexing completes (use an `AtomicBool` flag)

### Pattern 5: Live Update Debounce

**What:** Same debounce pattern as LinkIndexer -- hook into the same webhook callback, use mpsc channel + DashMap pending map.

**Key difference from link indexer:** On folder doc updates, the search indexer needs to detect title changes (renames) and re-index affected content docs with the new title. On content doc updates, re-index the body. Both follow the same debounce pattern.

**Integration point:** The webhook callback in `load_doc_with_user()` (server.rs line 360-368) already notifies the link indexer. Add a parallel notification for the search indexer.

### Pattern 6: Search Query Execution

**What:** Parse user query, execute against index, generate snippets, return JSON.

```rust
// Create query parser with AND semantics
let mut query_parser = QueryParser::for_index(&index, vec![title_field, body_field]);
query_parser.set_conjunction_by_default(); // AND semantics
query_parser.set_field_boost(title_field, 2.0); // Boost title matches

// Parse with lenient mode (tolerates minor syntax errors)
let (query, _errors) = query_parser.parse_query_lenient(user_query);

// Search
let searcher = reader.searcher();
let top_docs = searcher.search(&query, &TopDocs::with_limit(limit))?;

// Generate snippets
let mut snippet_gen = SnippetGenerator::create(&searcher, &*query, body_field)?;
snippet_gen.set_max_num_chars(150);
// Custom <mark> tags instead of default <b>
// Use snippet.highlighted() ranges + snippet.fragment() to build custom output
```

**Important:** Use `parse_query_lenient()` instead of `parse_query()` for user-facing search. The strict parser returns errors on malformed input; lenient mode does best-effort parsing.

### Anti-Patterns to Avoid
- **Creating IndexWriter per request:** tantivy only allows ONE writer per index. It must be shared (via Mutex) across the server lifetime.
- **Not committing after writes:** Documents are invisible until `commit()`. Always commit after indexing.
- **Using RamDirectory in production:** Wastes anonymous memory. Use MmapDirectory for the 4GB VPS.
- **Storing body without STORED flag:** SnippetGenerator requires the field to be STORED to generate snippets.
- **Tokenizing doc_id:** Use STRING (not TEXT) for doc_id to keep it as a single term for delete_term().

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| BM25 scoring | Custom scoring algorithm | tantivy built-in BM25 | Complex formula (term frequency, inverse doc frequency, field length normalization). tantivy implements it identically to Lucene. |
| Snippet generation | Custom text excerpt extraction | tantivy's `SnippetGenerator` | Handles tokenization, term matching, fragment selection, and highlighting. Custom snippets would need to re-implement the tokenizer to find matches. |
| Query parsing | Custom query string parser | tantivy's `QueryParser` | Handles quoted phrases, tokenization, multi-field search, boosting. Use `set_conjunction_by_default()` for AND semantics. |
| Inverted index | Custom HashMap-based index | tantivy `Index` with `MmapDirectory` | FST-based term dictionary, compressed postings lists, mmap-backed. Orders of magnitude more memory-efficient than naive approaches. |
| Debounce logic | Custom debounce implementation | Copy LinkIndexer pattern exactly | Already battle-tested in this codebase. Same DashMap + mpsc + Instant approach. |

**Key insight:** tantivy is a complete search engine library. Everything from indexing to querying to snippet generation is built-in. The only custom code needed is the glue between Y.Doc content and tantivy documents, and the HTTP endpoint.

## Common Pitfalls

### Pitfall 1: IndexWriter Single-Thread Constraint
**What goes wrong:** Attempting to create multiple IndexWriters for the same index, or calling `index_writer.commit()` from multiple threads simultaneously.
**Why it happens:** tantivy enforces a single-writer constraint via file lock. The writer internally uses a thread pool for indexing, but the API must be called from one logical owner.
**How to avoid:** Wrap `IndexWriter` in `Mutex<IndexWriter>`. Lock it for add/delete/commit operations. Keep lock duration minimal (add_document is fast, commit is heavier but still sub-second for small indexes).
**Warning signs:** "Failed to acquire lock" errors at runtime.

### Pitfall 2: Forgetting to Commit
**What goes wrong:** Documents are added to the index but never appear in search results.
**Why it happens:** tantivy documents are only visible after `commit()`. The `IndexReader` only sees committed data.
**How to avoid:** Always call `commit()` after batch operations. For the debounced worker, commit after each document reindex (the debounce already batches rapid updates).
**Warning signs:** Search returns stale or empty results despite documents being present.

### Pitfall 3: Body Field Not Stored
**What goes wrong:** `SnippetGenerator::create()` returns an error or empty snippets.
**Why it happens:** SnippetGenerator reads the stored field value to find match positions and extract fragments. If the field is not STORED, there is no text to generate snippets from.
**How to avoid:** Use `TEXT | STORED` for the body field.
**Warning signs:** Empty snippets, or errors from `snippet_from_doc()`.

### Pitfall 4: Memory Pressure from Large Body Content
**What goes wrong:** The tantivy doc store grows large because all body text is stored.
**Why it happens:** `STORED` compresses text (LZ4 by default) but still uses disk space. With MmapDirectory, the OS manages page cache, so RAM impact is controlled. But the on-disk index size matters.
**How to avoid:** MmapDirectory is the right choice -- the OS will page out unused data. For ~200-500 documents with typical markdown content (~1-50KB each), total index size will be well under 100MB. This is fine for the 4GB VPS.
**Warning signs:** High disk usage in the index directory. Monitor with `df`.

### Pitfall 5: Deleting Documents by Wrong Term
**What goes wrong:** `delete_term()` deletes wrong documents or nothing at all.
**Why it happens:** If doc_id is indexed as TEXT (tokenized), the term will be split into tokens. `delete_term(Term::from_field_text(doc_id_field, "some-uuid"))` won't match because the stored tokens are different from the original string.
**How to avoid:** Use `STRING` (not TEXT) for the doc_id field. STRING indexes the value as a single, untokenized term.
**Warning signs:** Documents not being deleted on reindex, leading to duplicate results.

### Pitfall 6: Snippet HTML Tag Mismatch
**What goes wrong:** Snippets use `<b>` tags instead of `<mark>` tags as specified in the API contract.
**Why it happens:** tantivy's default snippet HTML uses `<b>` tags.
**How to avoid:** Use custom snippet rendering via `snippet.highlighted()` and `snippet.fragment()` to wrap matches in `<mark>` tags, or call `snippet.set_snippet_prefix_postfix("<mark>", "</mark>")` before `to_html()`. Note: `set_snippet_prefix_postfix` is on `Snippet`, not `SnippetGenerator`.
**Warning signs:** API responses have `<b>` instead of `<mark>`.

### Pitfall 7: Blocking the Tokio Runtime with Tantivy Operations
**What goes wrong:** Search queries or commits block the async runtime, causing timeouts on other requests.
**Why it happens:** Tantivy operations are CPU-bound and synchronous. Running them directly in an async handler blocks the tokio worker thread.
**How to avoid:** Use `tokio::task::spawn_blocking()` for search queries and commit operations. The IndexWriter Mutex lock should also be held inside `spawn_blocking`.
**Warning signs:** Slow response times on unrelated endpoints during search queries or indexing.

## Code Examples

### Example 1: Creating the Index with MmapDirectory
```rust
use tantivy::{Index, directory::MmapDirectory};
use std::path::Path;
use std::fs;

fn create_or_open_index(index_path: &Path, schema: Schema) -> tantivy::Result<Index> {
    fs::create_dir_all(index_path)?;
    let dir = MmapDirectory::open(index_path)?;
    // open_or_create: opens existing index or creates new one
    Index::open_or_create(dir, schema)
}
```
Source: tantivy docs (verified via docs.rs)

### Example 2: Indexing a Document
```rust
fn index_document(
    writer: &Mutex<IndexWriter>,
    doc_id_field: Field,
    title_field: Field,
    body_field: Field,
    folder_field: Field,
    doc_id: &str,
    title: &str,
    body: &str,
    folder: &str,
) -> tantivy::Result<()> {
    let mut writer = writer.lock().unwrap();
    // Delete existing document first (idempotent reindex)
    let term = tantivy::Term::from_field_text(doc_id_field, doc_id);
    writer.delete_term(term);
    // Add new version
    writer.add_document(doc!(
        doc_id_field => doc_id,
        title_field => title,
        body_field => body,
        folder_field => folder,
    ))?;
    writer.commit()?;
    Ok(())
}
```
Source: tantivy examples/deleting_updating_documents.rs (verified)

### Example 3: Searching with Snippets
```rust
use tantivy::snippet::{Snippet, SnippetGenerator};

fn search(
    reader: &IndexReader,
    query_parser: &QueryParser,
    body_field: Field,
    query_str: &str,
    limit: usize,
) -> tantivy::Result<Vec<SearchResult>> {
    let searcher = reader.searcher();
    let (query, _errors) = query_parser.parse_query_lenient(query_str);
    let top_docs = searcher.search(&query, &TopDocs::with_limit(limit))?;

    let mut snippet_gen = SnippetGenerator::create(&searcher, &*query, body_field)?;
    snippet_gen.set_max_num_chars(150);

    let mut results = Vec::new();
    for (score, doc_address) in top_docs {
        let doc: TantivyDocument = searcher.doc(doc_address)?;
        let snippet = snippet_gen.snippet_from_doc(&doc);

        // Custom <mark> tag rendering
        let snippet_html = render_snippet_with_mark(&snippet);

        results.push(SearchResult {
            doc_id: /* extract from doc */,
            title: /* extract from doc */,
            folder: /* extract from doc */,
            snippet: snippet_html,
            score,
        });
    }
    Ok(results)
}

fn render_snippet_with_mark(snippet: &Snippet) -> String {
    let fragment = snippet.fragment();
    let mut result = String::new();
    let mut start = 0;
    for range in snippet.highlighted() {
        result.push_str(&fragment[start..range.start]);
        result.push_str("<mark>");
        result.push_str(&fragment[range.clone()]);
        result.push_str("</mark>");
        start = range.end;
    }
    result.push_str(&fragment[start..]);
    result
}
```
Source: tantivy examples/snippet.rs (verified, adapted for custom tags)

### Example 4: Axum Search Endpoint
```rust
#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize { 20 }

async fn handle_search(
    State(server_state): State<Arc<Server>>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = params.limit.min(100); // Cap at 100
    let q = params.q.trim().to_string();

    if q.is_empty() {
        return Ok(Json(json!({
            "results": [],
            "total_hits": 0,
            "query": ""
        })));
    }

    let search_index = server_state.search_index.as_ref()
        .ok_or_else(|| AppError(StatusCode::SERVICE_UNAVAILABLE, anyhow!("Search index not ready")))?;

    // Run search in blocking context (tantivy is sync)
    let results = tokio::task::spawn_blocking(move || {
        search_index.search(&q, limit)
    }).await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.into()))?
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(json!({
        "results": results,
        "total_hits": results.len(), // Approximate; exact count requires Count collector
        "query": params.q
    })))
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| tantivy 0.21 (old API) | tantivy 0.25 (latest, Aug 2025) | Aug 2025 | `TantivyDocument` replaces `Document` as the concrete type. `searcher.doc::<TantivyDocument>(addr)` syntax. |
| parse_query only | parse_query + parse_query_lenient | tantivy 0.19+ | Lenient mode is better for user-facing search -- does not error on malformed queries. |
| RamDirectory common | MmapDirectory preferred | Always | MmapDirectory is the production default. RamDirectory only for tests. |

**Deprecated/outdated:**
- `Document` type name: Now `TantivyDocument` in 0.25 (the old `Document` trait is generic). Use `TantivyDocument` for the concrete stored document type.
- `searcher.doc(&doc_address)` syntax: Now `searcher.doc::<TantivyDocument>(doc_address)?` with explicit type parameter.

## Open Questions

1. **Exact total_hits count**
   - What we know: `TopDocs::with_limit(N)` returns up to N results with scores, but not a total hit count. Getting exact total_hits requires a `Count` collector run in parallel.
   - What's unclear: Whether the performance cost of a Count collector is acceptable for v1.
   - Recommendation: For v1, return `results.len()` as total_hits (which is min(actual_hits, limit)). This is sufficient for MCP and UI consumers. True total_hits can be added later with `MultiCollector` if needed.

2. **Index persistence across restarts**
   - What we know: MmapDirectory persists the index to disk. On restart, the existing index can be opened.
   - What's unclear: Whether to re-use the persisted index or rebuild from Y.Docs on every startup.
   - Recommendation: Rebuild from scratch on every startup (same as link indexer reindex). The document count is small (~200-500), indexing is fast (<1 second), and this avoids stale-index bugs. The MmapDirectory path can be a tempdir or a fixed path that gets cleared on startup.

3. **IndexWriter memory budget**
   - What we know: tantivy's `index.writer(N)` allocates N bytes for the indexing buffer. Common values are 50MB-100MB.
   - What's unclear: Optimal budget for a small index on a 4GB VPS.
   - Recommendation: Use 15MB (`15_000_000`). The index is tiny (~200-500 small docs). 50MB is overkill and wastes memory. 15MB is generous for this scale.

## Sources

### Primary (HIGH confidence)
- tantivy docs.rs (0.25.0) -- Schema, QueryParser, SnippetGenerator, IndexWriter, MmapDirectory API
- tantivy GitHub examples (snippet.rs, basic_search.rs, deleting_updating_documents.rs) -- Verified code patterns
- tantivy ARCHITECTURE.md -- Design principles, segment model, mmap strategy
- Codebase: `crates/y-sweet-core/src/link_indexer.rs` -- Debounce pattern, Y.Doc access, folder doc scanning
- Codebase: `crates/relay/src/server.rs` -- Server struct, routing, startup flow, webhook callback integration
- Codebase: `crates/y-sweet-core/src/doc_sync.rs` -- DocWithSyncKv, awareness access pattern

### Secondary (MEDIUM confidence)
- ParadeDB tantivy introduction -- Confirmed architecture overview, BM25 defaults
- tantivy crates.io page -- Confirmed v0.25.0 as latest (Aug 2025)
- tantivy GitHub issues #550 -- Confirmed IndexWriter single-thread design, Mutex pattern

### Tertiary (LOW confidence)
- None -- all findings verified with primary sources

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- tantivy is the only serious choice for embedded Rust search. Version, API, and features verified via docs.rs.
- Architecture: HIGH -- Based on direct codebase analysis of the existing link indexer pattern plus verified tantivy API patterns.
- Pitfalls: HIGH -- All pitfalls derived from tantivy's documented constraints (single writer, STORED requirement, commit semantics) plus codebase-specific concerns (async/sync boundary, Y.Doc access patterns).

**Research date:** 2026-02-08
**Valid until:** 2026-03-08 (tantivy 0.25 is stable, no breaking changes expected in 30 days)
