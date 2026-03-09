import { useState, useEffect, useCallback } from 'react';

export interface SuggestionItem {
  type: 'addition' | 'deletion' | 'substitution';
  content: string;
  old_content: string | null;
  new_content: string | null;
  author: string | null;
  timestamp: number | null;
  from: number;
  to: number;
  raw_markup: string;
  context_before: string;
  context_after: string;
}

export interface FileSuggestions {
  path: string;
  doc_id: string;
  suggestions: SuggestionItem[];
}

export interface SuggestionsResponse {
  files: FileSuggestions[];
}

export function useSuggestions(folderIds: string[]) {
  const [data, setData] = useState<FileSuggestions[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const allFiles: FileSuggestions[] = [];
      for (const folderId of folderIds) {
        const res = await fetch(`/api/relay/suggestions?folder_id=${encodeURIComponent(folderId)}`);
        if (!res.ok) throw new Error(`Failed to fetch suggestions for ${folderId}`);
        const json: SuggestionsResponse = await res.json();
        allFiles.push(...json.files);
      }
      setData(allFiles);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Unknown error');
      setData([]);
    } finally {
      setLoading(false);
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [folderIds.join(',')]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return { data, loading, error, refresh };
}
