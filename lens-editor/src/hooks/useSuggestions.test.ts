/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, waitFor, act } from '@testing-library/react';
import { useSuggestions } from './useSuggestions';

// Mock global fetch
const mockFetch = vi.fn();
vi.stubGlobal('fetch', mockFetch);

describe('useSuggestions', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('starts in loading state', () => {
    mockFetch.mockReturnValue(new Promise(() => {})); // never resolves
    const { result } = renderHook(() => useSuggestions(['folder-1']));
    expect(result.current.loading).toBe(true);
    expect(result.current.data).toEqual([]);
    expect(result.current.error).toBeNull();
  });

  it('fetches suggestions for a single folder', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        files: [{ path: 'Notes/Test.md', doc_id: 'doc-1', suggestions: [{ type: 'addition', content: 'hello' }] }],
      }),
    } as Response);

    const { result } = renderHook(() => useSuggestions(['folder-1']));
    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.data).toHaveLength(1);
    expect(result.current.data[0].path).toBe('Notes/Test.md');
    expect(mockFetch).toHaveBeenCalledWith('/api/relay/suggestions?folder_id=folder-1');
  });

  it('aggregates suggestions across multiple folders', async () => {
    mockFetch
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({
          files: [{ path: 'A.md', doc_id: 'doc-a', suggestions: [] }],
        }),
      } as Response)
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({
          files: [{ path: 'B.md', doc_id: 'doc-b', suggestions: [] }],
        }),
      } as Response);

    const { result } = renderHook(() => useSuggestions(['folder-1', 'folder-2']));
    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.data).toHaveLength(2);
    expect(mockFetch).toHaveBeenCalledTimes(2);
  });

  it('sets error when fetch fails', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      json: async () => ({}),
    } as Response);

    const { result } = renderHook(() => useSuggestions(['folder-1']));
    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.error).toBeTruthy();
    expect(result.current.data).toEqual([]);
  });

  it('refresh re-fetches data', async () => {
    mockFetch.mockResolvedValue({
      ok: true,
      json: async () => ({ files: [] }),
    } as Response);

    const { result } = renderHook(() => useSuggestions(['folder-1']));
    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(mockFetch).toHaveBeenCalledTimes(1);
    await act(() => result.current.refresh());
    expect(mockFetch).toHaveBeenCalledTimes(2);
  });
});
