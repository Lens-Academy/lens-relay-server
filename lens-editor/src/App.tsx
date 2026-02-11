import { useState } from 'react';
import { RelayProvider } from './providers/RelayProvider';
import { Sidebar } from './components/Sidebar';
import { EditorArea } from './components/Layout';
import { AwarenessInitializer } from './components/AwarenessInitializer/AwarenessInitializer';
import { DisconnectionModal } from './components/DisconnectionModal/DisconnectionModal';
import { NavigationContext } from './contexts/NavigationContext';
import { DisplayNameProvider } from './contexts/DisplayNameContext';
import { DisplayNamePrompt } from './components/DisplayNamePrompt';
import { DisplayNameBadge } from './components/DisplayNameBadge';
import { useMultiFolderMetadata, type FolderConfig } from './hooks/useMultiFolderMetadata';
import { AuthProvider } from './contexts/AuthContext';
import type { UserRole } from './contexts/AuthContext';
import { getShareTokenFromUrl, stripShareTokenFromUrl, decodeRoleFromToken } from './lib/auth-share';
import { setShareToken } from './lib/auth';

// VITE_LOCAL_RELAY=true routes requests to a local relay-server via Vite proxy
const USE_LOCAL_RELAY = import.meta.env.VITE_LOCAL_RELAY === 'true';

// Relay server ID — switches between production and local test IDs
export const RELAY_ID = USE_LOCAL_RELAY
  ? 'a0000000-0000-4000-8000-000000000000'
  : 'cb696037-0f72-4e93-8717-4e433129d789';

// Folder configuration
const FOLDERS: FolderConfig[] = USE_LOCAL_RELAY
  ? [
      { id: 'b0000001-0000-4000-8000-000000000001', name: 'Relay Folder 1' },
      { id: 'b0000002-0000-4000-8000-000000000002', name: 'Relay Folder 2' },
    ]
  : [
      { id: 'fbd5eb54-73cc-41b0-ac28-2b93d3b4244e', name: 'Lens' },
      { id: 'ea4015da-24af-4d9d-ac49-8c902cb17121', name: 'Lens Edu' },
    ];

// Default document to show on load
const DEFAULT_DOC_ID = USE_LOCAL_RELAY
  ? `${RELAY_ID}-c0000001-0000-4000-8000-000000000001`
  : `${RELAY_ID}-76c3e654-0e77-4538-962f-1b419647206e`;

// Read share token from URL once at module load (before React renders)
const shareToken = getShareTokenFromUrl();
const shareRole: UserRole | null = shareToken ? decodeRoleFromToken(shareToken) : null;

// Store share token for all relay auth calls, then strip from URL bar
if (shareToken) {
  setShareToken(shareToken);
  stripShareTokenFromUrl();
}

function AccessDenied() {
  return (
    <div className="h-screen flex items-center justify-center bg-gray-50">
      <div className="text-center max-w-md px-6">
        <div className="text-5xl mb-4">🔒</div>
        <h1 className="text-2xl font-semibold text-gray-800 mb-2">Access Required</h1>
        <p className="text-gray-500">You need a share link to access this editor. Please ask the document owner for a link.</p>
      </div>
    </div>
  );
}

export function App() {
  const [activeDocId, setActiveDocId] = useState<string>(DEFAULT_DOC_ID);

  // No valid token → show access denied
  if (!shareToken || !shareRole) {
    return <AccessDenied />;
  }

  // Use multi-folder metadata hook
  const { metadata, folderDocs, errors } = useMultiFolderMetadata(FOLDERS);
  const folderNames = FOLDERS.map(f => f.name);

  return (
    <AuthProvider role={shareRole}>
      <DisplayNameProvider>
        <DisplayNamePrompt />
        <NavigationContext.Provider value={{ metadata, folderDocs, folderNames, errors, onNavigate: setActiveDocId }}>
          <div className="h-screen flex flex-col bg-gray-50">
            {/* Global identity bar */}
            <div className="flex items-center justify-end px-4 py-1 bg-white border-b border-gray-100">
              <DisplayNameBadge />
            </div>
            <div className="flex-1 flex min-h-0">
              {/* Sidebar is OUTSIDE the key boundary - stays mounted across document switches */}
              <Sidebar activeDocId={activeDocId} onSelectDocument={setActiveDocId} />

              {/* Only EditorArea remounts on doc change via key prop */}
              <RelayProvider key={activeDocId} docId={activeDocId}>
                <AwarenessInitializer />
                <EditorArea currentDocId={activeDocId} />
                <DisconnectionModal />
              </RelayProvider>
            </div>
          </div>
        </NavigationContext.Provider>
      </DisplayNameProvider>
    </AuthProvider>
  );
}
