import fm from 'front-matter';

export interface DocFrontmatter {
  discussion?: string;
  [key: string]: unknown;
}

/**
 * Extract frontmatter attributes from a markdown string.
 * Returns null if the text has no valid frontmatter delimiters.
 */
export function extractFrontmatter(text: string): DocFrontmatter | null {
  if (!fm.test(text)) return null;
  try {
    const { attributes } = fm<DocFrontmatter>(text);
    return attributes;
  } catch {
    return null;
  }
}
