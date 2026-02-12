import http from 'node:http';
import httpProxy from 'http-proxy';
import { Hono } from 'hono';
import { getRequestListener } from '@hono/node-server';
import { serveStatic } from '@hono/node-server/serve-static';
import { createAuthHandler, AuthError } from './auth-middleware.ts';
import { discordRoutes, initDiscordGateway } from './discord/routes.ts';

const relayUrl = process.env.RELAY_URL || 'http://relay-server:8080';
const relayServerToken = process.env.RELAY_SERVER_TOKEN;
const port = parseInt(process.env.PORT || '3000', 10);

// Reverse proxy for relay-server only (discord is now inline)
const proxy = httpProxy.createProxyServer();
proxy.on('error', (err, _req, res) => {
  console.error('[proxy] Error:', err.message);
  if ('writeHead' in res && typeof (res as http.ServerResponse).writeHead === 'function') {
    const sres = res as http.ServerResponse;
    if (!sres.headersSent) {
      sres.writeHead(502, { 'Content-Type': 'application/json' });
      sres.end(JSON.stringify({ error: 'Bad gateway' }));
    }
  }
});

// Auth handler
const authHandler = createAuthHandler({
  relayServerUrl: relayUrl,
  relayServerToken,
});

// Hono app for auth, discord, and static files
const app = new Hono();

app.post('/api/auth/token', async (c) => {
  try {
    const body = await c.req.json();
    const result = await authHandler(body);
    return c.json(result);
  } catch (error) {
    if (error instanceof AuthError) {
      return c.json({ error: error.message }, error.status as 400);
    }
    return c.json({ error: 'Internal server error' }, 500);
  }
});

// Mount discord routes under /api/discord
app.route('/api/discord', discordRoutes);

// Static files from Vite build output
app.use('/*', serveStatic({ root: './dist' }));
// SPA fallback
app.get('/*', serveStatic({ root: './dist', path: 'index.html' }));

const honoListener = getRequestListener(app.fetch);

// Node HTTP server: relay proxy bypasses Hono, everything else goes through it
const server = http.createServer((req, res) => {
  const url = req.url || '/';

  if (url.startsWith('/api/relay/') || url === '/api/relay') {
    req.url = url.replace(/^\/api\/relay/, '') || '/';
    if (relayServerToken) {
      req.headers['authorization'] = `Bearer ${relayServerToken}`;
    }
    proxy.web(req, res, { target: relayUrl, changeOrigin: true });
  } else {
    honoListener(req, res);
  }
});

// Start Discord Gateway (eager — connects on startup, no-op without DISCORD_BOT_TOKEN)
initDiscordGateway();

server.listen(port, () => {
  console.log(`[lens-editor] Production server on port ${port}`);
});
