# Discord Discussion Panel

## What This Is

An interactive Discord chat panel embedded in the lens-editor web client. When a document has a `discussion` frontmatter field linking to a Discord channel or forum thread, the editor shows that conversation alongside the document — users can read messages in real time and post via webhook using a self-reported name.

## Core Value

Users can participate in the Discord discussion about a document without leaving the editor.

## Requirements

### Validated

- ✓ Relay documents have YAML frontmatter with metadata fields (id, slug, title) — existing
- ✓ Web editor renders markdown documents with CodeMirror — existing
- ✓ Real-time document sync via WebSocket/yjs — existing
- ✓ Relay server handles document auth and WebSocket connections — existing

### Active

- [ ] Editor detects `discussion` frontmatter field pointing to a Discord channel/thread
- [ ] Discord chat panel displays message history from the linked channel
- [ ] Messages stream in live as they're posted in Discord
- [ ] Users can post messages via webhook with their self-reported name and "(unverified)" tag
- [ ] Messages show Discord username and avatar for Discord-native messages
- [ ] Supports forum thread channels
- [ ] Supports regular text channels
- [ ] Plain text message rendering (markdown/embeds/reactions deferred)

### Out of Scope

- Rich message rendering (embeds, reactions, replies, images) — v2, start with plain text
- Discord OAuth login — unnecessary complexity, self-reported name is sufficient for now
- Verified identity linking between Relay accounts and Discord — future feature
- Reusing the lens-platform Discord bot — separate systems, avoid premature coupling
- Mobile-optimized layout — editor UI overhaul planned separately

## Context

The Lens community uses Discord for discussions alongside collaborative documents in the relay. Currently users must switch between the editor and Discord manually. The `discussion` frontmatter field already exists in some documents, linking to Discord forum threads (e.g., `https://discord.com/channels/1440725236843806762/1465349126073094469`).

The existing lens-platform project has a full-featured Discord bot, but it's a separate system. Building a dedicated lightweight bot for the relay keeps the systems decoupled and avoids premature integration.

Discord webhooks allow posting messages that show a custom username and avatar, with an "APP" badge — this is acceptable for the self-reported name approach.

For live message streaming, a Discord bot connected to the gateway (WebSocket) can relay channel events to editor clients. This bot process can run as a sidecar service alongside the relay server.

## Constraints

- **Discord API**: Bot token required for reading messages and gateway events; webhook URL required per channel for posting
- **Rate limits**: Discord API has rate limits (~50 requests/second global, per-channel limits on message sends)
- **Bot permissions**: Needs MESSAGE_CONTENT intent (privileged) to read message content
- **Webhook format**: Messages sent via webhook show "APP" badge — cannot be removed
- **Stack**: Must integrate with existing React/TypeScript frontend and Vite build tooling

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Build new Discord bot vs reuse lens-platform bot | Separate systems, avoid coupling, Discord bots are simple | — Pending |
| Self-reported name with "(unverified)" tag | No Discord OAuth needed, low friction, honest about verification | — Pending |
| Plain text first for message rendering | Ship faster, iterate on richness later | — Pending |
| Sidecar service for Discord bot (not embedded in Rust relay) | Architectural decision deferred to research/planning | — Pending |

---
*Last updated: 2026-02-08 after initialization*
