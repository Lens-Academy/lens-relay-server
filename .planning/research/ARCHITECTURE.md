# Architecture: Discord-to-Browser Bridge

**Domain:** Real-time Discord chat integration in a web editor
**Researched:** 2026-02-08
**Overall confidence:** HIGH

## Executive Summary

The Discord discussion panel requires a bridge service that connects the Discord Gateway (for live message streaming) to browser clients (for display and interaction). After analyzing the existing codebase, Discord API constraints, and real-time communication patterns, the recommended architecture is a **standalone Node.js sidecar service** that maintains a single Discord Gateway connection and fans out events to browser clients via **Server-Sent Events (SSE)**. Webhook posting flows through this same sidecar as a simple HTTP proxy, keeping the Discord webhook URL server-side.

This architecture keeps the Rust relay server untouched, avoids adding a second WebSocket protocol to the browser client, and cleanly separates concerns between document sync (existing yjs/WebSocket) and chat display (new SSE stream).

## Recommended Architecture

```
                                        Discord API
                                       /          \
                              Gateway WS        REST API
                              (receive)        (history + webhooks)
                                 |                  |
                    +------------+------------------+------------+
                    |        discord-bridge (Node.js sidecar)    |
                    |                                            |
                    |  - discord.js Client (Gateway connection)  |
                    |  - Channel subscription registry           |
                    |  - SSE broadcast server (HTTP)             |
                    |  - Webhook proxy endpoint (HTTP)           |
                    |  - Message history proxy endpoint (HTTP)   |
                    +-----+--------------------+--------+--------+
                          |                    |        |
                       SSE stream         POST /send  GET /history
                     (live events)       (webhook)   (REST proxy)
                          |                    |        |
              +-----------+--------------------+--------+---------+
              |              lens-editor (Browser)                |
              |                                                   |
              |  Existing:                  New:                  |
              |  - YDocProvider (WS)        - useDiscordChat()    |
              |  - Editor, Sidebar          - DiscordPanel        |
              |  - CommentsPanel            - EventSource client  |
              |  - BacklinksPanel           - POST to /send       |
              +---------+-----------------------------------------+
                        |
                   WebSocket (yjs sync, unchanged)
                        |
              +---------+---------+
              |   relay-server    |
              |   (Rust/Axum)     |
              +-------------------+
```

## Component Boundaries

| Component | Responsibility | Communicates With | Technology |
|-----------|---------------|-------------------|------------|
| **discord-bridge** | Discord Gateway connection, SSE fan-out, webhook proxy, history proxy | Discord API, browser clients | Node.js, discord.js, native HTTP server |
| **lens-editor** (existing) | Document editing, yjs sync, UI rendering | relay-server (WebSocket), discord-bridge (SSE + HTTP) | React 19, TypeScript, Vite |
| **relay-server** (existing) | CRDT document sync, auth, file storage | lens-editor (WebSocket), cloud storage | Rust, Axum |
| **DiscordPanel** (new React component) | Chat display, message compose, frontmatter detection | discord-bridge via SSE + fetch | React, TypeScript |

### Why a Separate Node.js Service (Not Embedded in Rust)

| Factor | Separate Node.js | Embedded in Rust relay |
|--------|-----------------|----------------------|
| Discord library maturity | discord.js is battle-tested, actively maintained, handles gateway reconnection/heartbeat/sharding automatically | Rust Discord libraries (serenity, twilight) are capable but have smaller ecosystems; adding gateway management to the relay server increases its complexity |
| Deployment independence | Can restart/update the bridge without touching document sync | Any bridge crash could affect document sync |
| Existing relay server scope | Relay server is an upstream fork with minimal custom changes; adding Discord concerns makes future upstream merges harder | Tighter coupling |
| Development velocity | JavaScript/TypeScript is the same language as the frontend, easier to iterate | Rust compile times, different skill set |
| Operational risk | Bridge failure only affects Discord panel; documents continue working | Gateway reconnection storms could impact relay server performance |

**Recommendation:** Separate Node.js sidecar. HIGH confidence -- the operational isolation is the strongest argument.

### Why SSE (Not WebSocket or Polling) for Browser Communication

| Factor | SSE | WebSocket | Long Polling |
|--------|-----|-----------|-------------|
| Direction needed | Server-to-client only (chat messages arriving) | Bidirectional | Server-to-client |
| Browser already has WS | Adding a second WS connection creates confusion about which socket carries what | Reusing the existing yjs WS is not feasible (different protocol); a new WS adds complexity | N/A |
| Reconnection | Built-in automatic reconnection via EventSource API | Must implement manually | Must implement manually |
| HTTP/2 multiplexing | Works over existing HTTP/2 connection, no extra TCP handshake | Requires separate TCP connection | Works over HTTP but wasteful |
| Complexity | Minimal -- native browser EventSource, simple HTTP endpoint on server | Requires ws library, upgrade handling, connection state management | Simple but latency tradeoff |
| Client-to-server | Use regular fetch() POST for sending messages | Could use same connection | Use regular fetch() POST |

**Recommendation:** SSE for server-to-client events, regular HTTP POST for client-to-server actions. HIGH confidence -- this is the textbook use case for SSE (unidirectional server push).

## Data Flow

### Flow 1: Reading Message History (On Panel Open)

```
Browser                    discord-bridge              Discord API
  |                              |                         |
  |  GET /channels/:id/messages  |                         |
  |----------------------------->|                         |
  |                              |  GET /channels/:id/messages
  |                              |  (with bot token)       |
  |                              |------------------------>|
  |                              |                         |
  |                              |  200 OK [messages]      |
  |                              |<------------------------|
  |                              |                         |
  |  200 OK [messages]           |                         |
  |<-----------------------------|                         |
  |                              |                         |
  | (render in DiscordPanel)     |                         |
```

The bridge proxies the Discord REST API, adding the bot token server-side. The browser never sees the bot token. Pagination uses Discord's `before` parameter (max 100 messages per request). Initial load fetches the most recent 50 messages; older messages load on scroll-up.

### Flow 2: Live Message Streaming (Ongoing)

```
Browser                    discord-bridge              Discord Gateway
  |                              |                         |
  |  GET /events?channels=:id   |                         |
  |  (EventSource)               |                         |
  |----------------------------->|                         |
  |                              | (client added to        |
  |                              |  subscription set for   |
  |                              |  channel :id)           |
  |                              |                         |
  |                              |   MESSAGE_CREATE        |
  |                              |   (channel_id matches)  |
  |                              |<------------------------|
  |                              |                         |
  |  event: message_create       |                         |
  |  data: {id, content,         |                         |
  |         author, timestamp}   |                         |
  |<-----------------------------|                         |
  |                              |                         |
  | (append to message list)     |                         |
```

The bridge maintains a **single** Discord Gateway connection (one bot, one WebSocket to Discord). When it receives a `MESSAGE_CREATE` event, it checks which SSE clients are subscribed to that channel and pushes the event to each. The bridge filters gateway events by channel ID in application code -- Discord gateway intents operate at the guild level, not per-channel.

**Required Discord Gateway Intents:**
- `Guilds` -- populates guild/channel cache
- `GuildMessages` -- receives MESSAGE_CREATE events in guild channels
- `MessageContent` (privileged) -- receives actual message content, not just metadata

### Flow 3: Posting a Message (User Action)

```
Browser                    discord-bridge              Discord API
  |                              |                         |
  |  POST /channels/:id/send    |                         |
  |  {content, username}         |                         |
  |----------------------------->|                         |
  |                              |                         |
  |                              |  POST /webhooks/:id/:token
  |                              |  {content,              |
  |                              |   username: "Name (unverified)",
  |                              |   avatar_url: ...}      |
  |                              |------------------------>|
  |                              |                         |
  |                              |  200 OK                 |
  |                              |<------------------------|
  |                              |                         |
  |  200 OK                      |                         |
  |<-----------------------------|                         |
  |                              |                         |
  |                              |   MESSAGE_CREATE        |
  |                              |   (webhook message)     |
  |                              |<---- (via Gateway) -----|
  |                              |                         |
  |  event: message_create       |                         |
  |  data: {id, content,         |                         |
  |         author: webhook...}  |                         |
  |<-----------------------------|                         |
  |                              |                         |
  | (message appears in chat     |                         |
  |  via normal SSE flow)        |                         |
```

The posted message arrives back through the Gateway as a `MESSAGE_CREATE` event, so the chat updates via the same SSE flow -- no special client-side handling needed. The bridge maps channel IDs to webhook URLs from configuration.

### Flow 4: Frontmatter Detection (Document Context)

```
Browser (lens-editor)
  |
  | 1. User opens document
  | 2. Editor loads Y.Doc content
  | 3. useDiscordChat() hook reads frontmatter
  | 4. Extracts `discussion` field:
  |    "https://discord.com/channels/GUILD/CHANNEL"
  | 5. Parses channel ID from URL
  | 6. Opens SSE connection to bridge
  | 7. Fetches message history
  | 8. Renders DiscordPanel
```

Frontmatter parsing happens entirely client-side from the Y.Doc `contents` Y.Text. No server involvement needed. The `discussion` field contains a Discord channel URL from which the channel ID is extracted.

## discord-bridge Internal Architecture

```
discord-bridge/
  src/
    index.ts              # Entry point, starts HTTP server + Discord client
    discord-client.ts     # discord.js Client setup, gateway event handling
    sse-manager.ts        # SSE connection registry, fan-out logic
    routes/
      events.ts           # GET /events?channels=ID -- SSE endpoint
      messages.ts         # GET /channels/:id/messages -- history proxy
      send.ts             # POST /channels/:id/send -- webhook proxy
    config.ts             # Channel-to-webhook mapping, bot token
  package.json
  Dockerfile
```

### Key Internal Components

**DiscordClient wrapper** (`discord-client.ts`):
- Connects to Discord Gateway with `Guilds`, `GuildMessages`, `MessageContent` intents
- Listens for `messageCreate` events
- Emits normalized message objects to SSEManager
- Handles gateway reconnection automatically (discord.js built-in)

**SSEManager** (`sse-manager.ts`):
- Maintains a `Map<channelId, Set<Response>>` of connected SSE clients
- On `messageCreate` event, looks up channel subscribers and writes SSE data
- Handles client disconnect cleanup (response `close` event)
- Sends heartbeat comments (`:keepalive\n\n`) every 30 seconds to prevent proxy timeouts

**Routes**:
- `GET /events?channels=CHANNEL_ID` -- Opens SSE stream, registers client in SSEManager
- `GET /channels/:id/messages?before=MSG_ID&limit=50` -- Proxies to Discord REST API with bot token
- `POST /channels/:id/send` -- Validates payload, looks up webhook URL for channel, executes webhook

### Configuration

```typescript
// config.ts
interface BridgeConfig {
  discordBotToken: string;
  channels: {
    [channelId: string]: {
      webhookUrl: string;       // For posting messages
      guildId: string;          // For gateway filtering
    };
  };
  port: number;                 // HTTP server port (e.g., 8190)
  allowedOrigins: string[];     // CORS for lens-editor
}
```

Channel-to-webhook mapping is configured statically. Adding new channels requires updating config and restarting the bridge. This is acceptable for the initial implementation since the number of discussion channels is small and known.

## Browser-Side Integration

### New Components

**`useDiscordChat(channelId)` hook:**
- Manages EventSource connection lifecycle
- Maintains message array state
- Fetches initial history on mount
- Appends live messages from SSE
- Provides `sendMessage(content, username)` function
- Cleans up EventSource on unmount

**`DiscordPanel` component:**
- Renders in the right sidebar (similar to CommentsPanel, BacklinksPanel)
- Scrollable message list with auto-scroll on new messages
- Message compose input with username field
- Loading/error/empty states
- Mounted conditionally based on `discussion` frontmatter presence

### Integration with Existing React App

The DiscordPanel lives **outside** the `RelayProvider` key boundary. Unlike the CommentsPanel (which depends on the current Y.Doc and remounts with each document switch), the DiscordPanel's data source is the discord-bridge, not the Y.Doc.

However, it still needs to know the current document's frontmatter. Two approaches:

**Option A (recommended):** DiscordPanel receives `channelId` as a prop from EditorArea, which extracts it from the document content. The panel unmounts/remounts when `channelId` changes (using `key={channelId}`).

**Option B:** DiscordPanel reads the Y.Doc content directly via `useYDoc()` to extract frontmatter. This couples it to the RelayProvider boundary, which is fine since EditorArea already lives there.

```
EditorArea (inside RelayProvider key boundary)
  |
  +-- Editor
  +-- aside (right sidebar)
       +-- TableOfContents
       +-- BacklinksPanel
       +-- CommentsPanel
       +-- DiscordPanel (new, conditional on frontmatter)
```

### SSE Message Format

```
event: message_create
data: {"id":"123","content":"Hello","author":{"id":"456","username":"Alice","avatar":"hash","bot":false},"timestamp":"2026-02-08T12:00:00.000Z","channelId":"789"}

event: message_update
data: {"id":"123","content":"Hello (edited)","channelId":"789"}

event: message_delete
data: {"id":"123","channelId":"789"}

:keepalive
```

Events use Discord's event naming convention. The data payload is a normalized subset of Discord's message object -- enough for display, without excess fields.

## Patterns to Follow

### Pattern 1: SSE with Channel-Based Subscription

**What:** Client specifies which channel(s) to subscribe to via query parameter. Server filters gateway events and only sends relevant ones.

**When:** Always -- prevents clients from receiving events for channels they are not viewing.

```typescript
// Server: SSE endpoint
app.get('/events', (req, res) => {
  const channelIds = req.query.channels?.split(',') ?? [];

  res.writeHead(200, {
    'Content-Type': 'text/event-stream',
    'Cache-Control': 'no-cache',
    'Connection': 'keep-alive',
  });

  // Register this response in SSEManager for each channel
  for (const channelId of channelIds) {
    sseManager.subscribe(channelId, res);
  }

  req.on('close', () => {
    for (const channelId of channelIds) {
      sseManager.unsubscribe(channelId, res);
    }
  });
});
```

```typescript
// Client: React hook
function useDiscordChat(channelId: string | null) {
  const [messages, setMessages] = useState<DiscordMessage[]>([]);

  useEffect(() => {
    if (!channelId) return;

    const es = new EventSource(`/api/discord/events?channels=${channelId}`);

    es.addEventListener('message_create', (e) => {
      const msg = JSON.parse(e.data);
      setMessages(prev => [...prev, msg]);
    });

    return () => es.close();
  }, [channelId]);
}
```

### Pattern 2: Webhook URL Kept Server-Side

**What:** The browser never sees the Discord webhook URL. It sends a POST to the bridge with the channel ID and message content; the bridge resolves the webhook URL from its config.

**Why:** Webhook URLs are secrets -- anyone with the URL can post to the channel. Keeping them server-side prevents exposure via browser DevTools.

### Pattern 3: Optimistic UI Disabled for Chat

**What:** After posting a message, do NOT add it to the local message list immediately. Wait for it to arrive via the SSE stream (which happens within ~200ms as Discord echoes the webhook message back through the Gateway).

**Why:** This avoids duplicate messages and ensures the displayed message matches what Discord actually accepted (including any modifications Discord applies). The latency is imperceptible.

## Anti-Patterns to Avoid

### Anti-Pattern 1: Reusing the yjs WebSocket for Chat Events

**What:** Piggybacking Discord chat events on the existing yjs/y-sweet WebSocket connection.
**Why bad:** The y-sweet protocol is purpose-built for CRDT sync. Injecting arbitrary events breaks protocol assumptions, complicates the Rust relay server, and couples unrelated concerns. If the chat stream has issues, it could affect document sync.
**Instead:** Use a separate SSE connection for chat events.

### Anti-Pattern 2: Browser Connecting Directly to Discord API

**What:** Having the browser client call Discord's REST API directly or open a Gateway connection.
**Why bad:** Bot tokens cannot be exposed to the browser. Gateway connections from browsers are against Discord TOS for bot accounts. The browser would need the bot token for REST calls.
**Instead:** All Discord API calls go through the bridge service.

### Anti-Pattern 3: Polling Discord REST API for New Messages

**What:** Periodically fetching `/channels/:id/messages` to check for new messages instead of using the Gateway.
**Why bad:** Wastes API rate limit budget (Discord limits to ~50 req/s globally for a bot). Adds latency (polling interval). Does not scale with number of channels.
**Instead:** Use the Gateway for real-time events, REST only for initial history load.

### Anti-Pattern 4: One SSE Connection Per Channel

**What:** Opening a new SSE connection for each channel the user might view.
**Why bad:** Browsers limit concurrent HTTP connections per origin (typically 6 for HTTP/1.1). Users switching between documents would accumulate connections.
**Instead:** Use a single SSE connection with channel subscription via query parameter. When the user switches documents, close the old EventSource and open a new one with the new channel ID.

## Scalability Considerations

| Concern | Current Scale (1-10 users) | Growth (50+ users) | Notes |
|---------|---------------------------|--------------------|----|
| Gateway connection | Single bot, single shard | Single shard supports up to 2500 guilds | Not a concern for one guild |
| SSE connections | 1-10 open HTTP connections | 50+ connections; Node.js handles thousands | Standard EventSource; not a bottleneck |
| Discord API rate limits | ~5 req/2sec per webhook; 50 req/s global bot rate limit | May need per-channel webhook rate limiting in bridge | Add queue with backoff |
| Message history cache | None needed initially | Consider caching last N messages per channel to reduce API calls on panel open | Redis or in-memory LRU |
| Bridge availability | Single process, restart is fine | Consider health checks and auto-restart via Docker | Same infra as relay-server |

## Deployment

The discord-bridge runs as a Docker container on the same Hetzner VPS as the relay server. The Vite dev server proxies `/api/discord/*` to the bridge (same pattern as the existing `/api/relay` proxy for the Rust relay server).

```
# vite.config.ts addition
proxy: {
  '/api/relay': { ... },  // existing
  '/api/discord': {        // new
    target: 'http://localhost:8190',
    changeOrigin: true,
    rewrite: (path) => path.replace(/^\/api\/discord/, ''),
  },
}
```

In production, the Cloudflare Tunnel or nginx config would route `/api/discord/*` to the bridge container.

## Build Order (Dependencies Between Components)

The following build order respects technical dependencies:

```
Phase 1: discord-bridge core
  - Discord Gateway connection (discord.js Client)
  - REST proxy for message history
  - No browser code yet; test with curl/httpie
  Dependencies: Discord bot token, bot added to guild
  Validates: Bot can connect, receive events, fetch history

Phase 2: SSE fan-out
  - SSE endpoint with channel subscription
  - Wire messageCreate events to SSE stream
  - Keepalive mechanism
  Dependencies: Phase 1
  Validates: Messages stream to SSE clients in real time

Phase 3: Webhook proxy
  - POST endpoint that maps channel ID to webhook URL
  - Username formatting ("Name (unverified)")
  - Rate limit respect (honor Retry-After headers)
  Dependencies: Webhook URLs configured per channel
  Validates: Messages post to Discord and echo back via SSE

Phase 4: Frontend - useDiscordChat hook
  - EventSource connection management
  - Message history fetch on mount
  - Live message append from SSE
  - sendMessage() function
  Dependencies: Phase 2, Phase 3
  Validates: Hook works in isolation (Storybook or test harness)

Phase 5: Frontend - DiscordPanel component
  - Message list rendering
  - Compose input with username
  - Frontmatter detection
  - Integration into EditorArea sidebar
  Dependencies: Phase 4
  Validates: Full end-to-end flow in the editor

Phase 6: Vite proxy + deployment
  - Vite proxy config for dev
  - Docker container for bridge
  - Production routing
  Dependencies: Phase 5
  Validates: Works in both dev and production environments
```

**Critical path:** Phases 1-2-3 are backend (can be built and tested independently). Phases 4-5 are frontend (depend on backend being available). Phase 6 is infrastructure.

**Parallelization opportunity:** Phase 3 (webhook proxy) can be built in parallel with Phase 2 (SSE), since they are independent endpoints on the same HTTP server. Similarly, Phase 4 (hook) can start once Phase 2 is available, even before Phase 3 is complete (the hook can initially support read-only mode).

## Forum Thread vs. Text Channel Handling

Discord forum channels and text channels have slightly different API behavior:

| Aspect | Text Channel | Forum Thread |
|--------|-------------|-------------|
| Message fetch endpoint | `GET /channels/:id/messages` | Same endpoint -- thread ID works as channel ID |
| Gateway event | `MESSAGE_CREATE` with `channel_id` | Same -- thread ID appears as `channel_id` |
| Posting via webhook | Webhook attached to parent channel; use `thread_id` query param | Same webhook, specify `thread_id` in execute call |
| URL format | `discord.com/channels/GUILD/CHANNEL` | `discord.com/channels/GUILD/THREAD_ID` |

The `discussion` frontmatter URL already contains the thread/channel ID. The bridge treats them identically for message reading and SSE streaming. For webhook posting to forum threads, the bridge needs to pass `?thread_id=THREAD_ID` when executing the webhook.

**Implementation detail:** The webhook is created on the parent forum channel, but messages are posted to a specific thread via the `thread_id` parameter. The bridge config needs to know whether a channel is a forum thread (to determine which webhook URL and thread_id to use). This can be inferred from the Discord API's channel type field.

## Sources

- [Discord Gateway Documentation](https://discord.com/developers/docs/events/gateway) -- HIGH confidence (official docs)
- [Discord Gateway Events](https://discord.com/developers/docs/events/gateway-events) -- HIGH confidence (official docs)
- [Discord Webhook Resource](https://discord.com/developers/docs/resources/webhook) -- HIGH confidence (official docs)
- [discord.js Gateway Intents Guide](https://discordjs.guide/popular-topics/intents) -- HIGH confidence (official discord.js docs)
- [Discord Webhook Rate Limits](https://birdie0.github.io/discord-webhooks-guide/other/rate_limits.html) -- MEDIUM confidence (community docs, consistent with official rate limit headers)
- [Discord Webhooks Complete Guide 2025](https://inventivehq.com/blog/discord-webhooks-guide) -- MEDIUM confidence (well-sourced community guide)
- [SSE vs WebSockets Comparison](https://ably.com/blog/websockets-vs-sse) -- HIGH confidence (Ably is an authority on real-time protocols)
- [DigitalOcean SSE with Node.js Tutorial](https://www.digitalocean.com/community/tutorials/nodejs-server-sent-events-build-realtime-app) -- HIGH confidence (authoritative tutorial source)
- [discord.js @discordjs/ws Package](https://discord.js.org/docs/packages/ws/main) -- HIGH confidence (official discord.js docs)
- [Discord Message Content Privileged Intent FAQ](https://support-dev.discord.com/hc/en-us/articles/4404772028055-Message-Content-Privileged-Intent-FAQ) -- HIGH confidence (official Discord support)
