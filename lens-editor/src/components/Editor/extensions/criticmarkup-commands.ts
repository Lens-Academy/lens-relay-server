// src/components/Editor/extensions/criticmarkup-commands.ts
import type { EditorView, KeyBinding } from '@codemirror/view';
import { criticMarkupField } from './criticmarkup';
import type { CriticMarkupRange } from '../../../lib/criticmarkup-parser';

/**
 * Find the CriticMarkup range containing the given position.
 * Returns null if position is not inside any markup.
 */
export function findRangeAtPosition(view: EditorView, pos: number): CriticMarkupRange | null {
  const ranges = view.state.field(criticMarkupField);
  return ranges.find(r => pos >= r.from && pos <= r.to) ?? null;
}

/**
 * Find the CriticMarkup range containing the cursor position.
 */
function findRangeAtCursor(view: EditorView): CriticMarkupRange | null {
  return findRangeAtPosition(view, view.state.selection.main.head);
}

/**
 * Find all CriticMarkup ranges overlapping the current selection.
 * Returns empty array if selection is collapsed (cursor only).
 */
export function findRangesInSelection(view: EditorView): CriticMarkupRange[] {
  const sel = view.state.selection.main;
  if (sel.from === sel.to) return [];
  const ranges = view.state.field(criticMarkupField);
  return ranges.filter(r => r.from < sel.to && r.to > sel.from);
}

/**
 * Get the replacement text when accepting a CriticMarkup range.
 * Returns the content that should replace the entire markup.
 */
export function getAcceptReplacement(range: CriticMarkupRange): string {
  switch (range.type) {
    case 'addition':
      return range.content;
    case 'deletion':
      return ''; // Content is deleted
    case 'substitution':
      return range.newContent ?? '';
    case 'highlight':
      return range.content;
    case 'comment':
      return ''; // Comments are removed
    default:
      return '';
  }
}

/**
 * Get the replacement text when rejecting a CriticMarkup range.
 * Returns the content that should replace the entire markup.
 */
export function getRejectReplacement(range: CriticMarkupRange): string {
  switch (range.type) {
    case 'addition':
      return ''; // Addition is rejected, nothing added
    case 'deletion':
      return range.content; // Keep the "deleted" content
    case 'substitution':
      return range.oldContent ?? '';
    case 'highlight':
      return range.content;
    case 'comment':
      return ''; // Comments are removed either way
    default:
      return '';
  }
}

/**
 * Accept CriticMarkup changes. If a non-collapsed selection exists,
 * accepts all ranges overlapping the selection. Otherwise accepts
 * the single range at cursor position.
 * Returns true if any change was accepted.
 */
export function acceptChangeAtCursor(view: EditorView): boolean {
  const selected = findRangesInSelection(view);
  if (selected.length > 0) {
    const changes = selected.map(r => ({
      from: r.from, to: r.to, insert: getAcceptReplacement(r),
    }));
    view.dispatch({ changes });
    return true;
  }

  const range = findRangeAtCursor(view);
  if (!range) return false;

  view.dispatch({
    changes: { from: range.from, to: range.to, insert: getAcceptReplacement(range) },
  });
  return true;
}

/**
 * Reject CriticMarkup changes. If a non-collapsed selection exists,
 * rejects all ranges overlapping the selection. Otherwise rejects
 * the single range at cursor position.
 * Returns true if any change was rejected.
 */
export function rejectChangeAtCursor(view: EditorView): boolean {
  const selected = findRangesInSelection(view);
  if (selected.length > 0) {
    const changes = selected.map(r => ({
      from: r.from, to: r.to, insert: getRejectReplacement(r),
    }));
    view.dispatch({ changes });
    return true;
  }

  const range = findRangeAtCursor(view);
  if (!range) return false;

  view.dispatch({
    changes: { from: range.from, to: range.to, insert: getRejectReplacement(range) },
  });
  return true;
}

/**
 * Keymap for CriticMarkup accept/reject.
 * - Mod-Enter: Accept change at cursor
 * - Mod-Backspace: Reject change at cursor
 */
export const criticMarkupKeymap: KeyBinding[] = [
  {
    key: 'Mod-Enter',
    run: acceptChangeAtCursor,
  },
  {
    key: 'Mod-Backspace',
    run: rejectChangeAtCursor,
  },
];
