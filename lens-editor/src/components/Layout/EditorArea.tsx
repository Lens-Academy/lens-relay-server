import { useState, useCallback, useMemo } from 'react';
import { createPortal } from 'react-dom';
import { EditorView } from '@codemirror/view';
import { SyncStatus } from '../SyncStatus/SyncStatus';
import { Editor } from '../Editor/Editor';
import { DocumentTitle } from '../DocumentTitle';
import { SourceModeToggle } from '../SourceModeToggle/SourceModeToggle';
import { SuggestionModeToggle } from '../SuggestionModeToggle/SuggestionModeToggle';
import { PresencePanel } from '../PresencePanel/PresencePanel';
import { OverflowMenu } from '../OverflowMenu';
import { TableOfContents } from '../TableOfContents';
import { BacklinksPanel } from '../BacklinksPanel';
import { CommentMargin } from '../CommentMargin';
import { DebugYMapPanel } from '../DebugYMapPanel';
import { PanelDebugOverlay } from '../PanelDebugOverlay';
import { ConnectedDiscussionPanel } from '../DiscussionPanel';
import { ResizeHandle } from './ResizeHandle';
import { useHasDiscussion } from '../DiscussionPanel/useHasDiscussion';
import { useNavigation } from '../../contexts/NavigationContext';
import { useAuth } from '../../contexts/AuthContext';
import { useSidebar } from '../../contexts/SidebarContext';
import { findPathByUuid } from '../../lib/uuid-to-path';
import { pathToSegments } from '../../lib/path-display';
import { RELAY_ID, PANEL_CONFIG } from '../../App';

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
  const { manager, headerStage } = useSidebar();
  const hasDiscussion = useHasDiscussion();
  const [addCommentTrigger, setAddCommentTrigger] = useState(0);

  // Derive current file path from doc ID for wikilink resolution
  const currentFilePath = useMemo(() => {
    if (!metadata || !Object.keys(metadata).length) return undefined;
    const uuid = currentDocId.slice(RELAY_ID.length + 1);
    return findPathByUuid(uuid, metadata) ?? undefined;
  }, [currentDocId, metadata]);

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

  // Callback for "Add Comment" from editor context menu
  const handleRequestAddComment = useCallback(() => {
    manager.expand('comment-margin');
    setAddCommentTrigger(v => v + 1);
  }, [manager.expand]);

  // Local state for ToC/Backlinks vertical split inside right sidebar
  const [tocHeight, setTocHeight] = useState(200);

  // Portal targets in the global header
  const breadcrumbTarget = document.getElementById('header-breadcrumb');
  const portalTarget = document.getElementById('header-controls');
  const discussionToggleTarget = document.getElementById('header-discussion-toggle');
  const rightCollapsed = manager.collapsedState['right-sidebar'] ?? false;
  const commentMarginCollapsed = manager.collapsedState['comment-margin'] ?? false;
  const discussionCollapsed = manager.collapsedState['discussion'] ?? true;

  return (
    <main className="h-full flex flex-col min-h-0">
      {/* Portal breadcrumbs into global header */}
      {breadcrumbTarget && (() => {
        const segments = pathToSegments(currentFilePath);
        if (segments.length === 0) return null;
        return createPortal(
          <span className="text-sm text-gray-600 truncate">
            {segments.map((seg, i) => (
              <span key={i}>
                {i > 0 && <span className="mx-0.5">›</span>}
                {seg}
              </span>
            ))}
          </span>,
          breadcrumbTarget
        );
      })()}
      {/* Portal editor controls into global header */}
      {portalTarget && createPortal(
        <>
          <PanelDebugOverlay config={PANEL_CONFIG} manager={manager} />
          {headerStage === 'overflow' ? (
            <OverflowMenu>
              <SuggestionModeToggle view={editorView} iconOnly />
              <SourceModeToggle editorView={editorView} />
              <PresencePanel />
              <SyncStatus />
            </OverflowMenu>
          ) : (
            <>
              <DebugYMapPanel />
              <SuggestionModeToggle view={editorView} iconOnly={headerStage !== 'full'} />
              <SourceModeToggle editorView={editorView} />
              <PresencePanel />
              <SyncStatus />
            </>
          )}
        </>,
        portalTarget
      )}
      {/* Portal Discord toggle into global header — only when doc has discussion */}
      {discussionToggleTarget && hasDiscussion && createPortal(
        <button
          onClick={() => manager.toggle('discussion')}
          title="Toggle discussion"
          className="cursor-pointer text-gray-600 hover:text-gray-700 transition-colors"
        >
          <svg className="w-[22px] h-[22px]" viewBox="0 0 24 24" fill="currentColor" opacity={discussionCollapsed ? 0.2 : 0.45}>
            <path d="M19.73 4.87a18.2 18.2 0 0 0-4.6-1.44c-.2.36-.43.85-.59 1.23a16.84 16.84 0 0 0-5.07 0c-.16-.38-.4-.87-.6-1.23a18.17 18.17 0 0 0-4.6 1.44A19.25 19.25 0 0 0 .96 18.06a18.32 18.32 0 0 0 5.63 2.87c.46-.62.86-1.28 1.2-1.98a11.83 11.83 0 0 1-1.89-.91c.16-.12.31-.24.46-.37a12.97 12.97 0 0 0 11.28 0c.15.13.3.25.46.37-.6.36-1.23.67-1.9.92.35.7.75 1.35 1.2 1.97a18.27 18.27 0 0 0 5.63-2.87A19.22 19.22 0 0 0 19.73 4.87ZM8.3 15.12c-1.18 0-2.16-1.1-2.16-2.44 0-1.34.95-2.44 2.16-2.44 1.2 0 2.18 1.1 2.16 2.44 0 1.34-.95 2.44-2.16 2.44Zm7.4 0c-1.18 0-2.16-1.1-2.16-2.44 0-1.34.95-2.44 2.16-2.44 1.2 0 2.18 1.1 2.16 2.44 0 1.34-.96 2.44-2.16 2.44Z" />
          </svg>
        </button>,
        discussionToggleTarget
      )}
      {/* Editor + Sidebars container — CSS flexbox with pixel widths */}
      <div id="editor-area" className="flex-1 flex min-h-0">
        {/* Editor fills remaining space */}
        <div id="editor" className="flex-1 flex flex-col min-w-0 bg-white" style={{ minWidth: 250 }}>
          <div className="max-w-[700px] mx-auto w-full">
            <div className="px-6 pt-5 pb-1">
              <DocumentTitle currentDocId={currentDocId} />
            </div>
            <div className="mx-6 border-b border-gray-200" />
          </div>
          <div className="flex-1 min-h-0">
            <Editor
              readOnly={!canWrite}
              onEditorReady={handleEditorReady}
              onDocChange={handleDocChange}
              onNavigate={onNavigate}
              onRequestAddComment={handleRequestAddComment}
              metadata={metadata}
              currentFilePath={currentFilePath}
            />
          </div>
        </div>

        {/* Comment margin — always rendered, width 0 when collapsed */}
        <ResizeHandle
          onDragStart={() => manager.getWidth('comment-margin')}
          onDrag={(size) => manager.setWidth('comment-margin', size)}
          onDragEnd={() => manager.onDragEnd('comment-margin')}
          disabled={commentMarginCollapsed}
        />
        <div
          id="comment-margin"
          className={`overflow-hidden flex-shrink-0 ${commentMarginCollapsed ? '' : 'border-l border-gray-100 bg-gray-50/50'}`}
          style={{ width: commentMarginCollapsed ? 0 : manager.getWidth('comment-margin') }}
        >
          {editorView && (
            <CommentMargin
              view={editorView}
              stateVersion={stateVersion}
              addCommentTrigger={addCommentTrigger}
            />
          )}
        </div>

        {/* Right sidebar — always rendered, width 0 when collapsed */}
        <ResizeHandle
          onDragStart={() => manager.getWidth('right-sidebar')}
          onDrag={(size) => manager.setWidth('right-sidebar', size)}
          onDragEnd={() => manager.onDragEnd('right-sidebar')}
          disabled={rightCollapsed}
        />
        <div
          id="right-sidebar"
          className="overflow-hidden flex-shrink-0 bg-[#f6f6f6] flex flex-col"
          style={{ width: rightCollapsed ? 0 : manager.getWidth('right-sidebar') }}
        >
          <div style={{ height: tocHeight, flexShrink: 0 }} className="overflow-y-auto">
            <TableOfContents view={editorView} stateVersion={stateVersion} />
          </div>
          <ResizeHandle
            orientation="horizontal"
            onDragStart={() => tocHeight}
            onDrag={(size) => setTocHeight(Math.max(50, size))}
          />
          <div className="flex-1 min-h-0 overflow-y-auto">
            <BacklinksPanel currentDocId={currentDocId} />
          </div>
        </div>

        {/* Discussion — conditionally rendered (only when doc has discussion) */}
        {hasDiscussion && (
          <>
            <ResizeHandle
              onDragStart={() => manager.getWidth('discussion')}
              onDrag={(size) => manager.setWidth('discussion', size)}
              onDragEnd={() => manager.onDragEnd('discussion')}
              disabled={discussionCollapsed}
            />
            <div
              id="discussion"
              className="overflow-hidden flex-shrink-0"
              style={{ width: discussionCollapsed ? 0 : manager.getWidth('discussion') }}
            >
              <ConnectedDiscussionPanel />
            </div>
          </>
        )}
      </div>
    </main>
  );
}
