import { useState, useEffect, useCallback, useRef } from 'react';

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
  timestamp: string;
  type: number;
}

export interface DiscordChannel {
  id: string;
  name: string;
  type: number;
}

export type GatewayStatus = 'connected' | 'connecting' | 'disconnected' | 'reconnecting';

interface UseMessagesResult {
  messages: DiscordMessage[];
  channelName: string | null;
  loading: boolean;
  error: string | null;
  refetch: () => void;
  reconnect: () => void;
  gatewayStatus: GatewayStatus;
  sendMessage: (content: string, username: string) => Promise<void>;
}

/**
 * Hook: fetches messages from the discord proxy API.
 *
 * @param channelId - Discord channel ID to fetch, or null to disable
 */
export function useMessages(channelId: string | null): UseMessagesResult {
  const [messages, setMessages] = useState<DiscordMessage[]>([]);
  const [channelName, setChannelName] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [gatewayStatus, setGatewayStatus] = useState<GatewayStatus>('disconnected');
  const abortRef = useRef<AbortController | null>(null);
  const [fetchTrigger, setFetchTrigger] = useState(0);
  const [sseReconnectTrigger, setSseReconnectTrigger] = useState(0);

  const refetch = useCallback(() => {
    setFetchTrigger((t) => t + 1);
  }, []);

  const reconnect = useCallback(() => {
    setError(null);
    setSseReconnectTrigger((t) => t + 1);
    setFetchTrigger((t) => t + 1);
  }, []);

  useEffect(() => {
    if (!channelId) {
      setMessages([]);
      setChannelName(null);
      setLoading(false);
      setError(null);
      return;
    }

    // Abort previous request
    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;

    setLoading(true);
    setError(null);

    const fetchData = async () => {
      try {
        const [messagesRes, channelRes] = await Promise.all([
          fetch(`/api/discord/channels/${channelId}/messages?limit=50`, {
            signal: controller.signal,
          }),
          fetch(`/api/discord/channels/${channelId}`, {
            signal: controller.signal,
          }),
        ]);

        if (controller.signal.aborted) return;

        if (!messagesRes.ok) {
          if (messagesRes.status === 429) {
            const body = await messagesRes.json().catch(() => ({}));
            const retryAfter = body.retry_after ?? 'a few';
            setError(`Rate limited, try again in ${retryAfter} seconds`);
          } else {
            setError(`Failed to load messages (${messagesRes.status})`);
          }
          setLoading(false);
          return;
        }

        if (!channelRes.ok) {
          setError(`Failed to load channel info (${channelRes.status})`);
          setLoading(false);
          return;
        }

        const messagesData: DiscordMessage[] = await messagesRes.json();
        const channelData: DiscordChannel = await channelRes.json();

        if (controller.signal.aborted) return;

        // Discord returns newest-first; reverse to chronological (oldest-first)
        setMessages(messagesData.reverse());
        setChannelName(channelData.name);
        setLoading(false);
      } catch (err: unknown) {
        if (controller.signal.aborted) return;
        if (err instanceof DOMException && err.name === 'AbortError') return;
        setError('Could not connect to Discord bridge');
        setLoading(false);
      }
    };

    fetchData();

    return () => {
      controller.abort();
    };
  }, [channelId, fetchTrigger]);

  // SSE subscription for live message streaming
  useEffect(() => {
    if (!channelId) {
      setGatewayStatus('disconnected');
      return;
    }

    const HEARTBEAT_TIMEOUT_MS = 75_000; // 2.5x the 30s heartbeat interval
    let hasConnectedBefore = false;
    let heartbeatTimer: ReturnType<typeof setTimeout>;

    const resetHeartbeat = () => {
      clearTimeout(heartbeatTimer);
      heartbeatTimer = setTimeout(() => {
        setGatewayStatus('reconnecting');
      }, HEARTBEAT_TIMEOUT_MS);
    };

    const eventSource = new EventSource(`/api/discord/channels/${channelId}/events`);
    setGatewayStatus('connecting');

    eventSource.addEventListener('message', (e) => {
      resetHeartbeat();
      const newMsg: DiscordMessage = JSON.parse(e.data);
      setMessages((prev) => {
        // Dedup: skip if message ID already exists
        if (prev.some((m) => m.id === newMsg.id)) return prev;
        return [...prev, newMsg];
      });
    });

    eventSource.addEventListener('status', (e) => {
      resetHeartbeat();
      const { gateway } = JSON.parse(e.data);
      setGatewayStatus(gateway);
    });

    eventSource.addEventListener('heartbeat', () => {
      resetHeartbeat();
    });

    eventSource.onopen = () => {
      setGatewayStatus('connected');
      // Only clear SSE-specific errors; leave fetch errors intact
      setError((prev) => (prev === 'Connection lost' ? null : prev));
      resetHeartbeat();
      if (hasConnectedBefore) {
        // Reconnected after a drop — reload message history to fill gap
        setFetchTrigger((t) => t + 1);
      }
      hasConnectedBefore = true;
    };

    eventSource.onerror = () => {
      clearTimeout(heartbeatTimer);
      if (eventSource.readyState === EventSource.CLOSED) {
        // Terminal disconnect — browser will NOT auto-reconnect
        setGatewayStatus('disconnected');
        setError('Connection lost');
      } else {
        // readyState is CONNECTING — browser auto-reconnecting
        setGatewayStatus('reconnecting');
      }
    };

    return () => {
      clearTimeout(heartbeatTimer);
      eventSource.close();
      setGatewayStatus('disconnected');
    };
  }, [channelId, sseReconnectTrigger]);

  const sendMessage = useCallback(
    async (content: string, username: string) => {
      if (!channelId) throw new Error('No channel ID');

      const res = await fetch(`/api/discord/channels/${channelId}/messages`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ content, username }),
      });

      if (!res.ok) {
        const body = await res.json().catch(() => ({ error: 'Send failed' }));
        throw new Error(body.error || `HTTP ${res.status}`);
      }
      // No optimistic insert — message echoes back via SSE
    },
    [channelId]
  );

  return { messages, channelName, loading, error, refetch, reconnect, gatewayStatus, sendMessage };
}
