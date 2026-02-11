---
milestone: v1
audited: 2026-02-11T11:00:00Z
status: tech_debt
scores:
  requirements: 19/19
  phases: 3/4 verified (Phase 1 missing VERIFICATION.md)
  integration: 15/16 exports wired (1 orphaned endpoint)
  flows: 5/5 E2E flows complete
gaps: []
tech_debt:
  - phase: 01-bridge-history-display
    items:
      - "Missing VERIFICATION.md — all 3 plans completed with summaries but phase never formally verified"
  - phase: global
    items:
      - "Orphaned /api/gateway/status endpoint in discord-bridge (unused by frontend, SSE provides status instead)"
      - "Missing discord-bridge/.env.example for DISCORD_BOT_TOKEN documentation"
      - "Minor package.json discord:setup script text doesn't match actual startup flow"
---

# v1 Milestone Audit: Discord Discussion Panel

**Audited:** 2026-02-11
**Status:** Tech Debt (no blockers, all requirements satisfied)
**Integration Grade:** A- (95%)

## Requirements Coverage

| Requirement | Phase | Status | Evidence |
|-------------|-------|--------|----------|
| CHAN-01: Editor detects `discussion` frontmatter | 1 | ✓ SATISFIED | useDiscussion.ts observes Y.Doc, extractFrontmatter + parseDiscordUrl extract channelId |
| CHAN-02: Chat panel displays for discussion docs | 1 | ✓ SATISFIED | ConnectedDiscussionPanel in EditorArea, renders when channelId present |
| MSG-01: Fetches last 50 messages on open | 1 | ✓ SATISFIED | useMessages fetches /api/discord/channels/:id/messages?limit=50 |
| MSG-02: New messages stream live via gateway | 2 | ✓ SATISFIED | Gateway → gatewayEvents → SSE → EventSource → useMessages |
| MSG-03: Messages show username and avatar | 1 | ✓ SATISFIED | MessageItem renders getAvatarUrl + global_name/username |
| MSG-04: Messages show timestamps | 1 | ✓ SATISFIED | formatTimestamp in MessageItem (relative/absolute) |
| MSG-05: Discord-flavored markdown | 2 | ✓ SATISFIED | DiscordMarkdown component (AST-to-React, no innerHTML) |
| MSG-06: Auto-scroll, stops when scrolled up | 2 | ✓ SATISFIED | useAutoScroll with IntersectionObserver sentinel |
| MSG-07: New messages indicator when scrolled up | 2 | ✓ SATISFIED | NewMessagesBar with unseen count |
| POST-01: Post via bot API with display name | 3 | ✓ SATISFIED | ComposeBox → sendMessage → bridge POST → sendBotMessage |
| POST-02: Display name persisted in localStorage | 3 | ✓ SATISFIED | DisplayNameContext uses localStorage key 'lens-editor-display-name' |
| POST-03: "(unverified)" tag appended | 3 | ✓ SATISFIED | Bridge formats server-side: `${username} (unverified)` |
| UX-01: Loading spinner | 1 | ✓ SATISFIED | DiscussionPanel shows spinner while loading |
| UX-02: Error state with retry button | 4 | ✓ SATISFIED | Error message + Retry/Reconnect buttons |
| UX-03: Connection status indicator | 4 | ✓ SATISFIED | StatusIndicator with Live/Reconnecting/Disconnected text labels |
| INFRA-01: Discord bot sidecar service | 1 | ✓ SATISFIED | discord-bridge/ Hono server with Gateway + REST + SSE |
| INFRA-02: SSE endpoint for live events | 2 | ✓ SATISFIED | GET /api/channels/:channelId/events with streamSSE |
| INFRA-03: Bot proxy for posting | 3 | ✓ SATISFIED | POST /api/channels/:channelId/messages, token never exposed |
| INFRA-04: REST proxy for message history | 1 | ✓ SATISFIED | GET /api/channels/:channelId/messages with caching |

**Score: 19/19 requirements satisfied**

## Phase Verification Status

| Phase | VERIFICATION.md | Status | Score |
|-------|----------------|--------|-------|
| 1: Bridge + History Display | Missing | Unverified* | 3/3 plans completed |
| 2: Live Streaming | ✓ | Passed | 7/7 must-haves |
| 3: Posting Messages | ✓ | Passed | 4/4 requirements |
| 4: Connection Resilience | ✓ | Passed | 6/6 must-haves |

*Phase 1 has all 3 plan summaries showing completion but was never formally verified with a VERIFICATION.md. The integration checker confirmed all Phase 1 artifacts exist and are wired correctly.

## Cross-Phase Integration

### Wiring Status

- **15/16 exports** properly consumed across phases
- **1 orphaned endpoint:** `/api/gateway/status` (bridge exposes it, frontend never calls it — status comes via SSE instead)
- **0 missing connections**

### API Route Coverage

| Route | Method | Status |
|-------|--------|--------|
| /api/channels/:id/messages | GET | ✓ Connected |
| /api/channels/:id | GET | ✓ Connected |
| /api/channels/:id/messages | POST | ✓ Connected |
| /api/channels/:id/events | GET (SSE) | ✓ Connected |
| /api/gateway/status | GET | Orphaned |
| /health | GET | OK (internal) |

### Type Consistency

- Bridge types.ts ↔ Frontend useMessages.ts: **Match** (DiscordUser, DiscordMessage, DiscordChannel)
- Gateway status enum: **Match** (connected | connecting | reconnecting | disconnected)
- SSE event names: **Match** (message, status, heartbeat)

## E2E Flow Verification

| # | Flow | Status | Path |
|---|------|--------|------|
| 1 | Document → Panel display | ✓ Complete | Y.Doc → useDiscussion → parseDiscordUrl → useMessages → REST → MessageList |
| 2 | Live streaming | ✓ Complete | Gateway → gatewayEvents → SSE → EventSource → dedup → MessageList → auto-scroll |
| 3 | Posting messages | ✓ Complete | ComposeBox → sendMessage → bridge POST → "(unverified)" → Discord API → echo via SSE |
| 4 | Connection recovery | ✓ Complete | SSE drop → heartbeat timeout → Reconnecting → reconnect → refetch → Live |
| 5 | Terminal disconnect + retry | ✓ Complete | CLOSED → Disconnected → Reconnect button → new EventSource + refetch |

## Error Handling Coverage

| Error Type | Bridge Response | Frontend Handling | User Feedback |
|------------|----------------|-------------------|---------------|
| Missing bot token | 500 | Sets error state | Error message with Retry |
| Discord 429 | 429 + retryAfter | Displays rate limit message | "Rate limited" with Retry |
| Discord API error | 4xx | Displays error | Error message with Retry |
| SSE disconnect | EventSource.onerror | Sets reconnecting/disconnected | Yellow/gray dot + Reconnect |
| Webhook send failure | 500 | Restores message text | Inline error below compose |

## Anti-Patterns

No anti-patterns found across any phase. All code is production-quality with:
- No TODO/FIXME comments in implementation
- No placeholder content or stubs
- No empty return statements (guard clauses are appropriate)
- Console.log limited to server-side error logging

## Tech Debt

### Phase 1: Bridge + History Display
- Missing VERIFICATION.md (phase functionally complete per plan summaries and integration checker)

### Global
- Orphaned `/api/gateway/status` endpoint (could remove or keep as debug endpoint)
- Missing `discord-bridge/.env.example` for DISCORD_BOT_TOKEN documentation
- Minor `lens-editor/package.json` discord:setup script text mismatch

**Total: 4 items across 2 categories**

## Human Verification Record

- Phase 2: Live streaming E2E verified by user (message posted in Discord → appeared in panel)
- Phase 3: Posting E2E verified by user via browser (message posted from panel → appeared in Discord with "(unverified)" tag)
- Phase 4: Structural verification only (connection resilience tests cover all states)

---
*Audited: 2026-02-11*
*Integration checker: Claude (gsd-integration-checker)*
