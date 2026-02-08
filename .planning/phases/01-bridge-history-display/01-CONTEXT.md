# Phase 1: Bridge + History Display - Context

**Gathered:** 2026-02-08
**Status:** Ready for planning

<domain>
## Phase Boundary

A sidecar proxy bridges the lens-editor to Discord's API. The user opens a document with a `discussion` frontmatter field and sees the last 50 Discord messages displayed in a read-only chat panel with usernames, avatars, and timestamps. Documents without the field show no panel. Live streaming and posting are separate phases.

</domain>

<decisions>
## Implementation Decisions

### Panel layout & placement
- Right sidebar, vertical panel to the right of the editor
- Show the Discord channel name (e.g., #document-discussion) as the panel header

### Claude's Discretion
- Panel width and whether it's resizable or fixed
- Whether the panel has a toggle button or is always visible when a discussion field exists
- Message presentation: compact vs spacious, avatar size, timestamp format, grouping of consecutive messages from the same author
- Loading state design (spinner, skeleton, etc.)
- Empty state design (no messages in channel)
- Error state design (channel not found, API unreachable)
- How the panel opens — auto on document load vs user action
- Whether panel state persists across document switches

</decisions>

<specifics>
## Specific Ideas

No specific requirements — open to standard approaches.

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope.

</deferred>

---

*Phase: 01-bridge-history-display*
*Context gathered: 2026-02-08*
