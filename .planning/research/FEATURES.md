# Feature Landscape: Embedded Discord Chat Panel

**Domain:** Embedded Discord chat widget in a collaborative web editor
**Researched:** 2026-02-08
**Overall confidence:** MEDIUM (synthesized from multiple sources; some items verified via official docs, others from community patterns)

## Context

This research covers what features users expect from an in-app Discord chat experience embedded alongside a markdown editor. The comparison set includes:

- **WidgetBot** -- the dominant third-party Discord embed widget (20K+ servers, 12M+ users)
- **Titan Embeds** -- self-hostable open-source Discord embed (Python/Flask, PostgreSQL)
- **Discord's official widget** -- read-only member/channel list, no messaging
- **Custom bot+webhook architectures** -- the approach this project uses

Our project is *not* a general-purpose Discord embed. It is a purpose-built chat panel for a specific use case: discussion about the document being edited. This distinction drives what is table stakes vs. overkill.

---

## Table Stakes

Features users expect. Missing = product feels incomplete or broken.

| # | Feature | Why Expected | Complexity | Dependencies | Notes |
|---|---------|-------------|------------|--------------|-------|
| T1 | **Live message stream** | Users expect to see new messages appear in real time without refreshing. WidgetBot, Titan, and every chat widget does this. | Med | Bot WebSocket connection to Discord Gateway (MESSAGE_CREATE intent) | Requires MessageContent privileged intent. Messages arrive via Gateway, must be forwarded to browser clients. |
| T2 | **Message history on load** | Opening the panel should show recent messages, not a blank screen. Every chat widget loads history. | Low | Discord REST API (GET /channels/{id}/messages) | Fetch last 25-50 messages on panel open. Paginate on scroll-up if needed later. |
| T3 | **Post messages via webhook** | Users must be able to participate, not just lurk. WidgetBot does this with webhooks for guest users. | Low | Discord webhook URL per channel | Webhook allows custom username/avatar per message. Rate limit: ~5 req / 2 sec per webhook. |
| T4 | **Self-reported display name** | Users need an identity in chat. WidgetBot shows guest names; Titan supports guest usernames. Our "(unverified)" tag is a good honest approach. | Low | Local storage for name persistence, webhook `username` field | Persist chosen name in localStorage. Append "(unverified)" server-side or in the webhook username to prevent spoofing. |
| T5 | **Basic markdown rendering** | Discord messages use markdown. Showing raw `**bold**` text is unacceptable. WidgetBot renders full Discord markdown. | Med | discord-markdown-parser or similar npm library | Discord markdown differs from standard markdown (e.g., `~~strikethrough~~`, `||spoilers||`, `> quotes`). Use a Discord-specific parser. |
| T6 | **Timestamps on messages** | Every chat interface shows when messages were sent. Absence feels broken. | Low | Message `timestamp` field from Discord API | Relative timestamps ("2m ago") for recent, absolute for older. |
| T7 | **Author identification** | Users need to know who said what. Show username and avatar for Discord users; show name + "(unverified)" for webhook guests. | Low | Message `author` object from Discord API | Webhook messages have `author.bot: true` and `webhook_id` set. Distinguish visually. |
| T8 | **Scroll behavior** | Auto-scroll to newest messages; stop auto-scrolling if user scrolls up to read history. Standard chat UX. | Low | Frontend scroll logic | "New messages" indicator when scrolled up. Click to jump to bottom. |
| T9 | **Loading / error states** | Show spinner while loading history, error message if connection fails. Without these, users think the panel is broken. | Low | None | Include retry button on connection failure. |
| T10 | **Panel toggle / resize** | Users must be able to show/hide the chat panel. Editor space is primary. WidgetBot's "Crate" is a toggleable popup; their full widget is inline. | Low | UI layout state | Side panel with drag-to-resize or fixed width with toggle button. Remember open/closed state. |

---

## Differentiators

Features that set the product apart. Not universally expected, but create meaningful value in this specific context.

| # | Feature | Value Proposition | Complexity | Dependencies | Notes |
|---|---------|-------------------|------------|--------------|-------|
| D1 | **Document-aware channel mapping** | The killer feature: chat automatically shows the Discord channel for the document being edited. No other embed widget is context-aware. | Med | Mapping from document ID/path to Discord channel ID. Configuration or convention-based. | This is the core reason to build custom rather than use WidgetBot. |
| D2 | **Unobtrusive integration** | Chat panel sits alongside editor without stealing focus or space. Unlike WidgetBot's popup crate, this is architecturally part of the app. | Low | React component layout | Side panel, bottom panel, or collapsible drawer. Should not overlay editor content. |
| D3 | **Webhook identity with "(unverified)" tag** | Transparent trust model. Users know who is verified (Discord account) vs. who self-reported their name. No other embed does this clearly. | Low | T4 (display name) | Builds trust without requiring Discord login. WidgetBot's guest mode is less explicit about verification status. |
| D4 | **Message notifications (unread count)** | Badge on collapsed panel showing unread message count. WidgetBot's Crate does this. | Med | T1 (live stream), T10 (panel toggle) | Count messages received while panel is closed. Reset on open. |
| D5 | **Inline code block rendering** | Discord messages with code blocks render with syntax highlighting. Relevant for a developer/editor audience. | Med | T5 (markdown rendering), highlight.js or similar | discord-markdown supports highlight.js class output. Match editor's syntax highlighting theme. |
| D6 | **User mention rendering** | Render `<@123456789>` as `@Username` with styling. WidgetBot does this; raw mention IDs are ugly. | Med | Discord REST API to resolve user IDs, or cache from message authors | Can start with just styling the raw format, resolve names lazily. |
| D7 | **Emoji rendering** | Render Unicode emoji natively and custom Discord emoji as images. Messages with emoji are common. | Med | T5 (markdown rendering) | Unicode emoji: native browser rendering. Custom emoji: `<:name:id>` format, fetch from Discord CDN. Start with Unicode only. |
| D8 | **Message edit/delete reflection** | When someone edits or deletes a message on Discord, the panel updates. WidgetBot v3.8 added this. | Med | Bot Gateway events: MESSAGE_UPDATE, MESSAGE_DELETE | Important for accuracy. Without this, deleted messages persist in the panel and edited messages show stale content. |
| D9 | **Keyboard shortcut to toggle panel** | Quick access without mouse. Power user feature for an editor tool. | Low | T10 (panel toggle) | e.g., Ctrl+Shift+D. Must not conflict with editor shortcuts. |
| D10 | **Connection status indicator** | Show whether the Discord connection is live, reconnecting, or disconnected. | Low | T1 (live stream) | Small colored dot or text. Builds confidence that messages are current. |
| D11 | **Link previews / embed rendering** | Discord embeds (link previews, bot embeds) rendered inline. WidgetBot renders these. | High | T5 (markdown rendering) | Significant rendering complexity. Many embed fields: title, description, color, image, thumbnail, fields, footer, author. Defer to post-MVP. |
| D12 | **Attachment display** | Show images and file attachments inline. WidgetBot supports file uploading and display. | High | Image rendering, file type detection | Images: render inline with lightbox. Files: show filename + download link. Defer images to post-MVP; show download links early. |

---

## Anti-Features

Features to explicitly NOT build. Common mistakes or scope traps.

| # | Anti-Feature | Why Avoid | What to Do Instead |
|---|--------------|-----------|-------------------|
| A1 | **Full Discord client reproduction** | Titan Embeds explicitly warns: "This project is never to be used as a replacement for Discord app." Attempting channel lists, DMs, server switching, roles, etc. creates an impossible maintenance burden. Discord's rendering alone involves AST parsing, entity resolution, and cross-platform consistency. | Build a single-channel chat panel. Users who need full Discord functionality open Discord. |
| A2 | **Discord OAuth login** | Adding OAuth creates significant complexity (token management, refresh flows, permission scoping) and requires Discord app approval for larger scale. WidgetBot and Titan both support it, but for our use case (quick participation in document discussion), it is friction that reduces adoption. | Use self-reported names + webhook. Verified Discord users appear naturally when they post from Discord proper. |
| A3 | **Channel switching** | Tempting because WidgetBot supports it, but our value proposition is *automatic* channel selection based on document context. Manual channel switching breaks the mental model. | One document = one channel. Channel is derived from document, not chosen by user. |
| A4 | **Message sending from authenticated Discord accounts** | Requires OAuth + bot token management + user token proxying. Massive security surface. Discord ToS concerns with user token usage. | Webhook posting is the right abstraction. Webhook messages appear in Discord as bot messages with custom names -- this is the designed use case. |
| A5 | **Reactions / threading** | Discord reactions and threads are complex subsystems. Reactions require emoji picker UI, reaction state management, and per-user tracking. Threads require nested message views. | Display reactions as read-only counts if present on messages. Do not allow adding reactions from the panel. Ignore threads initially. |
| A6 | **File upload from panel** | Requires multipart form handling, file size limits, CDN URL generation, content type validation. Large attack surface for abuse. | Show download links for attachments posted from Discord. Do not allow uploads from the web panel. |
| A7 | **Message editing/deleting from panel** | Webhook messages cannot be edited/deleted by the sender through normal Discord mechanics (only by the webhook itself). Implementing this requires storing webhook message IDs and building edit/delete endpoints. Edge cases with rate limits and permission. | Messages sent from the panel are fire-and-forget. Users can clarify with follow-up messages. |
| A8 | **Typing indicators** | Requires real-time presence tracking, Gateway TYPING_START events, UI for "X is typing..." animation. Marginal value for a side panel. | Omit entirely. Messages appear fast enough via live stream. |
| A9 | **Member list / online status** | WidgetBot shows member lists. For a document discussion panel, this is noise. The focus is the conversation, not who is online. | Do not show member lists. If needed later, show a simple count ("3 online"). |
| A10 | **Custom CSS theming engine** | Both WidgetBot and Titan offer extensive CSS customization. Building a theming engine is a distraction. | Match the editor's existing design system. One theme that fits the app. Support dark/light mode via the editor's existing mode. |
| A11 | **Notification sounds** | Browser notification sounds are annoying and require audio permission UX. Users will mute immediately. | Visual-only notifications (unread count badge). No audio. |

---

## Feature Dependencies

```
T1 (Live message stream)
 ├── T8 (Scroll behavior) -- needs message flow to scroll
 ├── D4 (Unread count) -- counts messages from live stream
 ├── D8 (Edit/delete reflection) -- additional Gateway events
 └── D10 (Connection status) -- monitors stream health

T2 (Message history)
 └── T8 (Scroll behavior) -- initial scroll position

T3 (Post via webhook)
 └── T4 (Display name) -- username field for webhook

T5 (Markdown rendering)
 ├── D5 (Code blocks) -- extends markdown parser
 ├── D6 (Mention rendering) -- extends markdown parser
 ├── D7 (Emoji rendering) -- extends markdown parser
 └── D11 (Embed rendering) -- separate but related rendering

T10 (Panel toggle)
 ├── D4 (Unread count) -- shown on collapsed panel
 └── D9 (Keyboard shortcut) -- triggers toggle

D1 (Document-aware channel mapping)
 └── T1, T2, T3 all depend on knowing which channel to connect to
```

**Critical path:** D1 (channel mapping) -> T1 (live stream) + T2 (history) -> T5 (rendering) -> T3 (posting)

---

## MVP Recommendation

For MVP, prioritize these features in order:

### Must Have (launch blockers)

1. **D1 -- Document-aware channel mapping** (the entire value proposition)
2. **T2 -- Message history on load** (users see existing conversation)
3. **T1 -- Live message stream** (new messages appear in real time)
4. **T7 -- Author identification** (who said what)
5. **T6 -- Timestamps** (when was it said)
6. **T5 -- Basic markdown rendering** (bold, italic, code, quotes -- no custom emoji yet)
7. **T3 -- Post via webhook** (participate in conversation)
8. **T4 -- Display name** (identity for posting)
9. **T8 -- Scroll behavior** (usable chat UX)
10. **T9 -- Loading/error states** (reliability perception)
11. **T10 -- Panel toggle** (show/hide the panel)

### Should Have (soon after launch)

- **D3 -- "(unverified)" tag** (trust clarity)
- **D10 -- Connection status** (confidence indicator)
- **D4 -- Unread count** (engagement driver)
- **D7 -- Emoji rendering** (Unicode first, custom later)
- **D8 -- Edit/delete reflection** (accuracy)

### Defer to Post-MVP

- **D5 -- Syntax-highlighted code blocks**: Complexity of highlight.js integration. Plain code blocks (monospace, no highlighting) suffice initially.
- **D6 -- User mention resolution**: Showing raw `<@id>` is acceptable short-term; resolving to names requires API calls and caching.
- **D9 -- Keyboard shortcut**: Nice but not blocking.
- **D11 -- Rich embed rendering**: High complexity. Show embeds as simple linked text or omit.
- **D12 -- Attachment display**: Show as download links initially. Inline image rendering is post-MVP.

---

## Complexity Budget

| Complexity | Count | Features |
|------------|-------|----------|
| Low | 10 | T3, T4, T6, T7, T8, T9, T10, D2, D3, D9, D10 |
| Medium | 8 | T1, T2, T5, D1, D4, D5, D6, D7, D8 |
| High | 2 | D11, D12 |

The MVP is predominantly Low and Medium complexity items, which is appropriate for a first milestone.

---

## Key Technical Constraints (from research)

1. **Webhook rate limit:** ~5 requests per 2 seconds per webhook. Sufficient for normal chat but could be hit during rapid-fire conversations. Consider client-side debouncing or queueing.
2. **MessageContent privileged intent:** Required for the bot to receive message content via Gateway. Must be enabled in Discord Developer Portal. For bots in <100 servers, no approval needed.
3. **Discord markdown is not standard markdown:** Different parsing rules for spoilers (`||text||`), strikethrough (`~~text~~`), mentions (`<@id>`), channels (`<#id>`), emoji (`<:name:id>`), timestamps (`<t:unix:format>`). Use a Discord-specific parser like `discord-markdown-parser`.
4. **Message content limit:** 2000 characters for messages sent via webhook. Enforce client-side.
5. **Webhook messages are "bot" messages:** They show the BOT tag in Discord unless the webhook is specifically configured. The `username` and `avatar_url` fields override per-message.

---

## Sources

### Verified (MEDIUM-HIGH confidence)
- [Discord Developer Portal -- Webhooks](https://discord.com/developers/docs/resources/webhook) -- Official webhook API documentation
- [Discord Developer Portal -- Rate Limits](https://discord.com/developers/docs/topics/rate-limits) -- Official rate limit documentation
- [Discord Developer Portal -- Message Resource](https://discord.com/developers/docs/resources/message) -- Message object structure
- [Discord Blog -- How Discord Renders Rich Messages](https://discord.com/blog/how-discord-renders-rich-messages-on-the-android-app) -- Rendering architecture
- [Discord -- Message Content Privileged Intent FAQ](https://support-dev.discord.com/hc/en-us/articles/4404772028055-Message-Content-Privileged-Intent-FAQ) -- Intent requirements
- [WidgetBot Documentation](https://docs.widgetbot.io/embed/) -- Embed formats, API, features
- [WidgetBot Crate API](https://docs.widgetbot.io/embed/crate/api) -- Programmatic control
- [Titan Embeds GitHub](https://github.com/TitanEmbeds/Titan) -- Architecture, features, self-hosting

### Community / Synthesized (LOW-MEDIUM confidence)
- [Discord Webhooks Guide](https://birdie0.github.io/discord-webhooks-guide/discord_webhook.html) -- Community webhook documentation
- [Discord Webhooks Rate Limits](https://birdie0.github.io/discord-webhooks-guide/other/rate_limits.html) -- Rate limit specifics (community-verified)
- [discord-markdown-parser npm](https://www.npmjs.com/package/discord-markdown-parser) -- Parsing library based on simple-markdown
- [RPG Directory -- WidgetBot vs Titan comparison](https://rpg-directory.com/index.php?showtopic=95981) -- User experience comparison
- [Chat Widget Accessibility Best Practices](https://www.craigabbott.co.uk/blog/web-chat-accessibility-considerations/) -- A11y considerations
