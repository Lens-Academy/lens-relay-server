import path from 'path';
import { defineConfig } from 'vite';
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
const defaultBridgePort = 8091 + portOffset;

// https://vite.dev/config/
export default defineConfig(() => {
  // Use local relay-server when VITE_LOCAL_RELAY is set
  const useLocalRelay = process.env.VITE_LOCAL_RELAY === 'true';
  const relayPort = parseInt(process.env.RELAY_PORT || String(defaultRelayPort), 10);
  const relayTarget = useLocalRelay
    ? `http://localhost:${relayPort}`
    : 'https://relay.lensacademy.org';

  const bridgePort = parseInt(process.env.DISCORD_BRIDGE_PORT || String(defaultBridgePort), 10);

  console.log(`[vite] Workspace ${wsNum}: Vite port ${defaultVitePort}, Relay port ${relayPort}`);
  console.log(`[vite] Relay target: ${relayTarget}`);
  console.log(`[vite] Discord bridge port ${bridgePort}`);

  return {
    plugins: [react(), tailwindcss()],
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
        // Proxy Discord bridge requests to sidecar
        '/api/discord': {
          target: `http://localhost:${bridgePort}`,
          changeOrigin: true,
          rewrite: (path) => path.replace(/^\/api\/discord/, '/api'),
        },
      },
    },
  };
});
