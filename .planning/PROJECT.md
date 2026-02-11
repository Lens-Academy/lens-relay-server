# Lens Relay: Search & MCP

## What This Is

A keyword search index and MCP server for the Lens Relay ecosystem. The search index provides full-text BM25-ranked search across all documents in the Lens and Lens Edu shared folders, exposed to users via a search UI in lens-editor and to AI assistants via 5 MCP tools. The MCP server runs embedded in the relay server at `/mcp`, giving AI assistants the ability to list, read, search, navigate links, and propose CriticMarkup edits to relay documents.

## Core Value

AI assistants can find and work with the right documents across the knowledge base — not just access individual docs by name, but discover relevant content through search and link traversal.

## Requirements

### Validated

- ✓ Real-time collaborative document editing via WebSocket/yjs — existing
- ✓ Document persistence to Cloudflare R2 — existing
- ✓ Token-based authentication (HMAC/CWT) — existing
- ✓ Folder metadata management (filemeta_v0, docs maps) — existing
- ✓ Wikilink extraction and backlink indexing (server-side) — existing
- ✓ Web editor with CodeMirror + yjs binding — existing
- ✓ CriticMarkup rendering in editor — existing
- ✓ File attachment upload/download via presigned URLs — existing
- ✓ Webhook dispatch on document changes — existing
- ✓ Full-text keyword search index across Lens and Lens Edu folders — v1.0
- ✓ Search API accessible by both lens-editor and MCP server — v1.0
- ✓ Search UI in lens-editor (search bar, results with snippets, click-to-navigate) — v1.0
- ✓ MCP server embedded in relay at /mcp endpoint — v1.0
- ✓ MCP tool: list all documents (glob) — v1.0
- ✓ MCP tool: read document content (cat -n format) — v1.0
- ✓ MCP tool: edit document via CriticMarkup suggestions — v1.0
- ✓ MCP tool: regex keyword search across documents (grep) — v1.0
- ✓ MCP tool: backlinks and forward links (get_links, single-hop) — v1.0
- ✓ Index updates when documents change (debounced) — v1.0
- ✓ Read-before-edit enforcement per MCP session — v1.0

### Active

(No active requirements — next milestone not yet planned)

### Out of Scope

- Semantic/vector search (Pinecone, embeddings) — defer to future milestone
- Discord bot integration — requires separate codebase, defer
- Custom AuthZ / Discord OAuth — being handled separately
- Content validation — handled externally for now
- Direct document writes from MCP — CriticMarkup suggestions only, for safety without auth
- Mobile app — web-first
- SSE transport for MCP — Streamable HTTP POST sufficient for current use
- Multi-hop graph traversal — single-hop covers primary use case
- MCP Prompts — tools sufficient for v1

## Context

Shipped v1.0 with ~4,270 LOC across Rust and TypeScript.

Tech stack: Rust (relay server with tantivy search + custom MCP), TypeScript/React (lens-editor with search UI).

New modules:
- `crates/relay/src/mcp/` — 2,840 LOC Rust (JSON-RPC, sessions, transport, 5 tools)
- `crates/y-sweet-core/src/search_index.rs` — 496 LOC Rust (tantivy BM25 search)
- `crates/y-sweet-core/src/doc_resolver.rs` — 414 LOC Rust (path-UUID resolution)
- `lens-editor/src/` — 520 LOC TypeScript (useSearch hook, SearchPanel component)

95+ automated tests (80 Rust, 18 search UI). All passing.

Two shared folders indexed:
- **Lens** — main knowledge base
- **Lens Edu** — educational content

Infrastructure: Hetzner VPS (4GB RAM), Docker containers, Cloudflare R2 storage, Cloudflare Tunnel.

## Constraints

- **Runtime environment**: Hetzner VPS (4GB RAM) — search index uses tantivy MmapDirectory (memory-safe)
- **Existing stack**: Rust (relay server) + TypeScript/React (lens-editor)
- **Auth**: No custom AuthZ yet — MCP edits use CriticMarkup as safety mechanism
- **No external MCP SDK**: Custom JSON-RPC handlers (5 tools, full control over session state)
- **Deployment**: Docker containers on same VPS as relay server

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| CriticMarkup for MCP edits | No custom AuthZ yet; suggestions are safe without permission checks | ✓ Good — provides reviewable AI suggestions |
| Keyword search only (no semantic) | Keeps scope tight; semantic search deferred to future milestone | ✓ Good — BM25 sufficient for knowledge base search |
| Search index as shared service | Both lens-editor and MCP need search; avoids duplication | ✓ Good — single index, two consumers |
| MCP embedded in relay (`/mcp` endpoint) | URL-based setup for collaborators, direct access to Y.Docs and search index | ✓ Good — zero-install for AI assistants |
| Custom MCP transport (no rmcp) | 5 tools doesn't justify a framework; avoids Axum 0.7→0.8 upgrade; gives control over session state | ✓ Good — 2,840 LOC, full control |
| Read-before-edit enforcement | Session tracks read docs, rejects edits on unread docs; mirrors Claude Code's Edit tool pattern | ✓ Good — prevents blind AI edits |
| JSON-RPC parse by id field presence | Clearer than serde untagged enum; handles null id per spec | ✓ Good — clean implementation |
| Grep via regex on Y.Docs (not tantivy) | Grep is for pattern matching, search is for BM25 ranking | ✓ Good — each tool does one thing well |
| TOCTOU re-verify in edit transactions | Re-read content at write time to prevent stale edits | ✓ Good — prevents data corruption |
| 300ms debounce for search UI | Prevents API spam during typing; correct for server-side requests | ✓ Good — smooth UX |

---
*Last updated: 2026-02-11 after v1.0 milestone*
