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

interface UseMessagesResult {
  messages: DiscordMessage[];
  channelName: string | null;
  loading: boolean;
  error: string | null;
  refetch: () => void;
}

/**
 * Hook: fetches messages from the discord-bridge proxy API.
 *
 * @param channelId - Discord channel ID to fetch, or null to disable
 */
export function useMessages(channelId: string | null): UseMessagesResult {
  const [messages, setMessages] = useState<DiscordMessage[]>([]);
  const [channelName, setChannelName] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);
  const [fetchTrigger, setFetchTrigger] = useState(0);

  const refetch = useCallback(() => {
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

  return { messages, channelName, loading, error, refetch };
}
