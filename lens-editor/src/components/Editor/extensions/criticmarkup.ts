// src/components/Editor/extensions/criticmarkup.ts
import {
  StateField,
  StateEffect,
  RangeSetBuilder,
  EditorSelection,
  EditorState,
  Compartment,
  Transaction,
} from '@codemirror/state';
import type { ChangeSpec } from '@codemirror/state';
import {
  ViewPlugin,
  Decoration,
  EditorView,
  keymap,
  WidgetType,
} from '@codemirror/view';
import type { ViewUpdate, DecorationSet } from '@codemirror/view';
import { criticMarkupKeymap, acceptChangeAtCursor, rejectChangeAtCursor, findRangesInSelection } from './criticmarkup-commands';
import { parse, parseThreads, type CriticMarkupRange } from '../../../lib/criticmarkup-parser';

// Author context - can be set externally
let currentAuthor = 'anonymous';

/**
 * Set the current author for CriticMarkup metadata.
 * This is used when wrapping edits in suggestion mode.
 */
export function setCurrentAuthor(author: string) {
  currentAuthor = author;
}

/**
 * Get the current author for CriticMarkup metadata.
 */
export function getCurrentAuthor(): string {
  return currentAuthor;
}

// CSS class mapping for each CriticMarkup type
const TYPE_CLASSES: Record<CriticMarkupRange['type'], string> = {
  addition: 'cm-addition',
  deletion: 'cm-deletion',
  substitution: 'cm-substitution',
  comment: 'cm-comment',
  highlight: 'cm-highlight',
};

// Line-level CSS classes for blank lines within suggestions
const LINE_CLASSES: Partial<Record<CriticMarkupRange['type'], string>> = {
  addition: 'cm-addition-line',
  deletion: 'cm-deletion-line',
};

/**
 * StateEffect to toggle suggestion mode on/off.
 */
export const toggleSuggestionMode = StateEffect.define<boolean>();

/**
 * StateEffect dispatched when a comment badge is clicked.
 * Carries the thread's `from` position for focusing the comment card.
 */
export const focusCommentThread = StateEffect.define<number | null>();

/**
 * StateField tracking which comment thread is focused (by `from` position).
 * Updated when a comment badge is clicked or a card is focused in the margin.
 */
export const focusedThreadField = StateField.define<number | null>({
  create() { return null; },
  update(value, tr) {
    for (const e of tr.effects) {
      if (e.is(focusCommentThread)) return e.value;
    }
    if (value !== null && tr.docChanged) {
      return tr.changes.mapPos(value);
    }
    return value;
  },
});

// Superscript digit characters for badge numbers
const SUPERSCRIPT_DIGITS = ['\u2070', '\u00B9', '\u00B2', '\u00B3', '\u2074', '\u2075', '\u2076', '\u2077', '\u2078', '\u2079'];

function toSuperscript(n: number): string {
  return String(n).split('').map(d => SUPERSCRIPT_DIGITS[parseInt(d)]).join('');
}

/**
 * Widget that renders a numbered badge for comment threads.
 */
class CommentBadgeWidget extends WidgetType {
  constructor(
    private number: number,
    private threadFrom: number,
  ) {
    super();
  }

  toDOM(): HTMLElement {
    const span = document.createElement('span');
    span.className = 'cm-comment-badge';
    span.textContent = String(this.number);
    span.dataset.threadFrom = String(this.threadFrom);
    return span;
  }

  eq(other: CommentBadgeWidget): boolean {
    return this.number === other.number && this.threadFrom === other.threadFrom;
  }

  ignoreEvent(): boolean {
    return false; // Allow click events
  }
}

/**
 * StateField that tracks whether suggestion mode is active.
 * When true, edits are wrapped in CriticMarkup instead of being applied directly.
 */
export const suggestionModeField = StateField.define<boolean>({
  create() {
    return false;
  },
  update(value, tr) {
    for (const effect of tr.effects) {
      if (effect.is(toggleSuggestionMode)) {
        return effect.value;
      }
    }
    return value;
  },
});

/**
 * StateField that holds parsed CriticMarkup ranges from the document.
 * Ranges are re-parsed whenever the document changes.
 */
export const criticMarkupField = StateField.define<CriticMarkupRange[]>({
  create(state) {
    return parse(state.doc.toString());
  },
  update(ranges, transaction) {
    if (!transaction.docChanged) return ranges;
    return parse(transaction.state.doc.toString());
  },
});

/**
 * Helper function to check if the editor selection intersects a given range.
 */
function selectionIntersects(
  selection: EditorSelection,
  from: number,
  to: number
): boolean {
  return selection.ranges.some((range) => range.to >= from && range.from <= to);
}

/**
 * Widget that renders a zero-width space to give hidden-syntax-only lines
 * a proper line height so the cursor remains visible.
 */
class CursorAnchorWidget extends WidgetType {
  toDOM(): HTMLElement {
    const span = document.createElement('span');
    span.textContent = '\u200B'; // zero-width space
    span.className = 'cm-cursor-anchor';
    return span;
  }

  eq(): boolean {
    return true; // All instances are equivalent
  }
}

/**
 * Widget that renders accept (checkmark) and reject (X) buttons for CriticMarkup.
 * Appears at the end of markup content when cursor is inside.
 *
 * Uses data attributes for range identification - click handlers are
 * attached via event delegation in the ViewPlugin.
 */
class AcceptRejectWidget extends WidgetType {
  constructor(
    private rangeFrom: number,
    private rangeTo: number
  ) {
    super();
  }

  toDOM(): HTMLElement {
    const container = document.createElement('span');
    container.className = 'cm-criticmarkup-buttons';
    container.dataset.rangeFrom = String(this.rangeFrom);
    container.dataset.rangeTo = String(this.rangeTo);

    const acceptBtn = document.createElement('button');
    acceptBtn.className = 'cm-criticmarkup-accept';
    acceptBtn.textContent = '\u2713'; // checkmark
    acceptBtn.title = 'Accept change (Ctrl+Enter)';
    acceptBtn.setAttribute('aria-label', 'Accept change');

    const rejectBtn = document.createElement('button');
    rejectBtn.className = 'cm-criticmarkup-reject';
    rejectBtn.textContent = '\u2717'; // X mark
    rejectBtn.title = 'Reject change (Ctrl+Backspace)';
    rejectBtn.setAttribute('aria-label', 'Reject change');

    container.appendChild(acceptBtn);
    container.appendChild(rejectBtn);

    return container;
  }

  eq(other: AcceptRejectWidget): boolean {
    // Widgets are equal if they represent the same range
    return this.rangeFrom === other.rangeFrom && this.rangeTo === other.rangeTo;
  }

  ignoreEvent(): boolean {
    return false; // Allow click events
  }
}

/**
 * Widget that renders accept/reject buttons with a count badge for bulk operations.
 * Shown when a text selection spans multiple CriticMarkup ranges.
 */
class BulkAcceptRejectWidget extends WidgetType {
  constructor(private count: number) {
    super();
  }

  toDOM(): HTMLElement {
    const container = document.createElement('span');
    container.className = 'cm-criticmarkup-buttons cm-criticmarkup-bulk';

    const acceptBtn = document.createElement('button');
    acceptBtn.className = 'cm-criticmarkup-accept';
    acceptBtn.textContent = `\u2713 ${this.count}`;
    acceptBtn.title = `Accept ${this.count} changes (Ctrl+Enter)`;
    acceptBtn.setAttribute('aria-label', `Accept ${this.count} changes`);

    const rejectBtn = document.createElement('button');
    rejectBtn.className = 'cm-criticmarkup-reject';
    rejectBtn.textContent = `\u2717 ${this.count}`;
    rejectBtn.title = `Reject ${this.count} changes (Ctrl+Backspace)`;
    rejectBtn.setAttribute('aria-label', `Reject ${this.count} changes`);

    container.appendChild(acceptBtn);
    container.appendChild(rejectBtn);

    return container;
  }

  eq(other: BulkAcceptRejectWidget): boolean {
    return this.count === other.count;
  }

  ignoreEvent(): boolean {
    return false;
  }
}

/**
 * ViewPlugin that applies decorations (CSS classes) to CriticMarkup ranges.
 * Decorations are rebuilt when the document changes, viewport changes, or selection changes.
 * Delimiters and metadata are always hidden; accept/reject buttons shown when cursor is inside.
 */
export const criticMarkupPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;

    // Saved selection from mousedown, used for bulk accept/reject
    // (clicking a button collapses the selection before the click handler fires)
    private savedSelection: { from: number; to: number } | null = null;

    constructor(view: EditorView) {
      this.decorations = this.buildDecorations(view);

      // Capture selection on mousedown before the browser collapses it
      view.contentDOM.addEventListener('mousedown', (e) => {
        const target = e.target as HTMLElement;
        if (target.classList.contains('cm-criticmarkup-accept') ||
            target.classList.contains('cm-criticmarkup-reject')) {
          const sel = view.state.selection.main;
          if (sel.from !== sel.to) {
            this.savedSelection = { from: sel.from, to: sel.to };
            e.preventDefault(); // Prevent selection collapse
          }
        }
      });

      // Event delegation for accept/reject button clicks and comment badge clicks
      view.contentDOM.addEventListener('click', (e) => {
        const target = e.target as HTMLElement;
        if (target.classList.contains('cm-criticmarkup-accept')) {
          e.preventDefault();
          e.stopPropagation();
          if (this.savedSelection) {
            // Restore the selection so bulk accept works
            view.dispatch({ selection: { anchor: this.savedSelection.from, head: this.savedSelection.to } });
            this.savedSelection = null;
          }
          acceptChangeAtCursor(view);
        } else if (target.classList.contains('cm-criticmarkup-reject')) {
          e.preventDefault();
          e.stopPropagation();
          if (this.savedSelection) {
            view.dispatch({ selection: { anchor: this.savedSelection.from, head: this.savedSelection.to } });
            this.savedSelection = null;
          }
          rejectChangeAtCursor(view);
        } else if (target.classList.contains('cm-comment-badge')) {
          e.preventDefault();
          e.stopPropagation();
          const currentFocused = view.state.field(focusedThreadField);
          const threadFrom = parseInt(target.dataset.threadFrom ?? '', 10);
          if (!isNaN(threadFrom)) {
            view.dispatch({ effects: focusCommentThread.of(currentFocused === threadFrom ? null : threadFrom) });
          }
        }
      });
    }

    update(update: ViewUpdate) {
      if (update.docChanged || update.viewportChanged || update.selectionSet) {
        this.decorations = this.buildDecorations(update.view);
        return;
      }
      for (const tr of update.transactions) {
        for (const e of tr.effects) {
          if (e.is(focusCommentThread)) {
            this.decorations = this.buildDecorations(update.view);
            return;
          }
        }
      }
    }

    buildDecorations(view: EditorView): DecorationSet {
      const builder = new RangeSetBuilder<Decoration>();
      const ranges = view.state.field(criticMarkupField);
      const selection = view.state.selection;

      // Collect all decorations to sort before adding
      const decorations: Array<{ from: number; to: number; deco: Decoration }> = [];
      // Line decorations are added separately (must not be mixed into mark sort)
      const lineDecos: Array<{ from: number; deco: Decoration }> = [];

      // Build comment thread info for badge numbering
      const threads = parseThreads(ranges);
      // Map each comment range's `from` to its badge info
      const commentBadgeMap = new Map<number, { badgeNumber: number; isFirst: boolean; threadFrom: number }>();
      for (let ti = 0; ti < threads.length; ti++) {
        const thread = threads[ti];
        for (let ci = 0; ci < thread.comments.length; ci++) {
          commentBadgeMap.set(thread.comments[ci].from, {
            badgeNumber: ti + 1,
            isFirst: ci === 0,
            threadFrom: thread.from,
          });
        }
      }

      // Focused comment thread highlight
      const focusedFrom = view.state.field(focusedThreadField);
      if (focusedFrom !== null) {
        const focusedThread = threads.find(t => t.from === focusedFrom);
        if (focusedThread) {
          const doc = view.state.doc;
          const startLine = doc.lineAt(focusedThread.from).number;
          const endLine = doc.lineAt(focusedThread.to).number;
          for (let ln = startLine; ln <= endLine; ln++) {
            const line = doc.line(ln);
            lineDecos.push({
              from: line.from,
              deco: Decoration.line({ class: 'cm-comment-focused-line' }),
            });
          }
        }
      }

      // Detect bulk selection: if selection spans multiple markup ranges,
      // show a single bulk widget instead of per-range accept/reject buttons
      const sel = selection.main;
      const bulkRanges = sel.from !== sel.to
        ? ranges.filter(r => r.from < sel.to && r.to > sel.from)
        : [];
      const isBulkSelection = bulkRanges.length > 1;

      if (isBulkSelection) {
        decorations.push({
          from: sel.to,
          to: sel.to,
          deco: Decoration.widget({
            widget: new BulkAcceptRejectWidget(bulkRanges.length),
            side: 1,
          }),
        });
      }

      for (const range of ranges) {
        const className = TYPE_CLASSES[range.type];
        const cursorInside = selectionIntersects(selection, range.from, range.to);

        // Comments: hide all text, show badge widget for first in thread
        if (range.type === 'comment') {
          const badgeInfo = commentBadgeMap.get(range.from);
          const spansLineBreak = view.state.doc.lineAt(range.from).number !== view.state.doc.lineAt(range.to).number;

          if (spansLineBreak) {
            // Multiline comments can't use Decoration.replace (CM6 plugin limitation).
            // Hide all text with cm-hidden-syntax marks and place badge widget at start.
            decorations.push({
              from: range.from,
              to: range.to,
              deco: Decoration.mark({ class: 'cm-hidden-syntax' }),
            });
            if (badgeInfo?.isFirst) {
              decorations.push({
                from: range.from,
                to: range.from,
                deco: Decoration.widget({
                  widget: new CommentBadgeWidget(badgeInfo.badgeNumber, badgeInfo.threadFrom),
                  side: -1,
                }),
              });
            }

            // Collapse intermediate ghost lines (lines entirely within hidden comment range)
            const doc = view.state.doc;
            const startLine = doc.lineAt(range.from).number;
            const endLine = doc.lineAt(range.to).number;
            for (let ln = startLine; ln <= endLine; ln++) {
              const line = doc.line(ln);
              const lineFullyHidden = line.from >= range.from && line.to <= range.to;
              if (lineFullyHidden) {
                lineDecos.push({
                  from: line.from,
                  deco: Decoration.line({ class: 'cm-comment-collapsed-line' }),
                });
              }
            }
          } else if (badgeInfo?.isFirst) {
            // First comment in thread (single line): replace entire range with badge
            decorations.push({
              from: range.from,
              to: range.to,
              deco: Decoration.replace({
                widget: new CommentBadgeWidget(badgeInfo.badgeNumber, badgeInfo.threadFrom),
              }),
            });
          } else {
            // Non-first comment in thread (single line): hide entirely
            decorations.push({
              from: range.from,
              to: range.to,
              deco: Decoration.replace({}),
            });
          }
          continue;
        }

        // Non-comment ranges: standard three-decoration pattern
        // Always hide delimiters and metadata, only show colored content
        // Opening delimiter + metadata (everything before content)
        decorations.push({
          from: range.from,
          to: range.contentFrom,
          deco: Decoration.mark({ class: 'cm-hidden-syntax' }),
        });

        // Content (between delimiters)
        decorations.push({
          from: range.contentFrom,
          to: range.contentTo,
          deco: Decoration.mark({ class: className }),
        });

        // Closing delimiter
        decorations.push({
          from: range.contentTo,
          to: range.to,
          deco: Decoration.mark({ class: 'cm-hidden-syntax' }),
        });

        // Add colored left border on lines within additions/deletions that have
        // no visible content (blank lines, or lines where all text is hidden syntax)
        const lineClass = LINE_CLASSES[range.type];
        if (lineClass) {
          const doc = view.state.doc;
          // Iterate ALL lines in the markup range (including delimiter lines)
          const startLine = doc.lineAt(range.from).number;
          const endLine = doc.lineAt(range.to).number;
          for (let ln = startLine; ln <= endLine; ln++) {
            const line = doc.line(ln);
            // Check if this line has any visible suggestion content
            const visibleFrom = Math.max(line.from, range.contentFrom);
            const visibleTo = Math.min(line.to, range.contentTo);
            if (visibleFrom >= visibleTo || doc.sliceString(visibleFrom, visibleTo).trim() === '') {
              lineDecos.push({
                from: line.from,
                deco: Decoration.line({ class: lineClass }),
              });
            }
          }
        }

        // When cursor is inside, show accept/reject buttons and ensure cursor visibility
        // Skip per-range buttons when bulk widget is shown
        if (cursorInside && !isBulkSelection) {
          decorations.push({
            from: range.contentTo,
            to: range.contentTo,
            deco: Decoration.widget({
              widget: new AcceptRejectWidget(range.from, range.to),
              side: 1, // After the content
            }),
          });

          // If cursor is on a line where all text is hidden syntax (delimiter lines),
          // add a zero-width space widget so the cursor has proper height
          const cursorHead = view.state.selection.main.head;
          const cursorLine = view.state.doc.lineAt(cursorHead);
          const visFrom = Math.max(cursorLine.from, range.contentFrom);
          const visTo = Math.min(cursorLine.to, range.contentTo);
          const lineIsAllHidden = visFrom >= visTo || view.state.doc.sliceString(visFrom, visTo).trim() === '';
          if (lineIsAllHidden && cursorHead >= range.from && cursorHead <= range.to) {
            decorations.push({
              from: cursorHead,
              to: cursorHead,
              deco: Decoration.widget({
                widget: new CursorAnchorWidget(),
                side: 0,
              }),
            });
          }
        }
      }

      // Merge line decorations into the main array
      for (const ld of lineDecos) {
        decorations.push({ from: ld.from, to: ld.from, deco: ld.deco });
      }

      // Sort by position and startSide (required for RangeSetBuilder)
      // Line decos have startSide -200, marks -1, widgets vary
      decorations.sort((a, b) => {
        if (a.from !== b.from) return a.from - b.from;
        const aSide = (a.deco as any).startSide ?? 0;
        const bSide = (b.deco as any).startSide ?? 0;
        if (aSide !== bSide) return aSide - bSide;
        return a.to - b.to;
      });

      for (const d of decorations) {
        builder.add(d.from, d.to, d.deco);
      }

      return builder.finish();
    }
  },
  {
    decorations: (v) => v.decorations,
  }
);

/**
 * ViewPlugin for source mode - applies color classes to entire CriticMarkup ranges
 * without hiding any syntax. Shows raw markup with color coding.
 * Accept/reject buttons appear when cursor is inside a range.
 */
export const criticMarkupSourcePlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    private savedSelection: { from: number; to: number } | null = null;

    constructor(view: EditorView) {
      this.decorations = this.buildDecorations(view);

      // Capture selection on mousedown before the browser collapses it
      view.contentDOM.addEventListener('mousedown', (e) => {
        const target = e.target as HTMLElement;
        if (target.classList.contains('cm-criticmarkup-accept') ||
            target.classList.contains('cm-criticmarkup-reject')) {
          const sel = view.state.selection.main;
          if (sel.from !== sel.to) {
            this.savedSelection = { from: sel.from, to: sel.to };
            e.preventDefault();
          }
        }
      });

      // Event delegation for accept/reject button clicks
      view.contentDOM.addEventListener('click', (e) => {
        const target = e.target as HTMLElement;
        if (target.classList.contains('cm-criticmarkup-accept')) {
          e.preventDefault();
          e.stopPropagation();
          if (this.savedSelection) {
            view.dispatch({ selection: { anchor: this.savedSelection.from, head: this.savedSelection.to } });
            this.savedSelection = null;
          }
          acceptChangeAtCursor(view);
        } else if (target.classList.contains('cm-criticmarkup-reject')) {
          e.preventDefault();
          e.stopPropagation();
          if (this.savedSelection) {
            view.dispatch({ selection: { anchor: this.savedSelection.from, head: this.savedSelection.to } });
            this.savedSelection = null;
          }
          rejectChangeAtCursor(view);
        }
      });
    }

    update(update: ViewUpdate) {
      if (update.docChanged || update.selectionSet) {
        this.decorations = this.buildDecorations(update.view);
      }
    }

    buildDecorations(view: EditorView): DecorationSet {
      const builder = new RangeSetBuilder<Decoration>();
      const ranges = view.state.field(criticMarkupField);
      const selection = view.state.selection;

      const decorations: Array<{ from: number; to: number; deco: Decoration }> = [];

      // Detect bulk selection
      const sel = selection.main;
      const bulkRanges = sel.from !== sel.to
        ? ranges.filter(r => r.from < sel.to && r.to > sel.from)
        : [];
      const isBulkSelection = bulkRanges.length > 1;

      if (isBulkSelection) {
        decorations.push({
          from: sel.to,
          to: sel.to,
          deco: Decoration.widget({
            widget: new BulkAcceptRejectWidget(bulkRanges.length),
            side: 1,
          }),
        });
      }

      for (const range of ranges) {
        // Only decorate the content, not the delimiters/metadata
        decorations.push({
          from: range.contentFrom,
          to: range.contentTo,
          deco: Decoration.mark({ class: TYPE_CLASSES[range.type] }),
        });

        if (selectionIntersects(selection, range.from, range.to) && !isBulkSelection) {
          decorations.push({
            from: range.contentTo,
            to: range.contentTo,
            deco: Decoration.widget({
              widget: new AcceptRejectWidget(range.from, range.to),
              side: 1,
            }),
          });
        }
      }

      decorations.sort((a, b) => {
        if (a.from !== b.from) return a.from - b.from;
        if (a.to !== b.to) return a.to - b.to;
        const aIsWidget = a.from === a.to;
        const bIsWidget = b.from === b.to;
        if (aIsWidget !== bIsWidget) return aIsWidget ? 1 : -1;
        return 0;
      });

      for (const d of decorations) {
        builder.add(d.from, d.to, d.deco);
      }

      return builder.finish();
    }
  },
  {
    decorations: (v) => v.decorations,
  }
);

/**
 * Transaction filter that wraps insertions in CriticMarkup when suggestion mode is ON.
 * Insertions are wrapped in {++metadata@@content++} format.
 *
 * Continuous typing optimization: If cursor is inside an existing addition by the same
 * author, let the edit through without wrapping to avoid per-character markup.
 */
const suggestionModeFilter = EditorState.transactionFilter.of((tr: Transaction) => {
  // Only process document changes
  if (!tr.docChanged) return tr;

  // Only wrap when suggestion mode is ON
  if (!tr.startState.field(suggestionModeField)) return tr;

  // Only wrap user-initiated edits (typing, paste, delete).
  // Remote sync (y-codemirror.next) and programmatic dispatches don't set userEvent.
  if (!tr.annotation(Transaction.userEvent)) return tr;

  const cursorPos = tr.startState.selection.main.head;
  const ranges = tr.startState.field(criticMarkupField);

  // Check if cursor is inside an existing addition by the same author
  const ownAddition = ranges.find(
    (r) =>
      r.type === 'addition' &&
      r.metadata?.author === currentAuthor &&
      cursorPos > r.from &&
      cursorPos < r.to
  );

  // If inside own addition, let the edit through — unless it empties the content
  // or escapes the content boundaries (e.g. backspace at left edge)
  if (ownAddition) {
    let wouldEmpty = false;
    let escapesLeft = false;
    let escapesRight = false;
    tr.changes.iterChanges((fromA, toA, _fromB, _toB, inserted) => {
      const added = inserted.toString();
      if (fromA < ownAddition.contentFrom) {
        escapesLeft = true;
        return;
      }
      if (toA > ownAddition.contentTo) {
        escapesRight = true;
        return;
      }
      if (!added) {
        const contentBefore = tr.startState.doc.sliceString(ownAddition.contentFrom, fromA);
        const contentAfter = tr.startState.doc.sliceString(toA, ownAddition.contentTo);
        if (!contentBefore && !contentAfter) {
          wouldEmpty = true;
        }
      }
    });

    if (wouldEmpty) {
      return {
        changes: [{ from: ownAddition.from, to: ownAddition.to, insert: '' }],
        selection: EditorSelection.cursor(ownAddition.from),
        effects: tr.effects,
      };
    }

    if (escapesLeft && ownAddition.from > 0) {
      // Backspace at left edge of content: create deletion for char before the wrapper
      const timestamp = Date.now();
      const meta = JSON.stringify({ author: currentAuthor, timestamp });
      const charBefore = tr.startState.doc.sliceString(ownAddition.from - 1, ownAddition.from);
      const delInsert = `{--${meta}@@${charBefore}--}`;
      return {
        changes: [{ from: ownAddition.from - 1, to: ownAddition.from, insert: delInsert }],
        selection: EditorSelection.cursor(ownAddition.from - 1),
        effects: tr.effects,
      };
    }

    if (escapesRight && ownAddition.to < tr.startState.doc.length) {
      // Forward-delete at right edge: create deletion for char after the wrapper
      const timestamp = Date.now();
      const meta = JSON.stringify({ author: currentAuthor, timestamp });
      const charAfter = tr.startState.doc.sliceString(ownAddition.to, ownAddition.to + 1);
      const delInsert = `{--${meta}@@${charAfter}--}`;
      return {
        changes: [{ from: ownAddition.to, to: ownAddition.to + 1, insert: delInsert }],
        selection: EditorSelection.cursor(ownAddition.to + delInsert.length),
        effects: tr.effects,
      };
    }

    return tr;
  }

  // Check if cursor is inside an existing deletion by the same author
  const ownDeletion = ranges.find(
    (r) =>
      r.type === 'deletion' &&
      r.metadata?.author === currentAuthor &&
      cursorPos > r.from &&
      cursorPos < r.to
  );

  if (ownDeletion) {
    // Intercept edits inside own deletion to extend it
    const extChanges: ChangeSpec[] = [];
    let extCursorPos: number | undefined;

    tr.changes.iterChanges((fromA, toA, _fromB, _toB, inserted) => {
      const deleted = tr.startState.doc.sliceString(fromA, toA);
      const added = inserted.toString();

      if (deleted && !added) {
        const isForwardDelete = fromA >= cursorPos;

        if (!isForwardDelete && ownDeletion.from > 0) {
          // Backspace: grab text from before the block and prepend to content
          const grabLen = Math.min(toA - fromA, ownDeletion.from);
          const grabbed = tr.startState.doc.sliceString(
            ownDeletion.from - grabLen,
            ownDeletion.from
          );
          extChanges.push({ from: ownDeletion.from - grabLen, to: ownDeletion.from, insert: '' });
          extChanges.push({
            from: ownDeletion.contentFrom,
            to: ownDeletion.contentFrom,
            insert: grabbed,
          });
          extCursorPos = ownDeletion.contentFrom - grabLen;
        } else if (isForwardDelete && ownDeletion.to < tr.startState.doc.length) {
          // Forward delete: grab text from after the block and append to content
          const grabLen = Math.min(
            toA - fromA,
            tr.startState.doc.length - ownDeletion.to
          );
          const grabbed = tr.startState.doc.sliceString(
            ownDeletion.to,
            ownDeletion.to + grabLen
          );
          extChanges.push({
            from: ownDeletion.contentTo,
            to: ownDeletion.contentTo,
            insert: grabbed,
          });
          extChanges.push({ from: ownDeletion.to, to: ownDeletion.to + grabLen, insert: '' });
          // Cursor to the RIGHT of the appended char
          extCursorPos = ownDeletion.contentTo + grabLen;
        }
      } else if (added && !deleted) {
        // Typing inside deletion: create addition before the deletion block
        const ts = Date.now();
        const addMeta = JSON.stringify({ author: currentAuthor, timestamp: ts });
        const ins = `{++${addMeta}@@${added}++}`;
        extChanges.push({ from: ownDeletion.from, to: ownDeletion.from, insert: ins });
        extCursorPos = ownDeletion.from + ins.length - 3;
      }
    });

    if (extChanges.length > 0) {
      return {
        changes: extChanges,
        selection: extCursorPos !== undefined
          ? EditorSelection.cursor(extCursorPos)
          : tr.selection,
        effects: tr.effects,
      };
    }
    // Fall through for unhandled cases
  }

  const timestamp = Date.now();
  const meta = JSON.stringify({ author: currentAuthor, timestamp });

  const newChanges: ChangeSpec[] = [];
  let newCursorPos: number | undefined;

  tr.changes.iterChanges((fromA, toA, _fromB, _toB, inserted) => {
    const deleted = tr.startState.doc.sliceString(fromA, toA);
    const added = inserted.toString();

    if (deleted && added) {
      // Replacement -> substitution
      newChanges.push({
        from: fromA,
        to: toA,
        insert: `{~~${meta}@@${deleted}~>${added}~~}`,
      });
    } else if (deleted) {
      // Check for adjacent own deletion to extend (sequential backspace)
      const adjacentAfter = ranges.find(
        (r) =>
          r.type === 'deletion' &&
          r.metadata?.author === currentAuthor &&
          toA === r.from
      );

      if (adjacentAfter) {
        // Extend existing deletion by prepending deleted text
        newChanges.push({ from: fromA, to: toA, insert: '' });
        newChanges.push({
          from: adjacentAfter.contentFrom,
          to: adjacentAfter.contentFrom,
          insert: deleted,
        });
        // Cursor inside content (contentFrom shifted left by deleted chars)
        newCursorPos = adjacentAfter.contentFrom - deleted.length;
      } else {
        // Check for adjacent own deletion before (sequential forward-delete)
        const adjacentBefore = ranges.find(
          (r) =>
            r.type === 'deletion' &&
            r.metadata?.author === currentAuthor &&
            fromA === r.to
        );

        if (adjacentBefore) {
          // Extend existing deletion by appending deleted text
          newChanges.push({
            from: adjacentBefore.contentTo,
            to: adjacentBefore.contentTo,
            insert: deleted,
          });
          newChanges.push({ from: fromA, to: toA, insert: '' });
          // Cursor to the RIGHT of the appended char
          newCursorPos = adjacentBefore.contentTo + deleted.length;
        } else {
          // New deletion wrapper
          const delInsert = `{--${meta}@@${deleted}--}`;
          newChanges.push({
            from: fromA,
            to: toA,
            insert: delInsert,
          });
          // Position cursor inside the deletion content
          const contentStart = fromA + delInsert.indexOf('@@') + 2;
          const isForwardDel = cursorPos <= fromA;
          newCursorPos = isForwardDel
            ? contentStart + deleted.length  // RIGHT of deleted text
            : contentStart;                  // LEFT of deleted text
        }
      }
    } else if (added) {
      // Pure insertion
      newChanges.push({
        from: fromA,
        to: fromA,
        insert: `{++${meta}@@${added}++}`,
      });
      // Position cursor before ++} (inside the addition content)
      const change = newChanges[newChanges.length - 1] as {
        from: number;
        insert: string;
      };
      newCursorPos = change.from + change.insert.length - 3;
    }
  });

  if (newChanges.length === 0) return tr;

  return {
    changes: newChanges,
    selection:
      newCursorPos !== undefined
        ? EditorSelection.cursor(newCursorPos)
        : tr.selection,
    effects: tr.effects,
  };
});

/**
 * Compartment for toggling CriticMarkup decorations on/off (source mode toggle).
 * When source mode is ON, this compartment is emptied to show raw markup.
 */
export const criticMarkupCompartment = new Compartment();

/**
 * Extension that provides CriticMarkup parsing and decoration support.
 * Includes keyboard shortcuts:
 * - Mod-Enter: Accept change at cursor
 * - Mod-Backspace: Reject change at cursor
 */
export function criticMarkupExtension() {
  return [
    criticMarkupField,
    suggestionModeField,
    focusedThreadField,
    suggestionModeFilter,
    criticMarkupCompartment.of(criticMarkupPlugin),
    keymap.of(criticMarkupKeymap),
  ];
}
