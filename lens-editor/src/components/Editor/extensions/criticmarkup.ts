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
import { criticMarkupKeymap, acceptChangeAtCursor, rejectChangeAtCursor } from './criticmarkup-commands';
import { parse, type CriticMarkupRange } from '../../../lib/criticmarkup-parser';

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

/**
 * StateEffect to toggle suggestion mode on/off.
 */
export const toggleSuggestionMode = StateEffect.define<boolean>();

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
 * ViewPlugin that applies decorations (CSS classes) to CriticMarkup ranges.
 * Decorations are rebuilt when the document changes, viewport changes, or selection changes.
 * When cursor is outside a range, delimiters are hidden using cm-hidden-syntax.
 * When cursor is inside, the full markup (including delimiters) is shown.
 */
export const criticMarkupPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;

    constructor(view: EditorView) {
      this.decorations = this.buildDecorations(view);

      // Event delegation for accept/reject button clicks
      view.contentDOM.addEventListener('click', (e) => {
        const target = e.target as HTMLElement;
        if (target.classList.contains('cm-criticmarkup-accept')) {
          e.preventDefault();
          e.stopPropagation();
          acceptChangeAtCursor(view);
        } else if (target.classList.contains('cm-criticmarkup-reject')) {
          e.preventDefault();
          e.stopPropagation();
          rejectChangeAtCursor(view);
        }
      });
    }

    update(update: ViewUpdate) {
      if (update.docChanged || update.viewportChanged || update.selectionSet) {
        this.decorations = this.buildDecorations(update.view);
      }
    }

    buildDecorations(view: EditorView): DecorationSet {
      const builder = new RangeSetBuilder<Decoration>();
      const ranges = view.state.field(criticMarkupField);
      const selection = view.state.selection;

      // Collect all decorations to sort before adding
      const decorations: Array<{ from: number; to: number; deco: Decoration }> = [];

      for (const range of ranges) {
        const className = TYPE_CLASSES[range.type];
        const cursorInside = selectionIntersects(selection, range.from, range.to);

        if (cursorInside) {
          // Cursor inside - show everything, apply class to whole range
          decorations.push({
            from: range.from,
            to: range.to,
            deco: Decoration.mark({ class: className }),
          });

          // Add accept/reject buttons at end of content (before closing delimiter)
          decorations.push({
            from: range.contentTo,
            to: range.contentTo,
            deco: Decoration.widget({
              widget: new AcceptRejectWidget(range.from, range.to),
              side: 1, // After the content
            }),
          });
        } else {
          // Cursor outside - hide delimiters (including metadata), style content only
          // Use contentFrom/contentTo from parser - these account for metadata

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
        }
      }

      // Sort by position (required for RangeSetBuilder)
      // Widgets (from === to) should come after marks at the same position
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
  const insideOwnAddition = ranges.some(
    (r) =>
      r.type === 'addition' &&
      r.metadata?.author === currentAuthor &&
      cursorPos > r.from &&
      cursorPos < r.to
  );

  // If inside own addition, let the edit through without wrapping
  if (insideOwnAddition) {
    return tr;
  }

  const timestamp = Date.now();
  const meta = JSON.stringify({ author: currentAuthor, timestamp });

  const newChanges: ChangeSpec[] = [];

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
      // Pure deletion
      newChanges.push({
        from: fromA,
        to: toA,
        insert: `{--${meta}@@${deleted}--}`,
      });
    } else if (added) {
      // Pure insertion
      newChanges.push({
        from: fromA,
        to: fromA,
        insert: `{++${meta}@@${added}++}`,
      });
    }
  });

  if (newChanges.length === 0) return tr;

  // Calculate new cursor position: inside the wrapped content
  // For additions, cursor should be before ++} (end of insert minus 3)
  let newCursorPos: number | undefined;
  if (newChanges.length === 1) {
    const change = newChanges[0] as { from: number; to?: number; insert: string };
    if (change.insert.startsWith('{++')) {
      // Position cursor before ++} (end of insert minus 3)
      newCursorPos = change.from + change.insert.length - 3;
    }
  }

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
    suggestionModeFilter,
    criticMarkupCompartment.of(criticMarkupPlugin),
    keymap.of(criticMarkupKeymap),
  ];
}
