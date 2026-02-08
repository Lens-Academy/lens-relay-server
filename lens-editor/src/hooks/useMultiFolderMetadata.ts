// src/hooks/useMultiFolderMetadata.ts
import { useState, useEffect, useRef, useMemo } from 'react';
import * as Y from 'yjs';
import { YSweetProvider } from '@y-sweet/client';
import { getClientToken } from '../lib/auth';
import type { FileMetadata, FolderMetadata } from './useFolderMetadata';
import { mergeMetadata, type FolderInput } from '../lib/multi-folder-utils';

const RELAY_ID = 'cb696037-0f72-4e93-8717-4e433129d789';

export interface FolderConfig {
  id: string;
  name: string;
}

interface FolderConnection {
  doc: Y.Doc;
  provider: YSweetProvider;
  name: string;
}

export interface UseMultiFolderMetadataReturn {
  metadata: FolderMetadata;
  /** Map from folder NAME to Y.Doc (for CRUD routing by folder name) */
  folderDocs: Map<string, Y.Doc>;
  loading: boolean;
  /** Map from folder NAME to Error (for partial sync failure display) */
  errors: Map<string, Error>;
}

export function useMultiFolderMetadata(folders: FolderConfig[]): UseMultiFolderMetadataReturn {
  const [metadata, setMetadata] = useState<FolderMetadata>({});
  // KEY FIX: Map keyed by folder NAME (not ID) for easier CRUD routing
  const [folderDocs, setFolderDocs] = useState<Map<string, Y.Doc>>(new Map());
  const [loading, setLoading] = useState(true);
  const [errors, setErrors] = useState<Map<string, Error>>(new Map());

  const connectionsRef = useRef<Map<string, FolderConnection>>(new Map());
  const folderMetadataRef = useRef<Map<string, FolderMetadata>>(new Map());

  // Stable key for folders to avoid infinite loop
  // Only reconnect if folder IDs actually change
  const foldersKey = useMemo(
    () => folders.map(f => `${f.id}:${f.name}`).join('|'),
    [folders]
  );

  useEffect(() => {
    const connections = new Map<string, FolderConnection>();
    const docsMap = new Map<string, Y.Doc>();
    folderMetadataRef.current = new Map();
    let syncedCount = 0;

    // Recompute merged metadata from all folders
    const updateMergedMetadata = () => {
      const folderInputs: FolderInput[] = [];
      for (const folder of folders) {
        const meta = folderMetadataRef.current.get(folder.name) ?? {};
        folderInputs.push({ name: folder.name, metadata: meta });
      }
      const merged = mergeMetadata(folderInputs);
      setMetadata(merged);
    };

    for (const folder of folders) {
      const folderDocId = `${RELAY_ID}-${folder.id}`;
      const doc = new Y.Doc();

      const authEndpoint = () => getClientToken(folderDocId);
      const provider = new YSweetProvider(authEndpoint, folderDocId, doc, {
        connect: true,
      });

      // Get the filemeta_v0 Map for this folder
      const filemeta = doc.getMap<FileMetadata>('filemeta_v0');

      // Function to extract metadata from Y.Map
      const extractMetadata = (): FolderMetadata => {
        const entries: FolderMetadata = {};
        filemeta.forEach((value, key) => {
          entries[key] = value;
        });
        return entries;
      };

      // Update when synced
      provider.on('synced', () => {
        folderMetadataRef.current.set(folder.name, extractMetadata());
        updateMergedMetadata();
        syncedCount++;
        // Only set loading=false when ALL folders have synced
        if (syncedCount >= folders.length) {
          setLoading(false);
        }
      });

      // Handle connection errors for partial failure support
      provider.on('connection-error', (err: Error) => {
        setErrors(prev => new Map(prev).set(folder.name, err));
        syncedCount++;
        if (syncedCount >= folders.length) {
          setLoading(false);
        }
      });

      // Subscribe to changes
      filemeta.observe(() => {
        folderMetadataRef.current.set(folder.name, extractMetadata());
        updateMergedMetadata();
      });

      // Handle data already present
      if (filemeta.size > 0) {
        folderMetadataRef.current.set(folder.name, extractMetadata());
        updateMergedMetadata();
      }

      connections.set(folder.name, { doc, provider, name: folder.name });
      // KEY FIX: Use folder NAME as key for docsMap
      docsMap.set(folder.name, doc);
    }

    connectionsRef.current = connections;
    setFolderDocs(docsMap);

    return () => {
      connections.forEach((conn) => {
        conn.provider.destroy();
        conn.doc.destroy();
      });
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [foldersKey]);  // Use stable key instead of folders array

  return { metadata, folderDocs, loading, errors };
}
