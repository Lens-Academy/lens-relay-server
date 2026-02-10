/**
 * Format a timestamp for display.
 * Uses relative time for recent, absolute for older.
 *
 * Accepts either an ISO 8601 string (from Discord API) or
 * epoch milliseconds (for CommentsPanel compatibility).
 */
export function formatTimestamp(input: string | number): string {
  const timestamp = typeof input === 'string' ? new Date(input).getTime() : input;
  const now = Date.now();
  const diff = now - timestamp;

  // Less than 1 minute
  if (diff < 60_000) return 'just now';
  // Less than 1 hour
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  // Less than 1 day
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  // Less than 7 days
  if (diff < 604_800_000) return `${Math.floor(diff / 86_400_000)}d ago`;

  // Older - show date
  return new Date(timestamp).toLocaleDateString('en-US', {
    month: 'short',
    day: 'numeric',
    year: timestamp < now - 31_536_000_000 ? 'numeric' : undefined,
  });
}
