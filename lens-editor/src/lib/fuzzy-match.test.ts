import { describe, it, expect } from 'vitest';
import { fuzzyMatch } from './fuzzy-match';

describe('fuzzyMatch', () => {
  it('returns no match when query chars are not in target in order', () => {
    const result = fuzzyMatch('zxy', 'hello world');
    expect(result.match).toBe(false);
    expect(result.score).toBe(0);
    expect(result.ranges).toEqual([]);
  });

  it('matches exact substring', () => {
    const result = fuzzyMatch('hello', 'hello world');
    expect(result.match).toBe(true);
    expect(result.score).toBeGreaterThan(0);
    expect(result.ranges).toEqual([[0, 5]]);
  });

  it('matches scattered characters in order', () => {
    const result = fuzzyMatch('hlo', 'hello');
    expect(result.match).toBe(true);
    expect(result.ranges.length).toBeGreaterThan(0);
  });

  it('is case-insensitive', () => {
    const result = fuzzyMatch('HeLLo', 'hello world');
    expect(result.match).toBe(true);
  });

  it('scores contiguous matches higher than scattered', () => {
    const contiguous = fuzzyMatch('hell', 'hello');
    const scattered = fuzzyMatch('helo', 'help docs');
    expect(contiguous.score).toBeGreaterThan(scattered.score);
  });

  it('scores word-boundary matches higher', () => {
    const boundary = fuzzyMatch('tw', 'tree-walker');
    const mid = fuzzyMatch('tw', 'between');
    expect(boundary.score).toBeGreaterThan(mid.score);
  });

  it('scores shorter targets higher for same match quality', () => {
    const short = fuzzyMatch('abc', 'abc');
    const long = fuzzyMatch('abc', 'abc-something-very-long');
    expect(short.score).toBeGreaterThan(long.score);
  });

  it('returns correct ranges for highlighting', () => {
    const result = fuzzyMatch('ac', 'abcd');
    expect(result.match).toBe(true);
    // fuzzysort may find optimal positions; just verify ranges are valid
    expect(result.ranges.length).toBeGreaterThan(0);
    for (const [start, end] of result.ranges) {
      expect(start).toBeGreaterThanOrEqual(0);
      expect(end).toBeGreaterThan(start);
      expect(end).toBeLessThanOrEqual(4);
    }
  });

  it('handles empty query', () => {
    const result = fuzzyMatch('', 'hello');
    expect(result.match).toBe(true);
    expect(result.score).toBe(0);
    expect(result.ranges).toEqual([]);
  });

  it('handles empty target', () => {
    const result = fuzzyMatch('a', '');
    expect(result.match).toBe(false);
  });

  it('matches space in query against / in target (path-aware)', () => {
    const result = fuzzyMatch('resources links', 'Relay Folder 2/Resources/Links');
    expect(result.match).toBe(true);
    expect(result.score).toBeGreaterThan(0);
  });

  it('matches spaces in query against spaces in target (filename with spaces)', () => {
    const result = fuzzyMatch('Chat Panel', 'Lens/AI Chat Panel');
    expect(result.match).toBe(true);
    expect(result.score).toBeGreaterThan(0);
  });

  it('matches spaces in filenames across path segments', () => {
    const result = fuzzyMatch('Getting Started', 'Lens Edu/Modules/Module_x/Getting Started');
    expect(result.match).toBe(true);
    expect(result.score).toBeGreaterThan(0);
  });

  it('ranks substring match above scattered character match for "demo"', () => {
    const substringMatch = fuzzyMatch('demo', 'Detailed Demo Notes.md');
    const scatteredMatch = fuzzyMatch('demo', 'Docs/Early/Methods/Outline.md');
    expect(substringMatch.score).toBeGreaterThan(scatteredMatch.score);
  });
});
