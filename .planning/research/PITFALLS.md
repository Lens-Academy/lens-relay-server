# Domain Pitfalls: Discord Chat Integration in Web Editor

**Domain:** Embedded Discord chat widget with bot gateway, webhook posting, and browser bridge
**Researched:** 2026-02-08
**Overall confidence:** MEDIUM-HIGH (most pitfalls verified via official Discord docs, GitHub issues, and community post-mortems)

---

## Critical Pitfalls

Mistakes that cause rewrites, production outages, or project-blocking issues.

---

### Pitfall 1: Gateway Identify Exhaustion (1000/day Hard Limit)

**What goes wrong:** The bot enters a crash-restart loop (bug, unhandled exception, deployment churn) and burns through identify calls. Discord limits bots to 1000 identify (new session) calls per 24 hours globally. Upon hitting this limit, Discord terminates ALL active sessions, resets the bot token, and sends the account owner an email. The bot is completely dead until you generate a new token and redeploy.

**Why it happens:** Developers treat gateway disconnects as "just reconnect" without distinguishing between resume (free) and identify (limited). A bug that crashes the process every few seconds can burn 1000 identifies in under an hour.

**Consequences:** Total bot outage for up to 24 hours. Token reset means manual intervention -- you cannot recover programmatically. Every browser client loses live message streaming simultaneously.

**Warning signs:**
- Bot process restarting frequently in logs
- `session_id` and `resume_gateway_url` not being cached between reconnects
- Using `process.exit()` or crashing on unhandled promise rejections instead of graceful error handling

**Prevention:**
1. Always attempt **Resume** (opcode 6) before Identify. Resume does not count toward the limit.
2. Cache `session_id` and `resume_gateway_url` from the Ready event. discord.js does this automatically, but only if you don't kill the process.
3. Implement exponential backoff on reconnection: 1s, 2s, 4s, 8s... up to 60s.
4. Use a process supervisor (systemd, Docker restart policy) with a **max restart rate** -- e.g., `RestartSec=5` + `StartLimitBurst=10` + `StartLimitIntervalSec=60` to prevent runaway restarts.
5. Monitor identify calls. Log every IDENTIFY event and alert if count exceeds 100/hour.

**Phase:** Infrastructure/deployment setup. Must be addressed before the bot goes to production, even for a single-server bot.

**Confidence:** HIGH -- documented in [Discord Gateway docs](https://docs.discord.food/topics/gateway) and multiple library issue trackers.

---

### Pitfall 2: MESSAGE_CONTENT Intent Not Enabled in Both Places

**What goes wrong:** The bot connects to the gateway, receives MESSAGE_CREATE events, but message `content` is an empty string. The chat panel shows blank messages. Developers spend hours debugging their code when the problem is a missing checkbox in the Discord Developer Portal or a missing intent flag in the bot client configuration.

**Why it happens:** Discord requires privileged intents to be enabled in TWO places: (1) the Developer Portal bot settings page, AND (2) in your code when constructing the gateway client. Enabling one without the other silently degrades -- you get events but with empty content fields.

**Consequences:** If only the portal toggle is missed, the gateway closes with **close code 4014 (Disallowed Intents)** and the bot cannot connect at all. If only the code-side intent is missed, the bot connects but receives empty message content. Both are confusing failures with non-obvious root causes.

**Warning signs:**
- `message.content` is empty string but `message.author` and `message.timestamp` are populated
- Gateway close code 4014 in logs
- Messages from the bot itself or DMs work fine (they are exempt from the intent requirement)

**Prevention:**
1. Create a setup checklist: Developer Portal > Bot > Privileged Gateway Intents > toggle MESSAGE CONTENT on.
2. In discord.js client constructor, explicitly list `GatewayIntentBits.MessageContent` in the intents array.
3. Add a startup health check: send a test message to a test channel and read it back via the gateway. If content is empty, log an explicit error: "MESSAGE_CONTENT intent not configured."
4. Document the setup steps in the sidecar's README with screenshots.

**Phase:** Bot setup (Phase 1). This is a day-one configuration requirement.

**Confidence:** HIGH -- [Discord MESSAGE_CONTENT Intent FAQ](https://support-dev.discord.com/hc/en-us/articles/4404772028055-Message-Content-Privileged-Intent-FAQ), [Gateway Close Codes](https://discord-api-types.dev/api/discord-api-types-v10/enum/GatewayCloseCodes).

---

### Pitfall 3: Webhook URL Leakage Enables Channel Spam

**What goes wrong:** The Discord webhook URL is exposed to the browser client (in a network request, in client-side JavaScript, or hardcoded in frontend code). Anyone who obtains the URL can POST arbitrary messages to the Discord channel -- including @everyone pings, spam, scam links, and obscene content. Webhook messages bypass Discord AutoMod, custom bot filters, and all moderation tools.

**Why it happens:** The naive architecture sends webhook URLs to the browser so the client can POST directly. Or the sidecar proxies the request but includes the webhook URL in error responses, CORS headers, or debug logs accessible to the client.

**Consequences:** Unmoderated spam in the Discord channel. Reputation damage to the community. The only fix is deleting and recreating the webhook (new URL), which requires server admin intervention.

**Warning signs:**
- Webhook URL appears in browser DevTools Network tab
- Frontend code contains `discord.com/api/webhooks/` strings
- No server-side proxy between browser and Discord webhook

**Prevention:**
1. **Never expose webhook URLs to the browser.** The sidecar must proxy all message-posting requests. The browser POSTs to the sidecar's `/channels/:id/messages` endpoint; the sidecar forwards to Discord via the webhook URL stored server-side.
2. Store webhook URLs in environment variables or a config file on the sidecar, not in any frontend-accessible location.
3. Implement rate limiting on the sidecar's POST endpoint (e.g., 1 message per 2 seconds per client IP) to prevent abuse even if the sidecar endpoint is discovered.
4. Always set `allowed_mentions: { parse: [] }` on every webhook execution to suppress @everyone, @here, role, and user mention pings regardless of message content.

**Phase:** Architecture design (Phase 1). The proxy pattern must be established from the beginning.

**Confidence:** HIGH -- [GitGuardian Discord Webhook remediation guide](https://www.gitguardian.com/remediation/discord-webhook-url), [Discord webhook security docs](https://hookdeck.com/webhooks/platforms/guide-to-discord-webhooks-features-and-best-practices).

---

### Pitfall 4: Bot Token Exposed in Client Bundle or Git History

**What goes wrong:** The Discord bot token ends up in the frontend JavaScript bundle, in a git commit, in a `.env` file that gets committed, or in a Docker image layer. A leaked bot token gives attackers complete control of the bot account -- they can read all messages, delete channels, ban users, and abuse the bot's permissions.

**Why it happens:** Copy-paste mistakes during development. Using `VITE_` prefixed environment variables (which Vite injects into client code). Committing `.env` files. Using the token in a test that gets pushed.

**Consequences:** Full bot account compromise. Discord automatically detects some leaked tokens (especially on public GitHub repos) and resets them, but this causes a surprise outage. If Discord does not detect the leak, attackers have persistent access.

**Warning signs:**
- Any environment variable starting with `VITE_` that contains a token
- `.env` file not in `.gitignore`
- Bot token appearing in browser DevTools or frontend source maps
- Token in Docker build args visible in image history

**Prevention:**
1. Bot token lives ONLY in the sidecar's environment, never in any frontend code or config.
2. Add `.env` and `*.env.*` to `.gitignore` before writing any env file.
3. Use `DISCORD_BOT_TOKEN` (no `VITE_` prefix) so Vite does not expose it.
4. Run `grep -r "VITE_.*TOKEN\|VITE_.*SECRET" lens-editor/src/` as a pre-commit check.
5. Use Docker multi-stage builds so tokens in build-stage env vars are not in the final image.

**Phase:** Project initialization. Establish token management practices before writing any bot code.

**Confidence:** HIGH -- [GitGuardian bot token remediation](https://www.gitguardian.com/remediation/discord-bot-token), [Discord bot security guide 2025](https://friendify.net/blog/discord-bot-security-best-practices-2025.html).

---

## Moderate Pitfalls

Mistakes that cause delays, poor UX, or technical debt requiring refactoring.

---

### Pitfall 5: SSE 6-Connection Browser Limit (HTTP/1.1)

**What goes wrong:** Users open the editor in multiple browser tabs. Each tab opens an SSE connection to the sidecar. Under HTTP/1.1, browsers enforce a hard limit of 6 concurrent connections per domain. After 6 tabs, new SSE connections silently fail or block other HTTP requests (including REST calls for message history and posting). The 7th tab's chat panel appears dead with no error message.

**Why it happens:** The SSE spec over HTTP/1.1 uses one long-lived TCP connection per EventSource. Browser vendors (Chrome, Firefox) enforce a 6-connection-per-domain limit and have marked related bugs as "Won't fix."

**Warning signs:**
- Chat panel works in first few tabs but fails silently in subsequent tabs
- REST requests to the sidecar hang or timeout when many tabs are open
- No errors in browser console (the connection just never opens)

**Prevention:**
1. Serve the sidecar over HTTP/2 (or HTTP/3). HTTP/2 multiplexes streams over a single TCP connection with a default limit of 100 concurrent streams. This eliminates the problem entirely.
2. If HTTP/2 is not feasible initially (e.g., dev server without TLS), implement a fallback: detect HTTP version on connection, and if HTTP/1.1, use long-polling (close and reopen the connection after each event batch). EventSource handles this via the `retry` header.
3. Use a `SharedWorker` or `BroadcastChannel` to share a single SSE connection across tabs on the same domain (advanced; defer to post-MVP).
4. Document the limitation so users are not confused by the behavior.

**Phase:** Sidecar infrastructure (Phase 2, when deploying behind a reverse proxy). Acceptable to ignore during local development (where tab count is small) but must be solved before production.

**Confidence:** HIGH -- [MDN EventSource docs](https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events/Using_server-sent_events), [Firefox bug 906896](https://bugzilla.mozilla.org/show_bug.cgi?id=906896).

---

### Pitfall 6: Webhook Rate Limits Are Per-Webhook AND Per-Server

**What goes wrong:** The sidecar sends messages through webhooks and hits rate limits unexpectedly. Developers assume rate limits are per-channel, but Discord documentation says the rate limit bucket is per-webhook (5 requests per 2 seconds). Worse, there is evidence that all webhooks in a Discord community/server may share a single rate limit bucket, meaning a busy channel could throttle posting to other channels.

**Why it happens:** Discord's rate limit documentation describes per-resource limits keyed by `channel_id` for most endpoints, but webhook execution uses `webhook_id` or `webhook_id + webhook_token` as the rate limit key. An unresolved GitHub issue (#6753) reports that webhooks to different channels in the same server share the same `X-RateLimit-Bucket`.

**Warning signs:**
- 429 (rate limited) responses when posting to a channel that should have headroom
- `X-RateLimit-Bucket` header is the same for webhooks in different channels
- Users reporting "message not sent" errors during active discussions

**Prevention:**
1. Implement a server-side message queue in the sidecar. Do not fire-and-forget webhook requests. Queue messages per webhook and respect the 5/2s limit with a token bucket or sliding window.
2. Read and obey `X-RateLimit-Remaining` and `X-RateLimit-Reset` headers from every webhook response.
3. On 429 responses, honor the `retry_after` value. Do not retry immediately.
4. Set realistic user expectations: if multiple people are posting rapidly, some messages may be delayed by a few seconds.
5. Consider using a single webhook per server and varying the `username`/`avatar_url` per message, rather than creating per-channel webhooks (if the shared-bucket issue is confirmed).

**Phase:** Message posting implementation (Phase 2-3). Basic rate limit handling is needed from the start; the queue becomes important as usage grows.

**Confidence:** MEDIUM -- The 5/2s per-webhook limit is well-documented at [Discord Webhooks Guide](https://birdie0.github.io/discord-webhooks-guide/other/rate_limits.html). The shared-bucket-per-server issue is reported but [unconfirmed by Discord staff](https://github.com/discord/discord-api-docs/issues/6753).

---

### Pitfall 7: Webhook Username Validation Failures

**What goes wrong:** Users enter a display name that Discord rejects for the webhook `username` field. The message POST fails with a 400 Bad Request and no clear error message shown to the user. Names containing "clyde" (case-insensitive substring), "discord", "everyone", "here", or "system message" are all blocked. Empty strings and names under 1 or over 80 characters also fail.

**Why it happens:** Developers validate message content but forget that the `username` field has its own undocumented restrictions. The restrictions are not fully documented in the official API docs -- they are spread across user docs, webhook docs, and GitHub issues.

**Warning signs:**
- Sporadic "Bad Request" errors from webhook execution with no pattern
- Users with certain names (e.g., "Clydesdale", "Discord Fan") cannot post
- No client-side validation on the display name input

**Prevention:**
1. Validate the display name client-side before sending:
   - Length: 1-80 characters after trimming
   - Blacklist substrings (case-insensitive): "clyde", "discord"
   - Blacklist exact names (case-insensitive): "everyone", "here", "system message"
   - Strip excessive whitespace
2. Append " (unverified)" server-side (in the sidecar), not client-side, so users cannot omit it.
3. Show specific error messages: "Display names cannot contain 'discord' or 'clyde'" rather than a generic failure.

**Phase:** Display name and posting implementation (Phase 2).

**Confidence:** MEDIUM-HIGH -- [Webhook naming restrictions issue #4293](https://github.com/discord/discord-api-docs/issues/4293), [Discord Webhooks Guide - username](https://birdie0.github.io/discord-webhooks-guide/structure/username.html).

---

### Pitfall 8: Zombie Gateway Connections (Missed Heartbeat ACK)

**What goes wrong:** The bot's gateway connection becomes "zombied" -- the TCP socket is still open but Discord is no longer sending or receiving data. The bot appears connected but stops receiving MESSAGE_CREATE events. The chat panel stops updating without any error visible to the user. This can persist for minutes or hours until the heartbeat timeout triggers.

**Why it happens:** Network hiccups, cloud provider routing changes, or server load spikes cause packets to be lost. The TCP connection does not close cleanly (no FIN/RST), so the socket remains open. Discord sends heartbeat ACKs, but they never arrive.

**Warning signs:**
- Chat panel stops showing new messages but shows "Connected" status
- Last heartbeat ACK timestamp is stale (older than the heartbeat interval, ~41.25 seconds)
- Users report "messages stopped appearing" but refreshing the page fixes it

**Prevention:**
1. discord.js handles heartbeat ACK monitoring internally, but you must handle the `shardDisconnect` and `shardReconnecting` events to update your SSE clients. When the shard reconnects, replay missed events.
2. Forward connection status changes to browser clients via SSE control events (e.g., `event: status\ndata: {"connected": false}\n\n`). This drives the connection status indicator (feature D10).
3. Implement a secondary liveness check: if no MESSAGE_CREATE events arrive for an unusually long period in a normally-active channel, log a warning. (This is a heuristic, not a guarantee -- the channel may genuinely be quiet.)
4. Ensure the sidecar process does not swallow discord.js error events. Attach handlers to `client.on('error')`, `client.on('shardError')`, and `client.on('shardDisconnect')`.

**Phase:** Bot reliability (Phase 2-3). Initial implementation can rely on discord.js defaults; explicit monitoring should be added before production.

**Confidence:** HIGH -- [Discord Gateway docs on heartbeat](https://docs.discord.food/topics/gateway), [discordrb zombie connection issue #447](https://github.com/discordrb/discordrb/issues/447).

---

### Pitfall 9: Allowed Mentions Not Set on Webhook (Mention Abuse)

**What goes wrong:** A malicious or careless user types `@everyone hello` in the chat panel compose box. The sidecar forwards this to Discord via the webhook. Without `allowed_mentions` set, Discord processes the @everyone mention, pinging every member of the server. The same applies to @here, role mentions (`<@&role_id>`), and user mentions (`<@user_id>`).

**Why it happens:** Webhooks do not have Discord AutoMod or any server-configured moderation applied to them. The `allowed_mentions` field defaults to allowing all mention types unless explicitly restricted.

**Consequences:** Server-wide notification spam. Angry community members. Loss of trust in the chat panel integration.

**Warning signs:**
- No `allowed_mentions` field in the webhook payload
- Users discover they can ping @everyone from the web panel
- Webhook messages triggering notifications for hundreds of users

**Prevention:**
1. On EVERY webhook execution, include `allowed_mentions: { parse: [] }`. This suppresses ALL mention pings (the mention text still renders but does not notify anyone).
2. Apply this server-side in the sidecar's webhook proxy, not client-side. The client should never control mention behavior.
3. Optionally, strip `@everyone` and `@here` from message content server-side as a defense-in-depth measure.
4. Test this explicitly: send a message containing `@everyone` through the panel and verify it does not ping.

**Phase:** Message posting implementation (Phase 2). Must be in the first webhook POST implementation, not added later.

**Confidence:** HIGH -- [Discord Webhooks Guide - allowed_mentions](https://birdie0.github.io/discord-webhooks-guide/structure/allowed_mentions.html), [DiceCloud issue #255](https://github.com/ThaumRystra/DiceCloud/issues/255).

---

### Pitfall 10: Message History Returns Empty Content Without Intent

**What goes wrong:** The REST API endpoint `GET /channels/{id}/messages` returns messages with empty `content` fields. Developers expect the REST API to always return full message content, but the MESSAGE_CONTENT privileged intent affects REST responses too -- if the bot does not have the intent approved, content is stripped from REST responses for messages not authored by the bot.

**Why it happens:** The MESSAGE_CONTENT intent restriction applies to both gateway events AND REST API responses. This is documented but counterintuitive -- developers assume intent restrictions only affect the WebSocket gateway.

**Warning signs:**
- Message history loads with timestamps and authors but blank message text
- Messages sent by the bot or webhook show content, but other users' messages are blank
- DM messages have content but guild messages do not

**Prevention:**
1. Ensure MESSAGE_CONTENT intent is enabled (same fix as Pitfall 2).
2. Test message history fetch explicitly in the startup health check: fetch 5 messages and verify at least one has non-empty content.
3. For bots in a single server (under 100 guilds), this is a simple toggle. The risk increases if the bot grows to 100+ servers and needs Discord review approval.

**Phase:** Bot setup (Phase 1). Same configuration as Pitfall 2.

**Confidence:** HIGH -- [Discord MESSAGE_CONTENT FAQ](https://support-dev.discord.com/hc/en-us/articles/4404772028055-Message-Content-Privileged-Intent-FAQ): "Apps without the message content intent configured will receive empty values in fields that expose message content."

---

## Minor Pitfalls

Mistakes that cause annoyance, confusing bugs, or minor UX issues. Fixable without major refactoring.

---

### Pitfall 11: Invalid HTTP Requests Trigger IP-Level Ban

**What goes wrong:** The sidecar sends requests with malformed headers, expired tokens, or incorrect endpoint paths. Discord returns 401/403 errors. If the sidecar does not handle these properly and retries aggressively, it can accumulate 10,000 invalid requests in 10 minutes, triggering a 24-hour IP-level ban from the Discord API.

**Prevention:**
1. Never retry 401 (unauthorized) or 403 (forbidden) errors. These are permanent failures that require configuration changes, not retries.
2. discord.js handles this internally for most cases, but custom REST calls (e.g., direct webhook execution via fetch) must implement this logic.
3. Log and alert on auth errors. A spike in 401s means the bot token has been reset or has expired.

**Phase:** Sidecar implementation (Phase 2).

**Confidence:** HIGH -- [Discord rate limits documentation](https://discord.com/developers/docs/topics/rate-limits).

---

### Pitfall 12: Webhook Username Override Applied Globally (Race Condition)

**What goes wrong:** Two users post messages through the webhook simultaneously. Due to a reported Discord API behavior, the `username` override from the second message can retroactively display on the first message's webhook post, making both messages appear to be from the second user.

**Why it happens:** Discord issue #5953 reports that webhook username overrides changed from per-message to global behavior in some cases. The exact conditions are unclear, but rapid sequential posts through the same webhook are the trigger.

**Prevention:**
1. Implement server-side queuing: serialize webhook requests so they are sent one at a time with a brief delay (~100ms) between them.
2. If the issue persists, consider creating multiple webhooks per channel (one per active user) as a workaround, though this increases management complexity.
3. Monitor for user reports of "wrong name on my message" and correlate with webhook request timing.

**Phase:** Message posting hardening (Phase 3).

**Confidence:** LOW-MEDIUM -- [GitHub issue #5953](https://github.com/discord/discord-api-docs/issues/5953). The bug report exists but reproduction conditions are unclear.

---

### Pitfall 13: Forum Thread Channels Require Parent ID Awareness

**What goes wrong:** The chat panel tries to fetch messages from a forum channel ID instead of the specific thread (post) ID. Discord forums are not regular channels -- each "post" in a forum is a thread with its own channel ID. Using the forum channel ID for message history returns an error or empty results.

**Why it happens:** The `discussion` frontmatter field may contain a URL like `https://discord.com/channels/SERVER_ID/FORUM_CHANNEL_ID/THREAD_ID`. Developers parse this and use the wrong ID. The URL structure differs between regular channels (2 path segments after `/channels/`) and forum threads (3 path segments).

**Prevention:**
1. Parse Discord URLs carefully. Regular channel: `/channels/{server_id}/{channel_id}`. Forum thread: `/channels/{server_id}/{forum_id}/{thread_id}`. Use the last segment as the target channel for API calls.
2. Test with both regular text channels and forum thread channels during development.
3. Validate the parsed channel ID on first connection: if `GET /channels/{id}` returns type 15 (GUILD_FORUM), it is a forum parent, not a thread. Log an error and prompt the user to use the specific thread URL.

**Phase:** Channel mapping / frontmatter parsing (Phase 1).

**Confidence:** MEDIUM -- Discord API documentation on [channel types](https://discord.com/developers/docs/resources/channel#channel-object-channel-types).

---

### Pitfall 14: SSE Reconnection Floods After Sidecar Restart

**What goes wrong:** The sidecar restarts (deployment, crash, etc.). All connected browser clients simultaneously reconnect via EventSource's automatic retry. If 50 tabs are open, 50 SSE connections plus 50 message-history REST requests hit the sidecar at once on startup. The sidecar, which also needs to establish its own gateway connection to Discord, is overwhelmed during the most fragile moment of its lifecycle.

**Prevention:**
1. Set the SSE `retry` header to a value with jitter: base 3000ms + random 0-2000ms. This staggers reconnections.
2. Cache message history in memory so that history requests after restart do not all hit the Discord REST API simultaneously.
3. Delay SSE event streaming until the gateway connection is established. Send a `status: connecting` event to clients while the bot is initializing.
4. If the sidecar starts receiving REST requests before the gateway is ready, serve cached data and indicate "may be stale" in the response.

**Phase:** Sidecar reliability (Phase 3).

**Confidence:** MEDIUM -- General SSE architecture pattern. EventSource auto-reconnect behavior is documented at [MDN EventSource](https://developer.mozilla.org/en-US/docs/Web/API/EventSource).

---

### Pitfall 15: Discord Developer Policy on Data Storage

**What goes wrong:** The sidecar caches messages in memory or persists them to disk/database for performance. Discord's Developer Policy requires that bots only access information needed for their core functionality and that developers have a privacy policy explaining data usage. Storing message history longer than necessary, or making it accessible outside Discord's intended audience, could violate the policy.

**Prevention:**
1. Use short-lived in-memory caches only (TTL 30-60 seconds). Do not persist Discord messages to disk or database.
2. Do not expose cached messages to unauthenticated users. If the sidecar serves message history, ensure it requires the same access that reading the Discord channel would.
3. Review the [Discord Developer Policy](https://support-dev.discord.com/hc/en-us/articles/8563934450327-Discord-Developer-Policy) during architecture design.
4. Include a basic privacy policy page or statement for the bot application (required by Discord Developer Terms).

**Phase:** Architecture design (Phase 1). Caching strategy should be designed with this constraint from the start.

**Confidence:** MEDIUM -- The Developer Policy is documented but enforcement is on a case-by-case review basis. For a single-server bot, risk is low, but the policy should be respected.

---

### Pitfall 16: Classic Bot Token Format Invalidation (Nov 2025)

**What goes wrong:** Using an old-format bot token that was generated before the November 2025 token format change. Classic tokens were fully invalidated in November 2025. If you copy a token from old documentation, tutorials, or environment variable backups, it will not work.

**Prevention:**
1. Generate a fresh bot token from the Discord Developer Portal.
2. Do not copy tokens from any documentation or tutorial written before November 2025.
3. The new token format is visually longer. If the token looks short, it is probably the old format.

**Phase:** Bot setup (Phase 1).

**Confidence:** HIGH -- [Discord Development 2025 year-in-review](https://discord-media.com/en/news/development-2025-the-complete-year-in-review-api-migration-guide.html).

---

## Phase-Specific Warnings

| Phase | Likely Pitfall | Severity | Mitigation |
|-------|---------------|----------|------------|
| **Phase 1: Bot setup & channel mapping** | MESSAGE_CONTENT intent not enabled in both portal and code (P2) | Critical | Setup checklist + startup health check |
| **Phase 1: Bot setup** | Old token format (P16) | Minor | Generate fresh token from Developer Portal |
| **Phase 1: Architecture** | Webhook URL leakage to browser (P3) | Critical | Proxy-only architecture from day one |
| **Phase 1: Architecture** | Bot token in client bundle (P4) | Critical | No VITE_ prefix for secrets, .gitignore |
| **Phase 1: Channel mapping** | Forum thread ID vs forum channel ID confusion (P13) | Moderate | URL parsing logic handles 2 and 3 segment paths |
| **Phase 2: Message posting** | Mention abuse via webhook (P9) | Moderate | `allowed_mentions: { parse: [] }` on every POST |
| **Phase 2: Message posting** | Webhook username validation failures (P7) | Moderate | Client-side name validation with blacklist |
| **Phase 2: Message posting** | Webhook rate limits (P6) | Moderate | Server-side queue with token bucket |
| **Phase 2: Sidecar** | Invalid request IP ban (P11) | Minor | No retry on 401/403; discord.js handles most cases |
| **Phase 2-3: Bot reliability** | Zombie gateway connections (P8) | Moderate | Forward status events to SSE clients |
| **Phase 3: Reliability** | Gateway identify exhaustion (P1) | Critical | Process supervisor rate limiting, resume-first |
| **Phase 3: Reliability** | SSE reconnection flood (P14) | Minor | Jittered retry header, startup delay |
| **Phase 3: Scale** | SSE 6-connection limit (P5) | Moderate | HTTP/2 or SharedWorker |
| **Phase 3: Hardening** | Webhook username race condition (P12) | Minor | Serialize webhook requests |
| **Ongoing** | Data storage policy compliance (P15) | Moderate | In-memory cache only, short TTL |

---

## Pitfall Checklist for Code Review

Use this checklist when reviewing Discord integration PRs:

- [ ] Bot token is never in frontend code or any VITE_ env var
- [ ] Webhook URLs are never sent to the browser
- [ ] `allowed_mentions: { parse: [] }` is set on every webhook execution
- [ ] MESSAGE_CONTENT intent is in the gateway intents list
- [ ] 401/403 responses are not retried
- [ ] Webhook username is validated against Discord's blacklist
- [ ] Forum thread URLs are parsed correctly (3 segment paths)
- [ ] SSE retry header includes jitter
- [ ] Gateway connection status is forwarded to browser clients
- [ ] No Discord messages are persisted to disk

---

## Sources

### Verified (HIGH confidence)
- [Discord Gateway Documentation](https://docs.discord.food/topics/gateway) -- Identify limits, heartbeat, resume, close codes
- [Discord MESSAGE_CONTENT Privileged Intent FAQ](https://support-dev.discord.com/hc/en-us/articles/4404772028055-Message-Content-Privileged-Intent-FAQ) -- Intent requirements, REST API impact
- [Discord Rate Limits](https://discord.com/developers/docs/topics/rate-limits) -- Global and per-resource limits, invalid request bans
- [MDN EventSource](https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events/Using_server-sent_events) -- SSE connection limits, HTTP/2 workaround
- [GitGuardian Discord Webhook URL Remediation](https://www.gitguardian.com/remediation/discord-webhook-url) -- Webhook URL security
- [GitGuardian Discord Bot Token Remediation](https://www.gitguardian.com/remediation/discord-bot-token) -- Token security

### Community-verified (MEDIUM confidence)
- [Discord Webhooks Guide - allowed_mentions](https://birdie0.github.io/discord-webhooks-guide/structure/allowed_mentions.html) -- Mention suppression
- [Discord Webhooks Guide - username](https://birdie0.github.io/discord-webhooks-guide/structure/username.html) -- Username restrictions
- [Discord Webhooks Guide - rate limits](https://birdie0.github.io/discord-webhooks-guide/other/rate_limits.html) -- 5 req / 2 sec per webhook
- [Webhook naming restrictions issue #4293](https://github.com/discord/discord-api-docs/issues/4293) -- Blocked substrings
- [Shared webhook rate limit bucket issue #6753](https://github.com/discord/discord-api-docs/issues/6753) -- Per-server bucket sharing
- [Webhook username override issue #5953](https://github.com/discord/discord-api-docs/issues/5953) -- Global username race condition
- [Discord Development 2025 year-in-review](https://discord-media.com/en/news/development-2025-the-complete-year-in-review-api-migration-guide.html) -- Token invalidation, API changes
- [Firefox bug 906896](https://bugzilla.mozilla.org/show_bug.cgi?id=906896) -- SSE connection limit "Won't fix"
- [Discord bot security best practices 2025](https://friendify.net/blog/discord-bot-security-best-practices-2025.html) -- Token management
