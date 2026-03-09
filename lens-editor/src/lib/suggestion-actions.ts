import * as Y from 'yjs';
import type { SuggestionItem } from '../hooks/useSuggestions';

/**
 * Apply accept/reject to a suggestion in a Y.Doc.
 * Uses `raw_markup` from the server to find the exact string (avoids reconstruction fragility).
 * Searches near `suggestion.from` first, then falls back to searching the entire doc.
 */
export function applySuggestionAction(
  doc: Y.Doc,
  suggestion: SuggestionItem,
  action: 'accept' | 'reject',
) {
  const text = doc.getText('contents');
  const content = text.toString();

  const markup = suggestion.raw_markup;
  // Search near the expected position first (within 200 chars), then fall back to full search
  let idx = content.indexOf(markup, Math.max(0, suggestion.from - 200));
  if (idx === -1) {
    idx = content.indexOf(markup);
  }
  if (idx === -1) {
    throw new Error('Suggestion no longer found in document');
  }

  const replacement = action === 'accept'
    ? getAcceptText(suggestion)
    : getRejectText(suggestion);

  doc.transact(() => {
    text.delete(idx, markup.length);
    if (replacement) {
      text.insert(idx, replacement);
    }
  });
}

export function getAcceptText(s: SuggestionItem): string {
  switch (s.type) {
    case 'addition': return s.content;
    case 'deletion': return '';
    case 'substitution': return s.new_content ?? '';
  }
}

export function getRejectText(s: SuggestionItem): string {
  switch (s.type) {
    case 'addition': return '';
    case 'deletion': return s.content;
    case 'substitution': return s.old_content ?? '';
  }
}
