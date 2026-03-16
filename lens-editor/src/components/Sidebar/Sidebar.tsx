import { useState, useEffect, useRef, useDeferredValue, useMemo, useCallback } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { SearchInput } from './SearchInput';
import { SearchPanel } from './SearchPanel';
import { FileTree } from './FileTree';
import { FileTreeProvider } from './FileTreeContext';
import type { NodeApi } from 'react-arborist';
import type { TreeNode } from '../../lib/tree-utils';
import * as Dialog from '@radix-ui/react-dialog';
import { ConfirmDialog } from '../ConfirmDialog';
import { useNavigation } from '../../contexts/NavigationContext';
import { useResolvedDocId } from '../../hooks/useResolvedDocId';
import { useSearch } from '../../hooks/useSearch';
import { buildTreeFromPaths, filterTree, searchFileNames, buildDocIdToPathMap } from '../../lib/tree-utils';
import { createDocument, createFolder, deleteDocument, moveDocument } from '../../lib/relay-api';
import { getFolderDocForPath, getOriginalPath, getFolderNameFromPath, generateUntitledName } from '../../lib/multi-folder-utils';
import { RELAY_ID } from '../../App';
import { openDocInNewTab } from '../../lib/url-utils';

export function Sidebar() {
  const [searchTerm, setSearchTerm] = useState('');
  const deferredSearch = useDeferredValue(searchTerm);

  // Get metadata from NavigationContext (needed early for doc ID resolution)
  const { metadata, folderDocs, folderNames, onNavigate, justCreatedRef } = useNavigation();

  // Derive active doc ID from URL path (first segment is the doc UUID — may be short)
  const location = useLocation();
  const docUuidFromUrl = location.pathname.split('/')[1] || '';
  const shortCompoundId = docUuidFromUrl ? `${RELAY_ID}-${docUuidFromUrl}` : '';
  // Resolve short UUID to full compound ID (empty string = no active doc)
  const activeDocId = useResolvedDocId(shortCompoundId, metadata) || '';

  // State for file name filter (separate from full-text search)
  const [fileFilter, setFileFilter] = useState('');

  // State for inline editing
  const [editingPath, setEditingPath] = useState<string | null>(null);

  // State for delete confirmation
  const [deleteTarget, setDeleteTarget] = useState<{ path: string; name: string } | null>(null);

  // State for move dialog
  const [moveTarget, setMoveTarget] = useState<{ path: string; docId: string } | null>(null);
  const [moveNewPath, setMoveNewPath] = useState('');
  const [moveTargetFolder, setMoveTargetFolder] = useState<string>('');
  const [moveError, setMoveError] = useState<string | null>(null);
  const [isMoving, setIsMoving] = useState(false);

  // Navigation for review page link
  const navigate = useNavigate();

  // Ref for Ctrl+K keyboard shortcut focus
  const searchInputRef = useRef<HTMLInputElement>(null);

  // metadata, folderDocs, folderNames, onNavigate — destructured above (before resolution hook)

  // Server-side full-text search (activates when searchTerm >= 2 chars)
  const { results: searchResults, loading: searchLoading, error: searchError } = useSearch(searchTerm);

  // Build doc_id → display path lookup from metadata
  const docIdToPath = useMemo(() => buildDocIdToPathMap(metadata), [metadata]);

  // Enrich server search results with display paths
  const enrichedSearchResults = useMemo(() => {
    return searchResults.map(r => ({
      ...r,
      path: docIdToPath.get(r.doc_id) ?? r.folder,
    }));
  }, [searchResults, docIdToPath]);

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

  // Filter tree based on file filter input (used when search panel is NOT active)
  const filteredTree = useMemo(() => {
    if (!fileFilter) return treeData;
    return filterTree(treeData, fileFilter);
  }, [treeData, fileFilter]);

  // Client-side filename matches for the search panel (instant, no debounce needed)
  const fileNameMatches = useMemo(() => {
    if (deferredSearch.trim().length < 2) return [];
    return searchFileNames(metadata, deferredSearch, folderNames);
  }, [metadata, deferredSearch, folderNames]);

  // Visual feedback while deferred value is processing
  const isStale = searchTerm !== deferredSearch;

  // Whether to show server-side search results vs file tree
  const showSearchResults = searchTerm.trim().length >= 2;

  // Build compound doc ID and navigate via URL
  const handleSelect = useCallback((docId: string) => {
    const compoundDocId = `${RELAY_ID}-${docId}`;
    onNavigate(compoundDocId);
  }, [onNavigate]);

  // Open a document in a new browser tab
  const handleOpenNewTab = useCallback((docId: string) => {
    openDocInNewTab(RELAY_ID, docId, metadata);
  }, [metadata]);

  // CRUD handlers
  const handleRenameSubmit = useCallback(async (prefixedOldPath: string, newName: string, docId: string) => {
    const folderName = getFolderNameFromPath(prefixedOldPath, folderNames);
    if (!folderName) return;
    const oldPath = getOriginalPath(prefixedOldPath, folderName);
    const parts = oldPath.split('/');
    const filename = newName.endsWith('.md') ? newName : `${newName}.md`;
    parts[parts.length - 1] = filename;
    const newPath = parts.join('/');
    try {
      await moveDocument(docId, newPath);
    } catch (err: any) {
      console.error('Rename failed:', err);
      setMoveError(err.message || 'Rename failed');
    }
  }, [folderNames]);

  const handleDeleteConfirm = useCallback(() => {
    if (!deleteTarget) return;
    const doc = getFolderDocForPath(deleteTarget.path, folderDocs, folderNames);
    if (!doc) return;
    const folderName = getFolderNameFromPath(deleteTarget.path, folderNames)!;
    const originalPath = getOriginalPath(deleteTarget.path, folderName);
    deleteDocument(doc, originalPath);
    setDeleteTarget(null);
  }, [folderDocs, folderNames, deleteTarget]);

  const handleInstantCreate = useCallback(async (folderPath: string) => {
    const folderName = getFolderNameFromPath(folderPath, folderNames);
    if (!folderName) return;
    const doc = folderDocs.get(folderName);
    if (!doc) return;

    // Compute the relative path within the shared folder
    const originalFolderPath = getOriginalPath(folderPath, folderName);
    const untitledName = generateUntitledName(folderPath, metadata);
    const path = originalFolderPath === '' || originalFolderPath === '/'
      ? `/${untitledName}`
      : `${originalFolderPath}/${untitledName}`;

    try {
      const id = await createDocument(doc, path, 'markdown');
      justCreatedRef.current = true;
      const compoundDocId = `${RELAY_ID}-${id}`;
      onNavigate(compoundDocId);
    } catch (error) {
      console.error('Failed to create document:', error);
    }
  }, [folderDocs, folderNames, metadata, onNavigate, justCreatedRef]);

  const handleCreateFolder = useCallback((folderPath: string) => {
    const folderName = getFolderNameFromPath(folderPath, folderNames);
    if (!folderName) return;
    const doc = folderDocs.get(folderName);
    if (!doc) return;

    const originalFolderPath = getOriginalPath(folderPath, folderName);
    const basePath = originalFolderPath === '' || originalFolderPath === '/'
      ? '/New Folder'
      : `${originalFolderPath}/New Folder`;

    // Find a unique name if "New Folder" already exists
    let path = basePath;
    let counter = 2;
    const filemeta = doc.getMap('filemeta_v0');
    while (filemeta.has(path)) {
      path = `${basePath} ${counter}`;
      counter++;
    }

    createFolder(doc, path);
  }, [folderDocs, folderNames]);

  const handleMoveRequest = useCallback((prefixedPath: string, docId: string) => {
    // Pre-populate with current path (strip folder prefix)
    const folderName = getFolderNameFromPath(prefixedPath, folderNames);
    const originalPath = folderName ? getOriginalPath(prefixedPath, folderName) : prefixedPath;
    setMoveTarget({ path: prefixedPath, docId });
    setMoveNewPath(originalPath);
    setMoveTargetFolder(folderName || folderNames[0] || '');
    setMoveError(null);
  }, [folderNames]);

  const handleMoveConfirm = useCallback(async () => {
    if (!moveTarget || !moveNewPath.trim()) return;
    setIsMoving(true);
    setMoveError(null);
    try {
      // Determine if this is a cross-folder move
      const currentFolder = getFolderNameFromPath(moveTarget.path, folderNames);
      const targetFolder = moveTargetFolder !== currentFolder ? moveTargetFolder : undefined;
      await moveDocument(moveTarget.docId, moveNewPath, targetFolder);
      setMoveTarget(null);
      setMoveNewPath('');
    } catch (err: any) {
      setMoveError(err.message || 'Move failed');
    } finally {
      setIsMoving(false);
    }
  }, [moveTarget, moveNewPath, moveTargetFolder, folderNames]);

  const handleDragMove = useCallback(async (
    dragNodes: NodeApi<TreeNode>[],
    parentNode: NodeApi<TreeNode> | null,
  ) => {
    const dragNode = dragNodes[0];
    if (!dragNode?.data.docId || dragNode.data.isFolder) return;

    const fileName = dragNode.data.name;
    const oldPrefixedPath = dragNode.data.path;

    // Determine new parent path
    const newParentPath = parentNode?.data.path ?? '';

    // Compute source and target folder names
    const sourceFolderName = getFolderNameFromPath(oldPrefixedPath, folderNames);
    const targetFolderName = parentNode
      ? getFolderNameFromPath(parentNode.data.path, folderNames)
      : sourceFolderName; // dropping at root stays in same folder

    if (!sourceFolderName || !targetFolderName) return;

    // Strip folder prefix to get Y.Doc path
    const newOriginalPath = getOriginalPath(
      `${newParentPath}/${fileName}`,
      targetFolderName,
    );

    const crossFolder = sourceFolderName !== targetFolderName
      ? targetFolderName
      : undefined;

    try {
      await moveDocument(dragNode.data.docId, newOriginalPath, crossFolder);
    } catch (err: any) {
      console.error('Drag move failed:', err);
      setMoveError(err.message || 'Move failed');
    }
  }, [folderNames]);

  return (
    <aside className="w-full h-full bg-[#f6f6f6] flex flex-col">
      {/* Header with search */}
      <div className="p-3 border-b border-gray-200 space-y-2">
        <SearchInput
          ref={searchInputRef}
          value={searchTerm}
          onChange={setSearchTerm}
          placeholder="Search..."
        />
      </div>

      {/* Tree content or search results */}
      <div className={`flex-1 min-h-0 flex flex-col ${showSearchResults ? 'overflow-y-auto' : ''} ${isStale && !showSearchResults ? 'opacity-80' : ''}`}>
        {showSearchResults ? (
          <SearchPanel
            results={enrichedSearchResults}
            fileNameMatches={fileNameMatches}
            loading={searchLoading}
            error={searchError}
            query={searchTerm}
            onNavigate={onNavigate}
          />
        ) : (
          <>
            {/* File name filter input */}
            {folderDocs.size > 0 && (
              <div className="px-3 py-1.5 border-b border-gray-100">
                <div className="relative">
                  <svg
                    className="absolute left-2 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-gray-400"
                    fill="none"
                    viewBox="0 0 24 24"
                    stroke="currentColor"
                    strokeWidth={2}
                  >
                    <path strokeLinecap="round" strokeLinejoin="round" d="M3 4a1 1 0 011-1h16a1 1 0 011 1v2.586a1 1 0 01-.293.707l-6.414 6.414a1 1 0 00-.293.707V17l-4 4v-6.586a1 1 0 00-.293-.707L3.293 7.293A1 1 0 013 6.586V4z" />
                  </svg>
                  <input
                    type="text"
                    value={fileFilter}
                    onChange={(e) => setFileFilter(e.target.value)}
                    placeholder="Filter files..."
                    className="w-full pl-7 pr-6 py-1 text-xs bg-white border border-gray-200 rounded
                               placeholder-gray-400 outline-none focus:border-gray-300"
                  />
                  {fileFilter && (
                    <button
                      onClick={() => setFileFilter('')}
                      className="absolute right-1.5 top-1/2 -translate-y-1/2 p-0.5 text-gray-400 hover:text-gray-600"
                    >
                      <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
                      </svg>
                    </button>
                  )}
                </div>
              </div>
            )}

            {/* Loading state: no doc yet or empty metadata */}
            {folderDocs.size === 0 && Object.keys(metadata).length === 0 && (
              <div className="p-4 text-sm text-gray-500">
                Loading documents...
              </div>
            )}

            {filteredTree.length === 0 && folderDocs.size > 0 && (
              <div className="p-4 text-sm text-gray-500 text-center">
                {fileFilter ? (
                  'No matching documents'
                ) : (
                  <>
                    No documents yet.
                    <br />
                    Use the + button on a folder to create a document.
                  </>
                )}
              </div>
            )}

            {filteredTree.length > 0 && (
              <div className="flex-1 min-h-0">
                <FileTreeProvider
                  value={{
                    editingPath,
                    onEditingChange: setEditingPath,
                    onRequestRename: (path) => setEditingPath(path),
                    onRequestDelete: (path, name) => setDeleteTarget({ path, name }),
                    onRequestMove: handleMoveRequest,
                    onRenameSubmit: handleRenameSubmit,
                    onCreateDocument: handleInstantCreate,
                    onCreateFolder: handleCreateFolder,
                    onOpenNewTab: handleOpenNewTab,
                    activeDocId,
                  }}
                >
                  <FileTree
                    data={filteredTree}
                    onSelect={handleSelect}
                    onMove={handleDragMove}
                    openAll={!!fileFilter}
                    activeDocId={activeDocId}
                  />
                </FileTreeProvider>
              </div>
            )}
          </>
        )}
      </div>

      {/* Review link */}
      <div className="px-3 py-2 border-t border-gray-200">
        <button
          onClick={() => navigate('/review')}
          className="w-full text-left px-2 py-1.5 text-sm text-gray-600 hover:bg-gray-100 rounded"
        >
          Review Suggestions
        </button>
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

      {/* Move dialog */}
      <Dialog.Root open={!!moveTarget} onOpenChange={(open) => { if (!open) { setMoveTarget(null); setMoveError(null); } }}>
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 bg-black/50" />
          <Dialog.Content className="fixed top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 bg-white rounded-lg p-6 w-[420px]">
            <Dialog.Title className="text-lg font-semibold">
              Move {moveTarget?.path.split('/').pop()}
            </Dialog.Title>
            <Dialog.Description className="text-gray-600 mt-1 text-sm">
              Enter the new path for this document.
            </Dialog.Description>

            <div className="mt-4 space-y-3">
              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">New path</label>
                <input
                  type="text"
                  value={moveNewPath}
                  onChange={(e) => setMoveNewPath(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' && !isMoving) {
                      e.preventDefault();
                      handleMoveConfirm();
                    }
                  }}
                  className="w-full px-3 py-2 text-sm border border-gray-300 rounded-md outline-none focus:border-blue-400"
                  autoFocus
                />
              </div>

              {folderNames.length > 1 && (
                <div>
                  <label className="block text-sm font-medium text-gray-700 mb-1">Target folder</label>
                  <select
                    value={moveTargetFolder}
                    onChange={(e) => setMoveTargetFolder(e.target.value)}
                    className="w-full px-3 py-2 text-sm border border-gray-300 rounded-md outline-none focus:border-blue-400 bg-white"
                  >
                    {folderNames.map((name) => (
                      <option key={name} value={name}>{name}</option>
                    ))}
                  </select>
                </div>
              )}

              {moveError && (
                <p className="text-sm text-red-600">{moveError}</p>
              )}
            </div>

            <div className="flex justify-end gap-3 mt-4">
              <Dialog.Close asChild>
                <button className="px-4 py-2 rounded bg-gray-100 hover:bg-gray-200 text-sm">
                  Cancel
                </button>
              </Dialog.Close>
              <button
                className="px-4 py-2 rounded bg-blue-600 text-white hover:bg-blue-700 text-sm disabled:opacity-50 disabled:cursor-not-allowed"
                onClick={handleMoveConfirm}
                disabled={isMoving || !moveNewPath.trim()}
              >
                {isMoving ? 'Moving...' : 'Move'}
              </button>
            </div>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </aside>
  );
}
