import { describe, it, expect, beforeEach, vi } from 'vitest';
import * as Y from 'yjs';
import { createDocument, renameDocument, deleteDocument, createFolder } from './relay-api';
import type { FileMetadata } from '../hooks/useFolderMetadata';

// Mock fetch for server calls
const mockFetch = vi.fn();
vi.stubGlobal('fetch', mockFetch);

// Mock YSweetProvider and getClientToken to avoid real network connections.
// These are exercised by createDocument -> initializeContentDocument which
// connects to the content doc to add initial content for Obsidian sync.
vi.mock('@y-sweet/client', () => ({
  YSweetProvider: class MockYSweetProvider {
    on(event: string, callback: () => void) {
      // Immediately trigger 'synced' event so initializeContentDocument completes
      if (event === 'synced') {
        setTimeout(callback, 0);
      }
    }
    destroy() {}
  },
}));

vi.mock('./auth', () => ({
  getClientToken: vi.fn().mockResolvedValue({
    url: 'ws://localhost:8090',
    baseUrl: 'http://localhost:8090',
    docId: 'test-doc',
    token: 'test-token',
    authorization: 'full',
  }),
}));

describe('relay-api', () => {
  let doc: Y.Doc;
  let filemeta: Y.Map<FileMetadata>;
  let legacyDocs: Y.Map<string>;

  beforeEach(() => {
    doc = new Y.Doc();
    filemeta = doc.getMap<FileMetadata>('filemeta_v0');
    legacyDocs = doc.getMap<string>('docs');
    mockFetch.mockReset();
    // Default: successful server response
    mockFetch.mockResolvedValue({ ok: true });
  });

  describe('createDocument', () => {
    it('creates document with valid UUID', async () => {
      const id = await createDocument(doc, '/New File.md');

      expect(id).toMatch(
        /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/
      );
    });

    it('generates valid UUID even when crypto.randomUUID is unavailable', async () => {
      const original = crypto.randomUUID;
      // @ts-expect-error - simulating insecure context
      crypto.randomUUID = undefined;
      try {
        const id = await createDocument(doc, '/InsecureContext.md');
        expect(id).toMatch(
          /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/
        );
      } finally {
        crypto.randomUUID = original;
      }
    });

    it('adds entry to filemeta_v0 map', async () => {
      const id = await createDocument(doc, '/Test.md');
      const meta = filemeta.get('/Test.md');

      expect(meta).toBeDefined();
      expect(meta!.id).toBe(id);
      expect(meta!.type).toBe('markdown');
    });

    it('adds entry to legacy docs map for Obsidian compatibility', async () => {
      // Obsidian's SyncStore.getMeta() requires documents to exist in BOTH
      // filemeta_v0 AND the legacy 'docs' Y.Map, otherwise it marks them for deletion
      const id = await createDocument(doc, '/Test.md');

      expect(legacyDocs.get('/Test.md')).toBe(id);
    });

    it('defaults to markdown type', async () => {
      await createDocument(doc, '/Default.md');

      expect(filemeta.get('/Default.md')!.type).toBe('markdown');
    });

    it('allows canvas type', async () => {
      await createDocument(doc, '/Diagram.canvas', 'canvas');

      expect(filemeta.get('/Diagram.canvas')!.type).toBe('canvas');
    });

    it('calls server /doc/new endpoint before adding to filemeta', async () => {
      await createDocument(doc, '/ServerDoc.md');

      expect(mockFetch).toHaveBeenCalledTimes(1);
      const [url, options] = mockFetch.mock.calls[0];
      expect(url).toContain('/doc/new');
      expect(options.method).toBe('POST');
      expect(options.headers['Content-Type']).toBe('application/json');

      // Verify docId format in request body
      const body = JSON.parse(options.body);
      expect(body.docId).toMatch(/^cb696037-0f72-4e93-8717-4e433129d789-[0-9a-f-]+$/);
    });

    it('does not add to filemeta if server call fails', async () => {
      mockFetch.mockResolvedValue({ ok: false, status: 500, statusText: 'Internal Server Error' });

      await expect(createDocument(doc, '/FailedDoc.md')).rejects.toThrow('Failed to create document on server');
      expect(filemeta.get('/FailedDoc.md')).toBeUndefined();
    });
  });

  describe('renameDocument', () => {
    it('moves metadata from old path to new path', async () => {
      const id = await createDocument(doc, '/Old.md');
      renameDocument(doc, '/Old.md', '/New.md');

      expect(filemeta.get('/Old.md')).toBeUndefined();
      expect(filemeta.get('/New.md')!.id).toBe(id);
    });

    it('moves legacy docs entry for Obsidian compatibility', async () => {
      const id = await createDocument(doc, '/Old.md');
      renameDocument(doc, '/Old.md', '/New.md');

      expect(legacyDocs.get('/Old.md')).toBeUndefined();
      expect(legacyDocs.get('/New.md')).toBe(id);
    });

    it('preserves all metadata fields after rename', async () => {
      await createDocument(doc, '/Original.md');
      renameDocument(doc, '/Original.md', '/Renamed.md');

      const meta = filemeta.get('/Renamed.md');
      expect(meta!.type).toBe('markdown');
    });

    it('does nothing if old path does not exist', () => {
      renameDocument(doc, '/NonExistent.md', '/Whatever.md');

      expect(filemeta.get('/Whatever.md')).toBeUndefined();
    });
  });

  describe('deleteDocument', () => {
    it('removes entry from filemeta_v0 map', async () => {
      await createDocument(doc, '/ToDelete.md');
      deleteDocument(doc, '/ToDelete.md');

      expect(filemeta.get('/ToDelete.md')).toBeUndefined();
    });

    it('removes entry from legacy docs map for Obsidian compatibility', async () => {
      await createDocument(doc, '/ToDelete.md');
      deleteDocument(doc, '/ToDelete.md');

      expect(legacyDocs.get('/ToDelete.md')).toBeUndefined();
    });

    it('does nothing if path does not exist', () => {
      const sizeBefore = filemeta.size;
      deleteDocument(doc, '/NonExistent.md');

      expect(filemeta.size).toBe(sizeBefore);
    });
  });

  describe('createFolder', () => {
    it('creates folder entry in filemeta_v0 with type folder', () => {
      createFolder(doc, '/NewFolder');

      const entry = filemeta.get('/NewFolder');
      expect(entry).toBeDefined();
      expect(entry!.type).toBe('folder');
      expect(entry!.id).toBeDefined();
      expect(entry!.version).toBe(0);
    });

    it('creates entry in legacy docs map', () => {
      createFolder(doc, '/NewFolder');

      const entry = legacyDocs.get('/NewFolder');
      expect(entry).toBeDefined();
    });

    it('creates ancestor folders for nested paths', () => {
      createFolder(doc, '/A/B/C');

      expect(filemeta.get('/A')).toBeDefined();
      expect(filemeta.get('/A')!.type).toBe('folder');
      expect(filemeta.get('/A/B')).toBeDefined();
      expect(filemeta.get('/A/B')!.type).toBe('folder');
      expect(filemeta.get('/A/B/C')).toBeDefined();
      expect(filemeta.get('/A/B/C')!.type).toBe('folder');
    });

    it('does not overwrite existing folder', () => {
      createFolder(doc, '/Existing');
      const firstId = filemeta.get('/Existing')!.id;

      createFolder(doc, '/Existing');
      expect(filemeta.get('/Existing')!.id).toBe(firstId);
    });
  });

  describe('Y.Doc sync simulation', () => {
    it('syncs changes between two docs via Y.applyUpdate', async () => {
      const id = await createDocument(doc, '/Shared.md');

      const doc2 = new Y.Doc();
      Y.applyUpdate(doc2, Y.encodeStateAsUpdate(doc));

      const filemeta2 = doc2.getMap<FileMetadata>('filemeta_v0');
      expect(filemeta2.get('/Shared.md')!.id).toBe(id);

      doc2.destroy();
    });

    it('merges concurrent changes from multiple clients', async () => {
      await createDocument(doc, '/DocA.md');

      const doc2 = new Y.Doc();
      const filemeta2 = doc2.getMap<FileMetadata>('filemeta_v0');
      filemeta2.set('/DocB.md', { id: 'client2-id', type: 'markdown', version: 0 });

      // Cross-apply updates
      Y.applyUpdate(doc2, Y.encodeStateAsUpdate(doc));
      Y.applyUpdate(doc, Y.encodeStateAsUpdate(doc2));

      // Both docs should have both entries
      expect(filemeta.get('/DocA.md')).toBeDefined();
      expect(filemeta.get('/DocB.md')).toBeDefined();
      expect(filemeta2.get('/DocA.md')).toBeDefined();
      expect(filemeta2.get('/DocB.md')).toBeDefined();

      doc2.destroy();
    });
  });
});
