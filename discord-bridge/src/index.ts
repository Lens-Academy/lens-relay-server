import { serve } from '@hono/node-server';
import { Hono } from 'hono';
import { cors } from 'hono/cors';
import { logger } from 'hono/logger';
import {
  fetchChannelMessages,
  fetchChannelInfo,
  RateLimitError,
  DiscordApiError,
} from './discord-client.js';

const app = new Hono();

// Logging
app.use('*', logger());

// CORS for /api/* (needed in production; Vite proxy handles dev)
app.use('/api/*', cors());

// Health check
app.get('/health', (c) => c.json({ status: 'ok' }));

// GET /api/channels/:channelId/messages
app.get('/api/channels/:channelId/messages', async (c) => {
  const { channelId } = c.req.param();
  const limitParam = c.req.query('limit') || '50';
  const limit = Math.min(Math.max(parseInt(limitParam, 10) || 50, 1), 100);

  try {
    const messages = await fetchChannelMessages(channelId, limit);
    return c.json(messages);
  } catch (err) {
    if (err instanceof RateLimitError) {
      return c.json(
        { error: 'Rate limited by Discord', retryAfter: err.retryAfter },
        429
      );
    }
    if (err instanceof DiscordApiError) {
      return c.json(
        { error: 'Discord API error', details: err.body },
        err.status as 400
      );
    }
    // Token missing or unexpected error
    const message = err instanceof Error ? err.message : 'Unknown error';
    console.error('[discord-bridge] Error fetching messages:', message);
    return c.json({ error: message }, 500);
  }
});

// GET /api/channels/:channelId
app.get('/api/channels/:channelId', async (c) => {
  const { channelId } = c.req.param();

  try {
    const channel = await fetchChannelInfo(channelId);
    return c.json(channel);
  } catch (err) {
    if (err instanceof RateLimitError) {
      return c.json(
        { error: 'Rate limited by Discord', retryAfter: err.retryAfter },
        429
      );
    }
    if (err instanceof DiscordApiError) {
      return c.json(
        { error: 'Discord API error', details: err.body },
        err.status as 400
      );
    }
    const message = err instanceof Error ? err.message : 'Unknown error';
    console.error('[discord-bridge] Error fetching channel:', message);
    return c.json({ error: message }, 500);
  }
});

// Port detection: workspace convention
const cwdMatch = process.cwd().match(/ws(\d+)/);
const wsNum = cwdMatch ? parseInt(cwdMatch[1], 10) : 1;
const defaultPort = 8091 + (wsNum - 1) * 100;
const port = parseInt(
  process.env.DISCORD_BRIDGE_PORT || String(defaultPort),
  10
);

serve({ fetch: app.fetch, port }, () => {
  console.log(`[discord-bridge] Listening on port ${port}`);
});
