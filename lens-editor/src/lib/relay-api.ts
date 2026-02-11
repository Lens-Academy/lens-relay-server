import * as Y from 'yjs';
import { YSweetProvider } from '@y-sweet/client';
import type { FileMetadata } from '../hooks/useFolderMetadata';
import { getClientToken } from './auth';

// Relay server configuration (same as in auth.ts)
const SERVER_TOKEN = '2D3RhEOhAQSgWEGkAWxyZWxheS1zZXJ2ZXIDeB1odHRwczovL3JlbGF5LmxlbnNhY2FkZW15Lm9yZwYaaWdOJToAATlIZnNlcnZlckhUsS3xaA3zBw';
const RELAY_URL = 'https://relay.lensacademy.org';

const USE_LOCAL_RELAY = import.meta.env?.VITE_LOCAL_RELAY === 'true';
const RELAY_ID = USE_LOCAL_RELAY
  ? 'a0000000-0000-4000-8000-000000000000'
  : 'cb696037-0f72-4e93-8717-4e433129d789';

// In development, use Vite proxy to avoid CORS
const API_BASE = import.meta.env?.DEV ? '/api/relay' : RELAY_URL;

// Transaction origin identifier - Obsidian uses this pattern to identify
// the source of Y.js changes and avoid processing its own updates
const LENS_EDITOR_ORIGIN = 'lens-editor';

// Debug logging helper
function debug(operation: string, ...args: unknown[]) {
  console.log(`[relay-api] ${operation}:`, ...args);
}

/**
 * Create a document on the Relay server.
 * This must be called BEFORE adding to filemeta, otherwise the document
 * won't be accessible (auth endpoint returns 404 for non-existent docs).
 */
async function createDocumentOnServer(docId: string): Promise<void> {
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
  };

  // Only add auth header for production Relay (local relay-server has no auth)
  if (!USE_LOCAL_RELAY) {
    headers['Authorization'] = `Bearer ${SERVER_TOKEN}`;
  }

  const response = await fetch(`${API_BASE}/doc/new`, {
    method: 'POST',
    headers,
    body: JSON.stringify({ docId }),
  });

  if (!response.ok) {
    throw new Error(`Failed to create document on server: ${response.status} ${response.statusText}`);
  }
}

/**
 * Initialize a content document with an underscore character.
 * This triggers Obsidian to create the file immediately rather than waiting
 * for manual "Relay Sync". Using _ to make it visible/explicit.
 */
async function initializeContentDocument(fullDocId: string): Promise<void> {
  debug('initializeContentDocument', 'connecting to content doc...', { fullDocId });

  const doc = new Y.Doc();
  const authEndpoint = () => getClientToken(fullDocId);

  const provider = new YSweetProvider(authEndpoint, fullDocId, doc, {
    connect: true,
  });

  try {
    // Wait for sync to complete
    await new Promise<void>((resolve, reject) => {
      const timeout = setTimeout(() => {
        reject(new Error('Timeout waiting for content doc sync'));
      }, 10000);

      provider.on('synced', () => {
        clearTimeout(timeout);
        resolve();
      });
    });

    debug('initializeContentDocument', 'synced, adding initial content...');

    // Add an underscore to the contents Y.Text
    // This triggers Obsidian to create the actual file
    // Using _ instead of space to make it visible/explicit
    const contents = doc.getText('contents');
    doc.transact(() => {
      // Only add if empty to avoid overwriting existing content
      if (contents.length === 0) {
        contents.insert(0, '_');
        debug('initializeContentDocument', 'added initial underscore');
      } else {
        debug('initializeContentDocument', 'content already exists, skipping');
      }
    }, LENS_EDITOR_ORIGIN);

    // Wait a moment for the change to propagate
    await new Promise(resolve => setTimeout(resolve, 500));

    debug('initializeContentDocument', 'done');
  } finally {
    // Clean up the connection
    provider.destroy();
  }
}

/**
 * Create a new document in the folder's filemeta_v0 Y.Map.
 *
 * This function:
 * 1. Generates a new UUID for the document
 * 2. Creates the document on the Relay server (POST /doc/new)
 * 3. Adds the path -> UUID mapping to filemeta_v0
 *
 * Returns the generated document UUID.
 */
export async function createDocument(
  folderDoc: Y.Doc,
  path: string,
  type: 'markdown' | 'canvas' = 'markdown'
): Promise<string> {
  const filemeta = folderDoc.getMap<FileMetadata>('filemeta_v0');
  const id = crypto.randomUUID();
  const fullDocId = `${RELAY_ID}-${id}`;

  debug('createDocument', { path, type, id, fullDocId });

  // Step 1: Create document on server first
  debug('createDocument', 'calling server /doc/new...');
  await createDocumentOnServer(fullDocId);
  debug('createDocument', 'server doc created');

  // Step 2: Add to filemeta (this syncs via Y.js)
  // Use transact() with origin like Obsidian does - this allows other clients
  // to identify the source of the change
  const meta: FileMetadata = { id, type, version: 0 };
  debug('createDocument', 'adding to filemeta Y.Map...', { path, meta });

  // Check if entry already exists or is being deleted
  const existing = filemeta.get(path);
  if (existing) {
    debug('createDocument', 'WARNING: entry already exists!', existing);
  }

  // IMPORTANT: Obsidian's SyncStore.getMeta() requires document entries to exist
  // in BOTH filemeta_v0 AND the legacy "docs" Y.Map. If an entry exists only in
  // filemeta_v0, it gets marked for deletion! (SyncStore.ts:336-339)
  const legacyDocs = folderDoc.getMap<string>('docs');

  folderDoc.transact(() => {
    // Add to modern filemeta_v0
    filemeta.set(path, meta);
    // Add to legacy docs map (path -> guid)
    legacyDocs.set(path, id);
  }, LENS_EDITOR_ORIGIN);

  // Verify the entries were added
  const verifyFilemeta = filemeta.get(path);
  const verifyLegacy = legacyDocs.get(path);
  debug('createDocument', 'verification after set:', {
    path,
    filemetaExists: !!verifyFilemeta,
    legacyDocsExists: !!verifyLegacy,
    legacyDocsValue: verifyLegacy,
  });

  debug('createDocument', 'filemeta updated, current entries:',
    Array.from(filemeta.entries()).map(([p, m]) => ({ path: p, id: m.id })));

  // Step 3: Initialize content document to trigger Obsidian sync
  // This adds an underscore so Obsidian creates the file immediately
  try {
    await initializeContentDocument(fullDocId);
  } catch (err) {
    // Don't fail the whole operation if content init fails
    // The document is still created and will sync when edited
    debug('createDocument', 'WARNING: failed to initialize content', err);
  }

  return id;
}

/**
 * Rename a document by moving its metadata from oldPath to newPath.
 * Uses atomic transaction to ensure delete+set happen together.
 */
export function renameDocument(
  folderDoc: Y.Doc,
  oldPath: string,
  newPath: string
): void {
  const filemeta = folderDoc.getMap<FileMetadata>('filemeta_v0');
  const legacyDocs = folderDoc.getMap<string>('docs');
  const meta = filemeta.get(oldPath);
  const legacyId = legacyDocs.get(oldPath);

  debug('renameDocument', { oldPath, newPath, meta, legacyId });

  if (meta) {
    // Wrap in transaction for atomicity - Obsidian does the same
    // Both delete and set happen in a single Y.js update
    // Must update both filemeta_v0 AND legacy docs map
    folderDoc.transact(() => {
      filemeta.delete(oldPath);
      filemeta.set(newPath, meta);
      if (legacyId) {
        legacyDocs.delete(oldPath);
        legacyDocs.set(newPath, legacyId);
      }
    }, LENS_EDITOR_ORIGIN);

    debug('renameDocument', 'rename complete, current entries:',
      Array.from(filemeta.entries()).map(([p, m]) => ({ path: p, id: m.id })));
  } else {
    debug('renameDocument', 'WARNING: no metadata found for oldPath, rename skipped');
  }
}

/**
 * Delete a document from the folder's filemeta_v0 Y.Map.
 * Also removes from legacy docs map if present.
 */
export function deleteDocument(
  folderDoc: Y.Doc,
  path: string
): void {
  const filemeta = folderDoc.getMap<FileMetadata>('filemeta_v0');
  const legacyDocs = folderDoc.getMap<string>('docs');
  const existingMeta = filemeta.get(path);
  const existingLegacy = legacyDocs.get(path);

  debug('deleteDocument', { path, existingMeta, existingLegacy });

  if (existingMeta || existingLegacy) {
    folderDoc.transact(() => {
      if (existingMeta) filemeta.delete(path);
      if (existingLegacy) legacyDocs.delete(path);
    }, LENS_EDITOR_ORIGIN);

    debug('deleteDocument', 'delete complete, remaining entries:',
      Array.from(filemeta.entries()).map(([p, m]) => ({ path: p, id: m.id })));
  } else {
    debug('deleteDocument', 'WARNING: no metadata found for path, delete skipped');
  }
}

/**
 * Set up debug observer on filemeta Y.Map to log all changes.
 * Call this once after connecting to the folder doc.
 */
// --- Search API ---

export interface SearchResult {
  doc_id: string;   // UUID (no RELAY_ID prefix)
  title: string;
  folder: string;
  snippet: string;  // HTML with <mark> tags
  score: number;
}

export interface SearchResponse {
  results: SearchResult[];
  total_hits: number;
  query: string;
}

export async function searchDocuments(
  query: string,
  limit: number = 20,
  signal?: AbortSignal
): Promise<SearchResponse> {
  const params = new URLSearchParams({ q: query, limit: String(limit) });
  const response = await fetch(`${API_BASE}/search?${params}`, { signal });
  if (!response.ok) {
    throw new Error(`Search failed: ${response.status}`);
  }
  return response.json();
}

/**
 * Set up debug observer on filemeta Y.Map to log all changes.
 * Call this once after connecting to the folder doc.
 */
export function setupFilemetaDebugObserver(folderDoc: Y.Doc): () => void {
  const filemeta = folderDoc.getMap<FileMetadata>('filemeta_v0');

  const observer = (event: Y.YMapEvent<FileMetadata>) => {
    const origin = event.transaction.origin;
    const isLocal = origin === LENS_EDITOR_ORIGIN;
    const originName = origin?.constructor?.name ?? String(origin) ?? 'unknown';

    debug('filemeta Y.Map changed', {
      origin: originName,
      isLocalChange: isLocal,
      keysChanged: Array.from(event.keysChanged),
      totalEntries: filemeta.size,
    });

    event.changes.keys.forEach((change, key) => {
      if (change.action === 'add') {
        debug('  ADD', key, filemeta.get(key));
      } else if (change.action === 'update') {
        debug('  UPDATE', key, { oldValue: change.oldValue, newValue: filemeta.get(key) });
      } else if (change.action === 'delete') {
        // Log extra context for deletes - this is what we're debugging
        debug('  DELETE', key, {
          oldValue: change.oldValue,
          deletedById: (change.oldValue as FileMetadata)?.id,
          remainingEntries: filemeta.size,
        });
        console.warn(`[relay-api] ⚠️ EXTERNAL DELETE of ${key} - check Obsidian console for "Deleting doc" message`);
      }
    });
  };

  filemeta.observe(observer);
  debug('setupFilemetaDebugObserver', 'observer registered, current entries:', filemeta.size);

  // Return cleanup function
  return () => {
    filemeta.unobserve(observer);
    debug('setupFilemetaDebugObserver', 'observer removed');
  };
}
