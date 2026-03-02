import { describe, it, expect, afterEach } from 'vitest';
import { createMarkdownEditor } from '../../../test/codemirror-helpers';
import { checklistKeymap } from './checklistToggle';
import { EditorSelection, Prec } from '@codemirror/state';
import { keymap, EditorView } from '@codemirror/view';
import { markdown } from '@codemirror/lang-markdown';
import { WikilinkExtension } from './wikilinkParser';
import { tightMarkdownKeymap } from './tightListEnter';
import { defaultKeymap } from '@codemirror/commands';
import { EditorState } from '@codemirror/state';

function createChecklistEditor(
  content: string,
  cursorPos: number
): { view: EditorView; cleanup: () => void } {
  const state = EditorState.create({
    doc: content,
    selection: { anchor: cursorPos },
    extensions: [
      markdown({
        extensions: [WikilinkExtension],
        addKeymap: false,
      }),
      Prec.high(keymap.of(tightMarkdownKeymap)),
      Prec.high(keymap.of(checklistKeymap)),
      keymap.of(defaultKeymap),
    ],
  });

  const view = new EditorView({
    state,
    parent: document.body,
  });

  return {
    view,
    cleanup: () => view.destroy(),
  };
}

function pressCtrlL(view: EditorView): boolean {
  const binding = checklistKeymap.find((k) => k.key === 'Mod-l');
  return binding?.run?.(view) ?? false;
}

describe('Checklist Toggle (Ctrl+L)', () => {
  let cleanup: (() => void) | undefined;

  afterEach(() => {
    if (cleanup) cleanup();
  });

  it('converts plain text to unchecked checkbox', () => {
    const doc = 'hello world';
    const { view, cleanup: c } = createChecklistEditor(doc, 5);
    cleanup = c;

    const result = pressCtrlL(view);

    expect(result).toBe(true);
    expect(view.state.doc.toString()).toBe('- [ ] hello world');
  });

  it('converts list item (dash) to unchecked checkbox', () => {
    const doc = '- some item';
    const { view, cleanup: c } = createChecklistEditor(doc, 5);
    cleanup = c;

    pressCtrlL(view);

    expect(view.state.doc.toString()).toBe('- [ ] some item');
  });

  it('converts list item (asterisk) to unchecked checkbox', () => {
    const doc = '* some item';
    const { view, cleanup: c } = createChecklistEditor(doc, 5);
    cleanup = c;

    pressCtrlL(view);

    expect(view.state.doc.toString()).toBe('* [ ] some item');
  });

  it('converts list item (plus) to unchecked checkbox', () => {
    const doc = '+ some item';
    const { view, cleanup: c } = createChecklistEditor(doc, 5);
    cleanup = c;

    pressCtrlL(view);

    expect(view.state.doc.toString()).toBe('+ [ ] some item');
  });

  it('converts ordered list item to unchecked checkbox', () => {
    const doc = '1. some item';
    const { view, cleanup: c } = createChecklistEditor(doc, 5);
    cleanup = c;

    pressCtrlL(view);

    expect(view.state.doc.toString()).toBe('1. [ ] some item');
  });

  it('checks unchecked checkbox', () => {
    const doc = '- [ ] todo item';
    const { view, cleanup: c } = createChecklistEditor(doc, 10);
    cleanup = c;

    pressCtrlL(view);

    expect(view.state.doc.toString()).toBe('- [x] todo item');
  });

  it('unchecks checked checkbox', () => {
    const doc = '- [x] done item';
    const { view, cleanup: c } = createChecklistEditor(doc, 10);
    cleanup = c;

    pressCtrlL(view);

    expect(view.state.doc.toString()).toBe('- [ ] done item');
  });

  it('preserves indentation on plain text', () => {
    const doc = '  hello world';
    const { view, cleanup: c } = createChecklistEditor(doc, 5);
    cleanup = c;

    pressCtrlL(view);

    expect(view.state.doc.toString()).toBe('  - [ ] hello world');
  });

  it('preserves indentation on list item', () => {
    const doc = '  - nested item';
    const { view, cleanup: c } = createChecklistEditor(doc, 8);
    cleanup = c;

    pressCtrlL(view);

    expect(view.state.doc.toString()).toBe('  - [ ] nested item');
  });

  it('preserves indentation on checkbox', () => {
    const doc = '    - [ ] deeply nested';
    const { view, cleanup: c } = createChecklistEditor(doc, 15);
    cleanup = c;

    pressCtrlL(view);

    expect(view.state.doc.toString()).toBe('    - [x] deeply nested');
  });

  it('works with asterisk checkbox', () => {
    const doc = '* [ ] asterisk todo';
    const { view, cleanup: c } = createChecklistEditor(doc, 10);
    cleanup = c;

    pressCtrlL(view);

    expect(view.state.doc.toString()).toBe('* [x] asterisk todo');
  });

  it('works with ordered list checkbox', () => {
    const doc = '1. [ ] ordered todo';
    const { view, cleanup: c } = createChecklistEditor(doc, 10);
    cleanup = c;

    pressCtrlL(view);

    expect(view.state.doc.toString()).toBe('1. [x] ordered todo');
  });

  it('unchecks ordered list checkbox', () => {
    const doc = '1. [x] ordered done';
    const { view, cleanup: c } = createChecklistEditor(doc, 10);
    cleanup = c;

    pressCtrlL(view);

    expect(view.state.doc.toString()).toBe('1. [ ] ordered done');
  });

  it('places cursor at end of line after toggle', () => {
    const doc = '- [ ] hello';
    const { view, cleanup: c } = createChecklistEditor(doc, 3);
    cleanup = c;

    pressCtrlL(view);

    // After toggle: "- [x] hello" — cursor at end (11)
    expect(view.state.selection.main.head).toBe('- [x] hello'.length);
  });

  it('handles empty line', () => {
    const doc = '';
    const { view, cleanup: c } = createChecklistEditor(doc, 0);
    cleanup = c;

    pressCtrlL(view);

    expect(view.state.doc.toString()).toBe('- [ ] ');
  });

  it('only affects the line with the cursor in multiline doc', () => {
    const doc = 'line one\nline two\nline three';
    const { view, cleanup: c } = createChecklistEditor(doc, 12); // middle of "line two"
    cleanup = c;

    pressCtrlL(view);

    expect(view.state.doc.toString()).toBe('line one\n- [ ] line two\nline three');
  });

  it('handles multi-digit ordered list', () => {
    const doc = '12. some item';
    const { view, cleanup: c } = createChecklistEditor(doc, 5);
    cleanup = c;

    pressCtrlL(view);

    expect(view.state.doc.toString()).toBe('12. [ ] some item');
  });
});
