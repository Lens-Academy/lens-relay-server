# Discord Discussion Panel

## What This Is

An interactive Discord chat panel embedded in the lens-editor web client. When a document has a `discussion` frontmatter field linking to a Discord channel or forum thread, the editor shows that conversation alongside the document — users can read messages in real time and post via bot API using a self-reported name with "(unverified)" tag.

## Core Value

Users can participate in the Discord discussion about a document without leaving the editor.

## Requirements

### Validated

- ✓ Relay documents have YAML frontmatter with metadata fields (id, slug, title) — existing
- ✓ Web editor renders markdown documents with CodeMirror — existing
- ✓ Real-time document sync via WebSocket/yjs — existing
- ✓ Relay server handles document auth and WebSocket connections — existing
- ✓ Editor detects `discussion` frontmatter field pointing to a Discord channel/thread — v1
- ✓ Discord chat panel displays message history from the linked channel — v1
- ✓ Messages stream in live as they're posted in Discord — v1
- ✓ Users can post messages via bot API with their self-reported name and "(unverified)" tag — v1
- ✓ Messages show Discord username and avatar for Discord-native messages — v1
- ✓ Supports forum thread channels — v1
- ✓ Supports regular text channels — v1
- ✓ Discord-flavored markdown rendering (bold, italic, code, quotes, strikethrough) — v1
- ✓ Auto-scroll with "new messages" indicator — v1
- ✓ Connection resilience with status indicator and retry — v1
- ✓ Display name persisted in localStorage — v1
- ✓ Bot token never exposed to browser — v1

### Active

(No active requirements — next milestone TBD)

### Out of Scope

- Rich message rendering (embeds, reactions, replies, images) — v2
- Discord OAuth login — unnecessary complexity, self-reported name is sufficient
- Verified identity linking between Relay accounts and Discord — future feature
- Reusing the lens-platform Discord bot — separate systems, avoid premature coupling
- Mobile-optimized layout — editor UI overhaul planned separately
- Discord mention resolution (`<@id>` → `@Username`) — requires additional API calls, deferred

## Context

Shipped v1 with 4,676 LOC TypeScript across 69 files.

**Architecture:**
- `discord-bridge/` — Hono-based Node.js sidecar proxy (Gateway + REST + SSE + posting)
- `lens-editor/src/components/DiscussionPanel/` — React panel with hooks, markdown renderer, compose box
- `lens-editor/src/lib/` — Shared utilities (frontmatter, discord-url, avatar, timestamp)
- `lens-editor/src/contexts/DisplayNameContext.tsx` — App-global display name state

**Tech stack additions:** Hono, discord.js, discord-markdown-parser, react-textarea-autosize, front-matter

**Known issues:**
- Discord mention placeholders show raw IDs (need API calls to resolve)
- No `.env.example` in discord-bridge for token documentation
- Orphaned `/api/gateway/status` endpoint (unused, SSE provides status)

## Constraints

- **Discord API**: Bot token required for reading messages and gateway events
- **Rate limits**: Discord API has rate limits (~50 requests/second global, per-channel limits on message sends)
- **Bot permissions**: Needs MESSAGE_CONTENT intent (privileged) to read message content
- **Bot API posting**: Messages posted via bot show user's name formatted as "Name (unverified)" in message content
- **Stack**: React/TypeScript frontend, Vite build, Hono sidecar

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Build new Discord bot vs reuse lens-platform bot | Separate systems, avoid coupling, Discord bots are simple | ✓ Good — clean separation, dedicated sidecar |
| Self-reported name with "(unverified)" tag | No Discord OAuth needed, low friction, honest about verification | ✓ Good — server-side suffix enforcement |
| Discord markdown first (not plain text) | discord-markdown-parser handles AST rendering safely | ✓ Good — shipped in v1, no XSS risk |
| Hono sidecar for Discord bridge | 14KB vs Express 572KB, TypeScript-native, perfect for proxy | ✓ Good — fast, minimal |
| Bot API instead of webhooks for posting | Simpler setup, reuses existing bot token, no webhook URL management | ✓ Good — user preferred simplicity |
| IntersectionObserver for scroll detection | 1px sentinel more reliable than scroll math | ✓ Good — clean implementation |
| AST-to-React for markdown (no innerHTML) | XSS-free rendering, React-native component tree | ✓ Good — safe and composable |
| ConnectedDiscussionPanel wrapper pattern | Separates YDocProvider context from testable component | ✓ Good — clean testing |
| 75s heartbeat timeout (2.5x interval) | Balances false positives vs detection speed | ✓ Good — reliable in practice |

---
*Last updated: 2026-02-11 after v1 milestone*
