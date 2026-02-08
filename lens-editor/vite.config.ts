import path from 'path';
import { defineConfig } from 'vite';
import type { Plugin } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';

// Extract workspace number from directory name (e.g., "lens-editor-ws2" → 2)
// Used to auto-assign ports: ws1 gets 5173/8090, ws2 gets 5273/8190, etc.
// No workspace suffix → 5173/8090 (default)
const workspaceMatch = path.basename(__dirname).match(/-ws(\d+)$/);
const wsNum = workspaceMatch ? parseInt(workspaceMatch[1], 10) : 1;
const portOffset = (wsNum - 1) * 100; // ws1=0, ws2=100, ws3=200...
const defaultVitePort = 5173 + portOffset;
const defaultRelayPort = 8090 + portOffset;

// https://vite.dev/config/
export default defineConfig(() => {
  // Use local relay-server when VITE_LOCAL_RELAY is set
  const useLocalRelay = process.env.VITE_LOCAL_RELAY === 'true';
  const relayPort = parseInt(process.env.RELAY_PORT || String(defaultRelayPort), 10);
  const relayTarget = useLocalRelay
    ? `http://localhost:${relayPort}`
    : 'https://relay.lensacademy.org';

  // Server token for minting relay doc tokens (optional for local relay)
  const relayServerToken = process.env.RELAY_SERVER_TOKEN;

  console.log(`[vite] Workspace ${wsNum}: Vite port ${defaultVitePort}, Relay port ${relayPort}`);
  console.log(`[vite] Relay target: ${relayTarget}`);

  /**
   * Vite plugin that adds /api/auth/token endpoint for share token validation.
   * This is dev-only — configureServer only runs in `vite dev`, not production builds.
   */
  function shareTokenAuthPlugin(): Plugin {
    return {
      name: 'share-token-auth',
      configureServer(server) {
        server.middlewares.use('/api/auth/token', async (req, res) => {
          if (req.method !== 'POST') {
            res.writeHead(405, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({ error: 'Method not allowed' }));
            return;
          }

          // Read request body
          const chunks: Buffer[] = [];
          for await (const chunk of req) {
            chunks.push(chunk as Buffer);
          }
          const body = JSON.parse(Buffer.concat(chunks).toString());

          try {
            // Dynamic import to avoid loading server modules at config time
            const { createAuthHandler } = await import('./server/auth-middleware.ts');
            const handler = createAuthHandler({
              relayServerUrl: relayTarget,
              relayServerToken: relayServerToken,
            });
            const result = await handler(body);
            res.writeHead(200, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify(result));
          } catch (error: any) {
            const status = error.status || 500;
            res.writeHead(status, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({ error: error.message }));
          }
        });
      },
    };
  }

  return {
    plugins: [react(), tailwindcss(), shareTokenAuthPlugin()],
    server: {
      port: parseInt(process.env.VITE_PORT || String(defaultVitePort), 10),
      allowedHosts: ['dev.vps'],
      proxy: {
        // Proxy auth requests to relay-server to avoid CORS
        '/api/relay': {
          target: relayTarget,
          changeOrigin: true,
          rewrite: (path) => path.replace(/^\/api\/relay/, ''),
          secure: !useLocalRelay,
        },
      },
    },
  };
});
