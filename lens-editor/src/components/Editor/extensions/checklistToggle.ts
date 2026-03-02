/**
 * Checklist Toggle (Ctrl+L)
 *
 * Obsidian-style checklist toggling: pressing Ctrl+L cycles through
 * plain text → unchecked checkbox → checked checkbox → unchecked.
 * Handles `-`, `*`, `+` markers and ordered lists (`1.`).
 * Preserves leading indentation. Works with multiple cursors.
 */
import type { StateCommand } from '@codemirror/state';
import { EditorSelection } from '@codemirror/state';

// Matches: optional indent, then a checked checkbox with any list marker
// Groups: 1=indent, 2=marker+space, 3=rest of line
const CHECKED = /^(\s*)([-*+]|\d+\.)\s+\[x\]\s(.*)/;

// Matches: optional indent, then an unchecked checkbox with any list marker
const UNCHECKED = /^(\s*)([-*+]|\d+\.)\s+\[ \]\s(.*)/;

// Matches: optional indent, then a list marker (no checkbox)
// Groups: 1=indent, 2=marker, 3=space+rest
const LIST_MARKER = /^(\s*)([-*+]|\d+\.)\s+(.*)/;

export const toggleChecklist: StateCommand = ({ state, dispatch }) => {
  const changes = state.changeByRange((range) => {
    const line = state.doc.lineAt(range.head);
    const text = line.text;

    let newText: string;
    let checkedMatch: RegExpMatchArray | null;
    let uncheckedMatch: RegExpMatchArray | null;
    let listMatch: RegExpMatchArray | null;

    if ((checkedMatch = text.match(CHECKED))) {
      // [x] → [ ]
      const [, indent, marker, rest] = checkedMatch;
      newText = `${indent}${marker} [ ] ${rest}`;
    } else if ((uncheckedMatch = text.match(UNCHECKED))) {
      // [ ] → [x]
      const [, indent, marker, rest] = uncheckedMatch;
      newText = `${indent}${marker} [x] ${rest}`;
    } else if ((listMatch = text.match(LIST_MARKER))) {
      // list item without checkbox → add [ ]
      const [, indent, marker, rest] = listMatch;
      newText = `${indent}${marker} [ ] ${rest}`;
    } else {
      // plain text → add - [ ] (preserving indent)
      const indent = text.match(/^(\s*)/)?.[1] ?? '';
      const content = text.slice(indent.length);
      newText = `${indent}- [ ] ${content}`;
    }

    const change = { from: line.from, to: line.to, insert: newText };
    // Place cursor at end of new line
    const cursor = EditorSelection.cursor(line.from + newText.length);

    return { range: cursor, changes: [change] };
  });

  dispatch(state.update(changes, { scrollIntoView: true, userEvent: 'input' }));
  return true;
};

export const checklistKeymap = [
  { key: 'Mod-l' as const, run: toggleChecklist },
];
