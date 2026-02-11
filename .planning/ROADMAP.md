# Roadmap: Discord Discussion Panel

**Created:** 2026-02-08
**Depth:** Quick (4 phases)
**Core Value:** Users can participate in the Discord discussion about a document without leaving the editor.

## Phases

| # | Phase | Goal | Requirements | Plans (est.) |
|---|-------|------|--------------|--------------|
| 1 | Bridge + History Display | User sees Discord message history in a panel alongside the document | INFRA-01, INFRA-04, CHAN-01, CHAN-02, MSG-01, MSG-03, MSG-04, UX-01 | 3 plans |
| 2 | Live Streaming | New Discord messages appear in the panel in real time | INFRA-02, MSG-02, MSG-05, MSG-06, MSG-07 | 3 plans |
| 3 | Posting Messages | User can send messages to Discord from the editor panel | INFRA-03, POST-01, POST-02, POST-03 | 3 plans |
| 4 | Connection Resilience | Panel handles network failures gracefully with clear feedback | UX-02, UX-03 | 1 plan |

## Phase Details

### Phase 1: Bridge + History Display
**Goal:** User opens a document with a `discussion` frontmatter field and sees the last 50 Discord messages displayed in a chat panel with usernames, avatars, and timestamps.
**Requirements:** INFRA-01, INFRA-04, CHAN-01, CHAN-02, MSG-01, MSG-03, MSG-04, UX-01
**Plans:** 3 plans

Plans:
- [x] 01-01-PLAN.md -- TDD utility functions (frontmatter, Discord URL, avatar, timestamp)
- [x] 01-02-PLAN.md -- Discord bridge sidecar proxy + Vite proxy config
- [x] 01-03-PLAN.md -- DiscussionPanel UI + EditorArea integration

**Success Criteria:**
1. User opens a document containing `discussion: https://discord.com/channels/.../...` and a chat panel appears in the sidebar
2. The panel shows a loading spinner, then displays the last 50 messages from the linked Discord channel
3. Each message shows the author's Discord username and avatar image
4. Each message shows a human-readable timestamp (relative for recent, absolute for older)
5. Documents without a `discussion` field show no chat panel

### Phase 2: Live Streaming
**Goal:** After loading history, new messages posted in Discord appear in the panel in real time without page reload.
**Requirements:** INFRA-02, MSG-02, MSG-05, MSG-06, MSG-07
**Plans:** 3 plans

Plans:
- [x] 02-01-PLAN.md -- Gateway connection manager + SSE endpoint (bridge-side)
- [x] 02-02-PLAN.md -- Discord markdown rendering (DiscordMarkdown component)
- [x] 02-03-PLAN.md -- SSE client, auto-scroll, new messages indicator (frontend)

**Success Criteria:**
1. A message posted in Discord appears in the panel within 2 seconds
2. Messages render Discord-flavored markdown (bold, italic, code blocks, quotes, strikethrough)
3. The panel auto-scrolls to show new messages when the user is at the bottom
4. When the user scrolls up to read older messages, auto-scroll stops and a "new messages" indicator appears at the bottom

### Phase 3: Posting Messages
**Goal:** User can send messages to the Discord channel from within the editor panel using a self-reported display name.
**Requirements:** INFRA-03, POST-01, POST-02, POST-03
**Plans:** 3 plans

Plans:
- [x] 03-01-PLAN.md -- Bot message proxy endpoint in discord-bridge (INFRA-03, POST-03)
- [x] 03-02-PLAN.md -- Display name identity system with modal and badge (POST-02)
- [x] 03-03-PLAN.md -- Compose box and send integration (POST-01)

**Success Criteria:**
1. User enters a display name once and it persists across browser sessions
2. User types a message and presses Enter/Send; the message appears in Discord and echoes back to the panel
3. Messages posted from the editor show the user's name with "(unverified)" appended, distinguishing them from native Discord users
4. Webhook URLs are never exposed to the browser (posting goes through the sidecar proxy)

### Phase 4: Connection Resilience
**Goal:** The panel communicates connection problems clearly and helps the user recover without manual page reloads.
**Requirements:** UX-02, UX-03
**Plans:** 1 plan

Plans:
- [x] 04-01-PLAN.md -- Connection resilience: gateway status broadcasting, SSE reconnection with refetch, heartbeat timeout, terminal disconnect + retry, status text labels

**Success Criteria:**
1. When the SSE connection drops, a visible status indicator changes from "Live" to "Reconnecting" or "Disconnected"
2. When the Discord API or bridge is unreachable, the panel shows an error message with a "Retry" button
3. Clicking "Retry" re-establishes the connection and reloads message history without a full page reload

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

- **Phase 2** depends on Phase 1 (bridge must be running, panel must exist to stream into)
- **Phase 3** depends on Phase 2 (posted messages echo back via the live stream; the full read path must work first)
- **Phase 4** depends on Phase 2 (connection status only matters once live streaming exists)
- Phases 3 and 4 are independent of each other and can be built in parallel

---
*Roadmap created: 2026-02-08*
