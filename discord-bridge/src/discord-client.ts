import type { DiscordMessage, DiscordChannel } from './types.js';

const DISCORD_API_BASE = 'https://discord.com/api/v10';

// Simple in-memory cache with TTL
interface CacheEntry<T> {
  data: T;
  expiresAt: number;
}

const messageCache = new Map<string, CacheEntry<DiscordMessage[]>>();
const channelCache = new Map<string, CacheEntry<DiscordChannel>>();

const MESSAGE_CACHE_TTL_MS = 60 * 1000; // 60 seconds
const CHANNEL_CACHE_TTL_MS = 5 * 60 * 1000; // 5 minutes

function getToken(): string {
  const token = process.env.DISCORD_BOT_TOKEN;
  if (!token) {
    throw new Error(
      'DISCORD_BOT_TOKEN environment variable is not set. ' +
        'Get it from Discord Developer Portal -> Your App -> Bot -> Token'
    );
  }
  return token;
}

function authHeaders(): Record<string, string> {
  return {
    Authorization: `Bot ${getToken()}`,
    'Content-Type': 'application/json',
  };
}

/**
 * Error thrown when Discord returns a 429 rate-limit response.
 */
export class RateLimitError extends Error {
  retryAfter: number;

  constructor(retryAfter: number) {
    super(`Discord rate limited — retry after ${retryAfter}s`);
    this.name = 'RateLimitError';
    this.retryAfter = retryAfter;
  }
}

/**
 * Error thrown when Discord returns a non-OK, non-429 response.
 */
export class DiscordApiError extends Error {
  status: number;
  body: string;

  constructor(status: number, body: string) {
    super(`Discord API error ${status}: ${body}`);
    this.name = 'DiscordApiError';
    this.status = status;
    this.body = body;
  }
}

async function handleResponse<T>(res: Response): Promise<T> {
  if (res.status === 429) {
    const retryAfter = parseFloat(res.headers.get('retry-after') || '5');
    console.warn(`[discord-client] Rate limited — retry after ${retryAfter}s`);
    throw new RateLimitError(retryAfter);
  }

  if (!res.ok) {
    const body = await res.text();
    throw new DiscordApiError(res.status, body);
  }

  return (await res.json()) as T;
}

/**
 * Fetch messages from a Discord channel.
 * Results are cached for 60 seconds per channelId+limit combination.
 */
export async function fetchChannelMessages(
  channelId: string,
  limit: number = 50
): Promise<DiscordMessage[]> {
  const cacheKey = `${channelId}:${limit}`;
  const cached = messageCache.get(cacheKey);
  if (cached && Date.now() < cached.expiresAt) {
    return cached.data;
  }

  const url = `${DISCORD_API_BASE}/channels/${channelId}/messages?limit=${limit}`;
  const res = await fetch(url, { headers: authHeaders() });
  const data = await handleResponse<DiscordMessage[]>(res);

  messageCache.set(cacheKey, {
    data,
    expiresAt: Date.now() + MESSAGE_CACHE_TTL_MS,
  });

  return data;
}

/**
 * Fetch channel info (name, type, etc.) from Discord.
 * Results are cached for 5 minutes per channelId.
 */
export async function fetchChannelInfo(
  channelId: string
): Promise<DiscordChannel> {
  const cached = channelCache.get(channelId);
  if (cached && Date.now() < cached.expiresAt) {
    return cached.data;
  }

  const url = `${DISCORD_API_BASE}/channels/${channelId}`;
  const res = await fetch(url, { headers: authHeaders() });
  const data = await handleResponse<DiscordChannel>(res);

  channelCache.set(channelId, {
    data,
    expiresAt: Date.now() + CHANNEL_CACHE_TTL_MS,
  });

  return data;
}

// --- Webhook-based message sending ---

interface ChannelWebhook {
  id: string;
  token: string;
}

const WEBHOOK_NAME = 'Lens Editor Bridge';

// In-memory cache: channelId -> webhook credentials
const webhookCache = new Map<string, ChannelWebhook>();

/**
 * Get or create a webhook for the given channel.
 * Bot must have MANAGE_WEBHOOKS permission.
 */
async function getOrCreateWebhook(
  channelId: string
): Promise<ChannelWebhook> {
  const cached = webhookCache.get(channelId);
  if (cached) return cached;

  // Check for existing webhook we own
  const listUrl = `${DISCORD_API_BASE}/channels/${channelId}/webhooks`;
  const listRes = await fetch(listUrl, { headers: authHeaders() });
  const webhooks = await handleResponse<
    Array<{ id: string; token?: string; name: string; user?: { id: string } }>
  >(listRes);

  const botTokenId = getToken(); // used to identify our bot
  const existing = webhooks.find(
    (w) => w.name === WEBHOOK_NAME && w.token
  );

  if (existing) {
    const entry = { id: existing.id, token: existing.token! };
    webhookCache.set(channelId, entry);
    return entry;
  }

  // Create a new webhook
  const createUrl = `${DISCORD_API_BASE}/channels/${channelId}/webhooks`;
  const createRes = await fetch(createUrl, {
    method: 'POST',
    headers: authHeaders(),
    body: JSON.stringify({ name: WEBHOOK_NAME }),
  });
  const created = await handleResponse<{ id: string; token: string }>(
    createRes
  );

  const entry = { id: created.id, token: created.token };
  webhookCache.set(channelId, entry);
  console.log(
    `[discord-client] Created webhook for channel ${channelId}`
  );
  return entry;
}

/**
 * Send a message to a Discord channel via a bot-managed webhook.
 * The webhook is auto-created per channel; username/avatar can be customized per message.
 */
export async function sendWebhookMessage(
  channelId: string,
  content: string,
  username: string
): Promise<DiscordMessage> {
  const webhook = await getOrCreateWebhook(channelId);
  const url = `${DISCORD_API_BASE}/webhooks/${webhook.id}/${webhook.token}?wait=true`;
  const res = await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ content, username }),
  });

  return handleResponse<DiscordMessage>(res);
}
