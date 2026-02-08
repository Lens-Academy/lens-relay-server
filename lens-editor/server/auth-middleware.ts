import { verifyShareToken } from './share-token.ts';
import type { ClientToken, UserRole } from '../shared/types.ts';

interface AuthHandlerConfig {
  relayServerUrl: string;
  relayServerToken?: string; // Optional — local relay has no auth
}

interface AuthResponse {
  clientToken: ClientToken;
  role: UserRole;
}

/**
 * Creates a request handler for POST /api/auth/token
 *
 * Request body: { token: string, docId: string }
 * Response: { clientToken: ClientToken, role: UserRole }
 */
export function createAuthHandler(config: AuthHandlerConfig) {
  return async (body: { token: string; docId: string }): Promise<AuthResponse> => {
    const { token, docId } = body;

    // 1. Verify share token
    const payload = verifyShareToken(token);
    if (!payload) {
      throw new AuthError(401, 'Invalid or expired share token');
    }

    // 2. Determine relay authorization level
    // view -> read-only (server-enforced), suggest/edit -> full (frontend-enforced role distinction)
    const relayAuth = payload.r === 'view' ? 'read-only' : 'full';

    // 3. Mint relay doc token by proxying to relay server
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    };
    if (config.relayServerToken) {
      headers['Authorization'] = `Bearer ${config.relayServerToken}`;
    }

    const relayResponse = await fetch(`${config.relayServerUrl}/doc/${docId}/auth`, {
      method: 'POST',
      headers,
      body: JSON.stringify({ authorization: relayAuth }),
    });

    if (!relayResponse.ok) {
      throw new AuthError(502, `Relay server error: ${relayResponse.status}`);
    }

    const relayData = await relayResponse.json() as Record<string, unknown>;

    const clientToken: ClientToken = {
      url: relayData.url as string,
      baseUrl: (relayData.baseUrl as string) || config.relayServerUrl,
      docId: relayData.docId as string,
      token: relayData.token as string | undefined,
      authorization: relayAuth,
    };

    return { clientToken, role: payload.r };
  };
}

export class AuthError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.name = 'AuthError';
    this.status = status;
  }
}
