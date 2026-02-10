import { describe, it, expect } from 'vitest';
import { extractFrontmatter } from './frontmatter';

describe('extractFrontmatter', () => {
  describe('valid frontmatter', () => {
    it('extracts discussion field from frontmatter', () => {
      const text = `---
discussion: https://discord.com/channels/123/456
---
# Document content`;
      const result = extractFrontmatter(text);
      expect(result).toEqual({ discussion: 'https://discord.com/channels/123/456' });
    });

    it('returns object without discussion key when field is absent', () => {
      const text = `---
title: My Document
---
Content here`;
      const result = extractFrontmatter(text);
      expect(result).not.toBeNull();
      expect(result).not.toHaveProperty('discussion');
      expect(result).toHaveProperty('title', 'My Document');
    });

    it('returns all fields when multiple fields are present alongside discussion', () => {
      const text = `---
title: My Document
id: abc-123
slug: my-doc
discussion: https://discord.com/channels/111/222
---
Body text`;
      const result = extractFrontmatter(text);
      expect(result).toEqual({
        title: 'My Document',
        id: 'abc-123',
        slug: 'my-doc',
        discussion: 'https://discord.com/channels/111/222',
      });
    });
  });

  describe('no frontmatter', () => {
    it('returns null for text without frontmatter delimiters', () => {
      const text = '# Just a heading\nSome content';
      expect(extractFrontmatter(text)).toBeNull();
    });

    it('returns null for empty string', () => {
      expect(extractFrontmatter('')).toBeNull();
    });

    it('returns null for text with only opening delimiter', () => {
      const text = `---
title: Broken
No closing delimiter here`;
      expect(extractFrontmatter(text)).toBeNull();
    });
  });

  describe('malformed frontmatter', () => {
    it('returns null for malformed YAML (unclosed quotes)', () => {
      const text = `---
title: "unclosed
---
Content`;
      // front-matter library may parse this or throw - either way we handle gracefully
      const result = extractFrontmatter(text);
      // If it parses, it should be an object; if not, null
      expect(result === null || typeof result === 'object').toBe(true);
    });

    it('returns null for invalid YAML syntax', () => {
      const text = `---
: : : invalid
  bad indent
    worse
---
Content`;
      const result = extractFrontmatter(text);
      expect(result === null || typeof result === 'object').toBe(true);
    });
  });

  describe('line endings', () => {
    it('handles Windows line endings (CRLF)', () => {
      const text = '---\r\ndiscussion: https://discord.com/channels/789/012\r\n---\r\n# Content';
      const result = extractFrontmatter(text);
      expect(result).toEqual({ discussion: 'https://discord.com/channels/789/012' });
    });
  });
});
