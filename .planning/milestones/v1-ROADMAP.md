# Milestone v1: Discord Discussion Panel

**Status:** SHIPPED 2026-02-11
**Phases:** 1-4
**Total Plans:** 10

## Overview

Users can participate in the Discord discussion about a document without leaving the editor. When a document has a `discussion` frontmatter field linking to a Discord channel or forum thread, the editor shows that conversation alongside the document — users can read messages in real time and post via bot API using a self-reported name.

## Phases

### Phase 1: Bridge + History Display

**Goal:** User opens a document with a `discussion` frontmatter field and sees the last 50 Discord messages displayed in a chat panel with usernames, avatars, and timestamps.
**Depends on:** None
**Plans:** 3 plans

Plans:
- [x] 01-01-PLAN.md — TDD utility functions (frontmatter, Discord URL, avatar, timestamp)
- [x] 01-02-PLAN.md — Discord bridge sidecar proxy + Vite proxy config
- [x] 01-03-PLAN.md — DiscussionPanel UI + EditorArea integration

**Success Criteria:**
1. User opens a document containing `discussion: https://discord.com/channels/.../...` and a chat panel appears in the sidebar
2. The panel shows a loading spinner, then displays the last 50 messages from the linked Discord channel
3. Each message shows the author's Discord username and avatar image
4. Each message shows a human-readable timestamp (relative for recent, absolute for older)
5. Documents without a `discussion` field show no chat panel

**Requirements:** INFRA-01, INFRA-04, CHAN-01, CHAN-02, MSG-01, MSG-03, MSG-04, UX-01

### Phase 2: Live Streaming

**Goal:** After loading history, new messages posted in Discord appear in the panel in real time without page reload.
**Depends on:** Phase 1
**Plans:** 3 plans

Plans:
- [x] 02-01-PLAN.md — Gateway connection manager + SSE endpoint (bridge-side)
- [x] 02-02-PLAN.md — Discord markdown rendering (DiscordMarkdown component)
- [x] 02-03-PLAN.md — SSE client, auto-scroll, new messages indicator (frontend)

**Success Criteria:**
1. A message posted in Discord appears in the panel within 2 seconds
2. Messages render Discord-flavored markdown (bold, italic, code blocks, quotes, strikethrough)
3. The panel auto-scrolls to show new messages when the user is at the bottom
4. When the user scrolls up to read older messages, auto-scroll stops and a "new messages" indicator appears at the bottom

**Requirements:** INFRA-02, MSG-02, MSG-05, MSG-06, MSG-07

### Phase 3: Posting Messages

**Goal:** User can send messages to the Discord channel from within the editor panel using a self-reported display name.
**Depends on:** Phase 2
**Plans:** 3 plans

Plans:
- [x] 03-01-PLAN.md — Bot message proxy endpoint in discord-bridge (INFRA-03, POST-03)
- [x] 03-02-PLAN.md — Display name identity system with modal and badge (POST-02)
- [x] 03-03-PLAN.md — Compose box and send integration (POST-01)

**Success Criteria:**
1. User enters a display name once and it persists across browser sessions
2. User types a message and presses Enter/Send; the message appears in Discord and echoes back to the panel
3. Messages posted from the editor show the user's name with "(unverified)" appended, distinguishing them from native Discord users
4. Webhook URLs are never exposed to the browser (posting goes through the sidecar proxy)

**Requirements:** INFRA-03, POST-01, POST-02, POST-03

### Phase 4: Connection Resilience

**Goal:** The panel communicates connection problems clearly and helps the user recover without manual page reloads.
**Depends on:** Phase 2
**Plans:** 1 plan

Plans:
- [x] 04-01-PLAN.md — Connection resilience: gateway status broadcasting, SSE reconnection with refetch, heartbeat timeout, terminal disconnect + retry, status text labels

**Success Criteria:**
1. When the SSE connection drops, a visible status indicator changes from "Live" to "Reconnecting" or "Disconnected"
2. When the Discord API or bridge is unreachable, the panel shows an error message with a "Retry" button
3. Clicking "Retry" re-establishes the connection and reloads message history without a full page reload

**Requirements:** UX-02, UX-03

## Dependency Graph

```
Phase 1: Bridge + History Display
    |
    v
Phase 2: Live Streaming ──> Phase 4: Connection Resilience
    |
    v
Phase 3: Posting Messages
```

---

## Milestone Summary

**Key Decisions:**

| Decision | Phase | Rationale |
|----------|-------|-----------|
| Bot API instead of webhooks for posting | 03-01 | Simpler setup, reuses existing bot token |
| Server-side "(unverified)" suffix | 03-01 | Browser cannot bypass it |
| AST-to-React for Discord markdown | 02-02 | Safe XSS-free rendering |
| IntersectionObserver sentinel for scroll | 02-03 | More reliable than scroll math |
| EventSource dedup via state updater | 02-03 | Avoids stale closure issues |
| ConnectedDiscussionPanel wrapper pattern | 01-03 | Separates context from testable component |
| Hono over Express for sidecar | 01-02 | 14KB vs 572KB, TypeScript-native |
| front-matter npm for YAML parsing | 01-01 | Robust edge case handling |
| BigInt for Discord snowflake IDs | 01-01 | IDs exceed MAX_SAFE_INTEGER |
| 75s heartbeat timeout (2.5x interval) | 04-01 | Balances detection speed vs false positives |
| sseReconnectTrigger state for CLOSED recovery | 04-01 | Browsers don't auto-reconnect CLOSED EventSources |

**Issues Resolved:**
- Vite host binding for dev.vps tunnel access (added `host: true`)
- NewMessagesBar positioning (moved to wrapper div pattern)
- Tailwind cursor-pointer reset on buttons
- DisplayNameProvider needed in test renders after Phase 3

**Issues Deferred:**
- Discord mention resolution (show placeholder badges with raw IDs)
- Rich rendering (embeds, reactions, images) — v2
- Mobile-optimized layout — separate project

**Technical Debt Incurred:**
- Phase 1 missing formal VERIFICATION.md (all plans have summaries, integration verified)
- Orphaned `/api/gateway/status` endpoint (unused by frontend, SSE provides status)
- Missing `discord-bridge/.env.example` for token documentation

---

*For current project status, see .planning/PROJECT.md*
*Archived: 2026-02-11 as part of v1 milestone completion*
