# Project State: Discord Discussion Panel

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-08)

**Core value:** Users can participate in the Discord discussion about a document without leaving the editor.
**Current focus:** Phase 2 complete. Ready for Phase 3 (Posting Messages) or Phase 4 (Connection Resilience).

## Position

- **Current phase:** 2 of 4 (Live Streaming) -- COMPLETE
- **Plan:** 3 of 3 in phase complete
- **Status:** Phase complete
- **Last activity:** 2026-02-10 - Completed 02-03-PLAN.md (SSE client, auto-scroll, new messages indicator)

Progress: `[######...] 6/9 plans (67%)`

## Recent Decisions

| Decision | Made In | Rationale |
|----------|---------|-----------|
| EventSource dedup via state updater function | 02-03 | prev.some() inside setMessages avoids stale closure issues |
| IntersectionObserver sentinel for scroll detection | 02-03 | 1px sentinel div more reliable than scroll math calculations |
| Wrapper div pattern for floating indicator | 02-03 | NewMessagesBar outside scroll container prevents it scrolling away |
| AST-to-React rendering for Discord markdown | 02-02 | Safe XSS-free rendering without dangerouslySetInnerHTML |
| Graceful fallback for unresolved Discord mentions | 02-02 | Mentions need API calls to resolve; show styled placeholder badges |
| div wrapper instead of p for message content | 02-02 | DiscordMarkdown renders block-level elements (pre, blockquote) invalid inside p |
| ConnectedDiscussionPanel wrapper pattern | 01-03 | Separates YDocProvider context from testable component |
| APP badge for bot messages | 01-03 | Matches Discord native UI, user-requested enhancement |
| host: true in vite.config.ts | 01-03 | Required for dev.vps tunnel access |
| LucDevBot2 token from lens-platform | 01-02 | REST-only bridge (no Gateway), safe to reuse existing bot |
| formatTimestamp accepts `string\|number` union | 01-01 | Dual Discord API (ISO) and CommentsPanel (epoch) compatibility |
| front-matter npm package for YAML parsing | 01-01 | Robust edge case handling vs hand-rolled regex |
| BigInt for Discord snowflake ID arithmetic | 01-01 | User IDs exceed Number.MAX_SAFE_INTEGER |

## Blockers

(None)

## Session Continuity

- **Last session:** 2026-02-10
- **Stopped at:** Completed 02-03-PLAN.md (Phase 2 complete)
- **Resume file:** None (next phase planning needed)

---
*Last updated: 2026-02-10 after completing plan 02-03 (Phase 2 complete)*
