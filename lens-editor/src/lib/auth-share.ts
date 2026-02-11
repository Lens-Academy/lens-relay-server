import type { UserRole } from '../contexts/AuthContext';

const SESSION_KEY = 'lens-share-token';

/**
 * Read the share token from the URL query parameter ?t=,
 * falling back to localStorage (survives page refresh).
 */
export function getShareTokenFromUrl(): string | null {
  const params = new URLSearchParams(window.location.search);
  const fromUrl = params.get('t');
  if (fromUrl) {
    localStorage.setItem(SESSION_KEY, fromUrl);
    return fromUrl;
  }
  return localStorage.getItem(SESSION_KEY);
}

/**
 * Strip the share token from the URL bar via history.replaceState
 * to prevent leakage via Referer headers, bookmarks, and browser history.
 */
export function stripShareTokenFromUrl(): void {
  const url = new URL(window.location.href);
  if (!url.searchParams.has('t')) return;
  url.searchParams.delete('t');
  window.history.replaceState({}, '', url.pathname + url.search + url.hash);
}

const BYTE_TO_ROLE: Record<number, UserRole> = { 1: 'edit', 2: 'suggest', 3: 'view' };

/** base64url decode to Uint8Array (browser-compatible, no Buffer) */
function base64urlToBytes(str: string): Uint8Array {
  // base64url → base64
  const b64 = str.replace(/-/g, '+').replace(/_/g, '/');
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

/**
 * Decode the role from a compact binary share token (no signature verification).
 * Token format: base64url(role:1 + uuid:16 + expiry:4 + hmac:8)
 * Role is the first byte: 1=edit, 2=suggest, 3=view.
 */
export function decodeRoleFromToken(token: string): UserRole | null {
  try {
    const bytes = base64urlToBytes(token);
    if (bytes.length < 1) return null;
    return BYTE_TO_ROLE[bytes[0]] ?? null;
  } catch {
    return null;
  }
}
