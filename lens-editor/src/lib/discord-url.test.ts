import { describe, it, expect } from 'vitest';
import { parseDiscordUrl } from './discord-url';

describe('parseDiscordUrl', () => {
  describe('valid channel URLs', () => {
    it('parses standard Discord channel URL', () => {
      const result = parseDiscordUrl('https://discord.com/channels/1234/5678');
      expect(result).toEqual({ guildId: '1234', channelId: '5678' });
    });

    it('handles trailing slash', () => {
      const result = parseDiscordUrl('https://discord.com/channels/1234/5678/');
      expect(result).toEqual({ guildId: '1234', channelId: '5678' });
    });

    it('handles http (non-https)', () => {
      const result = parseDiscordUrl('http://discord.com/channels/1234/5678');
      expect(result).toEqual({ guildId: '1234', channelId: '5678' });
    });

    it('handles www prefix', () => {
      const result = parseDiscordUrl('https://www.discord.com/channels/1234/5678');
      expect(result).toEqual({ guildId: '1234', channelId: '5678' });
    });

    it('parses real-world snowflake IDs', () => {
      const result = parseDiscordUrl(
        'https://discord.com/channels/1440725236843806762/1465349126073094469'
      );
      expect(result).toEqual({
        guildId: '1440725236843806762',
        channelId: '1465349126073094469',
      });
    });
  });

  describe('invalid inputs', () => {
    it('returns null for empty string', () => {
      expect(parseDiscordUrl('')).toBeNull();
    });

    it('returns null for missing channel ID', () => {
      expect(parseDiscordUrl('https://discord.com/channels/1234')).toBeNull();
    });

    it('returns null for invite URL (not a channel URL)', () => {
      expect(parseDiscordUrl('https://discord.com/invite/abc')).toBeNull();
    });

    it('returns null for plain text', () => {
      expect(parseDiscordUrl('not a url')).toBeNull();
    });
  });
});
