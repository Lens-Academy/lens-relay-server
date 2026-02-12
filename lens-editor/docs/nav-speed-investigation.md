# Document Navigation Speed Investigation

## Problem

Switching documents in the sidebar takes ~500ms. Goal: significantly reduce this.

## Measured Timing Breakdown (navigation click → editor synced)

```
nav                          0ms
provider-mount              65ms  (+65ms)   React unmount/remount
auth-start                  80ms  (+15ms)
auth-end                   271ms  (+191ms)  ← AUTH: 191ms
ws-handshaking             484ms  (+213ms)  ← WS UPGRADE: 213ms
editor-synced              510ms  (+26ms)   ← Y.Sync: 26ms
```

Instrumentation code is in `src/lib/load-timing.ts`, with marks in `App.tsx`, `auth.ts`, `RelayProvider.tsx`, and `Editor.tsx`.

## Root Cause Analysis

### Auth is slow because of a Cloudflare hairpin

The auth flow is a double HTTP hop:

```
Browser → Vite middleware (in-process, fast)
       → relay.lensacademy.org/doc/{docId}/auth (through Cloudflare Tunnel)
       → back to browser
```

The relay server is on the **same VPS** as the Vite dev server, but the request goes through Cloudflare Tunnel and back. That's ~191ms of unnecessary network roundtrip.

Inside the relay server, `auth_doc` does:
1. `check_auth` — verify bearer token (local crypto, fast)
2. `doc_exists` — checks in-memory map first, falls back to S3/R2 (potentially slow for cold docs)
3. `gen_doc_token_auto` — sign a JWT (local crypto, fast)

### WS upgrade is 213ms

Each document navigation tears down the old WebSocket and creates a new one. The `key={activeDocId}` on `<RelayProvider>` forces full unmount/remount. The WS upgrade also goes through Cloudflare Tunnel.

### Y.Sync is only 26ms

Documents are hot in relay server memory during a session, so the actual CRDT sync is fast. This means IndexedDB caching (y-sweet's `offlineSupport`) would have minimal impact on navigation speed.

## Auth Token Details

- Tokens are valid for **60 minutes** (`DEFAULT_EXPIRATION_SECONDS = 60 * 60` in `crates/y-sweet-core/src/auth.rs:11`)
- Tokens are per-document (scoped to a specific `doc_id`)
- The relay server also has a `PrefixPermission` concept (folder-level auth) but no endpoint exposes it

## Options Considered

### A. Client-side token caching (recommended first step)

Cache `getClientToken()` results keyed by docId. Tokens are valid 60min, so revisits within a session skip auth entirely (191ms → 0ms).

- Zero server changes, no merge conflict risk
- Only helps revisits, not first visit to a doc
- Implementation: simple Map cache in `auth.ts` with TTL check

### B. Auth prefetching on hover/visibility

When a doc appears in the sidebar or user hovers, fire `getClientToken()` in the background. By click time, token is cached.

- Pairs with option A (prefetch fills the cache)
- Trades bandwidth for latency
- Need to be careful not to prefetch too aggressively

### C. Fix the Cloudflare hairpin (production config change)

Make the Express middleware call `localhost:8080` instead of `relay.lensacademy.org` for the relay auth call. Drops auth from 191ms to ~5ms for all docs.

- Production deployment change, not a code change
- Would need the relay server port/URL as a server-side env var
- Fixes auth for both first visits and revisits

### D. Connection pooling (keep providers alive)

Don't unmount `<RelayProvider>` on navigation — keep recent Y.Doc + WS connections alive in background. Revisits are instant (~20ms).

- Most complex option
- Risk: too many open WebSockets
- y-codemirror.next has a known issue (#19) where `ySyncFacet` caches at construction, making dynamic doc swapping fragile

### E. WebSocket reuse across documents

Not feasible — y-sweet WS connections are per-document by design. Tokens are scoped to a single doc_id.

### F. Yjs subdocuments

Not feasible with y-sweet architecture.

## Open Questions

1. **Is the 191ms auth time the same in production?** The measurements were taken in dev mode (Vite on VPS). In production, the Express server and relay server might communicate differently. Need to verify the production auth flow.

2. **What's the production deployment for lens-editor?** Is it served by the same Express server, or is there a separate setup? This affects whether the Cloudflare hairpin fix applies.

3. **Can we fix the hairpin with just an env var?** The `relayTarget` in `vite.config.ts` is already configurable. For production, if the Express server runs on the same machine as the relay, we could set `RELAY_SERVER_URL=http://localhost:8080` to bypass Cloudflare.

4. **How many docs does a typical session touch?** If users mostly bounce between 3-5 docs, token caching alone covers most navigations. If they're exploring many docs, prefetching becomes more important.

5. **What's the WS upgrade latency without Cloudflare?** If both auth AND WS go through Cloudflare, fixing the hairpin could cut total navigation time from 510ms to ~100ms (5ms auth + 50ms local WS + 26ms sync + overhead).

6. **Token caching vs. relay prefix tokens?** The relay server has `PrefixPermission` which could authenticate an entire folder at once. This would be one auth call for all docs in a folder, but requires a new server endpoint. Worth exploring if token caching isn't sufficient.

## Files Modified (instrumentation)

- `src/lib/load-timing.ts` — Created: timing instrumentation singleton
- `src/App.tsx` — Added `loadTimer.start()` on navigation
- `src/lib/auth.ts` — Added timing marks around `getClientToken()`
- `src/providers/RelayProvider.tsx` — Added `ConnectionTimingTracker`, provider-mount marks
- `src/components/Editor/Editor.tsx` — Added editor-create, editor-ready, editor-synced marks

## Key Files for Implementation

- `src/lib/auth.ts` — Where token caching would go
- `lens-editor/server/auth-middleware.ts` — Server-side auth proxy (hairpin fix here)
- `lens-editor/vite.config.ts` — `relayTarget` configuration
- `crates/relay/src/server.rs:2025` — Relay `auth_doc` endpoint
- `crates/y-sweet-core/src/auth.rs` — Token generation and expiration
