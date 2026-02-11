---
phase: 03-posting-messages
plan: 01
subsystem: api
tags: [discord, webhook, hono, proxy, validation]

# Dependency graph
requires:
  - phase: 01-read-only-panel
    provides: discord-bridge sidecar with Hono HTTP server, error handling patterns
provides:
  - POST /api/channels/:channelId/messages webhook proxy endpoint
  - executeWebhook function for Discord webhook execution
  - validateWebhookUsername validation function
  - WebhookPayload type
affects: [03-posting-messages, lens-editor compose UI]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Webhook proxy pattern: browser sends content+username, bridge appends suffix and forwards to Discord"
    - "Webhook URL resolution: DISCORD_WEBHOOK_MAP (per-channel) with DISCORD_WEBHOOK_URL fallback"

key-files:
  created: []
  modified:
    - discord-bridge/src/types.ts
    - discord-bridge/src/discord-client.ts
    - discord-bridge/src/index.ts

key-decisions:
  - "Webhook URL resolved from DISCORD_WEBHOOK_MAP (per-channel JSON) or DISCORD_WEBHOOK_URL (default fallback)"
  - "Username validation checks length (1-80) and clyde substring before sending to Discord"
  - "503 status for missing webhook config (not 500) with clear error message"

patterns-established:
  - "Webhook proxy: never expose webhook URL to browser; bridge constructs request internally"
  - "Server-side username suffix: ' (unverified)' appended in endpoint handler, not in client code"

# Metrics
duration: 5min
completed: 2026-02-11
---

# Phase 3 Plan 1: Webhook Proxy Endpoint Summary

**POST webhook proxy endpoint in discord-bridge with input validation, server-side (unverified) suffix, and 503 for missing webhook config**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-11T09:09:30Z
- **Completed:** 2026-02-11T09:14:46Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- WebhookPayload type, executeWebhook function, and validateWebhookUsername exported from discord-client
- POST /api/channels/:channelId/messages endpoint with full input validation (content required, max 2000 chars, username required, no "clyde")
- Server-side " (unverified)" suffix appended to username before sending to Discord
- Webhook URL never exposed in any API response or client-facing code
- Startup warning logged when no webhook URL is configured

## Task Commits

Each task was committed atomically:

1. **Task 1: Add WebhookPayload type and executeWebhook function** - `a495d5e7` (feat)
2. **Task 2: Add POST webhook proxy endpoint** - `a3c950a0` (feat)

## Files Created/Modified
- `discord-bridge/src/types.ts` - Added WebhookPayload interface
- `discord-bridge/src/discord-client.ts` - Added webhook URL resolution, validateWebhookUsername, and executeWebhook function
- `discord-bridge/src/index.ts` - Added POST /api/channels/:channelId/messages endpoint and webhook startup warning

## Decisions Made
- Webhook URL resolution uses DISCORD_WEBHOOK_MAP (JSON mapping channelId to URL) with DISCORD_WEBHOOK_URL as default fallback
- validateWebhookUsername checks the final username (with suffix) for length and clyde validation
- Missing webhook config returns 503 (Service Unavailable) rather than 500, matching the semantics of a missing configuration dependency
- Error handling follows exact same pattern as existing GET endpoints (RateLimitError -> 429, DiscordApiError -> forward status, generic -> 500)

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
- Linter reverted index.ts changes twice by removing unused imports (executeWebhook, validateWebhookUsername) during intermediate edits. Resolved by writing the complete file in a single operation so all imports are used at write time.

## User Setup Required

None - no external service configuration required. Webhook URL environment variables (DISCORD_WEBHOOK_URL or DISCORD_WEBHOOK_MAP) are configured at deployment time, not during development.

## Next Phase Readiness
- Webhook proxy endpoint is ready for the compose UI (plan 03-03) to POST messages
- DisplayNameContext (plan 03-02) provides the username for POST requests
- All validation error responses are structured JSON for the compose UI to display

---
*Phase: 03-posting-messages*
*Completed: 2026-02-11*
