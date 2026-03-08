import { describe, test, expect } from 'vitest';
import { maskNonContent } from './harper';

// Helper: check that masked text has same length as input
function expectSameLength(input: string) {
  const result = maskNonContent(input);
  expect(result.length).toBe(input.length);
  return result;
}

// Helper: check a character is blanked (space) at a given offset
function expectBlanked(result: string, from: number, to: number) {
  const region = result.slice(from, to);
  expect(region).toBe(' '.repeat(to - from));
}

// Helper: check a region is preserved exactly
function expectPreserved(input: string, result: string, from: number, to: number) {
  expect(result.slice(from, to)).toBe(input.slice(from, to));
}

describe('maskNonContent', () => {
  test('blanks frontmatter', () => {
    const input = '---\nid: abc\ntags: lens\n---\nsome text';
    const result = expectSameLength(input);
    // Each line before "some text" should be blanked (spaces), newlines preserved
    expect(result.slice(0, 3)).toBe('   '); // ---
    expect(result[3]).toBe('\n');
    // "some text" is not after content::, so it's also blanked
    const contentStart = input.indexOf('some text');
    expect(result.slice(contentStart).trim()).toBe('');
  });

  test('blanks headings', () => {
    const input = '### My Heading\nsome text';
    const result = expectSameLength(input);
    expectBlanked(result, 0, '### My Heading'.length);
  });

  test('blanks everything by default (no content:: field)', () => {
    const input = '---\nid: abc\n---\n### Heading\nsource:: http://example.com\nSome random text here';
    const result = expectSameLength(input);
    // Everything should be blanked — no content:: to whitelist
    expect(result.trim()).toBe('');
  });

  test('keeps text after content:: until next heading', () => {
    const input = [
      '---',
      'id: abc',
      '---',
      '### Article Title',
      'content::',
      'This is the prose that should be linted.',
      'Second paragraph of content.',
      '### Next Section',
      'source:: ../foo',
    ].join('\n');

    const result = expectSameLength(input);

    // The prose lines should be preserved
    const prose1 = 'This is the prose that should be linted.';
    const prose2 = 'Second paragraph of content.';
    const idx1 = input.indexOf(prose1);
    const idx2 = input.indexOf(prose2);
    expectPreserved(input, result, idx1, idx1 + prose1.length);
    expectPreserved(input, result, idx2, idx2 + prose2.length);

    // Frontmatter, headings, content:: line itself, and source:: should be blanked
    const headingIdx = input.indexOf('### Article Title');
    expectBlanked(result, headingIdx, headingIdx + '### Article Title'.length);

    const contentKeyIdx = input.indexOf('content::');
    expectBlanked(result, contentKeyIdx, contentKeyIdx + 'content::'.length);

    const nextHeadingIdx = input.indexOf('### Next Section');
    expectBlanked(result, nextHeadingIdx, nextHeadingIdx + '### Next Section'.length);
  });

  test('blanks the content:: key line itself', () => {
    const input = 'content::\nHello world';
    const result = expectSameLength(input);
    expectBlanked(result, 0, 'content::'.length);
    const helloIdx = input.indexOf('Hello world');
    expectPreserved(input, result, helloIdx, helloIdx + 'Hello world'.length);
  });

  test('another key:: line ends the content region', () => {
    const input = [
      'content::',
      'Prose to lint.',
      'source:: http://example.com',
      'More text after source.',
    ].join('\n');

    const result = expectSameLength(input);

    // Prose should be kept
    const proseIdx = input.indexOf('Prose to lint.');
    expectPreserved(input, result, proseIdx, proseIdx + 'Prose to lint.'.length);

    // source:: line should be blanked
    const sourceIdx = input.indexOf('source:: http://example.com');
    expectBlanked(result, sourceIdx, sourceIdx + 'source:: http://example.com'.length);

    // "More text after source." should be blanked (no longer in content region)
    const moreIdx = input.indexOf('More text after source.');
    expectBlanked(result, moreIdx, moreIdx + 'More text after source.'.length);
  });

  test('multiple content:: sections in one document', () => {
    const input = [
      '### Section 1',
      'content::',
      'First content block.',
      '### Section 2',
      'to:: "some quote"',
      '### Section 3',
      'content::',
      'Second content block.',
    ].join('\n');

    const result = expectSameLength(input);

    const first = 'First content block.';
    const second = 'Second content block.';
    const firstIdx = input.indexOf(first);
    const secondIdx = input.indexOf(second);

    expectPreserved(input, result, firstIdx, firstIdx + first.length);
    expectPreserved(input, result, secondIdx, secondIdx + second.length);

    // "some quote" after to:: should be blanked
    const quoteIdx = input.indexOf('"some quote"');
    expectBlanked(result, quoteIdx, quoteIdx + '"some quote"'.length);
  });

  test('realistic lens document', () => {
    const input = [
      '---',
      'id: 11f0d83f-f8ec-4549-b82c-460c22288a9b',
      'tags :',
      '  • lens',
      '  • work-in-progress',
      '---',
      '### Article: 6 reasons why alignment-is-hard',
      '',
      '',
      'source:: ../articles/byrnes-6-reasons',
      '',
      '',
      '#### Text',
      'content::',
      'The difficulty of AI alignment is often underestimated. This should be linted.',
      '',
      '',
      '#### Article-excerpt',
      'to:: "invoking a misleading mental image"',
      '',
      '',
      '#### Text',
      'content::',
      'Many people intuitively assume that a truly superintelligent AI will possess common sense.',
      '#### Chat: Discussion on X-Risk',
      'instructions::',
      'The participant is answering this question:',
    ].join('\n');

    const result = expectSameLength(input);

    // Content blocks should be preserved
    const content1 = 'The difficulty of AI alignment is often underestimated. This should be linted.';
    const content2 = 'Many people intuitively assume that a truly superintelligent AI will possess common sense.';
    expectPreserved(input, result, input.indexOf(content1), input.indexOf(content1) + content1.length);
    expectPreserved(input, result, input.indexOf(content2), input.indexOf(content2) + content2.length);

    // Non-content should be blanked
    const heading1 = '### Article: 6 reasons why alignment-is-hard';
    expectBlanked(result, input.indexOf(heading1), input.indexOf(heading1) + heading1.length);

    // instructions:: content should be blanked (not whitelisted)
    const instructions = 'The participant is answering this question:';
    const instrIdx = input.indexOf(instructions);
    expectBlanked(result, instrIdx, instrIdx + instructions.length);

    // to:: value should be blanked
    const toValue = '"invoking a misleading mental image"';
    const toIdx = input.indexOf(toValue);
    expectBlanked(result, toIdx, toIdx + toValue.length);
  });

  test('does not treat :: in prose as a key separator', () => {
    const input = [
      'content::',
      'The word foo:: appears in prose but should not break the content region.',
    ].join('\n');

    const result = expectSameLength(input);

    const proseIdx = input.indexOf('The word');
    const prose = 'The word foo:: appears in prose but should not break the content region.';
    expectPreserved(input, result, proseIdx, proseIdx + prose.length);
  });

  test('prose mentioning "content" is not treated as content:: key', () => {
    const input = [
      'content::',
      'The content of this document is important.',
    ].join('\n');

    const result = expectSameLength(input);

    // "The content of..." should be preserved — "content" mid-sentence is not a key
    const proseIdx = input.indexOf('The content');
    const prose = 'The content of this document is important.';
    expectPreserved(input, result, proseIdx, proseIdx + prose.length);
  });

  test('empty lines within content region are preserved', () => {
    const input = [
      'content::',
      'First paragraph.',
      '',
      'Second paragraph.',
      '### Next',
    ].join('\n');

    const result = expectSameLength(input);

    expectPreserved(input, result, input.indexOf('First'), input.indexOf('First') + 'First paragraph.'.length);
    expectPreserved(input, result, input.indexOf('Second'), input.indexOf('Second') + 'Second paragraph.'.length);
    // Empty line should also be preserved (it's just '\n')
    const emptyLineIdx = input.indexOf('\n\n') + 1;
    expect(result[emptyLineIdx]).toBe('\n');
  });

  test('empty document returns empty string of same length', () => {
    expect(maskNonContent('')).toBe('');
  });

  test('content with no structure is blanked entirely', () => {
    const input = 'Just some random text with no structure';
    const result = maskNonContent(input);
    expect(result).toBe(' '.repeat(input.length));
  });
});
