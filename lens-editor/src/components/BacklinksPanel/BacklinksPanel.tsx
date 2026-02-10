import { useMemo, useState, useEffect } from 'react';
import { useNavigation } from '../../contexts/NavigationContext';
import { findPathByUuid } from '../../lib/uuid-to-path';

interface BacklinksPanelProps {
  currentDocId: string;
}

/**
 * Panel displaying documents that link to the current document.
 * Observes backlinks_v0 Y.Map for live updates across all folder docs.
 */
export function BacklinksPanel({ currentDocId }: BacklinksPanelProps) {
  const { metadata, folderDocs, onNavigate } = useNavigation();

  // Force re-render when backlinks Y.Map changes
  const [backlinksVersion, setBacklinksVersion] = useState(0);

  // Subscribe to backlinks_v0 changes on all folder docs
  useEffect(() => {
    if (folderDocs.size === 0) return;

    const cleanups: (() => void)[] = [];
    const observer = () => setBacklinksVersion(v => v + 1);

    for (const doc of folderDocs.values()) {
      const backlinksMap = doc.getMap<string[]>('backlinks_v0');
      backlinksMap.observe(observer);
      cleanups.push(() => backlinksMap.unobserve(observer));
    }

    return () => cleanups.forEach(fn => fn());
  }, [folderDocs]);

  // Get backlinks from all folder docs' backlinks_v0 Y.Maps
  const backlinks = useMemo(() => {
    // Trigger re-compute when backlinksVersion changes
    void backlinksVersion;

    if (folderDocs.size === 0) return [];

    // Extract bare doc UUID and relay prefix from compound ID (relay_uuid-doc_uuid)
    const isCompound = currentDocId.length > 36 && currentDocId[36] === '-';
    const docUuid = isCompound ? currentDocId.slice(37) : currentDocId;
    const relayPrefix = isCompound ? currentDocId.slice(0, 37) : '';

    const allSourceUuids: string[] = [];
    for (const doc of folderDocs.values()) {
      const backlinksMap = doc.getMap<string[]>('backlinks_v0');
      const sourceUuids = backlinksMap.get(docUuid) || [];
      allSourceUuids.push(...sourceUuids);
    }

    // Resolve UUIDs to paths, filtering out missing docs
    return allSourceUuids
      .map((uuid: string) => {
        const path = findPathByUuid(uuid, metadata);
        if (!path) return null;

        // Extract filename without extension for display
        const filename = path.split('/').pop() || path;
        const displayName = filename.replace(/\.md$/i, '');

        // Build compound doc ID for navigation (relay_uuid-doc_uuid)
        const navId = relayPrefix + uuid;

        return { navId, path, displayName };
      })
      .filter((item): item is NonNullable<typeof item> => item !== null);
  }, [folderDocs, currentDocId, metadata, backlinksVersion]);

  // Loading state when no folder docs yet
  if (folderDocs.size === 0) {
    return (
      <div className="p-3 text-sm text-gray-400">
        Loading...
      </div>
    );
  }

  if (backlinks.length === 0) {
    return (
      <div className="p-3 text-sm text-gray-500">
        No backlinks
      </div>
    );
  }

  return (
    <div className="p-3">
      <h3 className="text-xs font-semibold text-gray-500 uppercase tracking-wider mb-2">
        Backlinks
      </h3>
      <ul className="space-y-1">
        {backlinks.map(({ navId, displayName }) => (
          <li key={navId}>
            <button
              onClick={() => onNavigate(navId)}
              className="w-full text-left px-2 py-1 text-sm text-gray-700 hover:bg-gray-100 rounded transition-colors cursor-pointer"
            >
              {displayName}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
