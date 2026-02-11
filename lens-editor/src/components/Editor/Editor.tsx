import { useEffect, useRef, useState, useMemo, useCallback } from 'react';
import { EditorView } from 'codemirror';
import {
  keymap,
  highlightSpecialChars,
  drawSelection,
  dropCursor,
  rectangularSelection,
  crosshairCursor,
} from '@codemirror/view';
import { EditorState } from '@codemirror/state';
import { history, defaultKeymap, historyKeymap } from '@codemirror/commands';
import { indentOnInput, syntaxHighlighting, defaultHighlightStyle, bracketMatching, foldKeymap } from '@codemirror/language';
import { closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete';
import { searchKeymap, highlightSelectionMatches } from '@codemirror/search';
import { completionKeymap } from '@codemirror/autocomplete';
import { lintKeymap } from '@codemirror/lint';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { WikilinkExtension } from './extensions/wikilinkParser';
import { indentMore, indentLess } from '@codemirror/commands';
import { yCollab } from 'y-codemirror.next';
import * as Y from 'yjs';
import { useYDoc, useYjsProvider } from '@y-sweet/react'
import { livePreview, updateWikilinkContext } from './extensions/livePreview';
import type { WikilinkContext } from './extensions/livePreview';
import { wikilinkAutocomplete } from './extensions/wikilinkAutocomplete';
import { remoteCursorTheme } from './remoteCursorTheme';
import { criticMarkupExtension } from './extensions/criticmarkup';
import { ContextMenu } from './ContextMenu';
import { getContextMenuItems } from './extensions/criticmarkup-context-menu';
import type { ContextMenuItem } from './extensions/criticmarkup-context-menu';
import { resolvePageName } from '../../lib/document-resolver';
import type { FolderMetadata } from '../../hooks/useFolderMetadata';
import { RELAY_ID } from '../../App';

// List indentation keymap - Tab/Shift+Tab to indent/de-indent
const listIndentKeymap = keymap.of([
  { key: 'Tab', run: indentMore },
  { key: 'Shift-Tab', run: indentLess },
]);

interface EditorProps {
  readOnly?: boolean;
  onEditorReady?: (view: EditorView) => void;
  onDocChange?: () => void;
  onNavigate?: (docId: string) => void;
  metadata?: FolderMetadata;
}

/**
 * Loading overlay shown while document syncs.
 * Renders on top of the editor so yCollab can bind from the start.
 */
function LoadingOverlay() {
  return (
    <div className="absolute inset-0 bg-white flex items-center justify-center z-10">
      <div className="flex flex-col items-center gap-3 text-gray-500">
        <svg
          className="w-8 h-8 animate-spin"
          fill="none"
          viewBox="0 0 24 24"
        >
          <circle
            className="opacity-25"
            cx="12"
            cy="12"
            r="10"
            stroke="currentColor"
            strokeWidth="4"
          />
          <path
            className="opacity-75"
            fill="currentColor"
            d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
          />
        </svg>
        <span className="text-sm">Loading document...</span>
      </div>
    </div>
  );
}

/**
 * Editor component with loading overlay.
 * Editor always renders so yCollab can sync initial content.
 * Loading overlay hides once synced.
 */
export function Editor({ readOnly, onEditorReady, onDocChange, onNavigate, metadata }: EditorProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const ydoc = useYDoc();
  const provider = useYjsProvider();
  const [synced, setSynced] = useState(false);
  const [contextMenu, setContextMenu] = useState<{
    items: ContextMenuItem[];
    position: { x: number; y: number };
  } | null>(null);

  // Store metadata in ref for autocomplete getter (avoids stale closures)
  const metadataRef = useRef<FolderMetadata | null>(null);
  metadataRef.current = metadata ?? null;

  // Stable getter function for autocomplete extension
  const getMetadata = useCallback(() => metadataRef.current, []);

  // Stable onClose callback to prevent effect re-runs
  const handleCloseContextMenu = useCallback(() => {
    setContextMenu(null);
  }, []);

  // Context menu handler - uses click position, not cursor position
  const handleContextMenu = useCallback(
    (e: React.MouseEvent) => {
      const view = viewRef.current;
      if (!view) return;

      // Convert click coordinates to editor document position
      const clickPos = view.posAtCoords({ x: e.clientX, y: e.clientY });
      if (clickPos === null) return;

      // Get items at click position (not cursor position)
      const items = getContextMenuItems(view, clickPos);
      if (items.length > 0) {
        e.preventDefault();
        setContextMenu({
          items,
          position: { x: e.clientX, y: e.clientY },
        });
      }
    },
    []
  );

  // Store wikilink context in ref to avoid re-creating editor when metadata changes
  // The livePreview extension uses a module-scoped variable that we update separately
  const wikilinkContextRef = useRef<WikilinkContext | undefined>(undefined);

  // Update the ref when dependencies change (doesn't cause re-render)
  wikilinkContextRef.current = useMemo((): WikilinkContext | undefined => {
    if (!metadata || !onNavigate) return undefined;

    return {
      onClick: (pageName: string) => {
        const resolved = resolvePageName(pageName, metadata);
        if (resolved) {
          onNavigate(`${RELAY_ID}-${resolved.docId}`);
        }
        // Unresolved wikilinks do nothing on click (document creation deferred)
      },
      isResolved: (pageName: string) => {
        return resolvePageName(pageName, metadata) !== null;
      },
    };
  }, [metadata, onNavigate]);

  // Update the module-scoped wikilink context when it changes
  // This is separate from the editor creation effect to avoid recreating the editor
  useEffect(() => {
    updateWikilinkContext(wikilinkContextRef.current);
  }, [metadata, onNavigate]);

  // Track sync state for loading overlay
  useEffect(() => {
    if ((provider as any).synced) {
      setSynced(true);
      return;
    }

    const handleSynced = () => setSynced(true);
    provider.on('synced', handleSynced);

    return () => {
      provider.off('synced', handleSynced);
    };
  }, [provider]);

  // Create editor - must happen before sync so yCollab binds initial content
  useEffect(() => {
    if (!containerRef.current || viewRef.current) return;

    // Get Y.Text for the editor content
    // Field name 'contents' matches Obsidian Relay document format
    const ytext = ydoc.getText('contents');

    // Create UndoManager with captureTimeout: 0
    // Note: Cross-user undo is a known limitation matching Obsidian+Relay behavior
    const undoManager = new Y.UndoManager(ytext, {
      captureTimeout: 0,
    });

    // Create EditorState with extensions
    // Custom setup based on basicSetup but without line numbers and active line highlighting
    const state = EditorState.create({
      extensions: [
        // Read-only mode for view-only users
        ...(readOnly ? [EditorView.editable.of(false), EditorState.readOnly.of(true)] : []),
        // Core editing
        highlightSpecialChars(),
        history(),
        drawSelection(),
        dropCursor(),
        EditorState.allowMultipleSelections.of(true),
        indentOnInput(),
        syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
        bracketMatching(),
        closeBrackets(),
        rectangularSelection(),
        crosshairCursor(),
        highlightSelectionMatches(),
        // Keymaps
        keymap.of([
          ...closeBracketsKeymap,
          ...defaultKeymap,
          ...searchKeymap,
          ...historyKeymap,
          ...foldKeymap,
          ...completionKeymap,
          ...lintKeymap,
        ]),
        markdown({
          base: markdownLanguage,
          extensions: [WikilinkExtension],
        }),
        livePreview(wikilinkContextRef.current),
        listIndentKeymap,
        yCollab(ytext, provider.awareness, { undoManager }),
        wikilinkAutocomplete(getMetadata),
        remoteCursorTheme,
        criticMarkupExtension(),
        EditorView.lineWrapping,
        EditorView.theme({
          '&': {
            height: '100%',
            fontSize: '14px',
            outline: 'none',
          },
          '&.cm-focused': {
            outline: 'none',
          },
          '.cm-scroller': {
            fontFamily: '"Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif',
            overflow: 'auto',
          },
          '.cm-content': {
            padding: '16px',
          },
          // Hide gutters (fold markers only, no line numbers)
          '.cm-gutters': {
            display: 'none',
          },
          // Remove underline from headings (override defaultHighlightStyle)
          '.tok-heading': {
            textDecoration: 'none !important',
          },
          '.cm-line .tok-heading': {
            textDecoration: 'none !important',
          },
        }),
        // Notify parent of document changes (for ToC updates)
        ...(onDocChange ? [EditorView.updateListener.of((update) => {
          if (update.docChanged) onDocChange();
        })] : []),
      ],
    });

    // Create EditorView
    const view = new EditorView({
      state,
      parent: containerRef.current,
    });

    viewRef.current = view;

    // Notify parent that editor is ready
    if (onEditorReady) {
      onEditorReady(view);
    }

    // Cleanup on unmount
    return () => {
      view.destroy();
      viewRef.current = null;
    };
  // Note: wikilinkContext is NOT a dependency - we update it via updateWikilinkContext()
  // to avoid recreating the editor (which would lose Y.Text sync state)
  }, [ydoc, provider, onEditorReady, onDocChange, readOnly]);

  return (
    <div className="relative h-full w-full">
      {!synced && <LoadingOverlay />}
      <div
        ref={containerRef}
        className="h-full w-full"
        onContextMenu={handleContextMenu}
      />
      {contextMenu && (
        <ContextMenu
          items={contextMenu.items}
          position={contextMenu.position}
          onClose={handleCloseContextMenu}
        />
      )}
    </div>
  );
}
