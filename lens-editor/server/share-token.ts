import { createHmac, timingSafeEqual } from 'node:crypto';
import type { UserRole } from '../shared/types.ts';

export interface ShareTokenPayload {
  role: UserRole;
  folder: string;   // UUID string
  expiry: number;    // unix seconds
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

const ROLE_TO_BYTE: Record<UserRole, number> = { edit: 1, suggest: 2, view: 3 };
const BYTE_TO_ROLE: Record<number, UserRole> = { 1: 'edit', 2: 'suggest', 3: 'view' };

const PAYLOAD_LEN = 21;  // 1 role + 16 uuid + 4 expiry
const SIG_LEN = 8;       // truncated HMAC-SHA256

/** Pack UUID string "xxxxxxxx-xxxx-..." into 16 raw bytes */
function uuidToBytes(uuid: string): Buffer {
  return Buffer.from(uuid.replace(/-/g, ''), 'hex');
}

/** Unpack 16 raw bytes back to UUID string */
function bytesToUuid(buf: Buffer): string {
  const hex = buf.toString('hex');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function packPayload(payload: ShareTokenPayload): Buffer {
  const buf = Buffer.alloc(PAYLOAD_LEN);
  buf[0] = ROLE_TO_BYTE[payload.role];
  uuidToBytes(payload.folder).copy(buf, 1);
  buf.writeUInt32BE(payload.expiry, 17);
  return buf;
}

function unpackPayload(buf: Buffer): ShareTokenPayload | null {
  if (buf.length < PAYLOAD_LEN) return null;
  const role = BYTE_TO_ROLE[buf[0]];
  if (!role) return null;
  const folder = bytesToUuid(buf.subarray(1, 17));
  const expiry = buf.readUInt32BE(17);
  return { role, folder, expiry };
}

/**
 * Sign a share token. Returns a compact base64url string (~39 chars).
 * Format: base64url(role:1 + uuid:16 + expiry:4 + hmac:8)
 */
export function signShareToken(payload: ShareTokenPayload): string {
  const secret = getSecret();
  const packed = packPayload(payload);
  const fullSig = createHmac('sha256', secret).update(packed).digest();
  const token = Buffer.concat([packed, fullSig.subarray(0, SIG_LEN)]);
  return token.toString('base64url');
}

/**
 * Verify and decode a share token. Returns null if invalid or expired.
 */
export function verifyShareToken(token: string): ShareTokenPayload | null {
  let raw: Buffer;
  try {
    raw = Buffer.from(token, 'base64url');
  } catch {
    return null;
  }

  if (raw.length !== PAYLOAD_LEN + SIG_LEN) return null;

  const packed = raw.subarray(0, PAYLOAD_LEN);
  const sig = raw.subarray(PAYLOAD_LEN);

  const secret = getSecret();
  const expectedSig = createHmac('sha256', secret).update(packed).digest().subarray(0, SIG_LEN);

  if (!timingSafeEqual(expectedSig, sig)) return null;

  const payload = unpackPayload(packed);
  if (!payload) return null;

  // Check expiration
  if (payload.expiry < Math.floor(Date.now() / 1000)) return null;

  return payload;
}

/**
 * Decode token payload WITHOUT verifying signature.
 * Used by frontend to extract role for UI purposes.
 */
export function decodeShareTokenPayload(token: string): ShareTokenPayload | null {
  try {
    const raw = Buffer.from(token, 'base64url');
    return unpackPayload(raw);
  } catch {
    return null;
  }
}
