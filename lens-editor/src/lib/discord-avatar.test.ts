import { describe, it, expect } from 'vitest';
import { getAvatarUrl } from './discord-avatar';

describe('getAvatarUrl', () => {
  describe('custom avatars', () => {
    it('returns PNG URL for custom avatar with default size', () => {
      const url = getAvatarUrl('123', 'abc123');
      expect(url).toBe('https://cdn.discordapp.com/avatars/123/abc123.png?size=64');
    });

    it('returns URL with explicit size', () => {
      const url = getAvatarUrl('123', 'abc123', 128);
      expect(url).toBe('https://cdn.discordapp.com/avatars/123/abc123.png?size=128');
    });

    it('returns GIF URL for animated avatar (hash starts with a_)', () => {
      const url = getAvatarUrl('123', 'a_abc123');
      expect(url).toBe('https://cdn.discordapp.com/avatars/123/a_abc123.gif?size=64');
    });
  });

  describe('default avatars (null hash)', () => {
    it('returns default avatar URL when hash is null', () => {
      const url = getAvatarUrl('123456789012345678', null);
      // Default index = (BigInt(userId) >> 22n) % 6n
      const expectedIndex = Number((BigInt('123456789012345678') >> 22n) % 6n);
      expect(url).toBe(`https://cdn.discordapp.com/embed/avatars/${expectedIndex}.png`);
    });

    it('default avatar index is between 0 and 5', () => {
      // Test several user IDs to verify index range
      const userIds = [
        '100000000000000000',
        '200000000000000000',
        '300000000000000000',
        '400000000000000000',
        '500000000000000000',
        '600000000000000000',
        '700000000000000000',
      ];

      for (const userId of userIds) {
        const url = getAvatarUrl(userId, null);
        const match = url.match(/embed\/avatars\/(\d+)\.png/);
        expect(match).not.toBeNull();
        const index = Number(match![1]);
        expect(index).toBeGreaterThanOrEqual(0);
        expect(index).toBeLessThanOrEqual(5);
      }
    });
  });
});
