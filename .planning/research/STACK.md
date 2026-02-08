# Technology Stack: Discord Chat Integration

**Project:** lens-editor Discord chat panel
**Researched:** 2026-02-08
**Overall confidence:** MEDIUM-HIGH

## Context

Adding an interactive Discord chat panel to the lens-editor web client. Documents have frontmatter linking to Discord channels/forum threads. The panel shows message history, streams live messages via a bot gateway connection, and lets users post via webhook with self-reported name. The existing lens-editor stack is React 19, TypeScript, Vite 7, TailwindCSS 4, CodeMirror 6, yjs. The relay backend is Rust/Axum. A new Node.js/TypeScript sidecar service will run the Discord bot.

---

## Recommended Stack

### Discord Bot Sidecar (Node.js Service)

| Technology | Version | Purpose | Confidence | Why |
|------------|---------|---------|------------|-----|
| **discord.js** | 14.25.x | Discord gateway + REST client | HIGH | De facto standard. 14.25.1 is latest stable (published Jan 2026). Modular internally but the main package is the pragmatic choice -- it bundles `@discordjs/rest`, `@discordjs/ws`, and types. No reason to use lower-level `@discordjs/core` for this use case; the overhead is negligible and discord.js gives you typed Client events, caching, and reconnection out of the box. |
| **hono** | ^4.11 | HTTP server for SSE + REST endpoints | HIGH | Lightweight (14kB), TypeScript-first, built-in `streamSSE()` helper. Runs on Node.js via `@hono/node-server`. Far simpler than Express for an API-only sidecar with no middleware requirements. Better SSE ergonomics than Fastify. |
| **@hono/node-server** | ^1.19 | Node.js adapter for Hono | HIGH | Required to run Hono on Node.js (vs. Cloudflare Workers / Bun). Published monthly, latest 1.19.9. |
| **TypeScript** | ~5.9 | Type safety | HIGH | Match the lens-editor version for consistency. |
| **tsx** | latest | Dev runner (ts-node replacement) | MEDIUM | Fast TypeScript execution for development. For production, compile with `tsc` and run with `node`. |

### Frontend (lens-editor additions)

| Technology | Version | Purpose | Confidence | Why |
|------------|---------|---------|------------|-----|
| **Native EventSource API** | Browser built-in | SSE client for live messages | HIGH | Zero dependencies. EventSource has built-in reconnection with configurable retry. Works in all browsers. A custom React hook (`useDiscordChat`) wraps it. No npm package needed. |
| **discord-markdown-parser** | ^1.3.1 | Parse Discord markdown to AST | MEDIUM | Actively maintained (published Feb 2026). Parses Discord-flavored markdown (bold, italic, strikethrough, spoilers, code blocks, mentions) into an AST you can render with React. Based on `simple-markdown`. |
| **Native fetch** | Browser built-in | POST webhook messages, fetch history | HIGH | Already available in modern browsers and the sidecar. No axios needed. |

### NOT Using (and Why)

| Technology | Why Not |
|------------|---------|
| **WidgetBot / iframe embeds** | Requires OAuth user login; no self-reported name posting; heavy; not customizable enough for our panel UX; brings in Discord's own UI chrome which clashes with our editor. |
| **`@discordjs/core` + `@discordjs/ws` + `@discordjs/rest` (separate packages)** | Unnecessary complexity for a single-bot sidecar. The main `discord.js` package wraps these internally with better ergonomics (typed events, automatic caching, reconnection). Only use the separate packages if building a framework or sharding across processes. |
| **discord.js v15** | Pre-release as of Feb 2026. Guide docs exist but no stable npm release. Stick with v14.25.x which is actively maintained. |
| **Express / Fastify** | Express is dead weight for a tiny API sidecar. Fastify is fine but Hono is lighter, has native SSE streaming, and TypeScript-first without decoration overhead. |
| **WebSocket (sidecar-to-browser)** | SSE is the right choice here. The data flow is unidirectional: sidecar pushes Discord events to browser. Posting goes through a separate REST endpoint. SSE is simpler (auto-reconnect, no ping/pong, works through HTTP proxies/tunnels natively). WebSocket is only justified if bidirectional streaming is needed. |
| **Socket.IO** | Massive dependency for a one-way event stream. SSE does the job with zero client-side dependencies. |
| **Pushing Discord events through the Rust relay server** | The relay server is upstream y-sweet. Adding Discord event bridging would create coupling and complicate upstream merges. A separate sidecar keeps concerns isolated. |
| **react-discord-message** | "Work in progress" status, low adoption. Better to write a thin custom renderer over `discord-markdown-parser` AST output -- more control, less dependency risk. |
| **discord-markdown (original)** | Last published 4 years ago. Effectively abandoned. The `discord-markdown-parser` package is the actively maintained successor. |

---

## Detailed Rationale

### Why discord.js (not raw Discord API)

**Confidence: HIGH** (verified via npm, GitHub releases, official docs)

The Discord gateway is a stateful WebSocket protocol requiring:
- Heartbeat management (or the bot disconnects)
- Resume/reconnect logic (session recovery after drops)
- Intent negotiation
- Rate limit handling on REST calls
- Proper op-code sequencing (IDENTIFY, RESUME, etc.)

Reimplementing this from raw `ws` + `fetch` is roughly 500-1000 lines of boilerplate that discord.js already handles. The library is the clear ecosystem winner: 14.25.1 stable, published Jan 2026, massive community, TypeScript-native.

The modular `@discordjs/core` package exists for advanced use cases (custom sharding, process distribution), but for a single-bot sidecar reading one server's messages, the full `discord.js` Client is the right abstraction.

### Why SSE (not WebSocket) for Sidecar-to-Browser

**Confidence: HIGH** (architectural pattern analysis)

The data flow is:

```
Discord Gateway --> Bot sidecar --> SSE --> Browser
Browser --> REST POST --> Bot sidecar --> Discord Webhook
```

SSE wins because:
1. **Unidirectional fit**: Messages flow server-to-client only. Posting is a separate action via REST.
2. **Auto-reconnect**: EventSource reconnects automatically with `retry` header. No manual reconnect logic needed in the React client.
3. **HTTP-native**: Works through Cloudflare tunnels, nginx proxies, and SSH tunnels without upgrade negotiation issues.
4. **Simpler server code**: Hono's `streamSSE()` is ~10 lines vs. WebSocket connection management.
5. **No client dependency**: Browser `EventSource` API is built-in.

The one downside (no binary data) is irrelevant -- we are streaming JSON message events.

### Why Hono (not Express/Fastify)

**Confidence: HIGH** (verified via npm, official docs)

The sidecar needs exactly three endpoints:
1. `GET /channels/:id/messages` -- fetch history (proxied REST call to Discord API)
2. `GET /channels/:id/events` -- SSE stream of live messages
3. `POST /channels/:id/messages` -- post via webhook

Hono is ideal for this because:
- Built-in `streamSSE()` helper (import from `hono/streaming`)
- TypeScript-first, no `@types` packages needed
- ~14kB, minimal dependency surface
- v4.11.9 actively maintained (published Feb 2026)
- Runs on Node.js via `@hono/node-server`

### Why discord-markdown-parser (not custom parsing)

**Confidence: MEDIUM** (package is small; may need to fork/extend)

Discord markdown is NOT standard markdown. It has:
- `||spoiler||` syntax
- `<@user_id>` mentions
- `<#channel_id>` channel links
- `<t:timestamp:format>` timestamps
- Custom emoji `<:name:id>`
- `>>> block quotes` (triple chevron)

`discord-markdown-parser` (v1.3.1, Feb 2026) parses these into an AST. We write a small React renderer (~100 lines) that walks the AST and renders `<span>`, `<code>`, `<strong>`, etc. For MVP (plain text first), most of this can be deferred -- just extract text content from the AST.

Risk: The package is small and may have edge cases. Mitigation: it is MIT licensed and simple enough to fork if needed.

### Discord API Essentials

**Confidence: HIGH** (verified via Discord Developer Portal docs)

| Concept | Details |
|---------|---------|
| **Bot Token** | Single token authenticates both gateway and REST. Store in env var, never in client code. |
| **Gateway Intents** | Need `GUILD_MESSAGES`, `MESSAGE_CONTENT` (privileged). For bots in <100 servers, enable in Developer Portal without approval. |
| **Message History** | `GET /channels/{id}/messages?limit=50&before={id}` -- paginate backwards with `before` param. Max 100 per request. |
| **Webhook Posting** | `POST {webhook_url}` with `{ content, username, avatar_url }`. Overrides webhook display name per-message. Rate limit: 5 requests / 2 seconds per webhook. |
| **Forum Threads** | Forum posts are threads. Each has a `channel_id`. Read messages with the same channel messages endpoint. |
| **Rate Limits** | Global: 50 req/s. Per-channel messages: 5/5s. discord.js handles rate limit queuing automatically. |

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────┐
│                    Browser                          │
│                                                     │
│  lens-editor (React 19)                             │
│  ┌───────────────────────────────────────────────┐  │
│  │  DiscordPanel component                       │  │
│  │  ├── useDiscordChat() hook                    │  │
│  │  │   ├── EventSource  ← SSE ←─────────────┐  │  │
│  │  │   └── fetch POST  → REST →──────────┐  │  │  │
│  │  ├── MessageList (virtual scroll)      │  │  │  │
│  │  └── ComposeBox                        │  │  │  │
│  └────────────────────────────────────────│──│──┘  │
└───────────────────────────────────────────│──│──────┘
                                            │  │
                                            │  │
┌───────────────────────────────────────────│──│──────┐
│  Discord Sidecar (Node.js / Hono)        │  │      │
│                                           │  │      │
│  GET  /channels/:id/events  ──────────────┘  │      │
│  POST /channels/:id/messages ────────────────┘      │
│  GET  /channels/:id/messages (history)              │
│                                                     │
│  discord.js Client ←── Gateway WS ──→ Discord API  │
│  (maintains live connection, receives events)       │
│                                                     │
│  Webhook URLs stored in config / env                │
└─────────────────────────────────────────────────────┘
```

---

## Installation Plan

### Sidecar Service (new `discord-sidecar/` directory)

```bash
# Initialize
mkdir discord-sidecar && cd discord-sidecar
npm init -y

# Runtime dependencies
npm install discord.js@^14.25.1 hono@^4.11 @hono/node-server@^1.19

# Dev dependencies
npm install -D typescript@~5.9 tsx @types/node@^24
```

### Frontend (lens-editor additions)

```bash
cd lens-editor

# Only one new dependency needed for Discord markdown parsing.
# SSE client (EventSource) and fetch are browser built-ins.
npm install discord-markdown-parser@^1.3
```

### Environment Variables (sidecar)

```env
DISCORD_BOT_TOKEN=<bot token from Developer Portal>
DISCORD_WEBHOOK_URL_DEFAULT=<webhook URL for default channel>
# Optional: map channel IDs to webhook URLs
DISCORD_WEBHOOK_MAP='{"channel_id":"webhook_url"}'
SIDECAR_PORT=8190
```

---

## Version Verification Log

All versions verified via web search on 2026-02-08:

| Package | Claimed Version | Verification Method | Last Published |
|---------|-----------------|--------------------|--------------------|
| discord.js | 14.25.1 | npm search + discord.js.org/docs | Jan 2026 (~23 days ago) |
| hono | 4.11.9 | npm search | Feb 2026 (same day) |
| @hono/node-server | 1.19.9 | npm search | Jan 2026 (~1 month ago) |
| @discordjs/rest | 2.6.0 | npm search | Sep 2025 (~5 months ago) |
| @discordjs/ws | 2.0.4 | npm search | Jan 2026 (~1 month ago) |
| discord-markdown-parser | 1.3.1 | npm search + GitHub | Feb 2026 (~6 days ago) |

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| `discord-markdown-parser` has edge cases / breaks on unusual Discord formatting | MEDIUM | LOW | Package is small (~300 LOC), MIT licensed. Fork if needed. MVP uses plain text only. |
| Discord rate limits hit when many users load history simultaneously | LOW | MEDIUM | Cache message history in sidecar memory (TTL 30s). Deduplicate concurrent requests to same channel. discord.js handles REST rate limits automatically. |
| Bot gateway disconnects during sidecar restarts | LOW | LOW | discord.js auto-reconnects. SSE clients auto-reconnect. Brief gap in live messages (acceptable for chat panel). |
| MESSAGE_CONTENT privileged intent requires Discord review | LOW (for <100 servers) | HIGH (if denied) | Bot is in a single server. Privileged intents are self-service for bots in <100 servers. No review needed. |
| Hono SSE streaming doesn't work through Cloudflare tunnel | LOW | HIGH | SSE is standard HTTP. Cloudflare supports it natively. Test early in development. Fallback: polling endpoint. |

---

## Sources

- [discord.js npm package](https://www.npmjs.com/package/discord.js) - Version and publish date
- [discord.js documentation](https://discord.js.org/docs) - API reference, v14.25.1
- [discord.js guide - Webhooks](https://discordjs.guide/popular-topics/webhooks.html) - Webhook posting with custom username/avatar
- [@discordjs/rest npm](https://www.npmjs.com/package/@discordjs/rest) - Standalone REST package (v2.6.0)
- [@discordjs/ws npm](https://www.npmjs.com/package/@discordjs/ws) - Standalone gateway WS package (v2.0.4)
- [@discordjs/core npm](https://www.npmjs.com/package/@discordjs/core) - Lightweight core wrapper
- [Discord Developer Portal - Gateway](https://discord.com/developers/docs/events/gateway) - Gateway intents, events
- [Discord Developer Portal - Channel Messages](https://discord.com/developers/docs/resources/channel) - REST API for message history
- [Discord Developer Portal - Rate Limits](https://discord.com/developers/docs/topics/rate-limits) - Rate limit documentation
- [Discord Webhooks Guide](https://birdie0.github.io/discord-webhooks-guide/discord_webhook.html) - Webhook rate limits, payload format
- [MESSAGE_CONTENT Privileged Intent FAQ](https://support-dev.discord.com/hc/en-us/articles/4404772028055-Message-Content-Privileged-Intent-FAQ) - Intent requirements
- [Hono - Streaming Helper](https://hono.dev/docs/helpers/streaming) - SSE streaming documentation
- [Hono npm package](https://www.npmjs.com/package/hono) - Version 4.11.9
- [@hono/node-server npm](https://www.npmjs.com/package/@hono/node-server) - Node.js adapter v1.19.9
- [discord-markdown-parser npm](https://www.npmjs.com/package/discord-markdown-parser) - v1.3.1, Discord markdown AST parser
- [discord-markdown-parser GitHub](https://github.com/ItzDerock/discord-markdown-parser) - Source and maintenance status
- [WidgetBot](https://widgetbot.io/) - Evaluated and rejected (requires OAuth, not customizable enough)
- [Discord.js v15 Guide](https://discordjs.guide/v15) - Pre-release status confirmed
