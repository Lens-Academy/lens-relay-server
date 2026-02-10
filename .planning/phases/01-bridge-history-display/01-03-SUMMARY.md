---
phase: 01-bridge-history-display
plan: 03
subsystem: discussion-panel-ui
tags: [react, tdd, discord, codemirror, ydoc, vitest, tailwind]
requires: ["01-01", "01-02"]
provides:
  - DiscussionPanel component rendering Discord messages in editor sidebar
  - ConnectedDiscussionPanel wrapper for YDocProvider context injection
  - useDiscussion hook for Y.Doc frontmatter observation
  - useMessages hook for Discord bridge API integration
  - MessageList with 5-minute author grouping
  - MessageItem with avatars, display names, timestamps, APP badges
affects:
  - Phase 2 (Live Streaming) will add WebSocket updates to useMessages
  - Phase 3 (Compose + Send) will add input below MessageList
tech-stack:
  added: []
  patterns: [tdd-red-green-refactor, connected-wrapper-pattern, real-fixture-testing]
key-files:
  created:
    - lens-editor/src/components/DiscussionPanel/DiscussionPanel.tsx
    - lens-editor/src/components/DiscussionPanel/DiscussionPanel.test.tsx
    - lens-editor/src/components/DiscussionPanel/DiscussionPanel.integration.test.tsx
    - lens-editor/src/components/DiscussionPanel/ConnectedDiscussionPanel.tsx
    - lens-editor/src/components/DiscussionPanel/MessageList.tsx
    - lens-editor/src/components/DiscussionPanel/MessageItem.tsx
    - lens-editor/src/components/DiscussionPanel/useDiscussion.ts
    - lens-editor/src/components/DiscussionPanel/useMessages.ts
    - lens-editor/src/components/DiscussionPanel/index.ts
  modified:
    - lens-editor/src/components/Layout/EditorArea.tsx
    - lens-editor/src/components/Layout/EditorArea.test.tsx
    - lens-editor/vite.config.ts
key-decisions:
  - "ConnectedDiscussionPanel wrapper separates YDocProvider context from testable DiscussionPanel (accepts doc prop)"
  - "APP badge on bot messages using Discord API bot field, styled with Discord blurple (#5865F2)"
  - "host: true added to vite.config.ts for dev.vps tunnel access (was missing per CLAUDE.md instructions)"
duration: ~20min
completed: 2026-02-10
---

# Phase 1 Plan 03: DiscussionPanel UI Summary

**TDD-built React component displaying Discord channel messages in the editor sidebar, with avatar grouping, APP badges for bots, and conditional rendering based on Y.Doc frontmatter.**

## Performance

- Duration: ~20 minutes (including visual verification checkpoint)
- TDD cycle: RED (13 failing, 3 passing) -> GREEN (16 passing) -> REFACTOR (minor cleanup) -> Checkpoint (visual approval + APP badge enhancement)

## Accomplishments

1. **DiscussionPanel.tsx** - Main panel component. Renders loading spinner, error state with retry button, or message list based on hook state. Returns null when no `discussion` frontmatter field.

2. **ConnectedDiscussionPanel.tsx** - Wrapper that reads Y.Doc from `@y-sweet/react` context and passes it as prop. Enables unit testing without YDocProvider.

3. **useDiscussion.ts** - Hook observing Y.Doc `getText('contents')` for frontmatter changes. Extracts `discussion` URL, parses with `parseDiscordUrl()`. Cleans up Y.Text observer on unmount.

4. **useMessages.ts** - Hook fetching messages + channel info in parallel from `/api/discord/channels/:id`. AbortController cleanup, 429 rate limit handling, chronological sort (API returns newest-first).

5. **MessageList.tsx** - Scrollable container with 5-minute same-author grouping logic. Auto-scrolls to bottom on load. Empty state message.

6. **MessageItem.tsx** - Avatar (64px via Discord CDN), display name (global_name fallback), relative timestamp, message content. Grouped messages show content only with left indent. APP badge (blurple pill) for bot authors.

7. **EditorArea integration** - ConnectedDiscussionPanel rendered as sibling after existing sidebar. No props needed. Existing tests protected with vi.mock.

## Task Commits

| Task | Name | Change ID | Type |
|------|------|-----------|------|
| 1 | Failing DiscussionPanel tests (RED) | mompwosqxnxn | test |
| 2 | Implement hooks and components (GREEN) | lqxtvmtnomvs | feat |
| 3 | Clean up useDiscussion hook (REFACTOR) | ykrkvtpmvpmz | refactor |
| 4 | APP badge + Vite host fix (checkpoint) | zprxommkrnyo | feat |

## Deviations from Plan

1. **ConnectedDiscussionPanel wrapper** - Plan called for DiscussionPanel to call `useYDoc()` directly. Testing required a wrapper pattern to avoid needing YDocProvider in test harness.

2. **EditorArea test mock** - Adding ConnectedDiscussionPanel to EditorArea broke 3 existing tests. Added `vi.mock` for DiscussionPanel module to isolate.

3. **APP badge for bots** - Not in original plan. Added during checkpoint verification per user feedback, matching Discord's native UI.

4. **Vite `host: true` fix** - vite.config.ts was missing `host: true`, causing dev.vps tunnel to fail. Fixed as part of checkpoint.

## Files Created/Modified

**Created:**
- `lens-editor/src/components/DiscussionPanel/DiscussionPanel.tsx` - Main panel
- `lens-editor/src/components/DiscussionPanel/DiscussionPanel.test.tsx` - 16 unit tests
- `lens-editor/src/components/DiscussionPanel/DiscussionPanel.integration.test.tsx` - Smoke tests (env-gated)
- `lens-editor/src/components/DiscussionPanel/ConnectedDiscussionPanel.tsx` - YDocProvider wrapper
- `lens-editor/src/components/DiscussionPanel/MessageList.tsx` - Grouped message list
- `lens-editor/src/components/DiscussionPanel/MessageItem.tsx` - Single message with avatar/badge
- `lens-editor/src/components/DiscussionPanel/useDiscussion.ts` - Y.Doc frontmatter hook
- `lens-editor/src/components/DiscussionPanel/useMessages.ts` - Discord API fetch hook
- `lens-editor/src/components/DiscussionPanel/index.ts` - Barrel export

**Modified:**
- `lens-editor/src/components/Layout/EditorArea.tsx` - Integrated ConnectedDiscussionPanel
- `lens-editor/src/components/Layout/EditorArea.test.tsx` - Added vi.mock for DiscussionPanel
- `lens-editor/vite.config.ts` - Added host: true for tunnel access
