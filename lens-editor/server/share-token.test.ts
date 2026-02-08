import { describe, it, expect, afterEach } from 'vitest';
import { signShareToken, verifyShareToken, decodeShareTokenPayload } from './share-token.ts';
import type { ShareTokenPayload } from './share-token.ts';

describe('share-token', () => {
  const validPayload: ShareTokenPayload = {
    r: 'edit',
    f: 'fbd5eb54-73cc-41b0-ac28-2b93d3b4244e',
    x: Math.floor(Date.now() / 1000) + 3600, // 1 hour from now
  };

  describe('signShareToken + verifyShareToken', () => {
    it('should sign and verify a valid token', () => {
      const token = signShareToken(validPayload);
      const result = verifyShareToken(token);
      expect(result).toEqual(validPayload);
    });

    it('should return null for tampered payload', () => {
      const token = signShareToken(validPayload);
      // Tamper with payload part
      const parts = token.split('.');
      const tampered = 'x' + parts[0].slice(1) + '.' + parts[1];
      expect(verifyShareToken(tampered)).toBeNull();
    });

    it('should return null for tampered signature', () => {
      const token = signShareToken(validPayload);
      const parts = token.split('.');
      const tampered = parts[0] + '.x' + parts[1].slice(1);
      expect(verifyShareToken(tampered)).toBeNull();
    });

    it('should return null for expired token', () => {
      const expired: ShareTokenPayload = {
        ...validPayload,
        x: Math.floor(Date.now() / 1000) - 1, // 1 second ago
      };
      const token = signShareToken(expired);
      expect(verifyShareToken(token)).toBeNull();
    });

    it('should return null for malformed token (no dot)', () => {
      expect(verifyShareToken('nodothere')).toBeNull();
    });

    it('should return null for invalid role', () => {
      const badRole = { ...validPayload, r: 'admin' as any };
      const token = signShareToken(badRole);
      expect(verifyShareToken(token)).toBeNull();
    });
  });

  describe('decodeShareTokenPayload', () => {
    it('should decode payload without verification', () => {
      const token = signShareToken(validPayload);
      const payload = decodeShareTokenPayload(token);
      expect(payload).toEqual(validPayload);
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
