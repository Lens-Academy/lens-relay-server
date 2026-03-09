import { useState, useEffect, useRef, useMemo, type ReactNode } from 'react';
import { useNavigate } from 'react-router-dom';
import { useSuggestions, type FileSuggestions, type SuggestionItem } from '../../hooks/useSuggestions';

/** Lightweight inline markdown renderer for context text.
 *  Handles: newlines, headers (as bold), **bold**, *italic*, _italic_ */
function renderMarkdownInline(text: string): ReactNode {
  if (!text) return null;
  return text.split('\n').map((line, lineIdx, lines) => {
    // Strip leading # headers → render as bold
    const headerMatch = line.match(/^(#{1,6})\s+(.*)/);
    const content = headerMatch ? headerMatch[2] : line;
    const isHeader = !!headerMatch;

    // Process inline bold/italic
    const parts: ReactNode[] = [];
    const re = /(\*\*(.+?)\*\*|\*(.+?)\*|_(.+?)_)/g;
    let lastIndex = 0;
    let match: RegExpExecArray | null;
    let partKey = 0;
    while ((match = re.exec(content)) !== null) {
      if (match.index > lastIndex) {
        parts.push(content.slice(lastIndex, match.index));
      }
      if (match[2]) {
        parts.push(<strong key={partKey++}>{match[2]}</strong>);
      } else {
        parts.push(<em key={partKey++}>{match[3] || match[4]}</em>);
      }
      lastIndex = re.lastIndex;
    }
    if (lastIndex < content.length) {
      parts.push(content.slice(lastIndex));
    }

    const lineContent = isHeader ? <strong key={`h${lineIdx}`}>{parts}</strong> : parts;
    return (
      <span key={lineIdx}>
        {lineContent}
        {lineIdx < lines.length - 1 && <br />}
      </span>
    );
  });
}

interface FolderInfo {
  id: string;
  name: string;
}

interface ReviewPageProps {
  folderIds: string[];
  folders?: FolderInfo[];
  relayId?: string;
  onAction?: (docId: string, suggestion: SuggestionItem, action: 'accept' | 'reject') => Promise<void>;
  onAcceptAll?: () => Promise<void>;
  onRejectAll?: () => Promise<void>;
}

// --- Time slider utilities ---
// Cubic power curve: slider position 0..1000 maps to 0ms..30 days ago.
// Cubic gives fine control for recent times, coarser for distant past.
const SLIDER_MAX = 1000;
const MAX_AGO_MS = 30 * 86400_000; // 30 days
const SLIDER_POWER = 3;

function sliderToMs(pos: number): number {
  if (pos <= 0) return 0;
  if (pos >= SLIDER_MAX) return Infinity; // leftmost position = all time
  return Math.round(MAX_AGO_MS * Math.pow(pos / SLIDER_MAX, SLIDER_POWER));
}

function msToSlider(ms: number): number {
  if (ms <= 0) return 0;
  if (!isFinite(ms)) return SLIDER_MAX; // all time = leftmost
  return Math.round((SLIDER_MAX - 1) * Math.pow(Math.min(ms, MAX_AGO_MS) / MAX_AGO_MS, 1 / SLIDER_POWER));
}

function formatAgo(ms: number): string {
  if (!isFinite(ms)) return 'all time';
  if (ms <= 0) return 'now';
  const minutes = Math.round(ms / 60_000);
  if (minutes < 1) return 'now';
  if (minutes === 1) return '1 min ago';
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(ms / 3600_000);
  if (hours === 1) return '1 hour ago';
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(ms / 86400_000);
  if (days === 1) return '1 day ago';
  return `${days}d ago`;
}

function isFullRange(fromAgo: number, toAgo: number): boolean {
  return !isFinite(fromAgo) && toAgo <= 0;
}

interface TimeRange {
  mode: 'all' | 'range' | 'custom';
  // For 'range' mode: ms ago from now (0 = now)
  fromAgo: number;  // the older end (larger number)
  toAgo: number;    // the newer end (smaller number)
  // For 'custom' mode: ISO datetime-local strings
  customFrom: string;
  customTo: string;
}

const TIME_QUICK_PRESETS = [
  { label: 'All', fromAgo: Infinity, toAgo: 0, mode: 'all' as const },
  { label: '1h', fromAgo: 3600_000, toAgo: 0, mode: 'range' as const },
  { label: '24h', fromAgo: 86400_000, toAgo: 0, mode: 'range' as const },
  { label: '7d', fromAgo: 604800_000, toAgo: 0, mode: 'range' as const },
];

interface LocationEntry {
  key: string;        // folderId or folderId:/prefix
  label: string;      // display name
  isSubfolder: boolean;
  folderId: string;
}

function DualRangeSlider({ fromAgo, toAgo, onChange }: {
  fromAgo: number;
  toAgo: number;
  onChange: (fromAgo: number, toAgo: number) => void;
}) {
  // Invert: slider 0 = max ago (left = past), slider MAX = 0ms ago (right = now)
  const fromPos = SLIDER_MAX - msToSlider(fromAgo);
  const toPos = SLIDER_MAX - msToSlider(toAgo);

  const leftPct = (Math.min(fromPos, toPos) / SLIDER_MAX) * 100;
  const rightPct = 100 - (Math.max(fromPos, toPos) / SLIDER_MAX) * 100;

  const full = isFullRange(fromAgo, toAgo);
  const thumbBase = `absolute inset-x-0 w-full appearance-none bg-transparent pointer-events-none
    [&::-webkit-slider-thumb]:pointer-events-auto [&::-webkit-slider-thumb]:appearance-none
    [&::-webkit-slider-thumb]:w-3.5 [&::-webkit-slider-thumb]:h-3.5 [&::-webkit-slider-thumb]:rounded-full
    [&::-webkit-slider-thumb]:border-2 [&::-webkit-slider-thumb]:border-white
    [&::-webkit-slider-thumb]:shadow [&::-webkit-slider-thumb]:cursor-pointer
    [&::-moz-range-thumb]:pointer-events-auto [&::-moz-range-thumb]:appearance-none
    [&::-moz-range-thumb]:w-3.5 [&::-moz-range-thumb]:h-3.5 [&::-moz-range-thumb]:rounded-full
    [&::-moz-range-thumb]:border-2 [&::-moz-range-thumb]:border-white
    [&::-moz-range-thumb]:shadow [&::-moz-range-thumb]:cursor-pointer`;
  const thumbClass = full
    ? `${thumbBase} [&::-webkit-slider-thumb]:bg-gray-400 [&::-moz-range-thumb]:bg-gray-400`
    : `${thumbBase} [&::-webkit-slider-thumb]:bg-blue-500 [&::-moz-range-thumb]:bg-blue-500`;

  return (
    <div className="flex items-center gap-3 flex-1 min-w-0">
      <span className="text-gray-500 whitespace-nowrap shrink-0 w-16 text-right">{formatAgo(fromAgo)}</span>
      <div className="relative flex-1 h-6 flex items-center">
        <div className="absolute inset-x-0 h-1 bg-gray-200 rounded-full" />
        <div
          className={`absolute h-1 rounded-full ${full ? 'bg-gray-300' : 'bg-blue-400'}`}
          style={{ left: `${leftPct}%`, right: `${rightPct}%` }}
        />
        {/* From slider (older end = left side) */}
        <input
          type="range"
          min={0}
          max={SLIDER_MAX}
          value={fromPos}
          onChange={e => {
            const pos = Number(e.target.value);
            const newFrom = sliderToMs(SLIDER_MAX - pos);
            onChange(Math.max(newFrom, toAgo), toAgo);
          }}
          className={thumbClass}
          style={{ zIndex: fromPos >= toPos ? 1 : 2 }}
        />
        {/* To slider (newer end = right side) */}
        <input
          type="range"
          min={0}
          max={SLIDER_MAX}
          value={toPos}
          onChange={e => {
            const pos = Number(e.target.value);
            const newTo = sliderToMs(SLIDER_MAX - pos);
            onChange(fromAgo, Math.min(newTo, fromAgo));
          }}
          className={thumbClass}
          style={{ zIndex: toPos >= fromPos ? 1 : 2 }}
        />
      </div>
      <span className="text-gray-500 whitespace-nowrap shrink-0 w-16">{formatAgo(toAgo)}</span>
    </div>
  );
}

function FilterBar({ authors, locations, authorFilter, timeRange, locationFilter, onAuthorToggle, onAuthorClear, onTimeRange, onLocationToggle, onLocationClear, onClear }: {
  authors: string[];
  locations: LocationEntry[];
  authorFilter: Set<string>;
  timeRange: TimeRange;
  locationFilter: Set<string>;
  onAuthorToggle: (author: string) => void;
  onAuthorClear: () => void;
  onTimeRange: (range: TimeRange) => void;
  onLocationToggle: (key: string) => void;
  onLocationClear: () => void;
  onClear: () => void;
}) {
  const isActive = authorFilter.size > 0 || timeRange.mode !== 'all' || locationFilter.size > 0;

  return (
    <div className="flex flex-col gap-2 mb-4 text-xs">
      {locations.length >= 2 && (
        <div className="flex items-center gap-1.5 flex-wrap">
          <span className="text-gray-500 font-semibold uppercase tracking-wider mr-0.5">Location</span>
          <button
            onClick={onLocationClear}
            className={`px-2 py-0.5 rounded-full transition-colors ${
              locationFilter.size === 0
                ? 'bg-blue-100 text-blue-700'
                : 'bg-gray-100 text-gray-500 hover:bg-gray-200'
            }`}
          >
            All
          </button>
          {locations.map(loc => (
            <button
              key={loc.key}
              onClick={() => onLocationToggle(loc.key)}
              className={`px-2 py-0.5 rounded-full transition-colors ${
                locationFilter.has(loc.key)
                  ? 'bg-blue-100 text-blue-700'
                  : 'bg-gray-100 text-gray-500 hover:bg-gray-200'
              } ${loc.isSubfolder ? 'ml-1' : ''}`}
            >
              {loc.label}
            </button>
          ))}
        </div>
      )}
      {authors.length > 0 && (
        <div className="flex items-center gap-1.5">
          <span className="text-gray-500 font-semibold uppercase tracking-wider mr-0.5">Author</span>
          <button
            onClick={onAuthorClear}
            className={`px-2 py-0.5 rounded-full transition-colors ${
              authorFilter.size === 0
                ? 'bg-blue-100 text-blue-700'
                : 'bg-gray-100 text-gray-500 hover:bg-gray-200'
            }`}
          >
            All
          </button>
          {authors.map(a => (
            <button
              key={a}
              onClick={() => onAuthorToggle(a)}
              className={`px-2 py-0.5 rounded-full transition-colors ${
                authorFilter.has(a)
                  ? 'bg-blue-100 text-blue-700'
                  : 'bg-gray-100 text-gray-500 hover:bg-gray-200'
              }`}
            >
              {a}
            </button>
          ))}
        </div>
      )}
      <div className="flex flex-col gap-1.5">
        <div className="flex items-center gap-1.5 flex-wrap">
          <span className="text-gray-500 font-semibold uppercase tracking-wider mr-0.5">Time</span>
          {TIME_QUICK_PRESETS.map(p => (
            <button
              key={p.label}
              onClick={() => onTimeRange({
                mode: p.mode,
                fromAgo: p.fromAgo,
                toAgo: p.toAgo,
                customFrom: '',
                customTo: '',
              })}
              className={`px-2 py-0.5 rounded-full transition-colors ${
                timeRange.mode === p.mode && (p.mode === 'all' || (timeRange.fromAgo === p.fromAgo && timeRange.toAgo === p.toAgo))
                  ? 'bg-blue-100 text-blue-700'
                  : 'bg-gray-100 text-gray-500 hover:bg-gray-200'
              }`}
            >
              {p.label}
            </button>
          ))}
          <button
            onClick={() => onTimeRange({ ...timeRange, mode: 'custom' })}
            className={`px-2 py-0.5 rounded-full transition-colors ${
              timeRange.mode === 'custom'
                ? 'bg-blue-100 text-blue-700'
                : 'bg-gray-100 text-gray-500 hover:bg-gray-200'
            }`}
          >
            Exact
          </button>
          {isActive && (
            <button onClick={onClear} className="text-blue-600 hover:text-blue-800 ml-2">
              Clear All Filters
            </button>
          )}
        </div>
        {timeRange.mode !== 'custom' && (
          <div className="flex items-center gap-2">
            <DualRangeSlider
              fromAgo={timeRange.fromAgo}
              toAgo={timeRange.toAgo}
              onChange={(fromAgo, toAgo) => onTimeRange({ ...timeRange, mode: isFullRange(fromAgo, toAgo) ? 'all' : 'range', fromAgo, toAgo })}
            />
          </div>
        )}
        {timeRange.mode === 'custom' && (
          <div className="flex items-center gap-1">
            <input
              type="datetime-local"
              value={timeRange.customFrom}
              onChange={e => onTimeRange({ ...timeRange, customFrom: e.target.value })}
              className="px-1.5 py-0.5 border border-gray-300 rounded text-xs bg-white text-gray-700"
            />
            <span className="text-gray-400">&mdash;</span>
            <input
              type="datetime-local"
              value={timeRange.customTo}
              onChange={e => onTimeRange({ ...timeRange, customTo: e.target.value })}
              className="px-1.5 py-0.5 border border-gray-300 rounded text-xs bg-white text-gray-700"
            />
          </div>
        )}
      </div>
    </div>
  );
}

export function ReviewPage({ folderIds, folders, onAction, onAcceptAll, onRejectAll }: ReviewPageProps) {
  const { data, loading, error, refresh } = useSuggestions(folderIds);
  const [expandedFiles, setExpandedFiles] = useState<Set<string>>(new Set());
  const autoExpandedRef = useRef(false);
  const navigate = useNavigate();

  // Filter state
  const [authorFilter, setAuthorFilter] = useState<Set<string>>(new Set());
  const [timeRange, setTimeRange] = useState<TimeRange>({ mode: 'all', fromAgo: Infinity, toAgo: 0, customFrom: '', customTo: '' });
  const [locationFilter, setLocationFilter] = useState<Set<string>>(new Set());
  const [confirmAction, setConfirmAction] = useState<'accept' | 'reject' | null>(null);

  const toggleSet = (prev: Set<string>, value: string) => {
    const next = new Set(prev);
    if (next.has(value)) next.delete(value);
    else next.add(value);
    return next;
  };

  // Map folder_id -> folder name for display
  const folderNameMap = useMemo(() => {
    const map = new Map<string, string>();
    if (folders) {
      for (const f of folders) map.set(f.id, f.name);
    }
    return map;
  }, [folders]);

  // Derive unique authors from data
  const uniqueAuthors = useMemo(() => {
    const authors = new Set<string>();
    for (const file of data) {
      for (const s of file.suggestions) {
        if (s.author) authors.add(s.author);
      }
    }
    return Array.from(authors).sort();
  }, [data]);

  // Derive location entries from data + folders
  const locations = useMemo<LocationEntry[]>(() => {
    if (!folders || folders.length < 2) return [];
    const folderMap = new Map(folders.map(f => [f.id, f.name]));

    // Collect prefixes per folder
    const prefixesByFolder = new Map<string, Set<string>>();
    for (const file of data) {
      const folderId = file.folder_id;
      if (!folderMap.has(folderId)) continue;
      if (!prefixesByFolder.has(folderId)) prefixesByFolder.set(folderId, new Set());
      const lastSlash = file.path.lastIndexOf('/');
      if (lastSlash > 0) {
        prefixesByFolder.get(folderId)!.add(file.path.slice(0, lastSlash));
      }
    }

    const entries: LocationEntry[] = [];
    for (const folder of folders) {
      entries.push({
        key: folder.id,
        label: folder.name,
        isSubfolder: false,
        folderId: folder.id,
      });
      const prefixes = prefixesByFolder.get(folder.id);
      if (prefixes && prefixes.size > 0) {
        const sorted = Array.from(prefixes).sort();
        for (const prefix of sorted) {
          entries.push({
            key: `${folder.id}:${prefix}`,
            label: `${folder.name} / ${prefix.replace(/^\//, '')}`,
            isSubfolder: true,
            folderId: folder.id,
          });
        }
      }
    }
    return entries;
  }, [data, folders]);

  // Filtering pipeline
  const filteredData = useMemo(() => {
    const now = Date.now();
    const hasTimeFilter = timeRange.mode !== 'all';

    let getTimeBounds: () => [number, number];
    if (timeRange.mode === 'custom') {
      getTimeBounds = () => [
        timeRange.customFrom ? new Date(timeRange.customFrom).getTime() : 0,
        timeRange.customTo ? new Date(timeRange.customTo).getTime() : Infinity,
      ];
    } else if (timeRange.mode === 'range') {
      getTimeBounds = () => [now - timeRange.fromAgo, now - timeRange.toAgo];
    } else {
      getTimeBounds = () => [0, Infinity];
    }

    return data
      .filter(file => {
        if (locationFilter.size === 0) return true;
        // Check if any selected location matches this file
        if (locationFilter.has(file.folder_id)) return true;
        // Check subfolder matches
        for (const key of locationFilter) {
          const colonIdx = key.indexOf(':');
          if (colonIdx === -1) continue;
          const folderId = key.slice(0, colonIdx);
          const prefix = key.slice(colonIdx + 1);
          if (file.folder_id === folderId && file.path.startsWith(prefix + '/')) return true;
          // Also match files directly in the prefix directory
          if (file.folder_id === folderId) {
            const lastSlash = file.path.lastIndexOf('/');
            const fileDir = lastSlash > 0 ? file.path.slice(0, lastSlash) : '';
            if (fileDir === prefix) return true;
          }
        }
        return false;
      })
      .map(file => {
        if (authorFilter.size === 0 && !hasTimeFilter) return file;
        const [fromTime, toTime] = getTimeBounds();
        const filtered = file.suggestions.filter(s => {
          if (authorFilter.size > 0 && (!s.author || !authorFilter.has(s.author))) return false;
          if (hasTimeFilter && (!s.timestamp || s.timestamp < fromTime || s.timestamp > toTime)) return false;
          return true;
        });
        return { ...file, suggestions: filtered };
      })
      .filter(file => file.suggestions.length > 0);
  }, [data, authorFilter, timeRange, locationFilter]);

  // Auto-expand the first file on initial load
  useEffect(() => {
    if (!autoExpandedRef.current && filteredData.length > 0) {
      autoExpandedRef.current = true;
      setExpandedFiles(new Set([filteredData[0].doc_id]));
    }
  }, [filteredData]);

  const isFiltered = authorFilter.size > 0 || timeRange.mode !== 'all' || locationFilter.size > 0;
  const totalFiltered = filteredData.reduce((sum, f) => sum + f.suggestions.length, 0);
  const totalAll = data.reduce((sum, f) => sum + f.suggestions.length, 0);

  const clearFilters = () => {
    setAuthorFilter(new Set());
    setTimeRange({ mode: 'all', fromAgo: Infinity, toAgo: 0, customFrom: '', customTo: '' });
    setLocationFilter(new Set());
  };

  const toggleFile = (docId: string) => {
    setExpandedFiles(prev => {
      const next = new Set(prev);
      if (next.has(docId)) next.delete(docId);
      else next.add(docId);
      return next;
    });
  };

  const expandAll = () => {
    setExpandedFiles(new Set(filteredData.map(f => f.doc_id)));
  };

  const collapseAll = () => {
    setExpandedFiles(new Set());
  };

  const navigateToSuggestion = (docId: string, from: number, e?: React.MouseEvent) => {
    const uuid = docId.slice(-36);
    const shortUuid = uuid.slice(0, 8);
    const path = `/${shortUuid}?pos=${from}`;
    if (e && (e.ctrlKey || e.metaKey)) {
      window.open(`${window.location.origin}${path}`, '_blank');
    } else {
      navigate(path);
    }
  };

  // Global accept/reject operate on filtered data
  const handleAcceptAllFiltered = onAction ? async () => {
    for (const file of filteredData) {
      for (const s of [...file.suggestions].sort((a, b) => b.from - a.from)) {
        try { await onAction(file.doc_id, s, 'accept'); } catch { /* skip not-found */ }
      }
    }
    refresh();
  } : onAcceptAll;

  const handleRejectAllFiltered = onAction ? async () => {
    for (const file of filteredData) {
      for (const s of [...file.suggestions].sort((a, b) => b.from - a.from)) {
        try { await onAction(file.doc_id, s, 'reject'); } catch { /* skip not-found */ }
      }
    }
    refresh();
  } : onRejectAll;

  if (loading) {
    return <div className="p-8 text-gray-500">Scanning documents for suggestions...</div>;
  }

  if (error) {
    return <div className="p-8 text-red-600">Error: {error}</div>;
  }

  if (data.length === 0) {
    return (
      <div className="p-8 text-center text-gray-500">
        <p className="text-lg">No pending suggestions</p>
        <p className="text-sm mt-2">All documents are clean.</p>
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto">
      <div className="max-w-4xl mx-auto p-6">
        <div className="flex items-center justify-between mb-6">
          <div>
            <h1 className="text-xl font-semibold text-gray-900">Review Suggestions</h1>
            <p className="text-sm text-gray-500 mt-1">
              {isFiltered
                ? `${totalFiltered} of ${totalAll} suggestion${totalAll !== 1 ? 's' : ''} across ${filteredData.length} of ${data.length} file${data.length !== 1 ? 's' : ''}`
                : `${totalAll} suggestion${totalAll !== 1 ? 's' : ''} across ${data.length} file${data.length !== 1 ? 's' : ''}`
              }
            </p>
          </div>
          <div className="flex items-center gap-2">
            {handleAcceptAllFiltered && (
              <button onClick={() => setConfirmAction('accept')} className="px-3 py-1.5 text-sm bg-green-600 text-white rounded hover:bg-green-700">
                {isFiltered ? 'Accept Filtered' : 'Accept All'}
              </button>
            )}
            {handleRejectAllFiltered && (
              <button onClick={() => setConfirmAction('reject')} className="px-3 py-1.5 text-sm bg-red-600 text-white rounded hover:bg-red-700">
                {isFiltered ? 'Reject Filtered' : 'Reject All'}
              </button>
            )}
            <button onClick={refresh} className="px-3 py-1.5 text-sm border border-gray-300 rounded hover:bg-gray-50">
              Refresh
            </button>
          </div>
        </div>

        <FilterBar
          authors={uniqueAuthors}
          locations={locations}
          authorFilter={authorFilter}
          timeRange={timeRange}
          locationFilter={locationFilter}
          onAuthorToggle={a => setAuthorFilter(prev => toggleSet(prev, a))}
          onAuthorClear={() => setAuthorFilter(new Set())}
          onTimeRange={setTimeRange}
          onLocationToggle={key => setLocationFilter(prev => toggleSet(prev, key))}
          onLocationClear={() => setLocationFilter(new Set())}
          onClear={clearFilters}
        />

        {filteredData.length === 0 ? (
          <div className="p-8 text-center text-gray-500">
            <p className="text-sm">No suggestions match the current filters.</p>
            <button onClick={clearFilters} className="text-sm text-blue-600 hover:text-blue-800 mt-2">Clear All Filters</button>
          </div>
        ) : (
          <>
            <div className="flex items-center gap-2 mb-3">
              <button onClick={expandAll} className="px-2.5 py-1 text-xs text-gray-500 border border-gray-200 rounded hover:bg-gray-50">
                Expand All
              </button>
              <button onClick={collapseAll} className="px-2.5 py-1 text-xs text-gray-500 border border-gray-200 rounded hover:bg-gray-50">
                Collapse All
              </button>
            </div>
            <div className="space-y-2">
              {filteredData.map(file => (
                <FileSection
                  key={file.doc_id}
                  file={file}
                  folderName={folderNameMap.get(file.folder_id)}
                  expanded={expandedFiles.has(file.doc_id)}
                  onToggle={() => toggleFile(file.doc_id)}
                  onAction={onAction}
                  onNavigate={navigateToSuggestion}
                />
              ))}
            </div>
          </>
        )}

        {confirmAction && (
          <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={() => setConfirmAction(null)}>
            <div className="bg-white rounded-lg shadow-xl p-6 max-w-sm mx-4" onClick={e => e.stopPropagation()}>
              <h2 className="text-lg font-semibold text-gray-900 mb-2">
                {confirmAction === 'accept' ? 'Accept' : 'Reject'} {isFiltered ? 'filtered' : 'all'} suggestions?
              </h2>
              <p className="text-sm text-gray-500 mb-4">
                This will {confirmAction === 'accept' ? 'accept' : 'reject'}{' '}
                <strong>{totalFiltered} suggestion{totalFiltered !== 1 ? 's' : ''}</strong> across{' '}
                <strong>{filteredData.length} file{filteredData.length !== 1 ? 's' : ''}</strong>.
                This action cannot be undone.
              </p>
              <div className="flex justify-end gap-2">
                <button
                  onClick={() => setConfirmAction(null)}
                  className="px-3 py-1.5 text-sm border border-gray-300 rounded hover:bg-gray-50"
                >
                  Cancel
                </button>
                <button
                  onClick={async () => {
                    const handler = confirmAction === 'accept' ? handleAcceptAllFiltered : handleRejectAllFiltered;
                    setConfirmAction(null);
                    if (handler) await handler();
                  }}
                  className={`px-3 py-1.5 text-sm text-white rounded ${
                    confirmAction === 'accept'
                      ? 'bg-green-600 hover:bg-green-700'
                      : 'bg-red-600 hover:bg-red-700'
                  }`}
                >
                  {confirmAction === 'accept' ? 'Accept' : 'Reject'} {totalFiltered} suggestion{totalFiltered !== 1 ? 's' : ''}
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function FileSection({ file, folderName, expanded, onToggle, onAction, onNavigate }: {
  file: FileSuggestions;
  folderName?: string;
  expanded: boolean;
  onToggle: () => void;
  onAction?: (docId: string, suggestion: SuggestionItem, action: 'accept' | 'reject') => Promise<void>;
  onNavigate: (docId: string, from: number, e?: React.MouseEvent) => void;
}) {
  type ResolvedStatus = 'accepted' | 'rejected' | 'not-found';
  const [resolvedMap, setResolvedMap] = useState<Record<number, ResolvedStatus>>({});

  const setResolved = (index: number, status: ResolvedStatus) => {
    setResolvedMap(prev => ({ ...prev, [index]: status }));
  };

  const handleAcceptAll = async () => {
    if (!onAction) return;
    for (let i = 0; i < file.suggestions.length; i++) {
      if (resolvedMap[i]) continue;
      try {
        await onAction(file.doc_id, file.suggestions[i], 'accept');
        setResolved(i, 'accepted');
      } catch {
        setResolved(i, 'not-found');
      }
    }
  };

  const handleRejectAll = async () => {
    if (!onAction) return;
    for (let i = 0; i < file.suggestions.length; i++) {
      if (resolvedMap[i]) continue;
      try {
        await onAction(file.doc_id, file.suggestions[i], 'reject');
        setResolved(i, 'rejected');
      } catch {
        setResolved(i, 'not-found');
      }
    }
  };

  return (
    <div className="border border-gray-200 rounded-lg overflow-hidden shadow-sm">
      <div className="flex items-center justify-between px-4 py-3 bg-gray-50 hover:bg-gray-100 transition-colors">
        <button onClick={onToggle} className="flex items-center gap-3 flex-1">
          <svg className={`w-3.5 h-3.5 text-gray-400 transition-transform duration-200 ${expanded ? 'rotate-90' : ''}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}><path strokeLinecap="round" strokeLinejoin="round" d="M9 5l7 7-7 7" /></svg>
          <span className="font-medium">
            {(() => {
              const fullPath = folderName ? `${folderName}${file.path}` : file.path;
              const segments = fullPath.split('/').filter(Boolean);
              const filename = (segments.pop() || '').replace(/\.md$/i, '');
              const parentPath = segments.join('/');
              return (
                <>
                  {parentPath && <span className="text-gray-400 font-normal">{parentPath}/</span>}
                  <span className="text-gray-800">{filename}</span>
                </>
              );
            })()}
          </span>
          <span className="text-xs text-gray-500 bg-gray-200 px-2 py-0.5 rounded-full">
            {file.suggestions.length} suggestion{file.suggestions.length !== 1 ? 's' : ''}
          </span>
        </button>
        {expanded && (
          <div className="flex gap-1 ml-2">
            {onAction && (
              <button onClick={handleAcceptAll} title="Accept all in file" className="px-2 py-1 text-xs text-green-700 hover:bg-green-50 rounded">
                Accept All
              </button>
            )}
            {onAction && (
              <button onClick={handleRejectAll} title="Reject all in file" className="px-2 py-1 text-xs text-red-700 hover:bg-red-50 rounded">
                Reject All
              </button>
            )}
          </div>
        )}
      </div>
      <div className={`grid transition-[grid-template-rows] duration-200 ease-in-out ${expanded ? 'grid-rows-[1fr]' : 'grid-rows-[0fr]'}`}>
        <div className="overflow-hidden">
          <div className="divide-y divide-gray-200">
            {file.suggestions.map((s, i) => (
              <SuggestionRow
                key={i}
                suggestion={s}
                resolved={resolvedMap[i] ?? null}
                onAccept={onAction ? async () => { try { await onAction(file.doc_id, s, 'accept'); setResolved(i, 'accepted'); } catch { setResolved(i, 'not-found'); } } : undefined}
                onReject={onAction ? async () => { try { await onAction(file.doc_id, s, 'reject'); setResolved(i, 'rejected'); } catch { setResolved(i, 'not-found'); } } : undefined}
                onNavigate={(e) => onNavigate(file.doc_id, s.from, e)}
              />
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

function SuggestionRow({ suggestion, resolved, onAccept, onReject, onNavigate }: {
  suggestion: SuggestionItem;
  resolved: 'accepted' | 'rejected' | 'not-found' | null;
  onAccept?: () => void;
  onReject?: () => void;
  onNavigate: (e: React.MouseEvent) => void;
}) {
  return (
    <div className={`px-4 py-3 transition-colors duration-300 ${resolved ? 'bg-gray-50' : ''}`}>
      <div className="flex items-center gap-2 mb-2">
        <div className="flex items-center gap-2 flex-1 min-w-0">
          {resolved === 'not-found' ? (
            <span className="text-xs font-medium px-2 py-0.5 rounded text-amber-700 bg-amber-100">
              No longer found (resolved or changed)
            </span>
          ) : resolved ? (
            <span className={`text-xs font-medium px-2 py-0.5 rounded ${
              resolved === 'accepted' ? 'text-green-700 bg-green-100' : 'text-red-700 bg-red-100'
            }`}>
              {suggestion.type === 'deletion' ? 'Deletion' : 'Suggestion'} {resolved}
            </span>
          ) : null}
          {suggestion.line > 0 && (
            <span className={`text-xs font-mono px-1.5 py-0.5 rounded ${resolved ? 'text-gray-400 bg-gray-100' : 'text-gray-500 bg-gray-100'}`}>L{suggestion.line}</span>
          )}
          {suggestion.author && (
            <span className={`text-xs px-1.5 py-0.5 rounded ${resolved ? 'text-gray-400 bg-gray-100' : 'text-gray-500 bg-gray-100'}`}>{suggestion.author}</span>
          )}
          {suggestion.timestamp && (
            <span className={`text-xs ${resolved ? 'text-gray-300' : 'text-gray-400'}`}>{new Date(suggestion.timestamp).toLocaleString()}</span>
          )}
        </div>
        <div className="flex gap-1 shrink-0">
          {!resolved && onAccept && (
            <button onClick={onAccept} title="Accept" className="px-2 py-1 text-xs font-medium text-green-700 bg-green-50 hover:bg-green-100 rounded border border-green-200">
              Accept
            </button>
          )}
          {!resolved && onReject && (
            <button onClick={onReject} title="Reject" className="px-2 py-1 text-xs font-medium text-red-700 bg-red-50 hover:bg-red-100 rounded border border-red-200">
              Reject
            </button>
          )}
          <button onClick={onNavigate} title="Open in editor" className="px-2 py-1 text-xs text-gray-500 hover:bg-gray-100 rounded border border-gray-200">
            Open
          </button>
        </div>
      </div>
      <button onClick={onNavigate} className={`w-full text-left hover:bg-gray-50 rounded p-2 -m-1 transition-colors ${resolved ? 'opacity-50' : ''}`} title="Open in editor">
        <div className="text-sm leading-relaxed">
          <span className="text-gray-500">{renderMarkdownInline(suggestion.context_before)}</span>
          {suggestion.type === 'substitution' ? (
            <>
              <span className="bg-red-100 text-red-800 line-through decoration-red-400">{renderMarkdownInline(suggestion.old_content ?? '')}</span>
              <span className="bg-green-100 text-green-800">{renderMarkdownInline(suggestion.new_content ?? '')}</span>
            </>
          ) : suggestion.type === 'deletion' ? (
            <span className="bg-red-100 text-red-800 line-through decoration-red-400">{renderMarkdownInline(suggestion.content)}</span>
          ) : (
            <span className="bg-green-100 text-green-800">{renderMarkdownInline(suggestion.content)}</span>
          )}
          <span className="text-gray-500">{renderMarkdownInline(suggestion.context_after)}</span>
        </div>
      </button>
    </div>
  );
}
