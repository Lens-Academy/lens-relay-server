import { createHmac, timingSafeEqual } from 'node:crypto';
import type { UserRole } from '../shared/types.ts';

export interface ShareTokenPayload {
  r: UserRole;  // role
  f: string;    // folder ID
  x: number;    // expiration (unix seconds)
}

const DEV_SECRET = 'lens-editor-dev-secret-do-not-use-in-production';

function getSecret(): string {
  const secret = process.env.SHARE_TOKEN_SECRET;
  if (secret) return secret;
  if (process.env.NODE_ENV === 'production') {
    throw new Error('SHARE_TOKEN_SECRET is required in production');
  }
  return DEV_SECRET;
}

function base64urlEncode(data: string | Buffer): string {
  const buf = typeof data === 'string' ? Buffer.from(data) : data;
  return buf.toString('base64url');
}

function base64urlDecode(str: string): Buffer {
  return Buffer.from(str, 'base64url');
}

export function signShareToken(payload: ShareTokenPayload): string {
  const secret = getSecret();
  const payloadJson = JSON.stringify(payload);
  const payloadB64 = base64urlEncode(payloadJson);
  const sig = createHmac('sha256', secret).update(payloadB64).digest();
  return `${payloadB64}.${base64urlEncode(sig)}`;
}

export function verifyShareToken(token: string): ShareTokenPayload | null {
  const parts = token.split('.');
  if (parts.length !== 2) return null;
  const [payloadB64, sigB64] = parts;

  const secret = getSecret();
  const expectedSig = createHmac('sha256', secret).update(payloadB64).digest();
  const actualSig = base64urlDecode(sigB64);

  if (expectedSig.length !== actualSig.length) return null;
  if (!timingSafeEqual(expectedSig, actualSig)) return null;

  try {
    const payload = JSON.parse(base64urlDecode(payloadB64).toString()) as ShareTokenPayload;
    // Check expiration
    if (payload.x < Math.floor(Date.now() / 1000)) return null;
    // Validate role
    if (!['edit', 'suggest', 'view'].includes(payload.r)) return null;
    return payload;
  } catch {
    return null;
  }
}

/**
 * Decode token payload WITHOUT verifying signature.
 * Used by frontend to extract role for UI purposes.
 */
export function decodeShareTokenPayload(token: string): ShareTokenPayload | null {
  const parts = token.split('.');
  if (parts.length !== 2) return null;
  try {
    return JSON.parse(base64urlDecode(parts[0]).toString()) as ShareTokenPayload;
  } catch {
    return null;
  }
}
