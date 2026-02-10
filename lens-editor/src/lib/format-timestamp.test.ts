import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { formatTimestamp } from './format-timestamp';

describe('formatTimestamp', () => {
  beforeEach(() => {
    // Fix "now" to 2025-06-15T12:00:00.000Z for deterministic tests
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2025-06-15T12:00:00.000Z'));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  describe('relative timestamps (ISO string input)', () => {
    it('returns "just now" for less than 1 minute ago', () => {
      expect(formatTimestamp('2025-06-15T11:59:30.000Z')).toBe('just now');
    });

    it('returns minutes ago for 5 minutes ago', () => {
      expect(formatTimestamp('2025-06-15T11:55:00.000Z')).toBe('5m ago');
    });

    it('returns hours ago for 3 hours ago', () => {
      expect(formatTimestamp('2025-06-15T09:00:00.000Z')).toBe('3h ago');
    });

    it('returns days ago for 2 days ago', () => {
      expect(formatTimestamp('2025-06-13T12:00:00.000Z')).toBe('2d ago');
    });
  });

  describe('absolute timestamps (ISO string input)', () => {
    it('returns month and day for 2 weeks ago', () => {
      expect(formatTimestamp('2025-06-01T12:00:00.000Z')).toBe('Jun 1');
    });

    it('includes year for timestamps over a year old', () => {
      expect(formatTimestamp('2024-01-15T12:00:00.000Z')).toBe('Jan 15, 2024');
    });
  });

  describe('numeric timestamp input (epoch millis)', () => {
    it('formats epoch milliseconds (for CommentsPanel compatibility)', () => {
      const thirtySecondsAgo = new Date('2025-06-15T11:59:30.000Z').getTime();
      expect(formatTimestamp(thirtySecondsAgo)).toBe('just now');
    });

    it('formats older epoch milliseconds with date', () => {
      const twoWeeksAgo = new Date('2025-06-01T12:00:00.000Z').getTime();
      expect(formatTimestamp(twoWeeksAgo)).toBe('Jun 1');
    });
  });
});
