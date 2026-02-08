# Lens Relay: Search & MCP

## What This Is

A keyword search index and MCP server for the Lens Relay ecosystem. The search index enables full-text keyword search across all documents in the Lens and Lens Edu shared folders, exposed both to users via the lens-editor UI and to AI assistants via MCP. The MCP server gives AI assistants (used by collaborators) the ability to list, read, edit, search, and navigate the link graph of relay documents.

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

### Active

- [ ] Full-text keyword search index across Lens and Lens Edu folders
- [ ] Search API accessible by both lens-editor and MCP server
- [ ] Search UI in lens-editor
- [ ] MCP server running on VPS alongside relay server
- [ ] MCP tool: list all documents
- [ ] MCP tool: read document content
- [ ] MCP tool: edit document via CriticMarkup (suggestions only)
- [ ] MCP tool: keyword search across documents
- [ ] MCP tool: traverse link graph (backlinks/forward links, N degrees deep)
- [ ] Index updates when documents change

### Out of Scope

- Semantic/vector search (Pinecone, embeddings) — defer to future milestone
- Discord bot integration — requires separate codebase, defer
- Custom AuthZ / Discord OAuth — being handled separately
- Content validation — handled externally for now
- Direct document writes from MCP — CriticMarkup suggestions only, for safety without auth
- Mobile app — web-first

## Context

This is a brownfield project building on the existing Lens Relay monorepo (Rust relay server + React lens-editor). The relay server is a y-sweet fork with custom HMAC auth and a link indexer. All document access currently goes through yjs WebSocket sync.

The backlinks system (feature #1 in the original roadmap) is being built separately — the MCP server can read backlink data from the `backlinks_v0` Y.Map in folder docs rather than computing it.

The search index is a shared service: lens-editor users get a search UI, and the MCP server queries the same index. This means the index needs its own API layer.

Transport between MCP server and relay server is TBD — could be WebSocket (yjs sync protocol) or HTTP if the relay server has suitable endpoints. Research will determine the right approach.

Two shared folders to index:
- **Lens** — main knowledge base
- **Lens Edu** — educational content

Infrastructure: Hetzner VPS, Docker containers, Cloudflare R2 storage, Cloudflare Tunnel.

## Constraints

- **Runtime environment**: Hetzner VPS (4GB RAM) — search index must be memory-conscious
- **Existing stack**: Rust (relay server) + TypeScript/React (lens-editor) — new components should align
- **Auth**: No custom AuthZ yet — MCP edits use CriticMarkup as safety mechanism
- **Transport**: Must work with existing relay server; may need to use yjs sync protocol
- **Deployment**: Docker containers on same VPS as relay server

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| CriticMarkup for MCP edits | No custom AuthZ yet; suggestions are safe without permission checks | — Pending |
| Keyword search only (no semantic) | Keeps scope tight; semantic search deferred to future milestone | — Pending |
| Search index as shared service | Both lens-editor and MCP need search; avoids duplication | — Pending |
| MCP server on VPS | Alongside relay server for low-latency local access | — Pending |

---
*Last updated: 2026-02-08 after initialization*
