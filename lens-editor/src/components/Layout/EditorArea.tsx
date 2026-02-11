import { useState, useCallback } from 'react';
import { EditorView } from '@codemirror/view';
import { SyncStatus } from '../SyncStatus/SyncStatus';
import { Editor } from '../Editor/Editor';
import { DocumentTitle } from '../DocumentTitle';
import { SourceModeToggle } from '../SourceModeToggle/SourceModeToggle';
import { SuggestionModeToggle } from '../SuggestionModeToggle/SuggestionModeToggle';
import { PresencePanel } from '../PresencePanel/PresencePanel';
import { TableOfContents } from '../TableOfContents';
import { BacklinksPanel } from '../BacklinksPanel';
import { CommentsPanel } from '../CommentsPanel';
import { DebugYMapPanel } from '../DebugYMapPanel';
import { ConnectedDiscussionPanel } from '../DiscussionPanel';
import { useNavigation } from '../../contexts/NavigationContext';
import { useAuth } from '../../contexts/AuthContext';

/**
 * Editor area component that lives INSIDE the RelayProvider key boundary.
 * This allows it to remount when switching documents while keeping
 * the Sidebar stable outside the boundary.
 */
export function EditorArea({ currentDocId }: { currentDocId: string }) {
  const [editorView, setEditorView] = useState<EditorView | null>(null);
  const [stateVersion, setStateVersion] = useState(0);
  const { metadata, onNavigate } = useNavigation();
  const { canWrite } = useAuth();

  // Callback to receive view reference from Editor
  const handleEditorReady = useCallback((view: EditorView) => {
    setEditorView(view);
    // Force re-render to pass view to ToC
    setStateVersion(v => v + 1);
  }, []);

  // Callback for document changes
  const handleDocChange = useCallback(() => {
    setStateVersion(v => v + 1);
  }, []);

  return (
    <main className="flex-1 flex flex-col min-h-0">
      {/* Header bar */}
      <header className="flex items-center justify-between px-4 py-3 bg-white shadow-sm border-b border-gray-200">
        <h1 className="text-lg font-semibold text-gray-900">Lens Editor</h1>
        <div className="flex items-center gap-4">
          <DebugYMapPanel />
          <SuggestionModeToggle view={editorView} />
          <SourceModeToggle editorView={editorView} />
          <PresencePanel />
          <SyncStatus />
        </div>
      </header>
      {/* Editor + Sidebars container */}
      <div className="flex-1 flex min-h-0">
        {/* Editor */}
        <div className="flex-1 flex flex-col min-w-0 bg-white">
          <div className="px-6 pt-5 pb-1">
            <DocumentTitle currentDocId={currentDocId} />
          </div>
          <div className="mx-6 border-b border-gray-200" />
          <div className="flex-1 min-h-0">
            <Editor
              readOnly={!canWrite}
              onEditorReady={handleEditorReady}
              onDocChange={handleDocChange}
              onNavigate={onNavigate}
              metadata={metadata}
            />
          </div>
        </div>
        {/* Right Sidebars */}
        <aside className="w-64 flex-shrink-0 border-l border-gray-200 bg-white flex flex-col">
          {/* ToC */}
          <div className="border-b border-gray-200 overflow-y-auto">
            <TableOfContents view={editorView} stateVersion={stateVersion} />
          </div>
          {/* Backlinks */}
          <div className="border-b border-gray-200 overflow-y-auto">
            <BacklinksPanel currentDocId={currentDocId} />
          </div>
          {/* Comments */}
          <div className="flex-1 overflow-y-auto">
            <CommentsPanel view={editorView} stateVersion={stateVersion} />
          </div>
        </aside>
        {/* Discussion panel - renders only when document has discussion frontmatter */}
        <ConnectedDiscussionPanel />
      </div>
    </main>
  );
}
