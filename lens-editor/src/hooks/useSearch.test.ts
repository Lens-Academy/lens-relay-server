/**
 * Unit+1 tests for useSearch hook.
 * Mocks searchDocuments from relay-api to test debounce, abort, loading, and error behavior.
 *
 * @vitest-environment happy-dom
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';

// Mock searchDocuments before importing the hook
const mockSearchDocuments = vi.fn();
vi.mock('../lib/relay-api', () => ({
  searchDocuments: (...args: unknown[]) => mockSearchDocuments(...args),
}));

import { useSearch } from './useSearch';

describe('useSearch', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    mockSearchDocuments.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('returns empty results and does NOT call searchDocuments for empty query', async () => {
    const { result } = renderHook(() => useSearch(''));

    await act(async () => {
      vi.advanceTimersByTime(500);
    });

    expect(result.current.results).toEqual([]);
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
    expect(mockSearchDocuments).not.toHaveBeenCalled();
  });

  it('returns empty results and does NOT call searchDocuments for query shorter than 2 chars', async () => {
    const { result } = renderHook(() => useSearch('a'));

    await act(async () => {
      vi.advanceTimersByTime(500);
    });

    expect(result.current.results).toEqual([]);
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
    expect(mockSearchDocuments).not.toHaveBeenCalled();
  });

  it('calls searchDocuments after debounce delay for valid query (2+ chars)', async () => {
    const mockResponse = {
      results: [{ doc_id: 'abc', title: 'Test', folder: 'Lens', snippet: 'test', score: 1 }],
      total_hits: 1,
      query: 'te',
    };
    mockSearchDocuments.mockResolvedValue(mockResponse);

    const { result } = renderHook(() => useSearch('te', 300));

    // Before debounce, loading should be true but searchDocuments not yet called
    expect(result.current.loading).toBe(true);
    expect(mockSearchDocuments).not.toHaveBeenCalled();

    // Advance past debounce
    await act(async () => {
      vi.advanceTimersByTime(300);
    });

    expect(mockSearchDocuments).toHaveBeenCalledWith('te', 20, expect.any(AbortSignal));
    expect(result.current.results).toEqual(mockResponse.results);
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it('does not call searchDocuments before debounce elapses', async () => {
    mockSearchDocuments.mockResolvedValue({ results: [], total_hits: 0, query: 'test' });

    renderHook(() => useSearch('test', 300));

    // Advance only 200ms (less than debounce)
    await act(async () => {
      vi.advanceTimersByTime(200);
    });

    expect(mockSearchDocuments).not.toHaveBeenCalled();
  });

  it('sets loading to true while request is in flight', async () => {
    let resolveSearch!: (value: unknown) => void;
    mockSearchDocuments.mockReturnValue(new Promise(r => { resolveSearch = r; }));

    const { result } = renderHook(() => useSearch('hello', 300));

    // loading should be true immediately (before debounce fires)
    expect(result.current.loading).toBe(true);

    // Fire the debounce
    await act(async () => {
      vi.advanceTimersByTime(300);
    });

    // Still loading (request in flight)
    expect(result.current.loading).toBe(true);

    // Resolve the request
    await act(async () => {
      resolveSearch({ results: [], total_hits: 0, query: 'hello' });
    });

    expect(result.current.loading).toBe(false);
  });

  it('sets error state when searchDocuments throws', async () => {
    mockSearchDocuments.mockRejectedValue(new Error('Search failed: 500'));

    const { result } = renderHook(() => useSearch('test', 300));

    await act(async () => {
      vi.advanceTimersByTime(300);
    });

    expect(result.current.error).toBe('Search failed: 500');
    expect(result.current.results).toEqual([]);
    expect(result.current.loading).toBe(false);
  });

  it('clears results when query changes to empty', async () => {
    const mockResponse = {
      results: [{ doc_id: 'abc', title: 'Test', folder: 'Lens', snippet: 'test', score: 1 }],
      total_hits: 1,
      query: 'test',
    };
    mockSearchDocuments.mockResolvedValue(mockResponse);

    const { result, rerender } = renderHook(
      ({ query }) => useSearch(query, 300),
      { initialProps: { query: 'test' } }
    );

    await act(async () => {
      vi.advanceTimersByTime(300);
    });

    expect(result.current.results).toEqual(mockResponse.results);

    // Clear query
    rerender({ query: '' });

    expect(result.current.results).toEqual([]);
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it('aborts previous request when query changes', async () => {
    let callCount = 0;
    const abortedSignals: boolean[] = [];

    mockSearchDocuments.mockImplementation((_query: string, _limit: number, signal: AbortSignal) => {
      callCount++;
      const myCall = callCount;
      return new Promise((resolve, reject) => {
        const checkAbort = () => {
          abortedSignals[myCall - 1] = signal.aborted;
          if (signal.aborted) {
            reject(new DOMException('Aborted', 'AbortError'));
          }
        };
        signal.addEventListener('abort', checkAbort);
        // Resolve after a short delay (simulated)
        setTimeout(() => {
          if (!signal.aborted) {
            resolve({ results: [{ doc_id: `result-${myCall}`, title: `Result ${myCall}`, folder: '', snippet: '', score: 1 }], total_hits: 1, query: _query });
          }
        }, 100);
      });
    });

    const { result, rerender } = renderHook(
      ({ query }) => useSearch(query, 300),
      { initialProps: { query: 'hello' } }
    );

    // Fire first debounce
    await act(async () => {
      vi.advanceTimersByTime(300);
    });

    // Change query before first request resolves
    rerender({ query: 'world' });

    // Fire second debounce
    await act(async () => {
      vi.advanceTimersByTime(300);
    });

    // Let the second request resolve
    await act(async () => {
      vi.advanceTimersByTime(100);
    });

    // The first call's signal should have been aborted
    expect(abortedSignals[0]).toBe(true);
    // Results should be from second call
    expect(result.current.results[0]?.doc_id).toBe('result-2');
  });

  it('handles 503 error gracefully with initialization message', async () => {
    mockSearchDocuments.mockRejectedValue(new Error('Search failed: 503'));

    const { result } = renderHook(() => useSearch('test', 300));

    await act(async () => {
      vi.advanceTimersByTime(300);
    });

    expect(result.current.error).toBe('Search is initializing...');
    expect(result.current.results).toEqual([]);
  });
});
