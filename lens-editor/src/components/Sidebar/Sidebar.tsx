import { useState, useEffect, useRef, useDeferredValue, useMemo, useCallback } from 'react';
import { SearchInput } from './SearchInput';
import { SearchPanel } from './SearchPanel';
import { FileTree } from './FileTree';
import { FileTreeProvider } from './FileTreeContext';
import { ConfirmDialog } from '../ConfirmDialog';
import { useNavigation } from '../../contexts/NavigationContext';
import { useSearch } from '../../hooks/useSearch';
import { buildTreeFromPaths, filterTree } from '../../lib/tree-utils';
import { createDocument, renameDocument, deleteDocument } from '../../lib/relay-api';
import { getFolderDocForPath, getOriginalPath, getFolderNameFromPath } from '../../lib/multi-folder-utils';
import { RELAY_ID } from '../../App';

interface SidebarProps {
  activeDocId: string;
  onSelectDocument: (docId: string) => void;
}

export function Sidebar({ activeDocId, onSelectDocument }: SidebarProps) {
  const [searchTerm, setSearchTerm] = useState('');
  const deferredSearch = useDeferredValue(searchTerm);

  // State for inline editing
  const [editingPath, setEditingPath] = useState<string | null>(null);

  // State for delete confirmation
  const [deleteTarget, setDeleteTarget] = useState<{ path: string; name: string } | null>(null);

  // State for creating new document
  const [isCreating, setIsCreating] = useState(false);
  const [newDocName, setNewDocName] = useState('');

  // Ref for Ctrl+K keyboard shortcut focus
  const searchInputRef = useRef<HTMLInputElement>(null);

  // Get metadata from NavigationContext (lifted to App level)
  const { metadata, folderDocs, folderNames, onNavigate } = useNavigation();

  // Server-side full-text search (activates when searchTerm >= 2 chars)
  const { results: searchResults, loading: searchLoading, error: searchError } = useSearch(searchTerm);

  // Ctrl+K / Cmd+K keyboard shortcut to focus search input
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault();
        searchInputRef.current?.focus();
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, []);

  // Build tree from metadata
  const treeData = useMemo(() => {
    return buildTreeFromPaths(metadata);
  }, [metadata]);

  // Filter tree based on search (used when searchTerm < 2 chars)
  const filteredTree = useMemo(() => {
    if (!deferredSearch) return treeData;
    return filterTree(treeData, deferredSearch);
  }, [treeData, deferredSearch]);

  // Visual feedback while filtering is processing
  const isStale = searchTerm !== deferredSearch;

  // Whether to show server-side search results vs file tree
  const showSearchResults = searchTerm.trim().length >= 2;

  // Build compound doc ID and call parent handler
  const handleSelect = useCallback((docId: string) => {
    const compoundDocId = `${RELAY_ID}-${docId}`;
    onSelectDocument(compoundDocId);
  }, [onSelectDocument]);

  // CRUD handlers
  const handleRenameSubmit = useCallback((prefixedOldPath: string, newName: string) => {
    const doc = getFolderDocForPath(prefixedOldPath, folderDocs, folderNames);
    if (!doc) return;
    // Strip folder prefix to get the original Y.Doc path
    const folderName = getFolderNameFromPath(prefixedOldPath, folderNames)!;
    const oldPath = getOriginalPath(prefixedOldPath, folderName);
    // Build new path by replacing the filename
    const parts = oldPath.split('/');
    // Preserve .md extension if user didn't include it
    const filename = newName.endsWith('.md') ? newName : `${newName}.md`;
    parts[parts.length - 1] = filename;
    const newPath = parts.join('/');
    renameDocument(doc, oldPath, newPath);
  }, [folderDocs, folderNames]);

  const handleDeleteConfirm = useCallback(() => {
    if (!deleteTarget) return;
    const doc = getFolderDocForPath(deleteTarget.path, folderDocs, folderNames);
    if (!doc) return;
    const folderName = getFolderNameFromPath(deleteTarget.path, folderNames)!;
    const originalPath = getOriginalPath(deleteTarget.path, folderName);
    deleteDocument(doc, originalPath);
    setDeleteTarget(null);
  }, [folderDocs, folderNames, deleteTarget]);

  const handleCreateDocument = useCallback(async () => {
    if (!newDocName.trim()) return;
    // Use first folder by default for new documents
    const targetFolder = folderNames[0];
    if (!targetFolder) return;
    const doc = folderDocs.get(targetFolder);
    if (!doc) return;
    const name = newDocName.trim();
    // Add .md extension if not present, and ensure path starts with /
    const filename = name.endsWith('.md') ? name : `${name}.md`;
    const path = `/${filename}`;

    try {
      // createDocument is now async - it creates on server first, then adds to filemeta
      await createDocument(doc, path, 'markdown');
      setNewDocName('');
      setIsCreating(false);
    } catch (error) {
      console.error('Failed to create document:', error);
    }
  }, [folderDocs, folderNames, newDocName]);

  const handleNewDocKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      handleCreateDocument();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      setIsCreating(false);
      setNewDocName('');
    }
  };

  return (
    <aside className="w-64 flex-shrink-0 bg-white border-r border-gray-200 flex flex-col h-full">
      {/* Header with search */}
      <div className="p-3 border-b border-gray-200 space-y-2">
        {/* New Document button/input */}
        {isCreating ? (
          <input
            type="text"
            value={newDocName}
            onChange={(e) => setNewDocName(e.target.value)}
            onKeyDown={handleNewDocKeyDown}
            onBlur={() => {
              if (!newDocName.trim()) {
                setIsCreating(false);
              }
            }}
            placeholder="New document name..."
            className="w-full px-3 py-1.5 text-sm border border-blue-400 rounded-md outline-none"
            autoFocus
          />
        ) : (
          <button
            onClick={() => setIsCreating(true)}
            disabled={folderDocs.size === 0}
            className="w-full px-3 py-1.5 text-sm font-medium text-gray-700 bg-gray-100
                       hover:bg-gray-200 rounded-md disabled:opacity-60 disabled:cursor-not-allowed"
          >
            + New Document
          </button>
        )}

        <SearchInput
          ref={searchInputRef}
          value={searchTerm}
          onChange={setSearchTerm}
          placeholder="Search..."
        />
      </div>

      {/* Tree content or search results */}
      <div className={`flex-1 overflow-y-auto ${isStale && !showSearchResults ? 'opacity-80' : ''}`}>
        {showSearchResults ? (
          <SearchPanel
            results={searchResults}
            loading={searchLoading}
            error={searchError}
            query={searchTerm}
            onNavigate={onNavigate}
          />
        ) : (
          <>
            {/* Loading state: no doc yet or empty metadata */}
            {folderDocs.size === 0 && Object.keys(metadata).length === 0 && (
              <div className="p-4 text-sm text-gray-500">
                Loading documents...
              </div>
            )}

            {filteredTree.length === 0 && folderDocs.size > 0 && (
              <div className="p-4 text-sm text-gray-500 text-center">
                {searchTerm ? (
                  'No matching documents'
                ) : (
                  <>
                    No documents yet.
                    <br />
                    Click &ldquo;New Document&rdquo; to create one.
                  </>
                )}
              </div>
            )}

            {filteredTree.length > 0 && (
              <FileTreeProvider
                value={{
                  editingPath,
                  onEditingChange: setEditingPath,
                  onRequestRename: (path) => setEditingPath(path),
                  onRequestDelete: (path, name) => setDeleteTarget({ path, name }),
                  onRenameSubmit: handleRenameSubmit,
                  activeDocId,
                }}
              >
                <FileTree
                  data={filteredTree}
                  onSelect={handleSelect}
                  openAll={!!deferredSearch}
                />
              </FileTreeProvider>
            )}
          </>
        )}
      </div>

      {/* Delete confirmation dialog */}
      <ConfirmDialog
        open={!!deleteTarget}
        onOpenChange={(open) => !open && setDeleteTarget(null)}
        title={`Delete ${deleteTarget?.name}?`}
        description="This cannot be undone."
        onConfirm={handleDeleteConfirm}
        confirmLabel="Delete"
      />
    </aside>
  );
}
