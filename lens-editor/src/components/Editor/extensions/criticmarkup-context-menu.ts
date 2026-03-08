// src/components/Editor/extensions/criticmarkup-context-menu.ts
import type { EditorView } from '@codemirror/view';
import { findRangeAtPosition, findRangesInSelection, acceptChangeAtCursor, rejectChangeAtCursor } from './criticmarkup-commands';

export interface ContextMenuItem {
  label: string;
  action: () => void;
  shortcut?: string;
}

/**
 * Get context menu items for CriticMarkup.
 *
 * @param view - The EditorView instance
 * @param atPosition - Optional position to check (for right-click). If not provided, uses cursor position.
 * @returns Menu items if position is inside markup, empty array otherwise.
 */
export function getContextMenuItems(view: EditorView, atPosition?: number): ContextMenuItem[] {
  // Check for bulk selection first
  const selectedRanges = findRangesInSelection(view);
  if (selectedRanges.length > 1) {
    const count = selectedRanges.length;
    return [
      {
        label: `Accept ${count} Changes`,
        action: () => acceptChangeAtCursor(view),
        shortcut: 'Ctrl+Enter',
      },
      {
        label: `Reject ${count} Changes`,
        action: () => rejectChangeAtCursor(view),
        shortcut: 'Ctrl+Backspace',
      },
    ];
  }

  const pos = atPosition ?? view.state.selection.main.head;
  const range = findRangeAtPosition(view, pos);

  if (!range) {
    return [];
  }

  // If we're checking a click position different from cursor,
  // first move cursor to that position so accept/reject work correctly
  const needsMoveCursor = atPosition !== undefined && atPosition !== view.state.selection.main.head;

  return [
    {
      label: 'Accept Change',
      action: () => {
        if (needsMoveCursor) {
          view.dispatch({ selection: { anchor: pos } });
        }
        acceptChangeAtCursor(view);
      },
      shortcut: 'Ctrl+Enter',
    },
    {
      label: 'Reject Change',
      action: () => {
        if (needsMoveCursor) {
          view.dispatch({ selection: { anchor: pos } });
        }
        rejectChangeAtCursor(view);
      },
      shortcut: 'Ctrl+Backspace',
    },
  ];
}
