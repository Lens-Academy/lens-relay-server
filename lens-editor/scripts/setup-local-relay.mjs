#!/usr/bin/env node
/**
 * Setup script for local relay-server development.
 *
 * Creates the folder document, test documents with content, and populates
 * the filemeta_v0 Y.Map so the file tree appears in the sidebar.
 *
 * Uses deterministic v4-format UUIDs so that compound doc IDs (73 chars)
 * are parseable by parse_doc_id() — required for backlinks/link indexer.
 *
 * Port auto-detection: Extracts workspace number from directory name
 * (e.g., "lens-editor-ws2" → port 8190). Override with RELAY_PORT env var.
 *
 * Prerequisites:
 *   npm run relay:start   (or: cd ../relay-server && cargo run -- --config relay.local.toml)
 *
 * Usage:
 *   npm run relay:setup
 */

import path from 'path';
import { fileURLToPath } from 'url';
import * as Y from 'yjs';
import { YSweetProvider } from '@y-sweet/client';

// Auto-detect workspace number from directory name (e.g., "lens-editor-ws2")
// or parent directory (e.g., "ws2/lens-editor")
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectDir = path.basename(path.resolve(__dirname, '..'));
const parentDir = path.basename(path.resolve(__dirname, '../..'));
const workspaceMatch = projectDir.match(/-ws(\d+)$/) || parentDir.match(/^ws(\d+)$/);
const wsNum = workspaceMatch ? parseInt(workspaceMatch[1], 10) : 1;
const portOffset = (wsNum - 1) * 100;
const defaultRelayPort = 8090 + portOffset;
const relayPort = parseInt(process.env.RELAY_PORT || String(defaultRelayPort), 10);

const RELAY_URL = `http://localhost:${relayPort}`;

// Deterministic v4-format UUIDs for local development.
// These produce 73-char compound IDs parseable by parse_doc_id().
const RELAY_ID = 'a0000000-0000-4000-8000-000000000000';

const FOLDER_1_ID = 'b0000001-0000-4000-8000-000000000001';
const FOLDER_2_ID = 'b0000002-0000-4000-8000-000000000002';

// Two test folders for multi-folder support testing
const TEST_FOLDERS = [
  {
    id: FOLDER_1_ID,
    name: 'Relay Folder 1',
    docs: [
      {
        path: '/Notes',
        id: 'c0000010-0000-4000-8000-000000000010',
        type: 'folder',
        version: 0,
        content: null,
      },
      {
        path: '/Welcome.md',
        id: 'c0000001-0000-4000-8000-000000000001',
        type: 'markdown',
        version: 0,
        content: `# Welcome to Lens Editor

This is a local development environment running against relay-server.

![Lens Academy](https://images.unsplash.com/photo-1451187580459-43490279c0fa?w=600&h=300&fit=crop)

## Features

- **Real-time collaboration** - Open multiple tabs to see sync in action
- **Markdown editing** - Full CodeMirror 6 editor with live preview
- **Document management** - Create, rename, and delete documents

## Getting Started

Check out [[Getting Started]] for how to use the local development environment.

You can also explore [[Notes/Ideas]] for project brainstorming.

## Links

Try editing this document, then open another tab to see the changes sync!

You can also create new documents using the "+ New Document" button in the sidebar.
`,
      },
      {
        path: '/Getting Started.md',
        id: 'c0000002-0000-4000-8000-000000000002',
        type: 'markdown',
        version: 0,
        content: `# Getting Started

## Local Development

You're running against a local relay-server, which means:

1. Documents are stored in the filesystem (./data-local-ws${wsNum})
2. No authentication is required
3. Data persists across server restarts

## Creating Documents

Click "+ New Document" in the sidebar to create a new file.

## Wikilinks

Try creating a [[Welcome]] link - it should navigate to the Welcome page!
`,
      },
      {
        path: '/Notes/Ideas.md',
        id: 'c0000003-0000-4000-8000-000000000003',
        type: 'markdown',
        version: 0,
        content: `# Ideas

This is a nested document in the Notes folder.

Check out [[../Welcome]] for an overview, or [[../Getting Started]] for setup instructions.

## Todo

- [ ] Test real-time sync between tabs
- [ ] Try creating wikilinks
- [ ] Test document creation
- [ ] Test document deletion
`,
      },
      {
        path: '/Discord.md',
        id: 'c0000008-0000-4000-8000-000000000008',
        type: 'markdown',
        version: 0,
        content: `---
discussion: https://discord.com/channels/1443369661847834688/1443369662560735264
---
# Discord Integration

This document has a linked Discord channel for discussion.

Use the discussion panel on the right to view and post messages from the linked Discord thread.

## How It Works

The \`discussion\` frontmatter field links this document to a Discord channel. The editor fetches messages from the Discord API and displays them in a side panel.

## Testing

- Open this document and check the discussion panel appears
- Try posting a message from the compose box
- Verify messages load from the linked channel
`,
      },
      {
        path: '/Projects',
        id: 'c0000011-0000-4000-8000-000000000011',
        type: 'folder',
        version: 0,
        content: null,
      },
      {
        path: '/Projects/Roadmap.md',
        id: 'c0000007-0000-4000-8000-000000000007',
        type: 'markdown',
        version: 0,
        content: `# Roadmap

This is a nested document in the Projects folder, a sibling to Notes.

## Milestones

- [ ] Wikilink autocomplete with relative paths
- [ ] Cross-folder link resolution
- [ ] Document search improvements

## Related

See [[../Notes/Ideas]] for brainstorming and [[../Welcome]] for an overview.
`,
      },
    ],
  },
  {
    id: FOLDER_2_ID,
    name: 'Relay Folder 2',
    docs: [
      {
        path: '/Resources',
        id: 'c0000020-0000-4000-8000-000000000020',
        type: 'folder',
        version: 0,
        content: null,
      },
      {
        path: '/Course Notes.md',
        id: 'c0000004-0000-4000-8000-000000000004',
        type: 'markdown',
        version: 0,
        content: `# Course Notes

This document is in the **Lens Edu** folder - a separate shared folder for educational content.

## Multi-Folder Support

You should see both folders as top-level entries in the sidebar.

Each folder syncs independently with its own Y.Doc.

## Related

See [[Syllabus]] for the course outline and [[Resources/Links]] for useful references.
`,
      },
      {
        path: '/Syllabus.md',
        id: 'c0000005-0000-4000-8000-000000000005',
        type: 'markdown',
        version: 0,
        content: `# Syllabus

See [[Course Notes]] for detailed notes on each topic.

## Week 1: Introduction

- Overview of collaborative editing
- Y.js fundamentals

## Week 2: Real-time Sync

- CRDTs explained
- Conflict resolution

## Week 3: Building UIs

- React integration
- CodeMirror setup
`,
      },
      {
        path: '/Resources/Links.md',
        id: 'c0000006-0000-4000-8000-000000000006',
        type: 'markdown',
        version: 0,
        content: `# Useful Links

See [[../Course Notes]] for course material and [[../Syllabus]] for the schedule.

## Documentation

- [Y.js Docs](https://docs.yjs.dev/)
- [Relay](https://relay.md/)
- [CodeMirror](https://codemirror.net/)

## Tutorials

- Getting started with CRDTs
- Building collaborative apps
`,
      },
    ],
  },
];

async function createDoc(docId) {
  const response = await fetch(`${RELAY_URL}/doc/new`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ docId }),
  });

  if (!response.ok && response.status !== 409) {
    throw new Error(`Failed to create ${docId}: ${response.status}`);
  }

  return response.json();
}

async function getClientToken(docId) {
  const response = await fetch(`${RELAY_URL}/doc/${docId}/auth`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ authorization: 'full' }),
  });

  if (!response.ok) {
    throw new Error(`Failed to get token for ${docId}: ${response.status}`);
  }

  return response.json();
}

async function checkServer() {
  try {
    const response = await fetch(`${RELAY_URL}/`);
    return response.ok || response.status === 404;
  } catch {
    return false;
  }
}

async function connectAndPopulate(docId, populateFn) {
  const doc = new Y.Doc();
  const authEndpoint = () => getClientToken(docId);

  const provider = new YSweetProvider(authEndpoint, docId, doc, {
    connect: true,
    showDebuggerLink: false,
  });

  // Wait for sync
  await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('Sync timeout')), 10000);

    provider.on('synced', () => {
      clearTimeout(timeout);
      resolve();
    });

    provider.on('connection-error', (err) => {
      clearTimeout(timeout);
      reject(err);
    });
  });

  // Run the populate function
  await populateFn(doc);

  // Wait for sync to propagate
  await new Promise(resolve => setTimeout(resolve, 300));

  // Cleanup
  provider.destroy();
  doc.destroy();
}

async function populateFolderDoc(folderDocId, folderName, testDocs) {
  console.log('\n  Populating filemeta_v0, legacy docs, and folder_config Y.Maps...');

  await connectAndPopulate(folderDocId, (doc) => {
    const filemeta = doc.getMap('filemeta_v0');
    const legacyDocs = doc.getMap('docs');

    doc.transact(() => {
      // Store folder display name so the backend can read it
      const folderConfig = doc.getMap('folder_config');
      folderConfig.set('name', folderName);

      for (const testDoc of testDocs) {
        if (!filemeta.has(testDoc.path)) {
          // Modern format: id, type, version
          filemeta.set(testDoc.path, {
            id: testDoc.id,
            type: testDoc.type,
            version: testDoc.version,
          });
          // Legacy format: path -> guid (required for Obsidian compatibility)
          if (testDoc.type !== 'folder') {
            legacyDocs.set(testDoc.path, testDoc.id);
          }
          console.log(`    ✓ Added ${testDoc.path}`);
        } else {
          console.log(`    ⚠ ${testDoc.path} already exists`);
        }
      }
    });
  });

  console.log('  ✓ filemeta_v0 + legacy docs + folder_config populated');
}

async function populateDocContent(testDoc) {
  // Skip folders - they don't have content
  if (testDoc.type === 'folder' || !testDoc.content) {
    console.log(`    - ${testDoc.path} (folder, no content)`);
    return;
  }

  const fullDocId = `${RELAY_ID}-${testDoc.id}`;

  await connectAndPopulate(fullDocId, (doc) => {
    // Editor uses getText('contents') - must match!
    const text = doc.getText('contents');

    // Only populate if empty
    if (text.length === 0) {
      text.insert(0, testDoc.content);
      console.log(`    ✓ ${testDoc.path} - content added`);
    } else {
      console.log(`    ⚠ ${testDoc.path} - already has content`);
    }
  });
}

async function main() {
  console.log('Setting up local relay-server for development...\n');

  // Check if server is running
  console.log(`Workspace ${wsNum}: expecting relay-server on port ${relayPort}\n`);

  const serverUp = await checkServer();
  if (!serverUp) {
    console.error('❌ Local relay-server not running!');
    console.error(`   Start it with: npm run relay:start`);
    console.error(`   Or manually: cd ../relay-server && PORT=${relayPort} cargo run -- --config relay.local.toml\n`);
    process.exit(1);
  }
  console.log('✓ Relay server is running\n');

  console.log('⚠ If upgrading from short IDs, delete old data: rm -rf ../relay-server/data-local-ws' + wsNum + '\n');

  // Process each test folder
  for (const folder of TEST_FOLDERS) {
    console.log('─'.repeat(50));
    console.log(`Setting up folder: ${folder.name} (${folder.id})`);
    console.log('─'.repeat(50));

    // Create folder document
    const folderDocId = `${RELAY_ID}-${folder.id}`;
    console.log(`\n  Creating folder document: ${folderDocId}`);
    try {
      await createDoc(folderDocId);
      console.log('  ✓ Folder document created');
    } catch (e) {
      console.log('    (may already exist, continuing...)');
    }

    // Create test documents on server (skip folders - they're just metadata)
    console.log('\n  Creating test documents on server:');
    for (const testDoc of folder.docs) {
      if (testDoc.type === 'folder') {
        console.log(`    - ${testDoc.path} (folder, no server doc needed)`);
        continue;
      }
      const fullDocId = `${RELAY_ID}-${testDoc.id}`;
      try {
        await createDoc(fullDocId);
        console.log(`    ✓ ${fullDocId}`);
      } catch (e) {
        console.log(`    ⚠ ${fullDocId} (may already exist)`);
      }
    }

    // Populate the folder doc's filemeta_v0 Y.Map and folder_config
    await populateFolderDoc(folderDocId, folder.name, folder.docs);

    // Populate each document with content
    console.log('\n  Populating document content...');
    for (const testDoc of folder.docs) {
      await populateDocContent(testDoc);
    }

    console.log(`\n✓ Folder "${folder.name}" setup complete\n`);
  }

  console.log('─'.repeat(50));
  console.log('All folders setup complete!\n');
  console.log('Test folders created:');
  for (const folder of TEST_FOLDERS) {
    console.log(`  - ${folder.name} (${folder.id})`);
  }
  console.log('\nRelay ID: ' + RELAY_ID);
  console.log('Compound doc ID format: ' + RELAY_ID + '-<doc-uuid> (73 chars)');
  console.log('\nStart the dev server with:');
  console.log('  npm run dev:local\n');
}

main().catch((err) => {
  console.error('Setup failed:', err);
  process.exit(1);
});
