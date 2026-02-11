export interface ClientToken {
  url: string;
  baseUrl: string;
  docId: string;
  token?: string;
  authorization: 'full' | 'read-only';
}

/**
 * Module-level share token — set once at app startup, used by all relay calls.
 * This ensures no relay connection can be made without a validated share token.
 */
let _shareToken: string | null = null;

/** Store the share token for all subsequent relay auth calls. */
export function setShareToken(token: string): void {
  _shareToken = token;
}

/**
 * Rewrite relay URLs to use the Vite WebSocket proxy in development.
 * The relay returns ws://localhost:PORT/... URLs which the browser can't reach
 * when accessing via SSH tunnel (dev.vps). Route through /ws/relay proxy instead.
 */
function rewriteRelayUrl(url: string): string {
  if (!import.meta.env.DEV) return url;
  try {
    const parsed = new URL(url);
    // Rewrite ws://host:port/path → ws://currentHost:currentPort/ws/relay/path
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    return `${protocol}//${window.location.host}/ws/relay${parsed.pathname}${parsed.search}`;
  } catch {
    return url;
  }
}

/** Rewrite HTTP base URLs similarly */
function rewriteRelayBaseUrl(url: string): string {
  if (!import.meta.env.DEV) return url;
  try {
    const parsed = new URL(url);
    return `${window.location.protocol}//${window.location.host}/ws/relay${parsed.pathname}${parsed.search}`;
  } catch {
    return url;
  }
}

/**
 * Get a client token for connecting to a relay document.
 * All access goes through server-side share token validation.
 */
export async function getClientToken(docId: string): Promise<ClientToken> {
  if (!_shareToken) {
    throw new Error('No share token — access denied');
  }

  const response = await fetch('/api/auth/token', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ token: _shareToken, docId }),
  });

  if (!response.ok) {
    const text = await response.text().catch(() => '');
    throw new Error(`Share token auth failed: ${response.status} ${text}`);
  }

  const data = await response.json();
  const token = data.clientToken as ClientToken;
  token.url = rewriteRelayUrl(token.url);
  token.baseUrl = rewriteRelayBaseUrl(token.baseUrl);
  return token;
}
