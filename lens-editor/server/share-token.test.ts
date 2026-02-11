import { describe, it, expect, afterEach } from 'vitest';
import { signShareToken, verifyShareToken, decodeShareTokenPayload } from './share-token.ts';
import type { ShareTokenPayload } from './share-token.ts';

describe('share-token', () => {
  const validPayload: ShareTokenPayload = {
    role: 'edit',
    folder: 'fbd5eb54-73cc-41b0-ac28-2b93d3b4244e',
    expiry: Math.floor(Date.now() / 1000) + 3600, // 1 hour from now
  };

  describe('signShareToken + verifyShareToken', () => {
    it('should sign and verify a valid token', () => {
      const token = signShareToken(validPayload);
      const result = verifyShareToken(token);
      expect(result).toEqual(validPayload);
    });

    it('should produce a compact token (~39 chars)', () => {
      const token = signShareToken(validPayload);
      // 29 bytes base64url → ceil(29*4/3) = 39 chars
      expect(token.length).toBeLessThanOrEqual(40);
    });

    it('should return null for tampered token', () => {
      const token = signShareToken(validPayload);
      // Flip a character in the middle of the token
      const mid = Math.floor(token.length / 2);
      const c = token[mid] === 'A' ? 'B' : 'A';
      const tampered = token.slice(0, mid) + c + token.slice(mid + 1);
      expect(verifyShareToken(tampered)).toBeNull();
    });

    it('should return null for truncated token', () => {
      const token = signShareToken(validPayload);
      expect(verifyShareToken(token.slice(0, -4))).toBeNull();
    });

    it('should return null for expired token', () => {
      const expired: ShareTokenPayload = {
        ...validPayload,
        expiry: Math.floor(Date.now() / 1000) - 1, // 1 second ago
      };
      const token = signShareToken(expired);
      expect(verifyShareToken(token)).toBeNull();
    });

    it('should return null for empty string', () => {
      expect(verifyShareToken('')).toBeNull();
    });

    it('should return null for garbage input', () => {
      expect(verifyShareToken('not-a-valid-token')).toBeNull();
    });

    it('should handle all three roles', () => {
      for (const role of ['edit', 'suggest', 'view'] as const) {
        const payload: ShareTokenPayload = { ...validPayload, role };
        const token = signShareToken(payload);
        const result = verifyShareToken(token);
        expect(result?.role).toBe(role);
      }
    });
  });

  describe('decodeShareTokenPayload', () => {
    it('should decode payload without verification', () => {
      const token = signShareToken(validPayload);
      const payload = decodeShareTokenPayload(token);
      expect(payload).toEqual(validPayload);
    });

    it('should decode even with tampered signature', () => {
      const token = signShareToken(validPayload);
      // Tamper last char (in the signature region)
      const c = token[token.length - 1] === 'A' ? 'B' : 'A';
      const tampered = token.slice(0, -1) + c;
      const payload = decodeShareTokenPayload(tampered);
      // Payload should still decode (signature not checked)
      expect(payload?.role).toBe('edit');
      expect(payload?.folder).toBe(validPayload.folder);
    });

    it('should return null for malformed token', () => {
      expect(decodeShareTokenPayload('garbage')).toBeNull();
    });
  });

  describe('production secret enforcement', () => {
    const origEnv = process.env.NODE_ENV;
    const origSecret = process.env.SHARE_TOKEN_SECRET;

    afterEach(() => {
      process.env.NODE_ENV = origEnv;
      if (origSecret !== undefined) {
        process.env.SHARE_TOKEN_SECRET = origSecret;
      } else {
        delete process.env.SHARE_TOKEN_SECRET;
      }
    });

    it('should throw in production without SHARE_TOKEN_SECRET', () => {
      process.env.NODE_ENV = 'production';
      delete process.env.SHARE_TOKEN_SECRET;
      expect(() => signShareToken(validPayload)).toThrow('SHARE_TOKEN_SECRET is required in production');
    });
  });
});
