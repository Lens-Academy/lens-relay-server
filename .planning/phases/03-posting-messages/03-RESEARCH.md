# Phase 3: Posting Messages - Research

**Researched:** 2026-02-11
**Domain:** Discord webhook proxy, message compose UI, client-side identity (localStorage), non-closable modal
**Confidence:** HIGH

## Summary

Phase 3 adds three interconnected capabilities to the existing Phase 2 architecture: (1) a webhook proxy endpoint in the discord-bridge sidecar that accepts messages from the browser and forwards them to Discord via a webhook URL (never exposing the webhook URL to the browser), (2) a compose input docked at the bottom of the DiscussionPanel with auto-growing textarea behavior and Enter-to-send, and (3) a global display name identity system using localStorage with a non-closable overlay modal on first visit.

The existing discord-bridge sidecar (Hono + discord.js) already handles REST proxying and SSE streaming. Adding a POST endpoint for webhook execution is straightforward -- the bridge receives `{ content, username }` from the browser, appends ` (unverified)` to the username, and POSTs to the Discord webhook URL stored server-side. The webhook URL is configured via environment variable (`DISCORD_WEBHOOK_URL`), following the same pattern as `DISCORD_BOT_TOKEN`. For multi-channel support, a `DISCORD_WEBHOOK_MAP` env var maps channel IDs to webhook URLs.

The compose UI uses a standard auto-growing textarea pattern. The `react-textarea-autosize` library (v8.5.9, 1500+ dependents on npm) handles this cleanly with `maxRows` prop. The CSS-native `field-sizing: content` property is NOT viable because Firefox does not support it. The identity system uses localStorage with a React context to make the display name available globally.

**Primary recommendation:** Add a single POST endpoint to discord-bridge for webhook execution. Use `react-textarea-autosize` for the compose input with `maxRows={4}`. Use a plain React context + localStorage for the display name identity. Build the modal as a simple non-closable overlay (not Radix Dialog, since Radix dialogs are dismissable by design and we need to prevent all dismissal).

## Standard Stack

### Core

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `hono` | ^4.11.9 | HTTP framework (already installed in bridge) | Existing bridge dependency; `c.req.json()` for POST body parsing |
| `react-textarea-autosize` | ^8.5.9 | Auto-growing textarea in compose input | 1500+ npm dependents; `maxRows` prop caps growth at 4 lines; drop-in `<textarea>` replacement |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| `@radix-ui/react-alert-dialog` | ^1.1.15 | Already installed | NOT for the identity modal (cannot prevent dismiss); used elsewhere in the app |
| `react` (built-in) | ^19.2.0 | Context API for global display name | Already installed |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `react-textarea-autosize` | CSS `field-sizing: content` | Not supported in Firefox (0% Firefox support as of Feb 2026). Would need JS fallback anyway. |
| `react-textarea-autosize` | CSS grid trick (`data-replicated-value` + `::after`) | Works cross-browser but requires manual wiring of the `data-` attribute on every input change; more error-prone than a maintained library |
| `react-textarea-autosize` | Manual `scrollHeight` adjustment | Common pattern but has edge cases (resize observer, initial render, SSR). Library handles all of them. |
| Plain overlay div for modal | Radix Dialog | Radix Dialog is designed to be dismissable (Escape key, overlay click). The identity modal must be non-closable. Fighting Radix's default behavior is more complex than a simple overlay. |
| React Context for identity | Zustand/Jotai | Overkill for a single localStorage-backed string. React Context is sufficient. |

### Installation

**lens-editor/ (add auto-growing textarea):**
```bash
cd lens-editor
npm install react-textarea-autosize
```

**discord-bridge/ (no new dependencies needed -- Hono already handles POST + JSON parsing)**

## Architecture Patterns

### Recommended Changes to Project Structure

```
discord-bridge/
  src/
    index.ts                  # Add POST /api/channels/:channelId/messages endpoint
    discord-client.ts         # Add executeWebhook() function
    types.ts                  # Add WebhookPayload type

lens-editor/
  src/
    contexts/
      DisplayNameContext.tsx   # NEW: React context for global display name
    components/
      DisplayNamePrompt/      # NEW: Non-closable overlay modal
        DisplayNamePrompt.tsx
        index.ts
      DisplayNameBadge/       # NEW: Clickable name display in header
        DisplayNameBadge.tsx
        index.ts
      DiscussionPanel/
        ComposeBox.tsx         # NEW: Message compose input with send button
        DiscussionPanel.tsx    # Add ComposeBox below MessageList
        useMessages.ts         # Add sendMessage function
```

### Pattern 1: Webhook Proxy Endpoint (Bridge Side)

**What:** A POST endpoint in the bridge that receives message content + display name from the browser, constructs the webhook payload with `(unverified)` suffix, and forwards to Discord.

**When to use:** Every time the user sends a message from the compose input.

**Example:**
```typescript
// discord-bridge/src/index.ts — new POST endpoint
app.post('/api/channels/:channelId/messages', async (c) => {
  const { channelId } = c.req.param();
  const body = await c.req.json<{ content: string; username: string }>();

  // Validate required fields
  if (!body.content?.trim()) {
    return c.json({ error: 'Message content is required' }, 400);
  }
  if (!body.username?.trim()) {
    return c.json({ error: 'Username is required' }, 400);
  }

  // Enforce content length limit (Discord max: 2000 chars)
  if (body.content.length > 2000) {
    return c.json({ error: 'Message exceeds 2000 character limit' }, 400);
  }

  try {
    const result = await executeWebhook(channelId, {
      content: body.content,
      username: `${body.username.trim()} (unverified)`,
    });
    return c.json(result, 200);
  } catch (err) {
    if (err instanceof RateLimitError) {
      return c.json({ error: 'Rate limited', retryAfter: err.retryAfter }, 429);
    }
    if (err instanceof DiscordApiError) {
      return c.json({ error: 'Discord API error', details: err.body }, err.status as 400);
    }
    const message = err instanceof Error ? err.message : 'Unknown error';
    console.error('[discord-bridge] Webhook error:', message);
    return c.json({ error: message }, 500);
  }
});
```

### Pattern 2: Webhook Execution (Bridge Discord Client)

**What:** Server-side function that POSTs to Discord's Execute Webhook endpoint using the webhook URL from environment variables.

**When to use:** Called by the POST endpoint above.

**Example:**
```typescript
// discord-bridge/src/discord-client.ts — new function
const WEBHOOK_MAP: Record<string, string> = (() => {
  const mapStr = process.env.DISCORD_WEBHOOK_MAP;
  if (mapStr) {
    try { return JSON.parse(mapStr); } catch { /* fall through */ }
  }
  return {};
})();

const DEFAULT_WEBHOOK_URL = process.env.DISCORD_WEBHOOK_URL;

function getWebhookUrl(channelId: string): string {
  const url = WEBHOOK_MAP[channelId] || DEFAULT_WEBHOOK_URL;
  if (!url) {
    throw new Error(
      'No webhook URL configured. Set DISCORD_WEBHOOK_URL or DISCORD_WEBHOOK_MAP.'
    );
  }
  return url;
}

interface WebhookPayload {
  content: string;
  username: string;
  avatar_url?: string;
}

export async function executeWebhook(
  channelId: string,
  payload: WebhookPayload
): Promise<{ id: string }> {
  const webhookUrl = getWebhookUrl(channelId);

  // ?wait=true returns the created message object
  const res = await fetch(`${webhookUrl}?wait=true`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  });

  return handleResponse<{ id: string }>(res);
}
```

### Pattern 3: Display Name Context (React Context + localStorage)

**What:** A React context that provides the current display name and a setter. On mount, reads from localStorage. The setter writes to both state and localStorage.

**When to use:** Wrap the entire app. Any component can read the display name.

**Example:**
```typescript
// lens-editor/src/contexts/DisplayNameContext.tsx
import { createContext, useContext, useState, useCallback, type ReactNode } from 'react';

const STORAGE_KEY = 'lens-editor-display-name';

interface DisplayNameContextValue {
  displayName: string | null;
  setDisplayName: (name: string) => void;
}

const DisplayNameContext = createContext<DisplayNameContextValue | null>(null);

export function DisplayNameProvider({ children }: { children: ReactNode }) {
  const [displayName, setDisplayNameState] = useState<string | null>(() => {
    try {
      return localStorage.getItem(STORAGE_KEY);
    } catch {
      return null;
    }
  });

  const setDisplayName = useCallback((name: string) => {
    const trimmed = name.trim();
    if (!trimmed) return;
    setDisplayNameState(trimmed);
    try {
      localStorage.setItem(STORAGE_KEY, trimmed);
    } catch {
      // localStorage full or unavailable — state still works for session
    }
  }, []);

  return (
    <DisplayNameContext.Provider value={{ displayName, setDisplayName }}>
      {children}
    </DisplayNameContext.Provider>
  );
}

export function useDisplayName(): DisplayNameContextValue {
  const ctx = useContext(DisplayNameContext);
  if (!ctx) throw new Error('useDisplayName must be used within DisplayNameProvider');
  return ctx;
}
```

### Pattern 4: Non-Closable Overlay Modal

**What:** A full-screen overlay that blocks all interaction until the user enters a display name. No close button, no Escape dismiss, no click-outside dismiss.

**When to use:** On every page load when `displayName` is null.

**Key implementation detail:** Use `onKeyDown` to prevent Escape from doing anything. Use a plain `<div>` overlay (not Radix Dialog) to avoid fighting Radix's built-in dismiss behavior.

**Example:**
```typescript
// lens-editor/src/components/DisplayNamePrompt/DisplayNamePrompt.tsx
import { useState, useEffect, useRef } from 'react';
import { useDisplayName } from '../../contexts/DisplayNameContext';

export function DisplayNamePrompt() {
  const { displayName, setDisplayName } = useDisplayName();
  const [input, setInput] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  // Auto-focus input on mount
  useEffect(() => {
    if (!displayName) {
      inputRef.current?.focus();
    }
  }, [displayName]);

  // Already have a name — don't show
  if (displayName) return null;

  const canSubmit = input.trim().length > 0;

  const handleSubmit = () => {
    if (!canSubmit) return;
    setDisplayName(input.trim());
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      // Prevent Escape and other keyboard shortcuts from propagating
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.preventDefault();
          e.stopPropagation();
        }
        if (e.key === 'Enter' && canSubmit) {
          handleSubmit();
        }
      }}
    >
      <div className="bg-white rounded-lg shadow-xl p-6 w-[400px] mx-4">
        <h2 className="text-lg font-semibold text-gray-900 mb-2">
          What should we call you?
        </h2>
        <p className="text-sm text-gray-600 mb-4">
          This name will be shown on your messages and comments.
        </p>
        <input
          ref={inputRef}
          type="text"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder="Enter your display name"
          className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
          maxLength={80}
        />
        <button
          onClick={handleSubmit}
          disabled={!canSubmit}
          className="mt-4 w-full px-4 py-2 bg-blue-600 text-white rounded-md hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        >
          Continue
        </button>
      </div>
    </div>
  );
}
```

### Pattern 5: Auto-Growing Compose Box

**What:** A textarea docked at the bottom of the DiscussionPanel that grows from 1 line to a maximum of 4 lines, with Enter-to-send and Shift+Enter for newlines.

**When to use:** Always present at the bottom of the DiscussionPanel (below the MessageList).

**Example:**
```typescript
// lens-editor/src/components/DiscussionPanel/ComposeBox.tsx
import { useState, useCallback } from 'react';
import TextareaAutosize from 'react-textarea-autosize';

interface ComposeBoxProps {
  channelName: string | null;
  onSend: (content: string) => Promise<void>;
  disabled?: boolean;
}

export function ComposeBox({ channelName, onSend, disabled }: ComposeBoxProps) {
  const [value, setValue] = useState('');
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const canSend = value.trim().length > 0 && !sending && !disabled;

  const handleSend = useCallback(async () => {
    if (!canSend) return;

    const content = value.trim();
    setValue('');      // Clear immediately
    setSending(true);  // Disable input to prevent double-send
    setError(null);

    try {
      await onSend(content);
    } catch {
      // Restore message text on failure
      setValue(content);
      setError('Failed to send -- try again');
    } finally {
      setSending(false);
    }
  }, [canSend, value, onSend]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  return (
    <div className="border-t border-gray-200 px-3 py-2">
      {error && (
        <p className="text-xs text-red-600 mb-1">{error}</p>
      )}
      <div className="flex items-end gap-2">
        <TextareaAutosize
          value={value}
          onChange={(e) => {
            setValue(e.target.value);
            setError(null);
          }}
          onKeyDown={handleKeyDown}
          placeholder={channelName ? `Message #${channelName}` : 'Send a message'}
          maxRows={4}
          minRows={1}
          disabled={sending || disabled}
          className="flex-1 resize-none px-3 py-2 border border-gray-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
        />
        <button
          onClick={handleSend}
          disabled={!canSend}
          className="p-2 text-blue-600 hover:text-blue-700 disabled:text-gray-300 disabled:cursor-not-allowed transition-colors flex-shrink-0"
          aria-label="Send message"
        >
          {/* Simple send arrow icon (SVG) */}
          <svg width="20" height="20" viewBox="0 0 20 20" fill="currentColor">
            <path d="M2.94 17.94a1 1 0 01-.34-1.47l4.13-6.47-4.13-6.47a1 1 0 011.34-1.47l14 7a1 1 0 010 1.88l-14 7a1 1 0 01-1-.06z" />
          </svg>
        </button>
      </div>
    </div>
  );
}
```

### Pattern 6: sendMessage in useMessages Hook

**What:** Extend the existing `useMessages` hook to include a `sendMessage` function that POSTs to the bridge.

**When to use:** Called by ComposeBox when the user sends a message.

**Example:**
```typescript
// Addition to useMessages.ts
const sendMessage = useCallback(
  async (content: string, username: string) => {
    const res = await fetch(`/api/discord/channels/${channelId}/messages`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ content, username }),
    });

    if (!res.ok) {
      const body = await res.json().catch(() => ({ error: 'Send failed' }));
      throw new Error(body.error || `HTTP ${res.status}`);
    }

    // No optimistic insert -- message echoes back via SSE
  },
  [channelId]
);
```

### Anti-Patterns to Avoid

- **Exposing webhook URL to browser:** Never send the Discord webhook URL to the frontend. It goes through the bridge proxy endpoint. Anyone with a webhook URL can post to the channel.
- **Optimistic message insert:** The user decision explicitly says "no optimistic insert" to avoid dedup complexity with SSE echo. Clear the input and wait for SSE.
- **Using Radix Dialog for the identity modal:** Radix Dialog has built-in Escape dismiss and overlay-click dismiss. The identity modal must be non-closable. Use a plain overlay div instead.
- **Fighting field-sizing: content:** Do not rely on this CSS property -- Firefox does not support it. Use `react-textarea-autosize` for cross-browser auto-growing.
- **Storing webhook URL in VITE_ env vars:** `VITE_` prefix makes env vars available in the browser bundle. Webhook URLs must only exist server-side.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Auto-growing textarea | Manual scrollHeight adjustment | `react-textarea-autosize` with `maxRows={4}` | Handles initial render, resize observer, and SSR edge cases. 8.5.9 is mature and well-tested. |
| Webhook username validation | Complex client-side regex for Discord rules | Let the bridge validate length (1-80 chars, no "clyde" substring) and return clear errors | Discord's actual validation rules are poorly documented and inconsistent with their docs (GitHub issue #4293). Server-side is more reliable. |
| localStorage wrapper | Custom hook with try/catch for every read/write | Simple try/catch in the context provider's `useState` initializer and setter | localStorage can throw (private browsing, quota exceeded). One try/catch in the provider covers all consumers. |
| POST body validation | Custom parser | Hono's `c.req.json()` + manual field checks | Hono parses JSON automatically. Simple field presence checks are enough for 2 fields. No need for Zod. |

**Key insight:** The webhook URL is the single most sensitive secret in this phase. The entire proxy pattern exists to keep it server-side. Every design decision should validate: "Does the webhook URL ever reach the browser?" Answer must always be no.

## Common Pitfalls

### Pitfall 1: Webhook URL Exposed in Browser

**What goes wrong:** The Discord webhook URL leaks to the browser, allowing anyone to post to the channel without the proxy.
**Why it happens:** Developer puts webhook URL in a `VITE_` env var, or returns it in an API response, or includes it in client-side code.
**How to avoid:** Webhook URL lives ONLY in the bridge process environment (`DISCORD_WEBHOOK_URL`, no VITE_ prefix). The bridge's POST endpoint accepts `{ content, username }` and internally constructs the webhook request. The response to the browser contains only the Discord message object (or error), never the webhook URL.
**Warning signs:** Any `VITE_WEBHOOK` in env files; webhook URL visible in browser DevTools Network tab.

### Pitfall 2: Double-Send on Fast Enter Key

**What goes wrong:** User presses Enter rapidly and sends the same message twice.
**Why it happens:** The async `handleSend` hasn't completed before the user presses Enter again, and the input still contains the message text.
**How to avoid:** Clear the input AND disable it immediately on send (before the await). Use the `sending` state to gate both the Enter handler and the Send button. Re-enable only after the POST completes or fails.
**Warning signs:** Duplicate messages in the channel from the same user within 1 second.

### Pitfall 3: Message Lost on Send Failure

**What goes wrong:** User sends a message, it fails, and the message text is gone.
**Why it happens:** The input was cleared optimistically but the error handler doesn't restore the text.
**How to avoid:** Save the message content before clearing, and restore it in the `catch` block. Show an inline error message below the input.
**Warning signs:** Empty input after seeing "Failed to send" error; user has to retype the message.

### Pitfall 4: Webhook Username Length Exceeds 80 Characters

**What goes wrong:** Discord rejects the webhook execution with a 400 error.
**Why it happens:** The display name is up to 80 characters, and the bridge appends ` (unverified)` (14 characters), pushing the total over 80.
**How to avoid:** The client-side max length for the display name input should be `80 - 14 = 66` characters (accounting for the ` (unverified)` suffix appended server-side). Additionally, validate in the bridge before sending to Discord.
**Warning signs:** 400 errors from Discord API when users with long names try to send messages.

### Pitfall 5: Webhook Username Contains "clyde"

**What goes wrong:** Discord rejects webhook execution because the username contains "clyde" (case-insensitive substring match).
**Why it happens:** A user enters a display name like "Clyde" or "myclydename".
**How to avoid:** Client-side validation in the DisplayNamePrompt: reject names containing "clyde" (case-insensitive). Also validate in the bridge as a safety net.
**Warning signs:** 400 errors from Discord with "Invalid username" type messages.

### Pitfall 6: Non-Closable Modal Can Be Bypassed

**What goes wrong:** User dismisses the modal using Escape key, browser back button, or by manipulating the DOM in devtools.
**Why it happens:** The modal doesn't prevent Escape propagation, or uses a library that has built-in dismiss behavior.
**How to avoid:** Use a plain `<div>` overlay (not Radix Dialog). Intercept `onKeyDown` and `preventDefault` on Escape. Accept that DevTools bypass is inevitable (this is a UX guardrail, not a security measure).
**Warning signs:** App is usable without a display name; messages fail to send because username is null.

### Pitfall 7: SSE Echo Delay After Send

**What goes wrong:** User sends a message, input clears, but nothing appears in the message list for several seconds.
**Why it happens:** The webhook POST goes to Discord, which creates the message, which triggers a Gateway MESSAGE_CREATE event, which the bridge receives and forwards via SSE. This round-trip takes 1-3 seconds typically.
**How to avoid:** This is expected behavior per the user's "no optimistic insert" decision. Optionally show a subtle "Sending..." indicator if the SSE echo takes longer than 3 seconds (marked as Claude's discretion). The auto-scroll behavior should handle the incoming message naturally.
**Warning signs:** Users confused by the delay; typing another message before the first one appears.

### Pitfall 8: Webhook Not Configured

**What goes wrong:** POST endpoint returns 500 with "No webhook URL configured" error.
**Why it happens:** The `DISCORD_WEBHOOK_URL` (or `DISCORD_WEBHOOK_MAP`) env var is not set on the bridge.
**How to avoid:** On bridge startup, log a warning if no webhook URL is configured (similar to how `DISCORD_BOT_TOKEN` absence is handled). The POST endpoint should return a clear 503 error with a message like "Webhook not configured". The compose UI should gracefully handle this (show error, not crash).
**Warning signs:** "Failed to send" errors immediately after clicking Send, with no Discord API errors in bridge logs.

## Code Examples

### Discord Execute Webhook API Call

```typescript
// Source: Discord API docs — POST /webhooks/{webhook.id}/{webhook.token}
// The ?wait=true query param makes Discord return the created message object.
const webhookUrl = 'https://discord.com/api/webhooks/123456/abcdef';

const response = await fetch(`${webhookUrl}?wait=true`, {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    content: 'Hello from lens-editor!',
    username: 'Alice (unverified)',
    // avatar_url is optional — omit to use default webhook avatar
  }),
});

// With wait=true, returns a Message object:
// { id: "snowflake", content: "...", author: { ... }, timestamp: "..." }
// Without wait=true, returns 204 No Content
const message = await response.json();
```

### react-textarea-autosize Usage

```typescript
// Source: github.com/Andarist/react-textarea-autosize
import TextareaAutosize from 'react-textarea-autosize';

// Drop-in replacement for <textarea> with auto-growing behavior
<TextareaAutosize
  value={value}
  onChange={(e) => setValue(e.target.value)}
  minRows={1}      // Start at 1 line
  maxRows={4}      // Grow up to 4 lines, then scroll internally
  placeholder="Message #general"
  className="..."  // Accepts all standard textarea attributes
/>
```

### localStorage with Error Handling

```typescript
// Source: MDN Web API — localStorage can throw in private browsing or when full
function safeGetItem(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function safeSetItem(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    console.warn('localStorage unavailable');
  }
}
```

### Hono POST JSON Body Parsing

```typescript
// Source: hono.dev/docs/api/request
app.post('/api/channels/:channelId/messages', async (c) => {
  // c.req.json() parses Content-Type: application/json body
  const body = await c.req.json<{ content: string; username: string }>();

  // Validate
  if (!body.content?.trim()) {
    return c.json({ error: 'content is required' }, 400);
  }

  // Process...
  return c.json({ success: true });
});
```

### Non-Closable Modal Pattern (Plain Overlay)

```typescript
// The key: use a plain div, NOT Radix Dialog.
// Radix Dialog's onOpenChange/onEscapeKeyDown cannot fully prevent dismiss in all cases.
// A plain div with onKeyDown gives us full control.
<div
  className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
  onKeyDown={(e) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
    }
  }}
>
  {/* Modal content */}
</div>
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Manual scrollHeight for auto-grow textarea | `react-textarea-autosize` (v8.5.9) or CSS `field-sizing: content` | 2024+ for field-sizing | `field-sizing` is CSS-native but no Firefox support; library is safer cross-browser |
| Discord bot `POST /channels/:id/messages` for sending | Webhook `POST /webhooks/:id/:token` for user-facing sends | Always (webhooks are designed for external integrations) | Webhooks allow per-message username override; no bot permissions needed; simpler auth |
| `username#discriminator` for Discord identity | `global_name` with discriminator=0 | 2023 (username migration) | Webhook username field accepts any 1-80 char string, no discriminator needed |

**Deprecated/outdated:**
- CSS `field-sizing: content` as a universal solution: Not yet -- Firefox has no support as of Feb 2026.
- Discord API v9 webhooks: v10 is current. Use `discord.com/api/webhooks/` (version-less URL) for webhooks -- they don't require API versioning.

## Open Questions

1. **Webhook URL for the specific channel**
   - What we know: The STACK.md mentions `DISCORD_WEBHOOK_URL` and `DISCORD_WEBHOOK_MAP` env vars. Webhooks are channel-specific -- each webhook URL posts to exactly one channel.
   - What's unclear: Whether the production deployment will use a single webhook URL (all documents share one discussion channel) or need the channel-to-webhook mapping.
   - Recommendation: Implement both `DISCORD_WEBHOOK_URL` (single default) and `DISCORD_WEBHOOK_MAP` (JSON map of channelId -> webhookUrl). Start with the default for simplicity. The bridge should validate that a webhook exists for the requested channel and return 503 if not.

2. **Display name validation edge cases**
   - What we know: Discord webhook username must be 1-80 chars, cannot contain "clyde" (case-insensitive substring). We append 14 chars of ` (unverified)`, so client max is 66 chars.
   - What's unclear: Discord's actual validation is inconsistent with their docs (GitHub issue #4293 shows "everyone" and "here" are accepted despite docs suggesting otherwise). Whether names containing "discord" are rejected is ambiguous.
   - Recommendation: Client-side: enforce max 66 chars, reject "clyde" substring. Server-side: pass through to Discord and return any 400 error with a clear message. Don't over-validate -- let Discord be the source of truth.

3. **Where to create the webhook**
   - What we know: Someone needs to create a webhook in the Discord channel and provide the URL.
   - What's unclear: Is this a one-time manual setup step? Should we document how to create a Discord webhook?
   - Recommendation: Document the webhook creation step (Discord server settings -> Integrations -> Webhooks -> New Webhook -> Copy URL). This is a one-time manual step, not automated.

## Sources

### Primary (HIGH confidence)
- Discord API docs (docs.discord.com/developers/resources/webhook) -- Execute Webhook endpoint, `?wait=true` parameter, username/content fields
- Discord Webhooks Guide (birdie0.github.io) -- Field limits: username 1-80 chars, content 2000 chars, rate limit 5 req/2s per webhook
- Existing codebase (`discord-bridge/src/index.ts`, `discord-client.ts`) -- Verified current Hono patterns, error handling, existing endpoint structure
- Existing codebase (`lens-editor/src/components/DisconnectionModal`) -- Verified plain overlay modal pattern already used in codebase

### Secondary (MEDIUM confidence)
- GitHub issue discord/discord-api-docs#4293 -- Webhook username validation inconsistencies (case-insensitive "clyde" substring match confirmed, other restrictions unclear)
- react-textarea-autosize (github.com/Andarist/react-textarea-autosize) -- v8.5.9, minRows/maxRows API, 1500+ npm dependents
- Can I Use (caniuse.com) -- field-sizing: content NOT supported in Firefox, supported in Chrome 123+ and Safari 26.2+

### Tertiary (LOW confidence)
- Discord webhook rate limit bucket sharing across channels in a community -- Reported in GitHub issue #6753 but not officially confirmed. Unlikely to be hit with single-user posting.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- Hono POST handling and react-textarea-autosize are well-documented, mature libraries. Discord webhook API is stable and well-documented.
- Architecture: HIGH -- Follows existing codebase patterns (bridge proxy, React context, overlay modals). All patterns verified against actual codebase.
- Pitfalls: HIGH -- Webhook URL exposure, double-send, username length, and "clyde" restriction are well-documented failure modes.
- Display name validation: MEDIUM -- Discord's actual validation behavior is inconsistent with documentation per GitHub issue #4293.

**Research date:** 2026-02-11
**Valid until:** 2026-03-13 (30 days -- Discord webhook API and React patterns are stable)
