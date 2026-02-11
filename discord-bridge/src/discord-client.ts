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

// --- Bot message sending ---

/**
 * Send a message to a Discord channel using the bot token.
 * Uses POST /channels/{channelId}/messages with bot auth.
 */
export async function sendBotMessage(
  channelId: string,
  content: string
): Promise<DiscordMessage> {
  const url = `${DISCORD_API_BASE}/channels/${channelId}/messages`;
  const res = await fetch(url, {
    method: 'POST',
    headers: authHeaders(),
    body: JSON.stringify({ content }),
  });

  return handleResponse<DiscordMessage>(res);
}
