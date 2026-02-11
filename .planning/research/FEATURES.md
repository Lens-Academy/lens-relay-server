# Feature Research

**Domain:** MCP server for collaborative document knowledge base + keyword search index
**Researched:** 2026-02-08
**Confidence:** MEDIUM (MCP ecosystem verified with official sources and multiple implementations; search feature expectations based on ecosystem survey)

## Feature Landscape

### Table Stakes (Users Expect These)

Features users assume exist. Missing these = the MCP server feels useless, or the search feels broken.

#### MCP Server Table Stakes

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| **List documents** | Every Obsidian/filesystem MCP server has this. AI needs to know what exists before it can work with anything. | LOW | Map folder doc `filemeta_v0` + `docs` maps to a list of document names/paths. Already have this data in folder Y.Docs. |
| **Read document content** | Core purpose of a knowledge-base MCP. AI cannot help with documents it cannot read. Every MCP server for notes/files exposes this. | LOW | Sync Y.Doc, extract `contents` Y.Text as markdown string. Must handle the yjs binary format. |
| **Search documents** | AI assistants need to find relevant documents without knowing exact names. Both Obsidian MCP servers and the Notion MCP expose search. This is the whole point of building the search index. | MEDIUM | Keyword search against the index. Returns document names + matching snippets. |
| **Read backlinks** | We already have backlink data in `backlinks_v0`. Exposing existing data is table stakes for a knowledge graph MCP. GraphThulhu exposes `get_links` for forward+backward links. | LOW | Read `backlinks_v0` Y.Map from folder doc. Translate UUIDs to document names via `filemeta_v0`. |
| **Read forward links** | Complement to backlinks. AI needs to traverse the link graph in both directions. GraphThulhu and Obsidian MCP servers expose this. | LOW | Already extracted during indexing. Stored per-document in the link indexer. |
| **List folders** | Users have two shared folders (Lens, Lens Edu). AI needs to know which folders exist and scope operations to them. | LOW | Enumerate loaded folder docs. Return folder names with document counts. |

#### Search Table Stakes

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| **Full-text keyword search** | Users and AI both expect to find documents by content words. This is the minimum viable search. | MEDIUM | TF-IDF or BM25 over extracted document text. Needs indexing pipeline from Y.Doc content. |
| **Search result snippets** | Showing matching context is essential. Just document names is not enough -- user/AI needs to see WHY the result matched. | LOW | Extract surrounding text around match positions. Standard in all search implementations. |
| **Search result ranking** | Results must be ordered by relevance, not arbitrary order. BM25 or TF-IDF provides this naturally. | LOW | Built into any standard full-text search library (tantivy, lunr, etc.). |
| **Search API endpoint** | Both lens-editor and MCP server consume the same search index. Needs HTTP API. | MEDIUM | REST endpoint on relay server or sidecar. Returns JSON results. |
| **Search UI in lens-editor** | Users expect to search from the web editor, not just via AI. | MEDIUM | Search input + results panel in React. Calls search API. |

### Differentiators (Competitive Advantage)

Features that set this MCP server apart from generic filesystem access. Not required, but make AI assistants significantly more effective with the knowledge base.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| **Graph traversal (N degrees)** | AI can discover related documents by walking the link graph multiple hops deep, not just direct links. Very few MCP servers offer this -- GraphThulhu does BFS traversal, but most don't. | MEDIUM | BFS/DFS from a starting document through backlinks_v0 and forward links. Return document names at each depth. Cap at configurable max depth. |
| **Edit via CriticMarkup** | AI can suggest edits without destructive writes. No other MCP server does this -- they either allow full writes (dangerous) or are read-only (useless for editing). CriticMarkup suggestions are reviewable by humans. | MEDIUM | Insert CriticMarkup syntax (`{++add++}`, `{--delete--}`, `{~~old~>new~~}`) into Y.Doc content. Requires yjs write access. Unique differentiator. |
| **Document context bundle** | Single tool call returns document content + its backlinks + its forward links + frontmatter. AI gets full context in one request instead of 3-4 separate calls. Token-efficient. | LOW | Combine read_document + get_backlinks + get_forward_links into one response. Reduces round-trips. |
| **Cross-folder link awareness** | AI can discover connections between Lens and Lens Edu folders. The system already tracks cross-folder backlinks. | LOW | Leverage existing cross-folder backlink indexing. Surface in graph traversal results. |
| **Search with link context** | Search results include backlink/forward link counts. AI can prioritize well-connected documents (likely more important). | LOW | Augment search results with link counts from backlinks_v0. |
| **MCP Resources for documents** | Expose documents as MCP Resources (not just tools). Clients can subscribe to document changes and get real-time updates. More aligned with MCP spec for data that the application manages. | MEDIUM | Requires MCP Resource protocol implementation with `resources/list`, `resources/read`, and optionally `resources/subscribe`. Complementary to tools. |
| **MCP Prompts for common workflows** | Pre-built prompt templates like "summarize this document", "find related documents", "review recent changes". Makes the MCP server immediately useful without AI needing to figure out tool composition. | LOW | Define 3-5 prompt templates that combine tool calls into useful workflows. Pure MCP spec feature. |

### Anti-Features (Commonly Requested, Often Problematic)

Features that seem good but create problems in this specific context.

| Feature | Why Requested | Why Problematic | Alternative |
|---------|---------------|-----------------|-------------|
| **Direct document writes (no CriticMarkup)** | "AI should just edit the document directly." | No AuthZ system yet. Unreviewed AI writes to a shared knowledge base are dangerous. Multiple humans collaborate on these docs via Obsidian. A rogue AI edit could corrupt content that others are actively editing. | CriticMarkup suggestions only. Humans review and accept/reject. This is a feature, not a limitation. |
| **Semantic/vector search** | "Keyword search is old-fashioned, use embeddings." | Adds significant infrastructure (embedding model, vector DB or FAISS index, GPU/API costs). Keyword search covers 80% of use cases for a knowledge base. Vector search is explicitly out of scope for this milestone. | Keyword search now. Design search API to be extensible so vector search can be added later without breaking consumers. |
| **Real-time document subscriptions via MCP** | "AI should know when a document changes." | MCP transport is typically stdio (Claude Code) or SSE (Claude Desktop). Real-time subscriptions create long-lived connections that most MCP clients don't handle well. The November 2025 spec adds async Tasks but client support is nascent. | Polling via search/read tools. The AI doesn't need real-time -- it can re-read when needed. Consider MCP Resource subscriptions later when client support matures. |
| **Document creation via MCP** | "AI should create new documents." | Creating documents requires writing to both `filemeta_v0` AND `docs` maps in the folder doc (Obsidian compatibility). Getting this wrong causes Obsidian to delete the document. Also, creating documents without human intent is more dangerous than editing existing ones. | Defer to post-AuthZ milestone. When permission system exists, allow document creation for authorized AI agents. |
| **Bulk operations (edit all, search-and-replace across docs)** | "AI should be able to refactor across the whole knowledge base." | Extremely dangerous without AuthZ. A single bad prompt could insert CriticMarkup suggestions into every document. Also creates massive yjs transactions that could overwhelm the relay server. | Single-document operations only. AI can iterate over search results one at a time. Rate limiting on edit operations. |
| **File attachment access** | "AI should be able to read images and PDFs attached to documents." | Attachments are binary files stored in R2. Most AI assistants can't do anything useful with raw binary data. Images would need OCR, PDFs need extraction. Adds complexity without clear value for the initial release. | Text content only. Document the limitation. Consider adding attachment listing (names only) as a future enhancement. |
| **Admin/management tools** | "AI should manage folders, configure settings, manage users." | MCP is for document access, not system administration. Admin operations should require human action through a dedicated interface. | No admin tools in MCP. Keep MCP focused on document content operations. |

## Feature Dependencies

```
[Search Index]
    |
    |--> requires --> [Y.Doc Content Extraction Pipeline]
    |                      |
    |                      +--> requires --> [Relay Server Document Access]
    |
    +--> enables --> [MCP: search_documents tool]
    +--> enables --> [Search UI in lens-editor]

[MCP: list_documents]
    +--> requires --> [Relay Server Document Access]
    +--> requires --> [Folder Doc Metadata Reading]

[MCP: read_document]
    +--> requires --> [Relay Server Document Access]
    +--> requires --> [Y.Doc Content Extraction]

[MCP: edit_document (CriticMarkup)]
    +--> requires --> [MCP: read_document] (must read before editing)
    +--> requires --> [Y.Doc Write Access]
    +--> requires --> [CriticMarkup Generation Logic]

[MCP: get_backlinks / get_forward_links]
    +--> requires --> [Backlinks Indexer] (already built)
    +--> requires --> [UUID-to-Name Resolution] (from filemeta_v0)

[MCP: traverse_links (N degrees)]
    +--> requires --> [MCP: get_backlinks]
    +--> requires --> [MCP: get_forward_links]
    +--> enhances --> [MCP: search_documents] (search + traverse = powerful discovery)

[MCP: get_document_context (bundle)]
    +--> requires --> [MCP: read_document]
    +--> requires --> [MCP: get_backlinks]
    +--> requires --> [MCP: get_forward_links]

[Search UI in lens-editor]
    +--> requires --> [Search API endpoint]
    +--> independent of --> [MCP server] (shared search index, independent consumers)
```

### Dependency Notes

- **Search Index requires Y.Doc Content Extraction:** Before you can index documents, you need a reliable way to extract markdown text from Y.Docs. This pipeline is shared between search indexing and MCP document reading.
- **MCP edit requires MCP read:** The AI must read a document before it can generate meaningful CriticMarkup suggestions. The edit tool should require (or at least strongly recommend) reading first.
- **Graph traversal requires backlinks + forward links:** Both must work before multi-hop traversal is possible. Backlinks already exist; forward links need to be exposed.
- **Search UI and MCP are independent consumers:** Both consume the same search API. Neither depends on the other. Can be built in parallel.

## MVP Definition

### Launch With (v1)

Minimum viable product -- what's needed to validate that AI assistants can usefully work with relay documents.

- [ ] **list_documents** -- AI needs to know what exists
- [ ] **read_document** -- AI needs to read content
- [ ] **search_documents** -- AI needs to find relevant documents (keyword search)
- [ ] **get_backlinks** -- AI can discover related documents via existing link graph
- [ ] **get_forward_links** -- Complement to backlinks for bidirectional traversal
- [ ] **Search index** -- Backend service that indexes Y.Doc content for keyword search
- [ ] **Search API** -- HTTP endpoint consumed by both MCP and lens-editor
- [ ] **Search UI** -- Basic search input + results in lens-editor

### Add After Validation (v1.x)

Features to add once core is working and AI assistants are successfully using the MCP server.

- [ ] **edit_document (CriticMarkup)** -- Add when read-only access proves useful and users want AI to suggest edits. Requires careful testing of yjs write operations.
- [ ] **traverse_links (N degrees)** -- Add when users find single-hop backlinks limiting. Needs BFS implementation and depth limiting.
- [ ] **get_document_context (bundle)** -- Add when round-trip costs prove painful. Optimization, not required for v1.
- [ ] **MCP Prompts** -- Add when usage patterns emerge. Pre-built workflows for common tasks.

### Future Consideration (v2+)

Features to defer until product-market fit is established.

- [ ] **MCP Resources** -- Defer until MCP client support for Resources matures. Currently most clients (Claude Code, Claude Desktop) handle Tools better than Resources.
- [ ] **Semantic/vector search** -- Defer to dedicated milestone. Requires embedding infrastructure.
- [ ] **Document creation** -- Defer until AuthZ exists. Too dangerous without permissions.
- [ ] **Cross-folder search scoping** -- Defer unless users specifically request searching only Lens or only Lens Edu.
- [ ] **Real-time notifications** -- Defer until MCP async Tasks are widely supported.

## Feature Prioritization Matrix

| Feature | User Value | Implementation Cost | Priority |
|---------|------------|---------------------|----------|
| list_documents | HIGH | LOW | P1 |
| read_document | HIGH | LOW | P1 |
| search_documents (keyword) | HIGH | MEDIUM | P1 |
| get_backlinks | HIGH | LOW | P1 |
| get_forward_links | MEDIUM | LOW | P1 |
| Search index service | HIGH | MEDIUM | P1 |
| Search API endpoint | HIGH | MEDIUM | P1 |
| Search UI in lens-editor | MEDIUM | MEDIUM | P1 |
| edit_document (CriticMarkup) | HIGH | MEDIUM | P2 |
| traverse_links (N degrees) | MEDIUM | MEDIUM | P2 |
| get_document_context (bundle) | MEDIUM | LOW | P2 |
| MCP Prompts | LOW | LOW | P2 |
| MCP Resources | LOW | MEDIUM | P3 |
| Semantic search | MEDIUM | HIGH | P3 |
| Document creation | LOW | MEDIUM | P3 |

**Priority key:**
- P1: Must have for launch
- P2: Should have, add when possible
- P3: Nice to have, future consideration

## Competitor Feature Analysis

| Feature | Filesystem MCP (Anthropic) | Obsidian MCP (cyanheads) | GraphThulhu (Logseq/Obsidian) | Notion MCP (Official) | Our Approach |
|---------|---------------------------|-------------------------|-------------------------------|----------------------|--------------|
| List documents | `list_directory`, `directory_tree` | `obsidian_list_notes` (folder filter) | `list_pages` (filter, sort) | `notion-search` | `list_documents` with folder scope |
| Read content | `read_text_file`, `read_multiple_files` | `obsidian_read_note` (with metadata) | `get_page` (recursive block tree) | `notion-fetch` | `read_document` with markdown content |
| Search | `search_files` (filename pattern only) | `obsidian_global_search` (text + regex) | N/A (graph-based discovery) | `notion-search` (full-text) | `search_documents` (keyword, BM25 ranking) |
| Edit content | `write_file`, `edit_file` (destructive) | `obsidian_update_note` (append/prepend/overwrite) | N/A | `notion-update-page` | `edit_document` (CriticMarkup suggestions only) |
| Backlinks | N/A | N/A | `get_links` (forward + backward) | N/A | `get_backlinks` (from existing backlinks_v0) |
| Graph traversal | N/A | N/A | `traverse` (BFS between pages) | N/A | `traverse_links` (N-degree BFS) |
| Metadata | `get_file_info` | `obsidian_manage_frontmatter` | N/A | `notion-update-page` (properties) | Via document content (frontmatter in markdown) |
| Comments | N/A | N/A | N/A | `notion-create-comment`, `notion-get-comments` | Via CriticMarkup `{>>comment<<}` syntax |
| Tags | N/A | `obsidian_manage_tags` | `find_by_tag` | N/A | Defer (tags are in frontmatter, readable via content) |
| Graph analysis | N/A | N/A | `topic_clusters`, `knowledge_gaps`, `find_connections` | N/A | Defer (advanced graph analysis is v2+) |

### Competitive Positioning

**Our unique advantages:**
1. **CriticMarkup editing** -- No other MCP server offers suggestion-based editing. Filesystem MCP does destructive writes. Obsidian MCP does append/prepend/overwrite. We offer reviewable suggestions. This is genuinely novel.
2. **Built-in link graph** -- Backlinks are already indexed server-side. Most MCP servers for notes (including Obsidian MCP by cyanheads) don't expose graph data. Only GraphThulhu does, and it requires a separate setup.
3. **Shared search index** -- Same search powers both the web editor and AI assistants. No duplication, consistent results.
4. **Real-time collaborative context** -- Documents are live Y.Docs. When the AI reads a document, it gets the current collaborative state, not a stale file on disk.

**Our limitations vs. competitors:**
1. No direct writes (by design -- safety without AuthZ)
2. No frontmatter management tools (deferred -- readable via content)
3. No advanced graph analytics (deferred -- not MVP)
4. Smaller document corpus than Notion workspaces (two shared folders vs. arbitrary workspaces)

## Sources

- [Obsidian MCP Server (cyanheads)](https://github.com/cyanheads/obsidian-mcp-server) -- 8 tools for vault interaction, HIGH confidence
- [MCP Filesystem Server (Anthropic)](https://github.com/modelcontextprotocol/servers/tree/main/src/filesystem) -- 13 tools for file operations, HIGH confidence
- [GraphThulhu](https://github.com/skridlevsky/graphthulhu) -- 37 tools for knowledge graph navigation, MEDIUM confidence
- [Notion MCP Server](https://developers.notion.com/docs/mcp-supported-tools) -- 16 tools for workspace operations, HIGH confidence
- [MCP Specification 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25) -- Resources vs Tools distinction, HIGH confidence
- [KB MCP Server (Geeksfino)](https://github.com/Geeksfino/kb-mcp-server) -- Semantic search with hybrid scoring, MEDIUM confidence
- [mcp-obsidian (bitbonsai)](https://github.com/bitbonsai/mcp-obsidian) -- 11 tools for safe vault access, MEDIUM confidence
- [Best MCP Servers for Knowledge Bases 2026](https://desktopcommander.app/blog/2026/02/06/best-mcp-servers-for-knowledge-bases-in-2026/) -- Ecosystem overview, LOW confidence
- [MCP Best Practices 2026](https://www.cdata.com/blog/mcp-server-best-practices-2026) -- Production patterns, LOW confidence

---
*Feature research for: MCP server + keyword search for CRDT document system*
*Researched: 2026-02-08*
