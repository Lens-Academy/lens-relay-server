import { describe, it, expect, afterEach } from 'vitest';
import { getShareTokenFromUrl, stripShareTokenFromUrl, decodeRoleFromToken } from './auth-share';

/** Build a fake binary token: base64url(roleByte + 16 padding bytes + 4 expiry bytes + 8 sig bytes) */
function makeFakeBinaryToken(roleByte: number): string {
  const bytes = new Uint8Array(29); // 1 + 16 + 4 + 8
  bytes[0] = roleByte;
  // Fill rest with arbitrary data (doesn't matter for role decode)
  for (let i = 1; i < 29; i++) bytes[i] = i;
  // base64url encode
  let binary = '';
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=/g, '');
}

describe('auth-share', () => {
  afterEach(() => {
    // Reset URL and localStorage after each test
    window.history.replaceState({}, '', '/');
    localStorage.clear();
  });

  describe('getShareTokenFromUrl', () => {
    it('should return token from ?t= parameter', () => {
      window.history.replaceState({}, '', '/?t=test-token-value');
      expect(getShareTokenFromUrl()).toBe('test-token-value');
    });

    it('should persist token in localStorage', () => {
      window.history.replaceState({}, '', '/?t=test-token-value');
      getShareTokenFromUrl();
      expect(localStorage.getItem('lens-share-token')).toBe('test-token-value');
    });

    it('should fall back to localStorage when no URL param', () => {
      localStorage.setItem('lens-share-token', 'stored-token');
      window.history.replaceState({}, '', '/');
      expect(getShareTokenFromUrl()).toBe('stored-token');
    });

    it('should return null when no token in URL or localStorage', () => {
      window.history.replaceState({}, '', '/');
      expect(getShareTokenFromUrl()).toBeNull();
    });

    it('should prefer URL param over localStorage', () => {
      localStorage.setItem('lens-share-token', 'old-token');
      window.history.replaceState({}, '', '/?t=new-token');
      expect(getShareTokenFromUrl()).toBe('new-token');
      expect(localStorage.getItem('lens-share-token')).toBe('new-token');
    });
  });

  describe('stripShareTokenFromUrl', () => {
    it('should remove ?t= from URL', () => {
      window.history.replaceState({}, '', '/?t=secret-token&other=keep');
      stripShareTokenFromUrl();
      expect(window.location.search).toBe('?other=keep');
    });

    it('should do nothing when no token present', () => {
      window.history.replaceState({}, '', '/?other=keep');
      stripShareTokenFromUrl();
      expect(window.location.search).toBe('?other=keep');
    });
  });

  describe('decodeRoleFromToken', () => {
    it('should decode edit role (byte 1)', () => {
      expect(decodeRoleFromToken(makeFakeBinaryToken(1))).toBe('edit');
    });

    it('should decode suggest role (byte 2)', () => {
      expect(decodeRoleFromToken(makeFakeBinaryToken(2))).toBe('suggest');
    });

    it('should decode view role (byte 3)', () => {
      expect(decodeRoleFromToken(makeFakeBinaryToken(3))).toBe('view');
    });

    it('should return null for unknown role byte', () => {
      expect(decodeRoleFromToken(makeFakeBinaryToken(0))).toBeNull();
      expect(decodeRoleFromToken(makeFakeBinaryToken(4))).toBeNull();
      expect(decodeRoleFromToken(makeFakeBinaryToken(255))).toBeNull();
    });

    it('should return null for empty string', () => {
      expect(decodeRoleFromToken('')).toBeNull();
    });

    it('should return null for garbage input', () => {
      expect(decodeRoleFromToken('not-valid-base64!!')).toBeNull();
    });
  });
});
