# Project State: Discord Discussion Panel

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-08)

**Core value:** Users can participate in the Discord discussion about a document without leaving the editor.
**Current focus:** Phase 3 complete. Phase 4 (Connection Resilience) remaining.

## Position

- **Current phase:** 3 of 4 complete (Posting Messages)
- **Plan:** 3 of 3 in phase complete (03-01, 03-02, 03-03 done)
- **Status:** Phase 3 verified and complete
- **Last activity:** 2026-02-11 - Completed Phase 3 (Posting Messages)

Progress: `[#########.] 9/10 plans (90%)`

## Recent Decisions

| Decision | Made In | Rationale |
|----------|---------|-----------|
| Bot API instead of webhooks for posting | 03-01 (refactored) | User preference: simpler setup, reuses existing bot token, no webhook URL needed |
| Server-side message formatting with "(unverified)" | 03-01 | Ensures suffix is always applied; browser cannot bypass it |
| Plain div overlay for non-closable modal (not Radix) | 03-02 | Radix Dialog is dismissable by design; plain div gives full control |
| maxLength 66 for display name input | 03-02 | 80 minus 14 chars for " (unverified)" suffix appended by bridge |
| Client-side "clyde" rejection | 03-02 | Discord rejects webhook usernames containing "clyde" |
| DisplayNameProvider outside NavigationContext | 03-02 | App-global identity scope, not navigation-scoped |
| Global identity bar above main layout | 03-02 | Flex-col restructure to stack identity bar above sidebar+editor |
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

- **Last session:** 2026-02-11
- **Stopped at:** Completed Phase 3 (Posting Messages) - all 3 plans executed, verified
- **Resume file:** None

---
*Last updated: 2026-02-11 after completing Phase 3 (Posting Messages)*
