# Phase 4: Connection Resilience - Research

**Researched:** 2026-02-11
**Domain:** SSE connection lifecycle, EventSource reconnection, React error/retry UX patterns
**Confidence:** HIGH

## Summary

Phase 4 enhances the existing DiscussionPanel to clearly communicate connection problems and help users recover without manual page reloads. The two requirements are UX-02 (error state with retry button) and UX-03 (connection status indicator: live/reconnecting/disconnected).

**The key finding of this research is that much of the infrastructure already exists.** The current codebase (useMessages.ts, DiscussionPanel.tsx) already has:
- A `GatewayStatus` type with four states: `connected`, `connecting`, `disconnected`, `reconnecting`
- Colored status dots in the panel header (green=connected, yellow-pulsing=connecting/reconnecting, gray=disconnected)
- An error state display with "Retry" button that calls `refetch()` (re-fetches REST messages)
- EventSource `onerror` handler that sets `gatewayStatus` to `'reconnecting'`
- Dedup logic preventing duplicate messages after reconnect

**What's missing** to fully satisfy the requirements:
1. **SSE reconnection does NOT trigger message history reload** -- When EventSource reconnects after a drop, the browser re-establishes the SSE connection, but any messages sent during the gap are lost. The `refetch()` function only runs from the error/retry button, not automatically after reconnection.
2. **Gateway status changes on the bridge are NOT pushed to SSE clients** -- The bridge only sends gateway status once (on initial SSE connection). If the Discord Gateway drops and reconnects, SSE clients are never notified. The `gatewayStatus` in the browser only reflects SSE connection state, not actual Discord Gateway state.
3. **The error state only covers REST fetch failures** -- If the initial fetch succeeds but the SSE connection permanently fails, the user sees a green dot that goes yellow, but never sees the error/retry UI because `error` state is only set during the REST fetch phase.
4. **The status indicator has no label text** -- Only a 2x2 dot with a tooltip title attribute. The success criteria require visible text: "Live", "Reconnecting", "Disconnected".
5. **No explicit "Disconnected" terminal state** -- If EventSource gives up (readyState=CLOSED), the UI should show a clear disconnected state with retry affordance. Currently it just stays on 'reconnecting' forever.

**Primary recommendation:** Enhance the existing code in a single focused plan. No new libraries needed. The work is: (1) add gateway status broadcasting from bridge to SSE clients, (2) enhance useMessages to reload history on SSE reconnection and detect terminal disconnection, (3) upgrade the status indicator to show text labels and a disconnected+retry state.

## Standard Stack

### Core

No new libraries needed. Everything required is already in the project:

| Library | Version | Purpose | Already Installed |
|---------|---------|---------|-------------------|
| React | (existing) | UI components, hooks, state management | Yes |
| EventSource | (browser built-in) | SSE client with auto-reconnect | Yes (Web API) |
| Hono `streamSSE` | (built-in to hono ^4.x) | SSE server endpoint | Yes |
| discord.js | ^14.25.1 | Gateway events for status broadcasting | Yes |

### Supporting

No additional libraries needed.

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Native EventSource | `eventsource` npm polyfill (EventSource/eventsource) | Adds configurable retry, headers support, but native EventSource already auto-reconnects. No benefit for this use case. |
| Manual reconnection logic | `reconnecting-eventsource` | Adds exponential backoff. But native EventSource already has built-in reconnection. The browser handles it. Manual reconnection is only needed if we want to control backoff timing, which we don't need. |
| Custom heartbeat timeout detection | No library needed | Use `setTimeout` to detect missed heartbeats. Simple enough to hand-roll (5 lines). |

### Installation

No installation needed. All dependencies are present.

## Architecture Patterns

### Current File Structure (No New Files Needed)

```
discord-bridge/
  src/
    index.ts                  # SSE endpoint (enhance: forward gateway status events)
    gateway.ts                # Gateway manager (enhance: emit status change events)

lens-editor/
  src/
    components/
      DiscussionPanel/
        DiscussionPanel.tsx   # Panel UI (enhance: text labels on status indicator, disconnected+retry state)
        useMessages.ts        # Messages hook (enhance: SSE reconnection triggers refetch, heartbeat timeout detection)
```

### Pattern 1: Gateway Status Broadcasting (Bridge-Side)

**What:** The gateway module emits status change events when the Discord Gateway connects, disconnects, or reconnects. The SSE endpoint forwards these to all connected browser clients.

**When to use:** Whenever the Discord Gateway status changes.

**Current gap:** `gateway.ts` logs reconnection/resume events to console but does not emit them via `gatewayEvents`. The SSE endpoint in `index.ts` only sends the initial status.

**Example:**
```typescript
// gateway.ts enhancement
client.on(Events.ClientReady, (c) => {
  console.log(`[gateway] Connected as ${c.user.tag}`);
  gatewayEvents.emit('status', { gateway: 'connected' });
});

client.on(Events.ShardReconnecting, () => {
  console.log('[gateway] Reconnecting...');
  gatewayEvents.emit('status', { gateway: 'reconnecting' });
});

client.on(Events.ShardResume, () => {
  console.log('[gateway] Resumed');
  gatewayEvents.emit('status', { gateway: 'connected' });
});

client.on(Events.ShardDisconnect, () => {
  console.log('[gateway] Disconnected');
  gatewayEvents.emit('status', { gateway: 'disconnected' });
});
```

```typescript
// index.ts SSE endpoint enhancement
const statusHandler = async (data: unknown) => {
  try {
    await stream.writeSSE({
      event: 'status',
      data: JSON.stringify(data),
    });
  } catch {
    // Client disconnected
  }
};

gatewayEvents.on('status', statusHandler);

stream.onAbort(() => {
  gatewayEvents.off(`message:${channelId}`, handler);
  gatewayEvents.off('status', statusHandler);
});
```

### Pattern 2: SSE Reconnection with History Reload

**What:** When the browser's EventSource reconnects (fires `onopen` after an `onerror`), automatically reload message history to fill any gap.

**When to use:** Every time EventSource transitions from error -> open.

**Current gap:** The `onopen` handler sets `gatewayStatus` to `'connected'` but does not reload messages. Messages sent during the disconnection gap are permanently lost from the panel.

**Example:**
```typescript
// useMessages.ts enhancement
const hasConnected = useRef(false);

eventSource.onopen = () => {
  setGatewayStatus('connected');
  if (hasConnected.current) {
    // This is a RE-connection, not first connection. Reload history.
    refetch();
  }
  hasConnected.current = true;
};
```

### Pattern 3: Heartbeat Timeout Detection

**What:** If no heartbeat is received within a timeout window (e.g., 2x the heartbeat interval), treat the connection as stale even if the browser hasn't fired `onerror` yet.

**When to use:** Detecting "zombie" connections where the TCP connection appears open but no data flows.

**Current state:** The bridge sends heartbeats every 30 seconds. The browser's `heartbeat` event listener exists but does nothing. No timeout detection.

**Example:**
```typescript
// useMessages.ts enhancement
const heartbeatTimeout = useRef<ReturnType<typeof setTimeout>>();
const HEARTBEAT_TIMEOUT_MS = 75000; // 2.5x the 30s heartbeat interval

function resetHeartbeatTimer() {
  clearTimeout(heartbeatTimeout.current);
  heartbeatTimeout.current = setTimeout(() => {
    setGatewayStatus('reconnecting');
  }, HEARTBEAT_TIMEOUT_MS);
}

eventSource.addEventListener('heartbeat', () => {
  resetHeartbeatTimer();
});

eventSource.onopen = () => {
  resetHeartbeatTimer();
  // ...
};
```

### Pattern 4: Status Indicator with Text Labels

**What:** Replace the bare colored dots with labeled indicators showing "Live", "Reconnecting...", or "Disconnected" text.

**When to use:** Always visible in the panel header.

**Current gap:** Only a 2x2 `<span>` dot with a `title` tooltip. The success criteria require visible text: "Live", "Reconnecting", "Disconnected".

**Example:**
```tsx
// DiscussionPanel.tsx header enhancement
function StatusIndicator({ status }: { status: GatewayStatus }) {
  switch (status) {
    case 'connected':
      return (
        <span className="flex items-center gap-1 text-xs text-green-600">
          <span className="w-2 h-2 rounded-full bg-green-500" />
          Live
        </span>
      );
    case 'connecting':
    case 'reconnecting':
      return (
        <span className="flex items-center gap-1 text-xs text-yellow-600">
          <span className="w-2 h-2 rounded-full bg-yellow-400 animate-pulse" />
          Reconnecting
        </span>
      );
    case 'disconnected':
      return (
        <span className="flex items-center gap-1 text-xs text-gray-500">
          <span className="w-2 h-2 rounded-full bg-gray-400" />
          Disconnected
        </span>
      );
  }
}
```

### Pattern 5: Terminal Disconnection with Retry

**What:** When EventSource's `readyState` becomes `CLOSED` (2), the connection won't auto-reconnect. The UI should show a clear disconnected state with a retry button that creates a new EventSource.

**When to use:** When EventSource gives up and won't auto-reconnect.

**Current gap:** The `onerror` handler always sets `'reconnecting'`. But if `readyState === EventSource.CLOSED`, the connection is permanently dead.

**Example:**
```typescript
// useMessages.ts enhancement
eventSource.onerror = () => {
  if (eventSource.readyState === EventSource.CLOSED) {
    // Terminal: EventSource gave up. Need manual retry.
    setGatewayStatus('disconnected');
    setError('Connection lost');
  } else {
    // Transient: EventSource will auto-reconnect
    setGatewayStatus('reconnecting');
  }
};
```

For the retry, the existing `refetch` mechanism (incrementing `fetchTrigger`) can be extended to also recreate the EventSource. Or, create a separate `reconnectSSE` counter that the SSE useEffect depends on.

### Anti-Patterns to Avoid

- **Hand-rolling EventSource reconnection logic:** The browser's EventSource already auto-reconnects with backoff. Don't replace it with custom WebSocket-style reconnection logic. Only handle the terminal CLOSED state.
- **Separate error states for SSE vs REST:** The user doesn't care about the underlying transport. There should be ONE connection status and ONE error/retry mechanism from their perspective.
- **Polling gateway status:** Don't poll `/api/gateway/status` from the browser. Push status changes through the existing SSE connection.
- **Showing error state during brief reconnection:** EventSource auto-reconnects in ~3 seconds. Don't show the scary red error state for transient reconnections. Show "Reconnecting..." with yellow indicator. Only show error state after prolonged disconnection.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| SSE auto-reconnection | Custom retry loop with exponential backoff | Browser's native EventSource auto-reconnect | Built into the Web Standard. EventSource retries automatically with configurable delay (default ~3s). Only need to handle the terminal CLOSED state. |
| Heartbeat keep-alive | Custom ping/pong protocol | Existing bridge heartbeat (30s interval) + setTimeout-based timeout detection | Bridge already sends heartbeats. Just add a client-side timeout timer (~75s). |
| Connection state machine | Redux/Zustand state machine for connection states | React useState with `GatewayStatus` type | Only 4 states, transitions are simple. useState is sufficient. |

**Key insight:** The native EventSource API handles 90% of connection resilience automatically. The work is almost entirely in the UX layer (showing the right indicator, triggering message reload on reconnect) and the bridge layer (broadcasting gateway status changes).

## Common Pitfalls

### Pitfall 1: EventSource `onerror` Doesn't Distinguish Error Types

**What goes wrong:** The `onerror` callback receives a generic `Event`, not an `Error`. You cannot tell if the error is a network failure, a server 500, or a connection timeout.
**Why it happens:** The SSE specification deliberately keeps the error event minimal. There's no error code or message.
**How to avoid:** Use `readyState` to determine what happened: if `readyState === EventSource.CONNECTING` (0), EventSource is auto-reconnecting. If `readyState === EventSource.CLOSED` (2), it gave up. That's all you need.
**Warning signs:** Trying to extract `event.message` or `event.code` from the error event -- these don't exist.

### Pitfall 2: Stale Closure in Reconnection Handler

**What goes wrong:** The `refetch` function called from `eventSource.onopen` captures a stale `fetchTrigger` value, causing no actual refetch.
**Why it happens:** JavaScript closure captures the variable at the time the useEffect runs. If `refetch` is captured in the SSE useEffect, it may be outdated.
**How to avoid:** Use a ref to hold the latest `refetch` function, or use a separate state counter that the SSE useEffect doesn't depend on. The existing pattern of `setFetchTrigger(t => t + 1)` is safe because it uses a state updater function (no stale closure). Call it directly rather than through a captured `refetch` reference.
**Warning signs:** Clicking retry works but auto-reconnection doesn't reload messages.

### Pitfall 3: Infinite Reconnection Loop Creating Multiple EventSource Instances

**What goes wrong:** A reconnection trigger (like incrementing a state counter) causes the SSE useEffect to re-run, creating a new EventSource without properly closing the old one.
**Why it happens:** If the reconnection counter is a dependency of the SSE useEffect, each increment destroys and recreates the EventSource.
**How to avoid:** Keep the SSE useEffect dependent ONLY on `channelId`. For reconnection, either (a) let EventSource handle it natively, or (b) use an independent state counter that only the REST fetch useEffect depends on. The SSE reconnection refetch should use `setFetchTrigger(t => t + 1)` which triggers the FETCH useEffect, not the SSE useEffect.
**Warning signs:** Multiple SSE connections visible in the Network tab; rapid open/close cycles.

### Pitfall 4: Race Between Refetch and SSE During Reconnection

**What goes wrong:** After SSE reconnects, `refetch()` is called, which re-fetches the last 50 messages. Meanwhile, the SSE stream might also deliver some of those same messages, causing duplicates or UI flicker.
**Why it happens:** The refetch and SSE message delivery happen concurrently.
**How to avoid:** The existing dedup logic (`prev.some(m => m.id === newMsg.id)`) inside `setMessages` prevents duplicates. The refetch replaces the entire message list with fresh data. As long as SSE message insertion uses the dedup guard, this is safe. The refetch should use `setMessages(data)` (complete replacement), not append.
**Warning signs:** Duplicate messages appearing briefly then resolving; message list "jumping" during reconnection.

### Pitfall 5: Heartbeat Timeout False Positives

**What goes wrong:** The heartbeat timeout fires during normal operation, causing a false "reconnecting" status.
**Why it happens:** The timeout interval is set too close to the heartbeat interval. Network jitter or browser tab throttling (background tabs) can delay events.
**How to avoid:** Set the timeout to 2-3x the heartbeat interval. The bridge sends heartbeats every 30 seconds. A timeout of 75 seconds (2.5x) provides good tolerance. Note: browser tabs in the background may throttle timers.
**Warning signs:** Status indicator flickers between "Live" and "Reconnecting" during normal use.

### Pitfall 6: Not Cleaning Up Heartbeat Timeout on Component Unmount

**What goes wrong:** The heartbeat timeout fires after the component unmounts, causing a React state update on an unmounted component.
**Why it happens:** `setTimeout` isn't cancelled in the useEffect cleanup.
**How to avoid:** Clear the timeout in the useEffect cleanup function alongside `eventSource.close()`.
**Warning signs:** React warnings about state updates on unmounted components; memory leaks.

## Code Examples

### Complete Enhanced useMessages SSE Effect

```typescript
// Source: Current useMessages.ts + enhancements for Phase 4

// SSE subscription for live message streaming
useEffect(() => {
  if (!channelId) {
    setGatewayStatus('disconnected');
    return;
  }

  const eventSource = new EventSource(`/api/discord/channels/${channelId}/events`);
  let hasConnectedBefore = false;
  let heartbeatTimer: ReturnType<typeof setTimeout>;
  const HEARTBEAT_TIMEOUT_MS = 75_000; // 2.5x the 30s interval

  function resetHeartbeat() {
    clearTimeout(heartbeatTimer);
    heartbeatTimer = setTimeout(() => {
      setGatewayStatus('reconnecting');
    }, HEARTBEAT_TIMEOUT_MS);
  }

  eventSource.addEventListener('message', (e) => {
    const newMsg: DiscordMessage = JSON.parse(e.data);
    setMessages((prev) => {
      if (prev.some((m) => m.id === newMsg.id)) return prev;
      return [...prev, newMsg];
    });
    resetHeartbeat();
  });

  eventSource.addEventListener('status', (e) => {
    const { gateway } = JSON.parse(e.data);
    setGatewayStatus(gateway);
    resetHeartbeat();
  });

  eventSource.addEventListener('heartbeat', () => {
    resetHeartbeat();
  });

  eventSource.onopen = () => {
    setGatewayStatus('connected');
    setError(null);
    resetHeartbeat();
    if (hasConnectedBefore) {
      // Reconnection: reload history to fill gap
      setFetchTrigger((t) => t + 1);
    }
    hasConnectedBefore = true;
  };

  eventSource.onerror = () => {
    clearTimeout(heartbeatTimer);
    if (eventSource.readyState === EventSource.CLOSED) {
      setGatewayStatus('disconnected');
      setError('Connection lost');
    } else {
      setGatewayStatus('reconnecting');
    }
  };

  return () => {
    clearTimeout(heartbeatTimer);
    eventSource.close();
    setGatewayStatus('disconnected');
  };
}, [channelId]);
```

### Gateway Status Broadcasting (Bridge-Side)

```typescript
// Source: Enhancement to discord-bridge/src/gateway.ts

client.on(Events.ClientReady, (c) => {
  console.log(`[gateway] Connected as ${c.user.tag}`);
  gatewayEvents.emit('status', { gateway: 'connected' });
});

client.on(Events.ShardReconnecting, () => {
  console.log('[gateway] Reconnecting...');
  gatewayEvents.emit('status', { gateway: 'reconnecting' });
});

client.on(Events.ShardResume, () => {
  console.log('[gateway] Resumed');
  gatewayEvents.emit('status', { gateway: 'connected' });
});

client.on(Events.ShardDisconnect, (closeEvent) => {
  console.log('[gateway] Disconnected:', closeEvent.code, closeEvent.reason);
  gatewayEvents.emit('status', { gateway: 'disconnected' });
});
```

### SSE Endpoint Status Forwarding

```typescript
// Source: Enhancement to discord-bridge/src/index.ts SSE endpoint

// Inside streamSSE callback, after the message handler setup:
const statusHandler = async (data: unknown) => {
  try {
    await stream.writeSSE({
      event: 'status',
      data: JSON.stringify(data),
    });
  } catch {
    // Client disconnected
  }
};

gatewayEvents.on('status', statusHandler);

stream.onAbort(() => {
  gatewayEvents.off(`message:${channelId}`, handler);
  gatewayEvents.off('status', statusHandler);
});
```

### Status Indicator Component

```tsx
// Source: Enhancement to DiscussionPanel.tsx

import type { GatewayStatus } from './useMessages';

function StatusIndicator({ status }: { status: GatewayStatus }) {
  switch (status) {
    case 'connected':
      return (
        <span className="flex items-center gap-1 text-xs text-green-600">
          <span className="w-2 h-2 rounded-full bg-green-500" />
          Live
        </span>
      );
    case 'connecting':
    case 'reconnecting':
      return (
        <span className="flex items-center gap-1 text-xs text-yellow-600">
          <span className="w-2 h-2 rounded-full bg-yellow-400 animate-pulse" />
          Reconnecting
        </span>
      );
    case 'disconnected':
      return (
        <span className="flex items-center gap-1 text-xs text-gray-500">
          <span className="w-2 h-2 rounded-full bg-gray-400" />
          Disconnected
        </span>
      );
  }
}
```

### MockEventSource Enhancement for Testing

```typescript
// Source: Enhancement to existing MockEventSource in DiscussionPanel.test.tsx

class MockEventSource {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSED = 2;

  readyState = MockEventSource.CONNECTING;
  onopen: ((ev: Event) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;

  private listeners: Record<string, ((ev: Event | MessageEvent) => void)[]> = {};
  private static instances: MockEventSource[] = [];

  constructor(public url: string) {
    MockEventSource.instances.push(this);
    setTimeout(() => {
      this.readyState = MockEventSource.OPEN;
      this.onopen?.(new Event('open'));
    }, 0);
  }

  addEventListener(type: string, listener: (ev: Event | MessageEvent) => void) {
    if (!this.listeners[type]) this.listeners[type] = [];
    this.listeners[type].push(listener);
  }

  removeEventListener(type: string, listener: (ev: Event | MessageEvent) => void) {
    if (this.listeners[type]) {
      this.listeners[type] = this.listeners[type].filter((l) => l !== listener);
    }
  }

  // Test helper: simulate receiving an SSE event
  _simulateEvent(type: string, data: string) {
    const event = new MessageEvent(type, { data });
    this.listeners[type]?.forEach((l) => l(event));
  }

  // Test helper: simulate connection error (transient, will auto-reconnect)
  _simulateError() {
    this.readyState = MockEventSource.CONNECTING;
    this.onerror?.(new Event('error'));
  }

  // Test helper: simulate terminal disconnection
  _simulateTerminalError() {
    this.readyState = MockEventSource.CLOSED;
    this.onerror?.(new Event('error'));
  }

  // Test helper: simulate reconnection
  _simulateReconnect() {
    this.readyState = MockEventSource.OPEN;
    this.onopen?.(new Event('open'));
  }

  close() {
    this.readyState = MockEventSource.CLOSED;
  }

  static getLastInstance(): MockEventSource | undefined {
    return MockEventSource.instances[MockEventSource.instances.length - 1];
  }

  static clearInstances() {
    MockEventSource.instances = [];
  }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Custom reconnection with exponential backoff | Native EventSource auto-reconnect + retry field from server | Always (SSE spec) | Less code, more reliable. Only handle terminal CLOSED state manually. |
| Polling for connection health | Heartbeat-based timeout detection | N/A (simple pattern) | Passive detection; no extra network traffic |
| Generic error messages | Distinct states: reconnecting vs disconnected | UX best practice | Users understand transient reconnection is normal vs terminal disconnection needs action |

**Deprecated/outdated:**
- Manually implementing SSE reconnection logic: Browser EventSource handles this natively. Only handle the CLOSED state.
- Using `eventsource` polyfill for reconnection: Modern browsers all support EventSource natively. The polyfill's value is mostly for Node.js or for adding custom headers (withCredentials), which we don't need.

## Open Questions

1. **Server-sent `retry:` field**
   - What we know: The SSE spec allows the server to set a retry interval by sending `retry: 5000\n`. Hono's `streamSSE` may support this.
   - What's unclear: Whether Hono's `writeSSE` supports the `retry` field, and what the optimal retry interval should be.
   - Recommendation: Don't use it for now. The browser's default retry (~3 seconds) is fine. If needed later, the bridge can send `retry:` in the initial status event.

2. **Background tab throttling effect on heartbeat timers**
   - What we know: Modern browsers throttle `setTimeout` in background tabs (minimum 1 second, sometimes up to 1 minute for inactive tabs).
   - What's unclear: Whether a 75-second heartbeat timeout will reliably fire in background tabs.
   - Recommendation: Set the timeout generously (75s, which is 2.5x the 30s heartbeat). Even with throttling, this should be reliable. If a false positive occurs in a background tab, it self-corrects when the tab becomes active (EventSource reconnects, heartbeat resumes).

3. **Should `refetch` on reconnect replace all messages or merge?**
   - What we know: Currently `setMessages(messagesData.reverse())` replaces the entire list. SSE dedup uses `prev.some()`.
   - What's unclear: If the user has scrolled up looking at older messages and a refetch replaces everything, does the scroll position reset?
   - Recommendation: The current replace behavior is fine. The refetch returns the latest 50 messages, which will include most/all currently displayed messages. The user's scroll position is maintained by React's key-based reconciliation (messages have stable `key={msg.id}`). Test this to verify.

## Sources

### Primary (HIGH confidence)
- MDN EventSource API documentation (developer.mozilla.org/en-US/docs/Web/API/EventSource) -- readyState values (CONNECTING=0, OPEN=1, CLOSED=2), error event behavior, auto-reconnection
- MDN EventSource error event (developer.mozilla.org/en-US/docs/Web/API/EventSource/error_event) -- Error event is generic Event (no code/message), readyState determines what happened
- Current codebase: `useMessages.ts`, `DiscussionPanel.tsx`, `gateway.ts`, `index.ts` -- Existing implementation to enhance

### Secondary (MEDIUM confidence)
- SSE specification (HTML Living Standard) -- Default retry interval is implementation-defined (~3 seconds in most browsers), `retry:` field from server
- discord.js Events documentation -- `ShardDisconnect`, `ShardReconnecting`, `ShardResume` events for gateway lifecycle
- Community articles on SSE error handling patterns -- Heartbeat timeout detection, reconnection with history reload

### Tertiary (LOW confidence)
- Browser background tab throttling behavior -- Varies by browser and version. Chrome throttles to 1 minute for inactive tabs, Firefox similar. Effect on heartbeat timeout detection is estimated, not measured.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- No new libraries needed; all built on existing Web APIs and installed packages
- Architecture (enhancement patterns): HIGH -- Straightforward enhancements to existing code; patterns are well-established
- Pitfalls: HIGH -- Based on direct code analysis of current implementation and well-known EventSource behaviors
- Testing patterns: HIGH -- MockEventSource already exists in test suite; enhancements follow same pattern

**Research date:** 2026-02-11
**Valid until:** 2026-03-13 (30 days -- stable Web APIs, no version churn)
