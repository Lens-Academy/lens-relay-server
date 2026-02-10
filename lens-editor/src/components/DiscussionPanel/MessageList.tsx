import { useRef, useEffect } from 'react';
import { MessageItem } from './MessageItem';
import { useAutoScroll } from './useAutoScroll';
import { NewMessagesBar } from './NewMessagesBar';
import type { DiscordMessage } from './useMessages';

interface MessageListProps {
  messages: DiscordMessage[];
}

const FIVE_MINUTES_MS = 5 * 60 * 1000;

/**
 * Determine whether consecutive messages from the same author within 5 minutes
 * should be grouped (showHeader = false for subsequent messages in a group).
 */
function shouldShowHeader(current: DiscordMessage, previous: DiscordMessage | null): boolean {
  if (!previous) return true;
  if (current.author.id !== previous.author.id) return true;

  const currentTime = new Date(current.timestamp).getTime();
  const previousTime = new Date(previous.timestamp).getTime();
  return Math.abs(currentTime - previousTime) > FIVE_MINUTES_MS;
}

/**
 * Scrollable message list with grouping logic and auto-scroll.
 * Consecutive messages from the same author within 5 minutes are visually grouped.
 * Auto-scrolls to new messages when at bottom; shows indicator when scrolled up.
 */
export function MessageList({ messages }: MessageListProps) {
  const { sentinelRef, containerRef, scrollToBottom, unseenCount } =
    useAutoScroll(messages.length);
  const initialScrollDone = useRef(false);

  // Scroll to bottom on initial load (when messages go from 0 to >0)
  useEffect(() => {
    if (messages.length > 0 && !initialScrollDone.current) {
      sentinelRef.current?.scrollIntoView({ behavior: 'instant' });
      initialScrollDone.current = true;
    }
    if (messages.length === 0) {
      initialScrollDone.current = false;
    }
  }, [messages.length, sentinelRef]);

  if (messages.length === 0) {
    return (
      <div className="flex-1 flex items-center justify-center p-4">
        <p className="text-sm text-gray-400">No messages yet</p>
      </div>
    );
  }

  return (
    <div className="flex-1 relative">
      <div ref={containerRef} className="absolute inset-0 overflow-y-auto">
        {messages.map((msg, i) => (
          <MessageItem
            key={msg.id}
            message={msg}
            showHeader={shouldShowHeader(msg, i > 0 ? messages[i - 1] : null)}
          />
        ))}
        <div ref={sentinelRef} className="h-1" />
      </div>
      <NewMessagesBar count={unseenCount} onClick={scrollToBottom} />
    </div>
  );
}
