import { EditorState, Prec } from '@codemirror/state';
import { EditorView, keymap } from '@codemirror/view';
import { markdown } from '@codemirror/lang-markdown';
import { TaskList } from '@lezer/markdown';
import { defaultKeymap } from '@codemirror/commands';
import { WikilinkExtension } from '../components/Editor/extensions/wikilinkParser';
import { livePreview } from '../components/Editor/extensions/livePreview';
import type { WikilinkContext } from '../components/Editor/extensions/livePreview';
import { criticMarkupExtension } from '../components/Editor/extensions/criticmarkup';
import { tightMarkdownKeymap } from '../components/Editor/extensions/tightListEnter';
import { checklistKeymap } from '../components/Editor/extensions/checklistToggle';

/**
 * Create an EditorView with live preview extension for testing.
 * Returns the view and a cleanup function.
 */
export function createTestEditor(
  content: string,
  cursorPos: number,
  wikilinkContext?: WikilinkContext
): { view: EditorView; cleanup: () => void } {
  const state = EditorState.create({
    doc: content,
    selection: { anchor: cursorPos },
    extensions: [
      markdown({ extensions: [WikilinkExtension, TaskList] }),
      livePreview(wikilinkContext),
    ],
  });

  const view = new EditorView({
    state,
    parent: document.body,
  });

  return {
    view,
    cleanup: () => {
      view.destroy();
    },
  };
}

/**
 * Check if a CSS class exists in the editor's content DOM.
 */
export function hasClass(view: EditorView, className: string): boolean {
  return view.contentDOM.querySelector(`.${className}`) !== null;
}

/**
 * Count elements with a specific class in the editor.
 */
export function countClass(view: EditorView, className: string): number {
  return view.contentDOM.querySelectorAll(`.${className}`).length;
}

/**
 * Get text content of elements with a specific class.
 */
export function getTextWithClass(view: EditorView, className: string): string[] {
  const elements = view.contentDOM.querySelectorAll(`.${className}`);
  return Array.from(elements).map((el) => el.textContent || '');
}

/**
 * Check if wikilink widget exists with specific text.
 */
export function hasWikilinkWidget(view: EditorView, pageName: string): boolean {
  const widgets = view.contentDOM.querySelectorAll('.cm-wikilink-widget');
  return Array.from(widgets).some((w) => w.textContent === pageName);
}

/**
 * Check if link widget exists with specific text.
 */
export function hasLinkWidget(view: EditorView, linkText: string): boolean {
  const widgets = view.contentDOM.querySelectorAll('.cm-link-widget');
  return Array.from(widgets).some((w) => w.textContent?.includes(linkText));
}

/**
 * Move cursor to a position and trigger decoration update.
 */
export function moveCursor(view: EditorView, pos: number): void {
  view.dispatch({
    selection: { anchor: pos },
  });
}

/**
 * Get the line number where the cursor is.
 */
export function getCursorLine(view: EditorView): number {
  return view.state.doc.lineAt(view.state.selection.main.head).number;
}

/**
 * Create an EditorView with CriticMarkup extension for testing.
 */
export function createCriticMarkupEditor(
  content: string,
  cursorPos: number
): { view: EditorView; cleanup: () => void } {
  const state = EditorState.create({
    doc: content,
    selection: { anchor: cursorPos },
    extensions: [
      markdown(),
      criticMarkupExtension(),
    ],
  });

  const view = new EditorView({
    state,
    parent: document.body,
  });

  return {
    view,
    cleanup: () => {
      view.destroy();
    },
  };
}

/**
 * Create an EditorView with both CriticMarkup and livePreview extensions.
 * This enables testing source mode toggling with CriticMarkup.
 */
export function createCriticMarkupEditorWithSourceMode(
  content: string,
  cursorPos: number
): { view: EditorView; cleanup: () => void } {
  const state = EditorState.create({
    doc: content,
    selection: { anchor: cursorPos },
    extensions: [
      markdown(),
      livePreview(),
      criticMarkupExtension(),
    ],
  });

  const view = new EditorView({
    state,
    parent: document.body,
  });

  return {
    view,
    cleanup: () => {
      view.destroy();
    },
  };
}

/**
 * Create an EditorView with the tight-list markdown keymap for testing.
 * Mirrors the Editor.tsx extension stack relevant to Enter/Backspace.
 */
export function createMarkdownEditor(
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

/**
 * Simulate pressing Enter through the tight-list markdown keymap.
 * Uses the same binding-lookup pattern as criticmarkup-commands.test.ts.
 */
export function pressEnter(view: EditorView): boolean {
  const binding = tightMarkdownKeymap.find((k) => k.key === 'Enter');
  return binding?.run?.(view) ?? false;
}

/**
 * Simulate pressing Ctrl+L (checklist toggle) through the checklist keymap.
 */
export function pressCtrlL(view: EditorView): boolean {
  const binding = checklistKeymap.find((k) => k.key === 'Mod-l');
  return binding?.run?.(view) ?? false;
}
