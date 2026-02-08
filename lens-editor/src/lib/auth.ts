export interface ClientToken {
  url: string;
  baseUrl: string;
  docId: string;
  token?: string;
  authorization: 'full' | 'read-only';
}

// Server token from existing relay-git-sync setup
const SERVER_TOKEN = '2D3RhEOhAQSgWEGkAWxyZWxheS1zZXJ2ZXIDeB1odHRwczovL3JlbGF5LmxlbnNhY2FkZW15Lm9yZwYaaWdOJToAATlIZnNlcnZlckhUsS3xaA3zBw';
const RELAY_URL = 'https://relay.lensacademy.org';

// Local relay server doesn't need auth
const USE_LOCAL_RELAY = import.meta.env.VITE_LOCAL_RELAY === 'true';

// In development, use Vite proxy to avoid CORS
const AUTH_BASE = import.meta.env.DEV ? '/api/relay' : RELAY_URL;

export async function getClientToken(docId: string): Promise<ClientToken> {
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
  };

  // Only add auth header for production Relay (local relay has no auth)
  if (!USE_LOCAL_RELAY) {
    headers['Authorization'] = `Bearer ${SERVER_TOKEN}`;
  }

  const response = await fetch(`${AUTH_BASE}/doc/${docId}/auth`, {
    method: 'POST',
    headers,
    body: JSON.stringify({
      authorization: 'full',
    }),
  });

  if (!response.ok) {
    throw new Error(`Auth failed: ${response.status} ${response.statusText}`);
  }

  const data = await response.json();

  return {
    url: data.url,
    baseUrl: data.baseUrl || (USE_LOCAL_RELAY ? `http://localhost:8090` : RELAY_URL),
    docId: data.docId,
    token: data.token,
    authorization: 'full',
  };
}
