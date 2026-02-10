---
phase: 02-live-streaming
plan: 03
subsystem: ui
tags: [sse, eventsource, intersection-observer, auto-scroll, react-hooks]

# Dependency graph
requires:
  - phase: 02-01
    provides: SSE endpoint (/api/discord/channels/:channelId/events) and gateway status route
provides:
  - SSE client subscription in useMessages hook with deduplication
  - IntersectionObserver-based auto-scroll hook (useAutoScroll)
  - NewMessagesBar floating indicator component
  - Gateway connection status dot in panel header
affects: [04-connection-resilience]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "EventSource SSE subscription with dedup in React state updater"
    - "IntersectionObserver sentinel for scroll position detection"
    - "Unseen message counter with auto-reset on scroll-to-bottom"

key-files:
  created:
    - lens-editor/src/components/DiscussionPanel/useAutoScroll.ts
    - lens-editor/src/components/DiscussionPanel/NewMessagesBar.tsx
  modified:
    - lens-editor/src/components/DiscussionPanel/useMessages.ts
    - lens-editor/src/components/DiscussionPanel/MessageList.tsx
    - lens-editor/src/components/DiscussionPanel/DiscussionPanel.tsx
    - lens-editor/src/components/DiscussionPanel/DiscussionPanel.test.tsx

key-decisions:
  - "EventSource dedup via prev.some() inside setMessages updater to avoid stale closures"
  - "IntersectionObserver sentinel (1px div) for scroll-bottom detection instead of scroll event math"
  - "Wrapper div pattern for NewMessagesBar positioning outside scrollable container"

patterns-established:
  - "SSE subscription separate from REST fetch effect (different dependency arrays)"
  - "Sentinel-based scroll position tracking with IntersectionObserver"

# Metrics
duration: ~25min
completed: 2026-02-10
---

# Phase 2 Plan 3: SSE Client and Auto-Scroll Summary

**EventSource SSE subscription with message deduplication, IntersectionObserver auto-scroll, and floating new-messages indicator bar**

## Performance

- **Duration:** ~25 min
- **Started:** 2026-02-10T14:20:00Z (estimated)
- **Completed:** 2026-02-10T15:45:03Z
- **Tasks:** 3 (2 code + 1 human verification checkpoint)
- **Files modified:** 6

## Accomplishments
- SSE subscription in useMessages hook streams new Discord messages in real time with deduplication against REST-fetched history
- Auto-scroll via IntersectionObserver sentinel -- scrolls when at bottom, pauses when user scrolls up to read
- NewMessagesBar floating indicator shows unseen count and scrolls to bottom on click
- Gateway connection status dot in panel header (green=live, yellow-pulse=connecting, gray=disconnected)
- Full end-to-end live streaming verified by user: Discord message -> Gateway -> SSE -> browser panel in <2s

## Task Commits

Each task was committed atomically:

1. **Task 1: Extend useMessages with SSE subscription and create auto-scroll hook + indicator** - `ef427d16` (feat)
2. **Task 2: Integrate auto-scroll into MessageList and add gateway status to DiscussionPanel** - `bd00b4ec` (feat)
3. **Task 3: Verify live streaming end-to-end** - checkpoint, approved by user (no code commit)

**Orchestrator fix:** `b84f0bfa` (fix) - indicator positioning and cursor-pointer on clickable elements

## Files Created/Modified
- `lens-editor/src/components/DiscussionPanel/useMessages.ts` - Added SSE EventSource subscription with dedup and gatewayStatus state
- `lens-editor/src/components/DiscussionPanel/useAutoScroll.ts` - New hook: IntersectionObserver sentinel, unseen counter, scrollToBottom
- `lens-editor/src/components/DiscussionPanel/NewMessagesBar.tsx` - New component: floating pill button showing unseen message count
- `lens-editor/src/components/DiscussionPanel/MessageList.tsx` - Replaced bottomRef with sentinel, integrated useAutoScroll and NewMessagesBar
- `lens-editor/src/components/DiscussionPanel/DiscussionPanel.tsx` - Added gateway status colored dot to panel header
- `lens-editor/src/components/DiscussionPanel/DiscussionPanel.test.tsx` - Added EventSource mock, updated useMessages mock with gatewayStatus

## Decisions Made
- **EventSource dedup via state updater:** `prev.some((m) => m.id === newMsg.id)` inside `setMessages` updater function avoids stale closure issues that would occur with a direct state reference
- **IntersectionObserver sentinel over scroll math:** A 1px sentinel div at the bottom of the scroll container is more reliable and performant than calculating scrollTop + clientHeight vs scrollHeight
- **Wrapper div pattern for indicator positioning:** NewMessagesBar placed outside the scrollable container using absolute positioning relative to a wrapper, preventing it from scrolling away with content

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added EventSource mock to test suite**
- **Found during:** Task 2 (test integration)
- **Issue:** happy-dom test environment does not provide a global EventSource, causing useMessages SSE effect to fail in tests
- **Fix:** Added mock EventSource class in test setup that satisfies the hook's addEventListener/close interface
- **Files modified:** DiscussionPanel.test.tsx
- **Verification:** `npx vitest run` passes
- **Committed in:** bd00b4ec (Task 2 commit)

**2. [Rule 1 - Bug] Fixed NewMessagesBar positioning (orchestrator)**
- **Found during:** Post-task verification
- **Issue:** NewMessagesBar was absolutely positioned inside the scrollable container, causing it to scroll with content instead of floating at the bottom
- **Fix:** Moved to wrapper div pattern -- absolute inset-0 for scroll area, relative wrapper for positioning context
- **Files modified:** MessageList.tsx, NewMessagesBar.tsx
- **Committed in:** b84f0bfa (orchestrator fix)

**3. [Rule 1 - Bug] Added cursor-pointer to clickable elements (orchestrator)**
- **Found during:** Post-task verification
- **Issue:** Tailwind preflight resets button cursor to default; NewMessagesBar and BacklinksPanel buttons showed arrow cursor instead of pointer
- **Fix:** Added `cursor-pointer` class to button elements
- **Files modified:** NewMessagesBar.tsx, BacklinksPanel.tsx
- **Committed in:** b84f0bfa (orchestrator fix)

---

**Total deviations:** 3 auto-fixed (2 bugs, 1 blocking)
**Impact on plan:** All fixes necessary for correct test execution and UI behavior. No scope creep.

## Issues Encountered
None beyond the deviations documented above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 2 (Live Streaming) is complete -- all 3 plans delivered
- Full pipeline working: Discord Gateway -> bridge SSE -> browser panel with auto-scroll
- Ready for Phase 3 (Posting Messages) or Phase 4 (Connection Resilience)
- Phase 4 will build on the gatewayStatus infrastructure established here for detailed connection status UI

---
*Phase: 02-live-streaming*
*Completed: 2026-02-10*
