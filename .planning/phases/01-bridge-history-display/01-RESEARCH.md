# Phase 1: Bridge + History Display - Research

**Researched:** 2026-02-08
**Domain:** Discord REST API proxy + React chat panel
**Confidence:** HIGH

## Summary

This phase requires two interconnected pieces: (1) a sidecar Node.js proxy that forwards Discord REST API calls (specifically "Get Channel Messages"), and (2) a React panel in the lens-editor that detects `discussion` frontmatter in Y.Doc content, extracts a Discord channel ID, fetches messages through the proxy, and renders them in a read-only chat view.

The codebase already has a well-established pattern for right-sidebar panels (TableOfContents, BacklinksPanel, CommentsPanel) inside `EditorArea.tsx`, making the UI integration straightforward. The Y.Doc content is accessible via `ydoc.getText('contents')` which contains raw markdown including YAML frontmatter. Parsing the frontmatter from this string and extracting a `discussion` field is a simple string operation.

The sidecar proxy is deliberately minimal -- a single GET endpoint that forwards requests to Discord's REST API. This avoids exposing the bot token to the browser and handles CORS. For Phase 1, only the "Get Channel Messages" endpoint is needed.

**Primary recommendation:** Use Hono + `@hono/node-server` for the sidecar proxy (lightweight, fast, TypeScript-native). Use the `front-matter` npm package for frontmatter parsing in the browser. Use plain `fetch` with manual rate-limit handling in the sidecar rather than `@discordjs/rest` (the sidecar only calls one endpoint and doesn't need the full discord.js rate-limit machinery). Proxy the sidecar through Vite's dev server to avoid CORS in development.

## Standard Stack

### Core

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `hono` | ^4.x | HTTP framework for sidecar proxy | 14KB footprint, 3.5x faster than Express, TypeScript-native, built on Web Standards |
| `@hono/node-server` | ^1.x | Node.js adapter for Hono | Official adapter, uses Node 18+ web standard APIs |
| `front-matter` | ^3.0.0 | YAML frontmatter parsing in browser | No Node.js dependencies (no `fs`), ~80 lines, regex-based, uses `js-yaml` internally |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| `discord-api-types` | ^0.37.x | TypeScript types for Discord API objects | Type-safe message/user objects without pulling in full discord.js |
| `tsx` | ^4.x | Run TypeScript files directly in Node.js | Running the sidecar in development without a build step |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Hono | Express | Express is 572KB vs 14KB, 3.5x slower, but has larger ecosystem. Not needed for 1-2 endpoints. |
| Hono | Fastify | Fastify has better plugin system but heavier for a single-endpoint proxy. |
| `front-matter` | `gray-matter` | gray-matter is more popular but has Node.js `fs` dependency issues in browser bundles, and `require('module')` calls that fail in Vite |
| `front-matter` | Hand-rolled regex | Frontmatter parsing looks simple but edge cases (BOM, Windows line endings, YAML escaping) make a library worthwhile |
| Plain `fetch` in sidecar | `@discordjs/rest` | `@discordjs/rest` has built-in rate limiting but pulls in many dependencies. For one GET endpoint, manual 429 handling is simpler. |

### Installation

**Sidecar (new `discord-bridge/` directory):**
```bash
npm init -y
npm install hono @hono/node-server discord-api-types
npm install -D tsx typescript @types/node
```

**lens-editor (add frontmatter parsing):**
```bash
cd lens-editor
npm install front-matter
```

## Architecture Patterns

### Recommended Project Structure

```
discord-bridge/               # Sidecar proxy service (NEW)
  src/
    index.ts                  # Hono server entry point
    discord-client.ts         # Discord REST API wrapper (fetch + rate-limit)
    types.ts                  # Shared types (or re-export from discord-api-types)
  package.json
  tsconfig.json

lens-editor/
  src/
    components/
      DiscussionPanel/        # NEW: Discord chat panel
        DiscussionPanel.tsx   # Main panel component
        MessageList.tsx       # Message rendering
        MessageItem.tsx       # Single message with avatar, username, timestamp
        useDiscussion.ts      # Hook: extract discussion field from Y.Doc
        useMessages.ts        # Hook: fetch messages from proxy
        index.ts
    lib/
      frontmatter.ts          # NEW: Extract frontmatter from Y.Text string
      discord-url.ts           # NEW: Parse Discord channel URL to IDs
```

### Pattern 1: Sidecar Proxy

**What:** A standalone Node.js process that proxies Discord API calls, adding the bot token server-side.

**When to use:** Always. The bot token must never reach the browser.

**Example:**
```typescript
// discord-bridge/src/index.ts
import { serve } from '@hono/node-server';
import { Hono } from 'hono';
import { cors } from 'hono/cors';

const app = new Hono();

// CORS for direct browser access in production
// (In dev, Vite proxy handles this)
app.use('/api/*', cors());

// GET /api/channels/:channelId/messages?limit=50
app.get('/api/channels/:channelId/messages', async (c) => {
  const channelId = c.req.param('channelId');
  const limit = c.req.query('limit') || '50';

  const res = await fetch(
    `https://discord.com/api/v10/channels/${channelId}/messages?limit=${limit}`,
    {
      headers: {
        Authorization: `Bot ${process.env.DISCORD_BOT_TOKEN}`,
      },
    }
  );

  if (res.status === 429) {
    const retryAfter = res.headers.get('Retry-After');
    return c.json({ error: 'rate_limited', retryAfter }, 429);
  }

  if (!res.ok) {
    return c.json({ error: 'discord_error', status: res.status }, res.status);
  }

  const messages = await res.json();
  return c.json(messages);
});

const port = parseInt(process.env.DISCORD_BRIDGE_PORT || '8091');
serve({ fetch: app.fetch, port }, (info) => {
  console.log(`Discord bridge listening on port ${info.port}`);
});
```

### Pattern 2: Vite Proxy to Sidecar (Development)

**What:** Route `/api/discord` requests through Vite's dev proxy to the sidecar, avoiding CORS.

**When to use:** Development only.

**Example (addition to vite.config.ts):**
```typescript
proxy: {
  '/api/relay': { /* existing */ },
  '/api/discord': {
    target: `http://localhost:${discordBridgePort}`,
    changeOrigin: true,
    rewrite: (path) => path.replace(/^\/api\/discord/, '/api'),
  },
}
```

### Pattern 3: Frontmatter Extraction from Y.Text

**What:** Extract YAML frontmatter from the Y.Doc's `contents` Y.Text to find the `discussion` field.

**When to use:** Every time a document loads or changes, to determine if a discussion panel should show.

**Example:**
```typescript
// lens-editor/src/lib/frontmatter.ts
import fm from 'front-matter';

interface DocFrontmatter {
  discussion?: string;  // Discord channel URL
  [key: string]: unknown;
}

export function extractFrontmatter(text: string): DocFrontmatter | null {
  if (!fm.test(text)) return null;
  try {
    const { attributes } = fm<DocFrontmatter>(text);
    return attributes;
  } catch {
    return null;
  }
}
```

### Pattern 4: Discord URL Parsing

**What:** Extract guild ID and channel ID from a Discord channel URL.

**When to use:** After extracting the `discussion` frontmatter field.

**Example:**
```typescript
// lens-editor/src/lib/discord-url.ts
interface DiscordIds {
  guildId: string;
  channelId: string;
}

const DISCORD_CHANNEL_RE = /^https?:\/\/(?:www\.)?discord\.com\/channels\/(\d+)\/(\d+)\/?$/;

export function parseDiscordUrl(url: string): DiscordIds | null {
  const match = url.match(DISCORD_CHANNEL_RE);
  if (!match) return null;
  return { guildId: match[1], channelId: match[2] };
}
```

### Pattern 5: Panel Integration in EditorArea

**What:** The DiscussionPanel slots into the existing right sidebar in `EditorArea.tsx`, following the same pattern as BacklinksPanel and CommentsPanel.

**When to use:** The panel renders conditionally based on whether a `discussion` field exists.

**Example (integration point in EditorArea.tsx):**
```tsx
// Inside the <aside> in EditorArea.tsx, after the existing panels:
{/* Discussion (conditional on frontmatter) */}
<DiscussionPanel docText={ytext} />
```

The DiscussionPanel internally:
1. Reads the Y.Text string via `.toString()`
2. Parses frontmatter with `front-matter`
3. Extracts and parses the Discord URL
4. Fetches messages from the proxy if a valid channel ID is found
5. Renders nothing (returns `null`) if no `discussion` field exists

### Anti-Patterns to Avoid

- **Exposing bot token to browser:** Never send the Discord bot token to the frontend. Always proxy through the sidecar.
- **Polling for history on every render:** Fetch messages once on panel mount, not on every re-render. Use `useEffect` with the channel ID as dependency.
- **Re-parsing frontmatter on every keystroke:** The frontmatter rarely changes. Debounce or only re-parse when the first few lines of the document change.
- **Building the sidecar into Vite's dev server:** Keep them separate. The sidecar has its own lifecycle and will eventually run in production as a Docker container alongside the relay server.

## Don't Hand-Roll

Problems that look simple but have existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| YAML frontmatter parsing | Regex for `---` delimiters + hand-rolled YAML | `front-matter` package | Edge cases: BOM markers, Windows line endings, YAML special characters, empty frontmatter |
| Discord avatar URLs | Manual URL construction | Helper function with fallback | Users without avatars need default avatar URL (`cdn.discordapp.com/embed/avatars/{index}.png`) where index = `(BigInt(userId) >> 22n) % 6n` for modern users |
| Relative time formatting | Custom date math | `Intl.RelativeTimeFormat` or port existing `formatTimestamp` from CommentsPanel | Already implemented in CommentsPanel -- reuse or extract to shared utility |
| Rate limit handling | Ignoring 429s | Check `Retry-After` header, queue retry | Discord will ban bots that don't respect rate limits |

**Key insight:** The Discord avatar URL construction has a subtle gotcha -- the default avatar index calculation changed in 2023. Old method: `discriminator % 5`. New method (for users who migrated to new username system): `(user_id >> 22) % 6`. Must handle both.

## Common Pitfalls

### Pitfall 1: MESSAGE_CONTENT Privileged Intent

**What goes wrong:** Bot fetches messages but `content` field is empty string for all messages.
**Why it happens:** Discord requires the MESSAGE_CONTENT privileged intent to be enabled in the Developer Portal for bots to read message content via the API.
**How to avoid:** When creating the Discord bot application, enable the MESSAGE_CONTENT intent under Bot > Privileged Gateway Intents in the Discord Developer Portal. Note: this is required for REST API reads as well, not just Gateway events.
**Warning signs:** All messages come back with empty `content`, `embeds`, `attachments`, and `components`.

### Pitfall 2: Bot Token Security

**What goes wrong:** Bot token is exposed in browser network tab or source code.
**Why it happens:** Developer passes token through Vite env vars (VITE_ prefix makes them public) or includes it in frontend fetch calls.
**How to avoid:** Token lives ONLY in the sidecar process environment. Never use `VITE_DISCORD_TOKEN`. Use `DISCORD_BOT_TOKEN` (no VITE_ prefix) in the sidecar's environment only.
**Warning signs:** Token visible in browser DevTools Network tab or source maps.

### Pitfall 3: Avatar URL for Users Without Custom Avatars

**What goes wrong:** Avatar images show broken image icons for some users.
**Why it happens:** When `user.avatar` is null, there's no hash to construct the CDN URL. Must use the default avatar endpoint instead.
**How to avoid:** Check `user.avatar !== null` before constructing CDN URL. Fallback: `https://cdn.discordapp.com/embed/avatars/${defaultIndex}.png`.
**Warning signs:** `<img>` tags with `src` containing "null" in the URL path.

### Pitfall 4: Y.Text Observation Timing

**What goes wrong:** Frontmatter is read before Y.Doc syncs, returning empty string.
**Why it happens:** The Y.Doc is empty until the provider fires the `synced` event.
**How to avoid:** Only parse frontmatter after sync completes. The existing `useSynced` hook or the `synced` state in the Editor component provides this signal. Alternatively, observe Y.Text changes and re-parse when the text updates.
**Warning signs:** Panel flashes "no discussion" then shows content, or never shows content on first load.

### Pitfall 5: Stale Channel ID Across Document Switches

**What goes wrong:** Panel shows messages from the previous document's channel after switching documents.
**Why it happens:** The EditorArea remounts on doc switch (via `key={activeDocId}`), but if the hook state isn't properly reset, stale data persists.
**How to avoid:** Since EditorArea already uses `key={activeDocId}` to force remount, hooks reset naturally. Verify this behavior in testing. The existing pattern works correctly for BacklinksPanel and CommentsPanel.
**Warning signs:** Messages from document A showing when viewing document B.

### Pitfall 6: Discord Rate Limits on Message Fetch

**What goes wrong:** Sidecar starts returning 429 errors when many documents are opened rapidly.
**Why it happens:** Discord rate limits the "Get Channel Messages" endpoint per-route. Rapidly switching between documents with different channels triggers many API calls.
**How to avoid:** Cache message responses in the sidecar with a short TTL (30-60 seconds). This also improves perceived performance.
**Warning signs:** 429 responses in sidecar logs, intermittent empty message lists.

## Code Examples

Verified patterns from official sources:

### Discord REST API: Get Channel Messages
```typescript
// Source: Discord API docs (github.com/discord/discord-api-docs)
// GET /channels/{channel.id}/messages
// Query params: limit (1-100, default 50), before, after, around (mutually exclusive)
// Auth: Bot token in Authorization header

const response = await fetch(
  `https://discord.com/api/v10/channels/${channelId}/messages?limit=50`,
  {
    headers: {
      Authorization: `Bot ${botToken}`,
    },
  }
);

// Response: Array of Message objects
// Key fields per message:
// - id: string (snowflake)
// - content: string (up to 2000 chars)
// - author: { id, username, avatar, global_name }
// - timestamp: string (ISO8601)
// - type: number (0 = DEFAULT, 19 = REPLY)
```

### Discord Avatar URL Construction
```typescript
// Source: Discord API docs - CDN section
function getAvatarUrl(userId: string, avatarHash: string | null, size = 64): string {
  if (avatarHash) {
    const ext = avatarHash.startsWith('a_') ? 'gif' : 'png';
    return `https://cdn.discordapp.com/avatars/${userId}/${avatarHash}.${ext}?size=${size}`;
  }
  // Default avatar for users without custom avatar
  // Modern calculation (post-username migration):
  const defaultIndex = Number((BigInt(userId) >> 22n) % 6n);
  return `https://cdn.discordapp.com/embed/avatars/${defaultIndex}.png`;
}
```

### Hono Sidecar Server Setup
```typescript
// Source: hono.dev/docs/getting-started/nodejs
import { serve } from '@hono/node-server';
import { Hono } from 'hono';
import { cors } from 'hono/cors';

const app = new Hono();
app.use('/api/*', cors());

app.get('/api/channels/:channelId/messages', async (c) => {
  // ... proxy to Discord
});

serve({ fetch: app.fetch, port: 8091 });
```

### front-matter Usage
```typescript
// Source: github.com/jxson/front-matter
import fm from 'front-matter';

const text = `---
discussion: https://discord.com/channels/123/456
title: My Document
---

Document content here...`;

if (fm.test(text)) {
  const { attributes, body } = fm(text);
  console.log(attributes.discussion); // "https://discord.com/channels/123/456"
  console.log(body); // "\nDocument content here..."
}
```

### Vite Proxy Configuration
```typescript
// Source: vite.dev/config/server-options
// Addition to existing vite.config.ts proxy section
'/api/discord': {
  target: 'http://localhost:8091',
  changeOrigin: true,
  rewrite: (path: string) => path.replace(/^\/api\/discord/, '/api'),
},
```

### React Hook for Discussion Detection
```typescript
// Pattern following existing hooks in the codebase (useSynced, useComments)
import { useState, useEffect } from 'react';
import * as Y from 'yjs';
import { extractFrontmatter } from '../../lib/frontmatter';
import { parseDiscordUrl } from '../../lib/discord-url';

export function useDiscussion(ytext: Y.Text) {
  const [channelId, setChannelId] = useState<string | null>(null);

  useEffect(() => {
    function update() {
      const text = ytext.toString();
      const fm = extractFrontmatter(text);
      if (!fm?.discussion) {
        setChannelId(null);
        return;
      }
      const parsed = parseDiscordUrl(fm.discussion);
      setChannelId(parsed?.channelId ?? null);
    }

    update(); // Initial read
    ytext.observe(update);
    return () => ytext.unobserve(update);
  }, [ytext]);

  return channelId;
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Express for lightweight APIs | Hono (Web Standards) | 2023-2024 | 3.5x perf, 40x smaller bundle, multi-runtime |
| `user.discriminator % 5` for default avatars | `(user_id >> 22) % 6` for default avatars | 2023 (username migration) | Must handle both old and new users |
| Discord API v9 | Discord API v10 | 2022 | v10 is current stable; v9 still works but deprecated |
| `gray-matter` for browser frontmatter | `front-matter` for browser frontmatter | N/A (both are stable) | `front-matter` is simpler and avoids Node.js dependency issues in Vite |

**Deprecated/outdated:**
- Discord API v6-v8: Fully deprecated, do not use.
- `user.discriminator` field: Still present but `0` for migrated users. Use `user.global_name` as display name, fall back to `user.username`.

## Open Questions

1. **Discord Bot Application**
   - What we know: A Discord bot token is needed. The bot must be in the server (guild) containing the channels.
   - What's unclear: Does the user already have a Discord bot application? Do they need guidance on creating one?
   - Recommendation: The sidecar should fail gracefully with a clear error message if `DISCORD_BOT_TOKEN` is not set. Document the bot setup in the sidecar's README.

2. **Sidecar Port Convention**
   - What we know: The project uses port conventions (5173/8090 for ws1, 5273/8190 for ws2).
   - What's unclear: What port should the Discord bridge use?
   - Recommendation: Use 8091 for ws1 (8191 for ws2), fitting the existing pattern. Add `DISCORD_BRIDGE_PORT` env var override.

3. **Message Caching in Sidecar**
   - What we know: Discord rate limits the messages endpoint. Users may rapidly switch between documents.
   - What's unclear: How aggressive should caching be? Should it be a simple in-memory TTL cache or something more sophisticated?
   - Recommendation: Simple in-memory Map with 60-second TTL per channel ID. Phase 1 only. Revisit if needed.

4. **Sidecar Lifecycle Management**
   - What we know: The relay server is started separately (`npm run relay:start`).
   - What's unclear: Should the sidecar auto-start with `npm run dev:local`, or be a separate terminal?
   - Recommendation: Separate terminal for now (like relay server). Add `npm run discord:start` script. Consider `concurrently` later if needed.

5. **Discussion Panel Width and the Existing Sidebar**
   - What we know: The existing right sidebar is `w-64` (256px) and contains ToC, Backlinks, and Comments panels stacked vertically.
   - What's unclear: Should the Discussion panel be a new section in the existing sidebar, or a separate sidebar to the right?
   - Recommendation: The CONTEXT.md says "right sidebar, vertical panel to the right of the editor." Given the existing sidebar already has three panels, adding a Discord chat panel there would make it very crowded. Create a SEPARATE right panel (to the right of the existing sidebar) that only appears when `discussion` frontmatter exists. This gives the chat panel enough width (~320px) for readable messages with avatars.

## Sources

### Primary (HIGH confidence)
- Discord API docs (raw markdown from GitHub): `discord/discord-api-docs` -- Get Channel Messages endpoint, Message object structure, API v10 base URL
- Hono official docs (`hono.dev`): Node.js adapter usage, proxy helper, CORS middleware
- `front-matter` npm package (`github.com/jxson/front-matter`): Source analysis confirms no Node.js-specific dependencies, ~80 lines, browser-safe

### Secondary (MEDIUM confidence)
- `@discordjs/rest` npm package: Verified exists and works standalone, but determined too heavyweight for this use case
- Discord CDN avatar URL format: Confirmed via multiple sources including discord.js docs and API docs discussions
- Vite proxy configuration: Confirmed via Vite official docs (`vite.dev/config/server-options`)

### Tertiary (LOW confidence)
- Default avatar index calculation change (discriminator to user_id >> 22): Multiple community sources agree but official docs are sparse on the exact migration date. Implementation should handle both methods defensively.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - Hono, front-matter, and plain fetch are well-documented and verified
- Architecture: HIGH - Follows existing codebase patterns (panel in EditorArea, hook-based state, Vite proxy)
- Pitfalls: HIGH - Rate limits, token security, and MESSAGE_CONTENT intent are well-documented by Discord
- Avatar URL construction: MEDIUM - Default avatar calculation for new usernames is community-documented but not prominently featured in official docs

**Research date:** 2026-02-08
**Valid until:** 2026-03-10 (30 days -- Discord API and Hono are stable)
