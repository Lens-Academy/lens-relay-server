# Research Summary: Discord Discussion Panel

**Project:** lens-editor Discord chat integration
**Synthesized:** 2026-02-08
**Overall Confidence:** HIGH

This document synthesizes technical research across stack selection, feature prioritization, architectural patterns, and critical pitfalls for embedding a Discord chat panel in the lens-editor web application.

---

## Stack Recommendation

The recommended stack balances maturity, simplicity, and operational safety by using battle-tested libraries for Discord interaction and minimal dependencies for browser communication.

### Discord Bot Sidecar (Node.js/TypeScript Service)

| Component | Choice | Rationale |
|-----------|--------|-----------|
| **Discord Client** | discord.js 14.25.x | De facto standard. Handles gateway reconnection, heartbeat, rate limits automatically. v14.25.1 stable (Jan 2026). Modular but pragmatic to use the full package vs. separate `@discordjs/core` + `@discordjs/ws` + `@discordjs/rest` packages. |
| **HTTP Server** | Hono 4.11.x + @hono/node-server | Lightweight (14kB), TypeScript-first, built-in `streamSSE()` helper. Far simpler than Express for a 3-endpoint sidecar. Better SSE ergonomics than Fastify. |
| **Runtime** | Node.js + TypeScript 5.9 | Match lens-editor version. Use `tsx` for dev, compile with `tsc` for production. |

### Frontend (lens-editor Additions)

| Component | Choice | Rationale |
|-----------|--------|-----------|
| **SSE Client** | Native EventSource API | Zero dependencies. Built-in reconnection. Works in all browsers. Wrapped in a `useDiscordChat()` React hook. |
| **Markdown Parser** | discord-markdown-parser 1.3.1 | Parses Discord-flavored markdown (spoilers, mentions, custom emoji) into AST. Actively maintained (Feb 2026). MIT licensed, small enough to fork if needed. |
| **HTTP Client** | Native fetch | Built-in browser API. No axios needed. |

### Rejected Alternatives

- **WidgetBot / iframe embeds**: Requires OAuth, not customizable, brings Discord UI chrome
- **WebSocket (sidecar-to-browser)**: SSE is the right abstraction for unidirectional server push; WebSocket adds unnecessary complexity
- **Socket.IO**: Massive dependency for one-way event streaming
- **Pushing events through Rust relay server**: Couples unrelated concerns, complicates upstream merges
- **Express / Fastify**: Dead weight for a 3-endpoint API sidecar

**Key Technical Dependencies:**
- Discord bot token (from Developer Portal)
- MESSAGE_CONTENT privileged intent (approved for bots in <100 servers without review)
- Webhook URLs per channel (stored server-side only)

---

## Feature Landscape

Features are categorized into table stakes (expected), differentiators (unique value), and anti-features (explicit scope boundaries).

### Table Stakes (Must Have for MVP)

These 10 features are launch-blocking -- missing any makes the product feel broken:

1. **Live message stream** (T1) - Real-time updates via Gateway → SSE
2. **Message history on load** (T2) - Last 25-50 messages fetched from REST API
3. **Post messages via webhook** (T3) - Participate, not just lurk
4. **Self-reported display name** (T4) - Identity for posting (persisted in localStorage)
5. **Basic markdown rendering** (T5) - Bold, italic, code, quotes (discord-markdown-parser)
6. **Timestamps** (T6) - Relative ("2m ago") for recent, absolute for older
7. **Author identification** (T7) - Username + avatar; distinguish bot/webhook messages
8. **Scroll behavior** (T8) - Auto-scroll to newest; stop on scroll-up; "new messages" indicator
9. **Loading/error states** (T9) - Spinner, error messages, retry button
10. **Panel toggle/resize** (T10) - Show/hide, remember state

### Differentiators (Unique Value Propositions)

| Feature | Value | Complexity | Priority |
|---------|-------|------------|----------|
| **D1: Document-aware channel mapping** | The killer feature -- chat automatically shows the channel linked in the document's frontmatter. No other embed is context-aware. | Medium | MUST HAVE |
| **D3: "(unverified)" tag on webhook names** | Transparent trust model. Users know who is verified (Discord account) vs. self-reported. | Low | Should Have Soon |
| **D4: Unread count badge** | Shows unread messages when panel is collapsed. WidgetBot's Crate does this. | Medium | Should Have Soon |
| **D8: Message edit/delete reflection** | Updates chat when Discord messages are edited/deleted. Important for accuracy. | Medium | Should Have Soon |
| **D5: Syntax-highlighted code blocks** | Relevant for developer audience. | Medium | Defer to Post-MVP |
| **D6: User mention rendering** | Resolve `<@id>` to `@Username` with styling. | Medium | Defer to Post-MVP |
| **D7: Emoji rendering** | Unicode (easy) + custom Discord emoji (harder). | Medium | Unicode for MVP, custom deferred |
| **D11: Link previews/embed rendering** | Discord embeds with title, description, images. | High | Defer to Post-MVP |
| **D12: Attachment display** | Images inline, files as download links. | High | Download links for MVP, inline images deferred |

### Anti-Features (Explicit Boundaries)

These are common mistakes or scope traps to actively avoid:

| Anti-Feature | Why Avoid |
|--------------|-----------|
| **A1: Full Discord client reproduction** | Impossible maintenance burden. Focus on single-channel chat panel. Users open Discord for full functionality. |
| **A2: Discord OAuth login** | Complexity (tokens, refresh, permissions) creates friction. Webhook posting with self-reported names is simpler. |
| **A3: Channel switching UI** | Breaks the value proposition -- channels are auto-selected from document context, not user-chosen. |
| **A4: Authenticated Discord account posting** | Requires OAuth + token proxying + Discord ToS concerns. Webhook posting is the designed use case. |
| **A5: Reactions / threading** | Complex subsystems. Display reactions read-only if needed; do not allow adding. Ignore threads initially. |
| **A6: File upload from panel** | Large attack surface. Show download links for Discord attachments; do not allow uploads. |
| **A7: Message editing/deleting from panel** | Webhook messages cannot be edited by sender. Fire-and-forget with follow-up clarifications. |
| **A8: Typing indicators** | Marginal value. Messages appear fast enough via live stream. |
| **A9: Member list / online status** | Focus is conversation, not presence. Omit or show simple count if needed. |
| **A10: Custom CSS theming engine** | Distraction. Match editor's existing design system. Support dark/light mode via existing mode. |
| **A11: Notification sounds** | Annoying, requires audio permission. Visual-only (unread count badge). |

**MVP Scope:** T1-T10 + D1 (document-aware mapping). This is 11 features, predominantly Low-Medium complexity.

---

## Architecture

The architecture uses a standalone Node.js sidecar service to bridge Discord and browser clients, keeping the existing Rust relay server untouched.

### Component Diagram

```
Discord API (Gateway + REST)
       |
       v
discord-bridge (Node.js sidecar)
  - discord.js Client (Gateway connection)
  - SSE broadcast server (HTTP)
  - Webhook/history proxy (HTTP)
       |
   SSE (events) + REST (history, posting)
       |
       v
lens-editor (Browser)
  - useDiscordChat() hook (EventSource + fetch)
  - DiscordPanel component (React)
       |
   WebSocket (yjs sync, unchanged)
       |
       v
relay-server (Rust/Axum, unchanged)
```

### Why a Separate Sidecar (Not Embedded in Rust Relay)

| Factor | Separate Node.js Sidecar | Embedded in Rust Relay |
|--------|--------------------------|------------------------|
| Library maturity | discord.js is battle-tested, handles gateway automatically | Rust Discord libs (serenity, twilight) are capable but smaller ecosystem |
| Deployment independence | Restart/update bridge without touching document sync | Bridge crash could affect document sync |
| Relay server scope | Upstream fork with minimal custom changes; Discord adds complexity | Tighter coupling, harder upstream merges |
| Development velocity | JavaScript/TypeScript matches frontend, easier iteration | Rust compile times, different skill set |
| Operational risk | Bridge failure only affects chat; documents continue working | Gateway issues could impact relay performance |

**Recommendation:** Separate Node.js sidecar. HIGH confidence.

### Why SSE (Not WebSocket) for Browser Communication

| Factor | SSE | WebSocket |
|--------|-----|-----------|
| Direction needed | Server-to-client only (correct fit) | Bidirectional (overkill) |
| Existing WS connection | Does not interfere with yjs WebSocket | Adding a second WS creates confusion |
| Reconnection | Built-in automatic reconnection | Must implement manually |
| HTTP/2 multiplexing | Works over existing HTTP/2 connection | Requires separate TCP connection |
| Complexity | Minimal (native browser EventSource) | Requires ws library, upgrade handling, state management |
| Client-to-server | Use regular fetch() POST | Could use same connection |

**Recommendation:** SSE for server-to-client, regular HTTP POST for client-to-server. HIGH confidence.

### Data Flow Patterns

**Reading history (on panel open):**
Browser → GET /channels/:id/messages → discord-bridge → Discord REST API → proxy response → render in DiscordPanel

**Live message streaming:**
Discord Gateway → MESSAGE_CREATE → discord-bridge → SSE broadcast to subscribed clients → append to message list

**Posting a message:**
Browser → POST /channels/:id/send → discord-bridge → Discord webhook → echoes back via Gateway → appears in chat via SSE

**Frontmatter detection:**
Editor loads Y.Doc → parse frontmatter → extract `discussion` field → parse Discord URL → open SSE connection + fetch history

### discord-bridge Internal Components

**DiscordClient** (`discord-client.ts`): Connects to Gateway with `Guilds`, `GuildMessages`, `MessageContent` intents. Listens for `messageCreate`, `messageUpdate`, `messageDelete`. Emits normalized events to SSEManager.

**SSEManager** (`sse-manager.ts`): Maintains `Map<channelId, Set<Response>>` of connected clients. Fans out Discord events to subscribed SSE streams. Sends keepalive every 30s.

**Routes:**
- `GET /events?channels=CHANNEL_ID` - SSE stream, registers client
- `GET /channels/:id/messages?before=MSG_ID&limit=50` - Proxies Discord REST API with bot token
- `POST /channels/:id/send` - Validates payload, looks up webhook URL, executes webhook

### Browser-Side Integration

**`useDiscordChat(channelId)` hook:**
- EventSource connection lifecycle
- Maintains message array state
- Fetches initial history on mount
- Appends live messages from SSE
- Provides `sendMessage(content, username)` function

**`DiscordPanel` component:**
- Renders in right sidebar (like CommentsPanel, BacklinksPanel)
- Scrollable message list with auto-scroll
- Message compose input with username field
- Mounted conditionally based on `discussion` frontmatter

### Build Order (Respects Technical Dependencies)

1. **Phase 1: discord-bridge core** - Gateway connection, REST history proxy. Test with curl. Validates bot can connect.
2. **Phase 2: SSE fan-out** - SSE endpoint, wire messageCreate events, keepalive. Validates real-time streaming.
3. **Phase 3: Webhook proxy** - POST endpoint, username formatting, rate limit handling. Validates posting works.
4. **Phase 4: useDiscordChat hook** - EventSource management, history fetch, sendMessage(). Test in isolation.
5. **Phase 5: DiscordPanel component** - Message list, compose, frontmatter detection, sidebar integration. Full end-to-end.
6. **Phase 6: Vite proxy + deployment** - Dev proxy config, Docker container, production routing.

**Critical path:** Phases 1→2→3 (backend), then 4→5 (frontend), then 6 (infrastructure).

**Parallelization:** Phase 3 (webhook) can run parallel to Phase 2 (SSE). Phase 4 (hook) can start once Phase 2 is available.

### Deployment

Sidecar runs as Docker container on same Hetzner VPS as relay server. Vite proxies `/api/discord/*` to sidecar (port 8190). In production, Cloudflare Tunnel or nginx routes `/api/discord/*` to bridge container.

---

## Critical Pitfalls

These are the top 7 mistakes with project-blocking or production-outage consequences, extracted from the 16 pitfalls researched.

### 1. Gateway Identify Exhaustion (1000/day Hard Limit)

**Problem:** Bot crash loop burns through 1000 identify calls in <1 hour. Discord terminates all sessions, resets token, and requires manual intervention. Total outage for up to 24 hours.

**Prevention:**
- Always attempt **Resume** (opcode 6) before Identify. discord.js handles this automatically if you don't kill the process.
- Implement exponential backoff: 1s, 2s, 4s, 8s... up to 60s on reconnection.
- Process supervisor with max restart rate (e.g., systemd `RestartSec=5`, `StartLimitBurst=10`).
- Monitor identify calls; alert if >100/hour.

**Phase:** Infrastructure setup. Must be addressed before production.

### 2. MESSAGE_CONTENT Intent Not Enabled in Both Places

**Problem:** Bot connects but `message.content` is empty string. Chat panel shows blank messages. Or gateway closes with close code 4014 (Disallowed Intents) and bot cannot connect.

**Cause:** Intent must be enabled in (1) Developer Portal AND (2) code (GatewayIntentBits.MessageContent). Missing one silently degrades or fails.

**Prevention:**
- Setup checklist with Developer Portal screenshot.
- Startup health check: send test message, read back via gateway, verify content is non-empty.
- Document in README.

**Phase:** Bot setup (Phase 1). Day-one configuration requirement.

### 3. Webhook URL Leakage Enables Channel Spam

**Problem:** Webhook URL exposed to browser (network tab, frontend code, debug logs). Anyone with URL can POST arbitrary messages -- @everyone pings, spam, scam links. Webhooks bypass Discord AutoMod.

**Cause:** Naive architecture sends webhook URLs to client for direct posting, or sidecar includes URL in error responses.

**Prevention:**
- **Never expose webhook URLs to browser.** Sidecar proxies all posting. Browser POSTs to sidecar; sidecar forwards to Discord.
- Store webhook URLs in environment variables on sidecar, never in frontend code.
- Rate limit sidecar POST endpoint (1 msg / 2 sec per client IP).
- Always set `allowed_mentions: { parse: [] }` to suppress @everyone/role/user pings.

**Phase:** Architecture design (Phase 1). Proxy pattern from day one.

### 4. Bot Token Exposed in Client Bundle or Git History

**Problem:** Bot token ends up in frontend bundle, git commit, or .env file in repo. Leaked token gives attackers complete bot control. Discord auto-detects some leaks and resets token (surprise outage).

**Cause:** Copy-paste mistakes. Using `VITE_` prefix (exposes to client). Committing .env files.

**Prevention:**
- Bot token ONLY in sidecar environment, never in frontend.
- Add `.env` to `.gitignore` before writing any env file.
- Use `DISCORD_BOT_TOKEN` (no `VITE_` prefix).
- Pre-commit check: `grep -r "VITE_.*TOKEN\|VITE_.*SECRET" lens-editor/src/`
- Docker multi-stage builds so tokens in build stage are not in final image.

**Phase:** Project initialization. Establish token management before writing bot code.

### 5. SSE 6-Connection Browser Limit (HTTP/1.1)

**Problem:** Users open 7+ tabs. Browser enforces 6 concurrent HTTP/1.1 connections per domain. Tab 7+ silently fails -- no SSE connection, no error message. REST requests may also hang.

**Cause:** SSE uses one long-lived TCP connection per EventSource. HTTP/1.1 limits connections per domain.

**Prevention:**
- Serve sidecar over HTTP/2 (or HTTP/3). HTTP/2 multiplexes streams over single TCP connection (default limit: 100 streams). This eliminates the problem.
- If HTTP/2 not feasible initially, use long-polling fallback or `SharedWorker` to share SSE connection across tabs.
- Document limitation for HTTP/1.1 dev environments.

**Phase:** Sidecar infrastructure (Phase 2). Can ignore during local dev; must solve before production.

### 6. Webhook Rate Limits Are Per-Webhook AND Per-Server

**Problem:** Multiple users post simultaneously. Sidecar hits rate limits unexpectedly. Discord rate limit is 5 req / 2 sec per webhook. Evidence suggests all webhooks in same server may share one rate limit bucket.

**Cause:** Fire-and-forget webhook POSTs without queueing. Misunderstanding of rate limit buckets (per-webhook vs. per-channel vs. per-server).

**Prevention:**
- Server-side message queue in sidecar. Do not fire-and-forget. Token bucket or sliding window enforcing 5/2s per webhook.
- Read and obey `X-RateLimit-Remaining` and `X-RateLimit-Reset` headers from every response.
- On 429 (rate limited), honor `retry_after` value. Do not retry immediately.
- Set user expectations: messages may be delayed a few seconds during active discussions.

**Phase:** Message posting (Phase 2-3). Basic rate limit handling from the start; queue becomes important as usage grows.

### 7. Webhook Username Validation Failures

**Problem:** User enters display name containing "clyde", "discord", "everyone", "here". Webhook POST fails with 400 Bad Request. Chat panel shows generic error. User cannot post.

**Cause:** Discord blacklists substrings in webhook `username` field (undocumented). Names <1 or >80 characters also fail.

**Prevention:**
- Client-side validation before sending:
  - Length: 1-80 characters after trimming
  - Blacklist substrings (case-insensitive): "clyde", "discord"
  - Blacklist exact names (case-insensitive): "everyone", "here", "system message"
  - Strip excessive whitespace
- Append " (unverified)" server-side (in sidecar), not client-side.
- Specific error messages: "Display names cannot contain 'discord' or 'clyde'."

**Phase:** Display name and posting (Phase 2).

### Other Notable Pitfalls (Non-Critical but Important)

- **P8: Zombie Gateway Connections** - Missed heartbeat ACK. Bot appears connected but stops receiving events. Forward discord.js status events to SSE clients.
- **P9: Allowed Mentions Not Set** - User sends `@everyone`, pings entire server. Always set `allowed_mentions: { parse: [] }` on every webhook execution.
- **P10: Message History Returns Empty Content** - MESSAGE_CONTENT intent affects REST API too, not just Gateway. Same fix as P2.
- **P11: Invalid HTTP Requests Trigger IP Ban** - 10,000 invalid requests in 10 minutes = 24-hour IP ban. Never retry 401/403; discord.js handles most cases.
- **P13: Forum Thread Channels Require Parent ID Awareness** - Forum post URLs have 3 path segments. Parse Discord URLs carefully (last segment is thread ID).

---

## Implications for Requirements

### v1 Scope Definition

**MVP is 11 features:** T1-T10 (table stakes) + D1 (document-aware channel mapping).

This scope is deliberately minimal:
- **Read-first**: Users can see conversations and understand context without posting.
- **Defer rich rendering**: Plain text or basic markdown initially. Inline images, custom emoji, syntax highlighting are post-MVP.
- **No moderation features**: Relying on Discord's server-level moderation and the `allowed_mentions` restriction. No bot-side filtering beyond display name validation.
- **No multi-channel navigation**: One document = one channel. No channel picker.

**Post-MVP (Phase 2):**
- D3: "(unverified)" tag (simple, high trust value)
- D4: Unread count badge (engagement driver)
- D7: Unicode emoji rendering (easy win)
- D8: Edit/delete reflection (accuracy)
- D10: Connection status indicator (confidence)

**Post-MVP (Phase 3+):**
- D5: Syntax-highlighted code blocks
- D6: User mention resolution
- D11: Rich embed rendering
- D12: Inline image display

### Non-Functional Requirements from Pitfalls

1. **Security:**
   - Webhook URLs never exposed to browser
   - Bot token never in client code or git
   - Rate limiting on all posting endpoints
   - `allowed_mentions: { parse: [] }` on all webhook executions
   - No Discord messages persisted to disk (only in-memory cache with short TTL)

2. **Reliability:**
   - Gateway reconnection with exponential backoff
   - Process supervisor with restart rate limiting to avoid identify exhaustion
   - SSE reconnection with jittered retry header
   - Health check on startup (test message content retrieval)

3. **Performance:**
   - HTTP/2 for SSE (or SharedWorker fallback) to avoid 6-connection limit
   - Message history cache (in-memory, 30-60s TTL) to reduce Discord API calls
   - Webhook rate limit queue (5 req / 2 sec per webhook)

4. **Compliance:**
   - Discord Developer Policy: short-lived in-memory caches only
   - MESSAGE_CONTENT privileged intent enabled (approved for bots <100 servers)

### Open Questions for Requirements Phase

1. **Frontmatter convention:** What format for the `discussion` field? Just a Discord URL, or a structured object with channel ID + webhook URL + display config?
2. **Missing frontmatter UX:** If a document has no `discussion` field, does the panel show "No discussion linked" or disappear entirely?
3. **Permission model:** Does the sidecar need per-user auth, or is it open to anyone with lens-editor access? (Current design: open to anyone with editor access.)
4. **Multi-server support:** If documents link to channels in multiple Discord servers, does the bot need to be in all of them? (Current design: yes, but MVP targets one server.)
5. **Deployment topology:** Should the sidecar run as a standalone service accessible to all lens-editor users, or should each user run their own instance? (Current design: single shared sidecar.)

---

## Open Questions

### Unresolved Technical Questions

1. **Shared webhook rate limit bucket:** GitHub issue #6753 reports that all webhooks in a server may share one rate limit bucket, not per-webhook buckets. If confirmed, this changes rate limiting strategy (need per-server queue instead of per-webhook).

2. **Webhook username override race condition:** GitHub issue #5953 reports that simultaneous webhook posts can cause the second message's username to retroactively display on the first. Reproduction conditions unclear. May require serialization of webhook requests.

3. **SSE over HTTP/1.1 in production:** If Cloudflare Tunnel or nginx uses HTTP/1.1 (not HTTP/2), the 6-connection limit applies. Verify tunnel supports HTTP/2 or plan SharedWorker fallback.

4. **discord-markdown-parser edge cases:** Package is small (~300 LOC) and actively maintained, but may have edge cases with unusual Discord formatting. Plan to fork if needed (MIT license).

### Unresolved Product Questions

1. **Historical message depth:** Should the panel load more than the initial 50 messages? Infinite scroll backward, or hard cap?

2. **Thread handling:** Discord forum posts are threads. Should the panel support replies in threads, or just the root post's messages?

3. **Reaction display:** Should reactions on Discord messages be shown read-only, or omitted entirely in MVP?

4. **Bot identity:** What username/avatar should the bot have in Discord? Should it match the lens-relay branding?

5. **Multiple discussion channels per document:** Should a document support multiple `discussion` links (e.g., one for technical discussion, one for feedback)? Current design assumes one channel per document.

---

*Research synthesized: 2026-02-08*

**Next Steps:**
1. Review this summary with stakeholders to confirm scope and priorities.
2. Define v1 requirements document based on MVP feature set and non-functional requirements.
3. Create roadmap with phases 1-6 build order and timelines.
4. Set up Discord bot in Developer Portal and verify MESSAGE_CONTENT intent approval.
