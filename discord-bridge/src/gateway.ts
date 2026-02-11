import { Client, GatewayIntentBits, Events } from 'discord.js';
import { EventEmitter } from 'events';

// EventEmitter for decoupling Gateway events from SSE delivery.
// setMaxListeners(0) avoids Node.js warnings when many SSE clients connect
// (each subscribes a listener per channel).
export const gatewayEvents = new EventEmitter();
gatewayEvents.setMaxListeners(0);

let client: Client | null = null;

/**
 * Start the Discord Gateway connection.
 * If DISCORD_BOT_TOKEN is not set, logs a warning and returns
 * (REST-only mode still works from Phase 1).
 */
export function startGateway(): void {
  const token = process.env.DISCORD_BOT_TOKEN;
  if (!token) {
    console.warn('[gateway] DISCORD_BOT_TOKEN not set, Gateway disabled');
    return;
  }

  client = new Client({
    intents: [
      GatewayIntentBits.Guilds,
      GatewayIntentBits.GuildMessages,
      GatewayIntentBits.MessageContent,
    ],
  });

  client.on(Events.MessageCreate, (message) => {
    // Emit channel-scoped event with serializable payload
    gatewayEvents.emit(`message:${message.channelId}`, {
      id: message.id,
      content: message.content,
      author: {
        id: message.author.id,
        username: message.author.username,
        global_name: message.author.globalName,
        avatar: message.author.avatar,
        bot: message.author.bot,
      },
      timestamp: message.createdAt.toISOString(),
      type: message.type,
    });
  });

  client.on(Events.ClientReady, (c) => {
    console.log(`[gateway] Connected as ${c.user.tag}`);
    gatewayEvents.emit('status', { gateway: 'connected' });
  });

  client.on(Events.ShardReconnecting, () => {
    console.log('[gateway] Reconnecting...');
    gatewayEvents.emit('status', { gateway: 'reconnecting' });
  });

  client.on(Events.ShardResume, () => {
    console.log('[gateway] Resumed');
    gatewayEvents.emit('status', { gateway: 'connected' });
  });

  client.on(Events.ShardDisconnect, (ev, shardId) => {
    console.log(`[gateway] Shard ${shardId} disconnected (code ${ev.code})`);
    gatewayEvents.emit('status', { gateway: 'disconnected' });
  });

  client.login(token).catch((err) => {
    console.error('[gateway] Login failed:', err.message);
  });
}

/**
 * Get the current Gateway connection status.
 */
export function getGatewayStatus():
  | 'connected'
  | 'connecting'
  | 'disconnected' {
  if (!client) return 'disconnected';
  if (client.isReady()) return 'connected';
  return 'connecting';
}
