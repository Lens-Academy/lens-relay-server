export interface DiscordChannel {
  guildId: string;
  channelId: string;
}

export function parseDiscordUrl(_url: string): DiscordChannel | null {
  // STUB: returns wrong value for RED phase
  return { guildId: 'STUB', channelId: 'STUB' };
}
