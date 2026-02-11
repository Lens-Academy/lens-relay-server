import { RELAY_ID } from '../../App';
import type { SearchResult } from '../../lib/relay-api';

interface SearchPanelProps {
  results: SearchResult[];
  loading: boolean;
  error: string | null;
  query: string;
  onNavigate: (docId: string) => void;
}

export function SearchPanel({ results, loading, error, query, onNavigate }: SearchPanelProps) {
  if (loading) {
    return <div className="p-4 text-sm text-gray-500">Searching...</div>;
  }
  if (error) {
    return <div className="p-4 text-sm text-red-500">{error}</div>;
  }
  if (query && results.length === 0) {
    return <div className="p-4 text-sm text-gray-500">No results found</div>;
  }

  return (
    <ul className="divide-y divide-gray-100">
      {results.map((result) => (
        <li key={result.doc_id}>
          <button
            onClick={() => onNavigate(`${RELAY_ID}-${result.doc_id}`)}
            className="w-full text-left px-3 py-2 hover:bg-gray-50 transition-colors"
          >
            <div className="text-sm font-medium text-gray-900 truncate">
              {result.title}
            </div>
            {result.folder && (
              <span className="text-xs text-gray-400">{result.folder}</span>
            )}
            <div
              className="text-xs text-gray-600 mt-0.5 line-clamp-2 [&_mark]:bg-yellow-200 [&_mark]:rounded-sm"
              dangerouslySetInnerHTML={{ __html: result.snippet }}
            />
          </button>
        </li>
      ))}
    </ul>
  );
}
