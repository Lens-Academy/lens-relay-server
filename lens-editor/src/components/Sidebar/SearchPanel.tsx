import { RELAY_ID } from '../../App';
import type { SearchResult } from '../../lib/relay-api';

interface SearchPanelProps {
  results: SearchResult[];
  fileNameMatches: SearchResult[];
  loading: boolean;
  error: string | null;
  query: string;
  onNavigate: (docId: string) => void;
}

export function SearchPanel({ results, fileNameMatches, loading, error, query, onNavigate }: SearchPanelProps) {
  // Filter out filename matches that already appear in server results
  const serverDocIds = new Set(results.map((r) => r.doc_id));
  const uniqueFileMatches = fileNameMatches.filter((m) => !serverDocIds.has(m.doc_id));

  const hasFileMatches = uniqueFileMatches.length > 0;
  const hasContentResults = results.length > 0;
  const isEmpty = !hasFileMatches && !hasContentResults && !loading;

  if (error) {
    return <div className="p-4 text-sm text-red-500">{error}</div>;
  }

  return (
    <div>
      {/* Files section — instant filename matches */}
      {hasFileMatches && (
        <>
          <div className="px-3 pt-3 pb-1 text-xs font-semibold text-gray-400 uppercase tracking-wider">
            Files
          </div>
          <ul className="divide-y divide-gray-100">
            {uniqueFileMatches.map((match) => (
              <li key={match.doc_id}>
                <button
                  onClick={() => onNavigate(`${RELAY_ID}-${match.doc_id}`)}
                  className="w-full text-left px-3 py-2 hover:bg-gray-50 transition-colors"
                >
                  <div className="text-sm font-medium text-gray-900 truncate">
                    {match.title}
                  </div>
                  {match.folder && (
                    <span className="text-xs text-gray-400">{match.folder}</span>
                  )}
                </button>
              </li>
            ))}
          </ul>
        </>
      )}

      {/* Content section — server search results */}
      {(hasContentResults || loading) && (
        <>
          {(hasFileMatches || hasContentResults) && (
            <div className="px-3 pt-3 pb-1 text-xs font-semibold text-gray-400 uppercase tracking-wider">
              Content
            </div>
          )}
          {loading && !hasContentResults ? (
            <div className="p-4 text-sm text-gray-500">Searching...</div>
          ) : (
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
                    {result.snippet && (
                      <div
                        className="text-xs text-gray-600 mt-0.5 line-clamp-3 [&_mark]:bg-yellow-200 [&_mark]:rounded-sm"
                        dangerouslySetInnerHTML={{ __html: result.snippet }}
                      />
                    )}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </>
      )}

      {/* Empty state */}
      {isEmpty && query && (
        <div className="p-4 text-sm text-gray-500">No results found</div>
      )}
    </div>
  );
}
