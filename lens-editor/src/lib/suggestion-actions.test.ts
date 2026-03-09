import { describe, it, expect } from 'vitest';
import * as Y from 'yjs';
import { applySuggestionAction, getAcceptText, getRejectText } from './suggestion-actions';
import type { SuggestionItem } from '../hooks/useSuggestions';

function makeDoc(content: string): Y.Doc {
  const doc = new Y.Doc();
  doc.getText('contents').insert(0, content);
  return doc;
}

function makeSuggestion(overrides: Partial<SuggestionItem> & { type: SuggestionItem['type'] }): SuggestionItem {
  return {
    content: '',
    old_content: null,
    new_content: null,
    author: null,
    timestamp: null,
    from: 0,
    to: 0,
    raw_markup: '',
    context_before: '',
    context_after: '',
    ...overrides,
  };
}

describe('getAcceptText', () => {
  it('returns content for addition', () => {
    expect(getAcceptText(makeSuggestion({ type: 'addition', content: 'hello' }))).toBe('hello');
  });

  it('returns empty string for deletion', () => {
    expect(getAcceptText(makeSuggestion({ type: 'deletion', content: 'bye' }))).toBe('');
  });

  it('returns new_content for substitution', () => {
    expect(getAcceptText(makeSuggestion({ type: 'substitution', old_content: 'old', new_content: 'new' }))).toBe('new');
  });
});

describe('getRejectText', () => {
  it('returns empty string for addition', () => {
    expect(getRejectText(makeSuggestion({ type: 'addition', content: 'hello' }))).toBe('');
  });

  it('returns content for deletion', () => {
    expect(getRejectText(makeSuggestion({ type: 'deletion', content: 'bye' }))).toBe('bye');
  });

  it('returns old_content for substitution', () => {
    expect(getRejectText(makeSuggestion({ type: 'substitution', old_content: 'old', new_content: 'new' }))).toBe('old');
  });
});

describe('applySuggestionAction', () => {
  it('accept addition: keeps content, removes markup', () => {
    const markup = '{++{"author":"AI","timestamp":1000}@@world++}';
    const doc = makeDoc(`Hello ${markup} end`);
    applySuggestionAction(doc, makeSuggestion({
      type: 'addition',
      content: 'world',
      raw_markup: markup,
      from: 6,
    }), 'accept');
    expect(doc.getText('contents').toString()).toBe('Hello world end');
  });

  it('reject addition: removes entirely', () => {
    const markup = '{++{"author":"AI","timestamp":1000}@@world++}';
    const doc = makeDoc(`Hello ${markup} end`);
    applySuggestionAction(doc, makeSuggestion({
      type: 'addition',
      content: 'world',
      raw_markup: markup,
      from: 6,
    }), 'reject');
    expect(doc.getText('contents').toString()).toBe('Hello  end');
  });

  it('accept deletion: removes content', () => {
    const markup = '{--{"author":"AI","timestamp":1000}@@removed--}';
    const doc = makeDoc(`Keep ${markup} this`);
    applySuggestionAction(doc, makeSuggestion({
      type: 'deletion',
      content: 'removed',
      raw_markup: markup,
      from: 5,
    }), 'accept');
    expect(doc.getText('contents').toString()).toBe('Keep  this');
  });

  it('reject deletion: keeps content', () => {
    const markup = '{--{"author":"AI","timestamp":1000}@@removed--}';
    const doc = makeDoc(`Keep ${markup} this`);
    applySuggestionAction(doc, makeSuggestion({
      type: 'deletion',
      content: 'removed',
      raw_markup: markup,
      from: 5,
    }), 'reject');
    expect(doc.getText('contents').toString()).toBe('Keep removed this');
  });

  it('accept substitution: keeps new content', () => {
    const markup = '{~~{"author":"AI","timestamp":1000}@@hello~>goodbye~~}';
    const doc = makeDoc(`Say ${markup} now`);
    applySuggestionAction(doc, makeSuggestion({
      type: 'substitution',
      old_content: 'hello',
      new_content: 'goodbye',
      raw_markup: markup,
      from: 4,
    }), 'accept');
    expect(doc.getText('contents').toString()).toBe('Say goodbye now');
  });

  it('reject substitution: keeps old content', () => {
    const markup = '{~~{"author":"AI","timestamp":1000}@@hello~>goodbye~~}';
    const doc = makeDoc(`Say ${markup} now`);
    applySuggestionAction(doc, makeSuggestion({
      type: 'substitution',
      old_content: 'hello',
      new_content: 'goodbye',
      raw_markup: markup,
      from: 4,
    }), 'reject');
    expect(doc.getText('contents').toString()).toBe('Say hello now');
  });

  it('finds markup even if position has shifted', () => {
    const markup = '{++{"author":"AI","timestamp":1000}@@world++}';
    const doc = makeDoc(`Extra --- Hello ${markup} end`);
    applySuggestionAction(doc, makeSuggestion({
      type: 'addition',
      content: 'world',
      raw_markup: markup,
      from: 5, // stale position
    }), 'accept');
    expect(doc.getText('contents').toString()).toBe('Extra --- Hello world end');
  });

  it('throws if markup not found in document', () => {
    const doc = makeDoc('No markup here');
    expect(() =>
      applySuggestionAction(doc, makeSuggestion({
        type: 'addition',
        content: 'world',
        raw_markup: '{++world++}',
        from: 0,
      }), 'accept')
    ).toThrow('Suggestion no longer found in document');
  });
});
