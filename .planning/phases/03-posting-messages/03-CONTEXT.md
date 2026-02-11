# Phase 3: Posting Messages - Context

**Gathered:** 2026-02-11
**Status:** Ready for planning

<domain>
## Phase Boundary

User can send messages to the Discord channel from within the editor panel using a self-reported display name. The display name is also used for comments/suggestions (broader than just Discord posting). Webhook URLs are never exposed to the browser.

</domain>

<decisions>
## Implementation Decisions

### Identity setup
- On page load, check localStorage for a stored display name
- If no name exists: show a **non-closable overlay modal** that blocks all interaction until a name is set
- If a name exists: display it at the top of the screen (global, not just in the panel)
- The name is used for both Discord posting AND marking suggestions/comments elsewhere in the editor
- Clicking the displayed name allows editing it
- Name is stored in localStorage and persists across sessions (POST-02)

### Compose experience
- Single-line text input docked at the bottom of the 320px discussion panel
- Send button on the right side of the input
- Enter sends the message, Shift+Enter inserts a newline
- Input grows vertically up to ~4 lines, then scrolls internally
- No markdown toolbar — users type raw Discord markdown (we already render it)
- Placeholder text: "Message #channel-name" (Discord-style)
- Send button disabled and Enter ignored on whitespace-only input

### Send feedback
- No optimistic insert — avoids dedup complexity with the SSE echo
- On send: input clears immediately, input briefly disables to prevent double-send
- Message echoes back through the existing SSE stream and appears naturally in the message list
- On POST failure: message text restores into the input, inline error appears below ("Failed to send — try again")

### Discord-side appearance
- Webhook username format: `DisplayName (unverified)` — satisfies POST-03
- No custom avatar (Discord's default webhook avatar) — revisit if a lens-editor logo is created later
- On the panel side, echoed messages render identically to all other Discord messages — the `(unverified)` suffix in the author name is the only distinguisher

### Claude's Discretion
- Exact modal styling and copy for the display name prompt
- Name validation approach (client-side vs letting bridge return errors for Discord-invalid names)
- Whether to show a subtle "Sending..." indicator if SSE echo takes >3s
- Input field styling details (border, focus state, send button icon)
- Error message styling and dismiss behavior

</decisions>

<specifics>
## Specific Ideas

- Identity modal should be the same one used for other editor features (comments/suggestions) — this is a global user identity, not Discord-specific
- The modal is non-closable: no X button, no click-outside-to-dismiss, no Escape key dismiss
- Name displayed at top of screen should be visible from any editor view

</specifics>

<deferred>
## Deferred Ideas

- Custom avatar selection for webhook messages — could be added later if lens-editor gets a logo/brand
- Rich compose (markdown toolbar, emoji picker, file attachments) — separate phase if needed
- User authentication / verified identity — fundamentally different from self-reported names

</deferred>

---

*Phase: 03-posting-messages*
*Context gathered: 2026-02-11*
