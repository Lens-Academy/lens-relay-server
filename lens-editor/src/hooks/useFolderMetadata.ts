import { useState, useEffect, useCallback, useRef } from 'react';
import * as Y from 'yjs';
import { YSweetProvider } from '@y-sweet/client';
import { getClientToken } from '../lib/auth';
import { setupFilemetaDebugObserver } from '../lib/relay-api';

const USE_LOCAL_RELAY = import.meta.env.VITE_LOCAL_RELAY === 'true';
const RELAY_ID = USE_LOCAL_RELAY
  ? 'a0000000-0000-4000-8000-000000000000'
  : 'cb696037-0f72-4e93-8717-4e433129d789';

export interface FileMetadata {
  id: string;
  type: 'markdown' | 'canvas' | 'folder' | 'image' | 'file' | 'pdf' | 'audio' | 'video';
  version: number;
  hash?: string;
  // Image-specific fields
  mimetype?: string;
  synctime?: number;
}

export type FolderMetadata = Record<string, FileMetadata>;

/**
 * Hook to fetch and subscribe to folder metadata from the Relay server.
 * This connects to a SEPARATE Y.Doc for the shared folder's filemeta_v0 Map,
 * distinct from the current document being edited.
 */
export function useFolderMetadata(folderId: string) {
  const [metadata, setMetadata] = useState<FolderMetadata>({});
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);
  // State for doc - triggers re-render when Y.Doc is created
  const [doc, setDoc] = useState<Y.Doc | null>(null);

  // Keep refs for cleanup
  const providerRef = useRef<YSweetProvider | null>(null);
  const docRef = useRef<Y.Doc | null>(null);
  const debugObserverCleanupRef = useRef<(() => void) | null>(null);

  const connect = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);

      // Construct compound doc ID for the folder
      const folderDocId = `${RELAY_ID}-${folderId}`;

      // Create a new Y.Doc for the folder metadata
      const newDoc = new Y.Doc();
      docRef.current = newDoc;
      setDoc(newDoc);  // Trigger re-render when doc is ready

      // Debug: expose to window for console inspection
      (window as any).__folderDoc = newDoc;
      (window as any).__filemeta = newDoc.getMap('filemeta_v0');
      console.log('[DEBUG] Y.Doc created and exposed as window.__folderDoc and window.__filemeta');

      // Set up debug observer to log all filemeta changes
      debugObserverCleanupRef.current = setupFilemetaDebugObserver(newDoc);

      // Auth endpoint function for the folder doc
      const authEndpoint = () => getClientToken(folderDocId);

      // Connect to the Relay server
      const provider = new YSweetProvider(authEndpoint, folderDocId, newDoc, {
        connect: true,
      });
      providerRef.current = provider;

      // Get the filemeta_v0 Map
      const filemeta = newDoc.getMap<FileMetadata>('filemeta_v0');

      // Function to update state from Y.Map
      const updateMetadata = () => {
        const entries: FolderMetadata = {};
        filemeta.forEach((value, key) => {
          entries[key] = value;
        });
        setMetadata(entries);
        setLoading(false);
      };

      // Initial sync once connected
      provider.on('synced', () => {
        updateMetadata();
      });

      // Subscribe to changes
      filemeta.observe(updateMetadata);

      // If already has data, update immediately
      if (filemeta.size > 0) {
        updateMetadata();
      }

    } catch (err) {
      setError(err instanceof Error ? err : new Error('Failed to connect'));
      setLoading(false);
    }
  }, [folderId]);

  useEffect(() => {
    connect();

    return () => {
      // Cleanup on unmount
      if (debugObserverCleanupRef.current) {
        debugObserverCleanupRef.current();
        debugObserverCleanupRef.current = null;
      }
      if (providerRef.current) {
        providerRef.current.destroy();
        providerRef.current = null;
      }
      if (docRef.current) {
        docRef.current.destroy();
        docRef.current = null;
        setDoc(null);  // Clear state on cleanup
      }
    };
  }, [connect]);

  return { metadata, loading, error, doc };  // Return state variable, not ref
}
