# Requirements: Discord Discussion Panel

**Defined:** 2026-02-08
**Core Value:** Users can participate in the Discord discussion about a document without leaving the editor.

## v1 Requirements

### Channel Mapping

- [x] **CHAN-01**: Editor detects `discussion` frontmatter field and extracts Discord channel/thread ID
- [x] **CHAN-02**: Chat panel automatically displays for documents with a `discussion` link

### Message Display

- [x] **MSG-01**: Panel fetches and displays last 50 messages on open
- [x] **MSG-02**: New messages stream in live via Discord bot gateway
- [x] **MSG-03**: Messages show author username and avatar
- [x] **MSG-04**: Messages show relative/absolute timestamps
- [x] **MSG-05**: Messages render Discord-flavored markdown (bold, italic, code, quotes, strikethrough)
- [x] **MSG-06**: Panel auto-scrolls to newest messages; stops when user scrolls up
- [x] **MSG-07**: "New messages" indicator appears when scrolled up and new messages arrive

### Posting

- [x] **POST-01**: User can post messages via bot API with self-reported display name
- [x] **POST-02**: Display name persisted in localStorage across sessions
- [x] **POST-03**: Bot messages show "(unverified)" tag appended to username

### UX

- [x] **UX-01**: Loading spinner shown while fetching message history
- [x] **UX-02**: Error state with retry button on connection failure
- [x] **UX-03**: Connection status indicator (live/reconnecting/disconnected)

### Infrastructure

- [x] **INFRA-01**: Discord bot sidecar service connects to gateway and streams events
- [x] **INFRA-02**: SSE endpoint delivers live channel events to browser clients
- [x] **INFRA-03**: Bot API proxy endpoint posts messages without exposing bot token to browser
- [x] **INFRA-04**: REST proxy endpoint fetches message history from Discord API

## v2 Requirements

### Rich Rendering

- **RICH-01**: Emoji rendering (Unicode native, custom Discord emoji as images)
- **RICH-02**: Syntax-highlighted code blocks (highlight.js integration)
- **RICH-03**: User mention resolution (`<@id>` → `@Username`)
- **RICH-04**: Link previews / embed rendering
- **RICH-05**: Inline image display for attachments

### UX Enhancements

- **UXV2-01**: Panel toggle (show/hide chat panel)
- **UXV2-02**: Keyboard shortcut to toggle panel (e.g., Ctrl+Shift+D)
- **UXV2-03**: Unread message count badge on collapsed panel
- **UXV2-04**: Edit/delete reflection (panel updates when Discord messages are edited/deleted)

## Out of Scope

| Feature | Reason |
|---------|--------|
| Discord OAuth login | Unnecessary complexity; self-reported name is sufficient |
| Channel switching | One document = one channel; auto-mapped from frontmatter |
| Posting from authenticated Discord accounts | Massive security surface; webhook is the designed abstraction |
| Reactions / threading | Complex subsystems with marginal value for a side panel |
| File upload from panel | Large attack surface; show download links for Discord attachments instead |
| Message editing/deleting from panel | Webhook messages are fire-and-forget; users clarify with follow-ups |
| Typing indicators | Marginal value for a side panel |
| Member list / online status | Noise; focus is the conversation |
| Custom CSS theming engine | Match editor's existing design system instead |
| Notification sounds | Annoying; visual-only notifications |
| Full Discord client reproduction | Impossible maintenance burden; users open Discord for full features |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| CHAN-01 | 1 | Complete |
| CHAN-02 | 1 | Complete |
| MSG-01 | 1 | Complete |
| MSG-02 | 2 | Complete |
| MSG-03 | 1 | Complete |
| MSG-04 | 1 | Complete |
| MSG-05 | 2 | Complete |
| MSG-06 | 2 | Complete |
| MSG-07 | 2 | Complete |
| POST-01 | 3 | Complete |
| POST-02 | 3 | Complete |
| POST-03 | 3 | Complete |
| UX-01 | 1 | Complete |
| UX-02 | 4 | Complete |
| UX-03 | 4 | Complete |
| INFRA-01 | 1 | Complete |
| INFRA-02 | 2 | Complete |
| INFRA-03 | 3 | Complete |
| INFRA-04 | 1 | Complete |

**Coverage:**
- v1 requirements: 19 total
- Mapped to phases: 19
- Unmapped: 0
- Phase 1: 8 requirements (CHAN-01, CHAN-02, MSG-01, MSG-03, MSG-04, UX-01, INFRA-01, INFRA-04)
- Phase 2: 5 requirements (MSG-02, MSG-05, MSG-06, MSG-07, INFRA-02)
- Phase 3: 4 requirements (POST-01, POST-02, POST-03, INFRA-03)
- Phase 4: 2 requirements (UX-02, UX-03)

---
*Requirements defined: 2026-02-08*
*Last updated: 2026-02-08 after roadmap creation (phase mapping added)*
