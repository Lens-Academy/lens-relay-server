import { useRef, useState, useCallback, useEffect } from 'react';

interface AutoScrollResult {
  sentinelRef: React.RefObject<HTMLDivElement | null>;
  containerRef: React.RefObject<HTMLDivElement | null>;
  isAtBottom: boolean;
  scrollToBottom: () => void;
  unseenCount: number;
  resetUnseen: () => void;
}

export function useAutoScroll(messageCount: number): AutoScrollResult {
  const sentinelRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [isAtBottom, setIsAtBottom] = useState(true);
  const [unseenCount, setUnseenCount] = useState(0);
  const prevCountRef = useRef(messageCount);

  // Track whether sentinel is visible via IntersectionObserver
  useEffect(() => {
    const sentinel = sentinelRef.current;
    if (!sentinel) return;

    const observer = new IntersectionObserver(
      ([entry]) => {
        setIsAtBottom(entry.isIntersecting);
        if (entry.isIntersecting) {
          setUnseenCount(0);
        }
      },
      { threshold: 0.1 },
    );

    observer.observe(sentinel);
    return () => observer.disconnect();
  }, []);

  // When new messages arrive, auto-scroll or increment unseen
  useEffect(() => {
    const newCount = messageCount - prevCountRef.current;
    prevCountRef.current = messageCount;

    if (newCount <= 0) return;

    if (isAtBottom) {
      sentinelRef.current?.scrollIntoView({ behavior: 'smooth' });
    } else {
      setUnseenCount((c) => c + newCount);
    }
  }, [messageCount, isAtBottom]);

  const scrollToBottom = useCallback(() => {
    sentinelRef.current?.scrollIntoView({ behavior: 'smooth' });
    setUnseenCount(0);
  }, []);

  const resetUnseen = useCallback(() => setUnseenCount(0), []);

  return { sentinelRef, containerRef, isAtBottom, scrollToBottom, unseenCount, resetUnseen };
}
