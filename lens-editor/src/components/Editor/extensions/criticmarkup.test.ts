// src/components/Editor/extensions/criticmarkup.test.ts
import { describe, it, expect, afterEach } from 'vitest';
import { Transaction } from '@codemirror/state';
import { createCriticMarkupEditor, createCriticMarkupEditorWithSourceMode, hasClass, moveCursor } from '../../../test/codemirror-helpers';
import { criticMarkupField, toggleSuggestionMode, suggestionModeField } from './criticmarkup';
import { toggleSourceMode } from './livePreview';

describe('CriticMarkup Extension', () => {
  let cleanup: (() => void) | undefined;

  afterEach(() => {
    if (cleanup) cleanup();
  });

  describe('StateField', () => {
    it('parses CriticMarkup ranges from document', () => {
      const { view, cleanup: c } = createCriticMarkupEditor(
        'hello {++world++} end',
        21
      );
      cleanup = c;

      const ranges = view.state.field(criticMarkupField);

      expect(ranges).toHaveLength(1);
      expect(ranges[0].type).toBe('addition');
    });

    it('updates ranges when document changes', () => {
      const { view, cleanup: c } = createCriticMarkupEditor(
        'hello {++world++} end',
        21
      );
      cleanup = c;

      // Initially one addition
      let ranges = view.state.field(criticMarkupField);
      expect(ranges).toHaveLength(1);

      // Add a deletion
      view.dispatch({
        changes: { from: 21, insert: ' {--removed--}' },
      });

      ranges = view.state.field(criticMarkupField);
      expect(ranges).toHaveLength(2);
      expect(ranges[1].type).toBe('deletion');
    });

    it('returns empty array when no CriticMarkup in document', () => {
      const { view, cleanup: c } = createCriticMarkupEditor(
        'hello world',
        5
      );
      cleanup = c;

      const ranges = view.state.field(criticMarkupField);
      expect(ranges).toHaveLength(0);
    });

    it('parses all markup types', () => {
      const { view, cleanup: c } = createCriticMarkupEditor(
        '{++added++} {--deleted--} {~~old~>new~~} {>>comment<<} {==highlight==}',
        0
      );
      cleanup = c;

      const ranges = view.state.field(criticMarkupField);
      expect(ranges).toHaveLength(5);
      expect(ranges.map(r => r.type)).toEqual([
        'addition',
        'deletion',
        'substitution',
        'comment',
        'highlight',
      ]);
    });
  });

  describe('Decorations', () => {
    it('applies cm-addition class to additions', () => {
      const { view, cleanup: c } = createCriticMarkupEditor(
        'hello {++world++} end',
        21
      );
      cleanup = c;

      expect(hasClass(view, 'cm-addition')).toBe(true);
    });

    it('applies cm-deletion class to deletions', () => {
      const { view, cleanup: c } = createCriticMarkupEditor(
        'hello {--removed--} end',
        23
      );
      cleanup = c;

      expect(hasClass(view, 'cm-deletion')).toBe(true);
    });

    it('applies cm-highlight class to highlights', () => {
      const { view, cleanup: c } = createCriticMarkupEditor(
        'hello {==important==} end',
        25
      );
      cleanup = c;

      expect(hasClass(view, 'cm-highlight')).toBe(true);
    });

    it('applies cm-comment class to comments', () => {
      const { view, cleanup: c } = createCriticMarkupEditor(
        'hello {>>note<<} end',
        20
      );
      cleanup = c;

      expect(hasClass(view, 'cm-comment')).toBe(true);
    });

    it('applies cm-substitution class to substitutions', () => {
      const { view, cleanup: c } = createCriticMarkupEditor(
        'hello {~~old~>new~~} end',
        24
      );
      cleanup = c;

      expect(hasClass(view, 'cm-substitution')).toBe(true);
    });
  });

  describe('Live Preview', () => {
    it('hides markup syntax when cursor is outside', () => {
      const { view, cleanup: c } = createCriticMarkupEditor(
        'hello {++world++} end',
        21 // cursor at "end"
      );
      cleanup = c;

      // The {++ and ++} should be hidden
      expect(hasClass(view, 'cm-hidden-syntax')).toBe(true);
    });

    it('shows markup syntax when cursor is inside', () => {
      const { view, cleanup: c } = createCriticMarkupEditor(
        'hello {++world++} end',
        10 // cursor inside "world"
      );
      cleanup = c;

      // The {++ and ++} should be visible (no hidden-syntax on them)
      // Check that hidden-syntax count is 0
      const hiddenCount = view.contentDOM.querySelectorAll('.cm-hidden-syntax').length;
      expect(hiddenCount).toBe(0);
    });

    it('updates decorations when cursor moves in and out', () => {
      const { view, cleanup: c } = createCriticMarkupEditor(
        'hello {++world++} end',
        21 // start outside
      );
      cleanup = c;

      // Initially outside - syntax hidden
      expect(hasClass(view, 'cm-hidden-syntax')).toBe(true);

      // Move cursor inside
      moveCursor(view, 10);

      // Now inside - syntax visible
      expect(hasClass(view, 'cm-hidden-syntax')).toBe(false);

      // Move cursor back outside
      moveCursor(view, 21);

      // Outside again - syntax hidden
      expect(hasClass(view, 'cm-hidden-syntax')).toBe(true);
    });

    it('hides metadata and @@ when cursor is outside (metadata-aware)', () => {
      // With metadata: {++{"author":"alice"}@@content++}
      const { view, cleanup: c } = createCriticMarkupEditor(
        '{++{"author":"alice"}@@hello++} end',
        35 // cursor at "end"
      );
      cleanup = c;

      // The entire {++{"author":"alice"}@@ prefix and ++} suffix should be hidden
      // Only "hello" should be visible with cm-addition styling
      const hiddenElements = view.contentDOM.querySelectorAll('.cm-hidden-syntax');
      expect(hiddenElements.length).toBeGreaterThan(0);

      // The visible content should just be "hello"
      const additionElements = view.contentDOM.querySelectorAll('.cm-addition');
      expect(additionElements.length).toBe(1);
      expect(additionElements[0].textContent).toBe('hello');
    });
  });

  describe('Suggestion Mode', () => {
    describe('mode toggle', () => {
      it('starts in editing mode by default', () => {
        const { view, cleanup: c } = createCriticMarkupEditor('hello', 5);
        cleanup = c;

        const isSuggestionMode = view.state.field(suggestionModeField);
        expect(isSuggestionMode).toBe(false);
      });

      it('can toggle to suggestion mode', () => {
        const { view, cleanup: c } = createCriticMarkupEditor('hello', 5);
        cleanup = c;

        view.dispatch({ effects: toggleSuggestionMode.of(true) });

        const isSuggestionMode = view.state.field(suggestionModeField);
        expect(isSuggestionMode).toBe(true);
      });

      it('can toggle back to editing mode', () => {
        const { view, cleanup: c } = createCriticMarkupEditor('hello', 5);
        cleanup = c;

        view.dispatch({ effects: toggleSuggestionMode.of(true) });
        view.dispatch({ effects: toggleSuggestionMode.of(false) });

        const isSuggestionMode = view.state.field(suggestionModeField);
        expect(isSuggestionMode).toBe(false);
      });
    });

    describe('wrapping insertions', () => {
      it('wraps inserted text in addition markup when suggestion mode is ON', () => {
        const { view, cleanup: c } = createCriticMarkupEditor('hello', 5);
        cleanup = c;

        // Enable suggestion mode
        view.dispatch({ effects: toggleSuggestionMode.of(true) });

        // Insert text (annotate as user input so suggestion filter activates)
        view.dispatch({
          changes: { from: 5, insert: ' world' },
          annotations: Transaction.userEvent.of('input'),
        });

        const doc = view.state.doc.toString();
        expect(doc).toMatch(/\{\+\+.*@@ world\+\+\}/);
      });

      it('does NOT wrap insertions when suggestion mode is OFF', () => {
        const { view, cleanup: c } = createCriticMarkupEditor('hello', 5);
        cleanup = c;

        // Suggestion mode is OFF by default
        view.dispatch({
          changes: { from: 5, insert: ' world' },
        });

        const doc = view.state.doc.toString();
        expect(doc).toBe('hello world');
      });

      it('includes metadata in wrapped insertion', () => {
        const { view, cleanup: c } = createCriticMarkupEditor('hello', 5);
        cleanup = c;

        view.dispatch({ effects: toggleSuggestionMode.of(true) });
        view.dispatch({
          changes: { from: 5, insert: 'X' },
          annotations: Transaction.userEvent.of('input'),
        });

        const doc = view.state.doc.toString();
        // Should have JSON metadata with author and timestamp
        expect(doc).toMatch(/\{\+\+\{.*"author".*\}@@X\+\+\}/);
        expect(doc).toMatch(/\{\+\+\{.*"timestamp".*\}@@X\+\+\}/);
      });

      it('continuous typing extends existing addition (not per-character)', () => {
        const { view, cleanup: c } = createCriticMarkupEditor('hello', 5);
        cleanup = c;

        view.dispatch({ effects: toggleSuggestionMode.of(true) });

        // Type 'h'
        view.dispatch({ changes: { from: 5, insert: 'h' }, annotations: Transaction.userEvent.of('input') });

        // Get cursor position - should be inside the addition
        const cursorPos = view.state.selection.main.head;

        // Type 'i' at cursor position
        view.dispatch({ changes: { from: cursorPos, insert: 'i' }, annotations: Transaction.userEvent.of('input') });

        const doc = view.state.doc.toString();

        // Should have ONE addition with "hi", not two separate ones
        const additionMatches = doc.match(/\{\+\+/g);
        expect(additionMatches?.length).toBe(1);
        expect(doc).toMatch(/@@hi\+\+\}/);
      });
    });

    describe('wrapping deletions', () => {
      it('wraps deleted text in deletion markup', () => {
        const { view, cleanup: c } = createCriticMarkupEditor('hello world', 5);
        cleanup = c;

        view.dispatch({ effects: toggleSuggestionMode.of(true) });

        // Delete " world"
        view.dispatch({
          changes: { from: 5, to: 11, insert: '' },
          annotations: Transaction.userEvent.of('delete'),
        });

        const doc = view.state.doc.toString();
        expect(doc).toMatch(/\{--.*@@ world--\}/);
      });

      it('does NOT wrap deletions when suggestion mode is OFF', () => {
        const { view, cleanup: c } = createCriticMarkupEditor('hello world', 5);
        cleanup = c;

        // Suggestion mode is OFF by default
        view.dispatch({
          changes: { from: 5, to: 11, insert: '' },
        });

        const doc = view.state.doc.toString();
        expect(doc).toBe('hello');
      });

      it('includes metadata in wrapped deletion', () => {
        const { view, cleanup: c } = createCriticMarkupEditor('hello world', 5);
        cleanup = c;

        view.dispatch({ effects: toggleSuggestionMode.of(true) });
        view.dispatch({
          changes: { from: 5, to: 11, insert: '' },
          annotations: Transaction.userEvent.of('delete'),
        });

        const doc = view.state.doc.toString();
        // Should have JSON metadata with author and timestamp
        expect(doc).toMatch(/\{--\{.*"author".*\}@@ world--\}/);
        expect(doc).toMatch(/\{--\{.*"timestamp".*\}@@ world--\}/);
      });
    });

    describe('wrapping replacements', () => {
      it('wraps selection replacement in substitution markup', () => {
        const { view, cleanup: c } = createCriticMarkupEditor('hello world', 6);
        cleanup = c;

        view.dispatch({ effects: toggleSuggestionMode.of(true) });

        // Replace "world" with "there"
        view.dispatch({
          changes: { from: 6, to: 11, insert: 'there' },
          annotations: Transaction.userEvent.of('input'),
        });

        const doc = view.state.doc.toString();
        expect(doc).toMatch(/\{~~.*@@world~>there~~\}/);
      });

      it('does NOT wrap replacements when suggestion mode is OFF', () => {
        const { view, cleanup: c } = createCriticMarkupEditor('hello world', 6);
        cleanup = c;

        // Suggestion mode is OFF by default
        view.dispatch({
          changes: { from: 6, to: 11, insert: 'there' },
        });

        const doc = view.state.doc.toString();
        expect(doc).toBe('hello there');
      });

      it('includes metadata in wrapped substitution', () => {
        const { view, cleanup: c } = createCriticMarkupEditor('hello world', 6);
        cleanup = c;

        view.dispatch({ effects: toggleSuggestionMode.of(true) });
        view.dispatch({
          changes: { from: 6, to: 11, insert: 'there' },
          annotations: Transaction.userEvent.of('input'),
        });

        const doc = view.state.doc.toString();
        // Should have JSON metadata with author and timestamp
        expect(doc).toMatch(/\{~~\{.*"author".*\}@@world~>there~~\}/);
        expect(doc).toMatch(/\{~~\{.*"timestamp".*\}@@world~>there~~\}/);
      });
    });
  });

  describe('Accept/Reject Buttons', () => {
    it('shows accept/reject buttons when cursor is inside markup', () => {
      const { view, cleanup: c } = createCriticMarkupEditor(
        'hello {++world++} end',
        10 // cursor inside
      );
      cleanup = c;

      const acceptBtn = view.contentDOM.querySelector('.cm-criticmarkup-accept');
      const rejectBtn = view.contentDOM.querySelector('.cm-criticmarkup-reject');

      expect(acceptBtn).not.toBeNull();
      expect(rejectBtn).not.toBeNull();
    });

    it('hides buttons when cursor is outside markup', () => {
      const { view, cleanup: c } = createCriticMarkupEditor(
        'hello {++world++} end',
        3 // cursor outside
      );
      cleanup = c;

      const acceptBtn = view.contentDOM.querySelector('.cm-criticmarkup-accept');
      const rejectBtn = view.contentDOM.querySelector('.cm-criticmarkup-reject');

      expect(acceptBtn).toBeNull();
      expect(rejectBtn).toBeNull();
    });

    it('buttons appear for all markup types', () => {
      // Test with deletion
      const { view, cleanup: c } = createCriticMarkupEditor(
        'hello {--removed--} end',
        10
      );
      cleanup = c;

      expect(view.contentDOM.querySelector('.cm-criticmarkup-accept')).not.toBeNull();
      expect(view.contentDOM.querySelector('.cm-criticmarkup-reject')).not.toBeNull();
    });
  });

  describe('Button Click Behavior', () => {
    it('clicking accept button applies the change', () => {
      const { view, cleanup: c } = createCriticMarkupEditor(
        'hello {++world++} end',
        10
      );
      cleanup = c;

      const acceptBtn = view.contentDOM.querySelector('.cm-criticmarkup-accept') as HTMLButtonElement;
      expect(acceptBtn).not.toBeNull();

      acceptBtn.click();

      expect(view.state.doc.toString()).toBe('hello world end');
    });

    it('clicking reject button reverts the change', () => {
      const { view, cleanup: c } = createCriticMarkupEditor(
        'hello {++world++} end',
        10
      );
      cleanup = c;

      const rejectBtn = view.contentDOM.querySelector('.cm-criticmarkup-reject') as HTMLButtonElement;
      expect(rejectBtn).not.toBeNull();

      rejectBtn.click();

      expect(view.state.doc.toString()).toBe('hello  end');
    });
  });

  describe('Source Mode Integration', () => {
    it('hides CriticMarkup decorations when source mode is ON', () => {
      const { view, cleanup: c } = createCriticMarkupEditorWithSourceMode(
        'hello {++world++} end',
        21 // cursor outside markup
      );
      cleanup = c;

      // Initially in live preview mode - decorations should be applied
      expect(hasClass(view, 'cm-addition')).toBe(true);
      expect(hasClass(view, 'cm-hidden-syntax')).toBe(true);

      // Enable source mode
      toggleSourceMode(view, true);

      // In source mode - decorations should NOT be applied
      // Raw markup should be visible (no cm-addition styling)
      expect(hasClass(view, 'cm-addition')).toBe(false);
      expect(hasClass(view, 'cm-hidden-syntax')).toBe(false);
    });

    it('restores CriticMarkup decorations when source mode is OFF', () => {
      const { view, cleanup: c } = createCriticMarkupEditorWithSourceMode(
        'hello {++world++} end',
        21
      );
      cleanup = c;

      // Enable source mode
      toggleSourceMode(view, true);
      expect(hasClass(view, 'cm-addition')).toBe(false);

      // Disable source mode
      toggleSourceMode(view, false);

      // Decorations should be back
      expect(hasClass(view, 'cm-addition')).toBe(true);
      expect(hasClass(view, 'cm-hidden-syntax')).toBe(true);
    });

    it('shows raw deletion markup in source mode', () => {
      const { view, cleanup: c } = createCriticMarkupEditorWithSourceMode(
        'hello {--removed--} end',
        23
      );
      cleanup = c;

      // Enable source mode
      toggleSourceMode(view, true);

      // Raw markup should be visible - no decoration classes
      expect(hasClass(view, 'cm-deletion')).toBe(false);
      expect(hasClass(view, 'cm-hidden-syntax')).toBe(false);
    });
  });
});
