/**
 * Minimal Discord API types — only the fields we actually use.
 * Avoids pulling in the large discord-api-types package.
 */

export interface DiscordUser {
  id: string;
  username: string;
  global_name: string | null;
  avatar: string | null;
  bot?: boolean;
}

export interface DiscordMessage {
  id: string;
  content: string;
  author: DiscordUser;
  timestamp: string; // ISO8601
  type: number;
}

export interface DiscordChannel {
  id: string;
  name: string;
  type: number;
}

