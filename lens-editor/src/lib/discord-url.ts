export interface DiscordChannel {
  guildId: string;
  channelId: string;
}

const DISCORD_CHANNEL_RE =
  /^https?:\/\/(?:www\.)?discord\.com\/channels\/(\d+)\/(\d+)\/?$/;

/**
 * Parse a Discord channel URL into guild and channel IDs.
 * Returns null if the URL does not match the expected format.
 */
export function parseDiscordUrl(url: string): DiscordChannel | null {
  const match = url.match(DISCORD_CHANNEL_RE);
  if (!match) return null;
  return { guildId: match[1], channelId: match[2] };
}
