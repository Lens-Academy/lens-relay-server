/**
 * @vitest-environment happy-dom
 *
 * Integration smoke tests that hit a running discord dev server (npm run discord:start).
 * Skipped unless DISCORD_BOT_TOKEN and DISCORD_TEST_CHANNEL_ID are set.
 */
import { describe, it, expect } from 'vitest';

const CHANNEL_ID = process.env.DISCORD_TEST_CHANNEL_ID;

describe.skipIf(!process.env.DISCORD_BOT_TOKEN || !CHANNEL_ID)(
  'DiscussionPanel integration (live Discord)',
  () => {
    const BRIDGE_URL = process.env.DISCORD_BRIDGE_URL || 'http://localhost:8091';

    it('fetches messages with expected structure', async () => {
      const res = await fetch(`${BRIDGE_URL}/api/channels/${CHANNEL_ID}/messages?limit=10`);
      expect(res.ok).toBe(true);
      const messages = await res.json();

      // Structural invariants (not exact content)
      expect(messages.length).toBeGreaterThan(0);

      for (const msg of messages) {
        expect(msg).toHaveProperty('id');
        expect(msg).toHaveProperty('content');
        expect(msg).toHaveProperty('timestamp');
        expect(msg.author).toHaveProperty('id');
        expect(msg.author).toHaveProperty('username');
        expect(msg.author).toHaveProperty('avatar');
      }

      // Verify timestamps are parseable
      for (const msg of messages) {
        expect(new Date(msg.timestamp).getTime()).not.toBeNaN();
      }
    });

    it('fetches channel info with expected structure', async () => {
      const res = await fetch(`${BRIDGE_URL}/api/channels/${CHANNEL_ID}`);
      expect(res.ok).toBe(true);
      const channel = await res.json();

      expect(channel).toHaveProperty('id', CHANNEL_ID);
      expect(channel).toHaveProperty('name');
      expect(typeof channel.name).toBe('string');
      expect(channel.name.length).toBeGreaterThan(0);
    });
  },
);
