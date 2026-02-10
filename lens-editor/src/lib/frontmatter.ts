export interface DocFrontmatter {
  discussion?: string;
  [key: string]: unknown;
}

export function extractFrontmatter(_text: string): DocFrontmatter | null {
  // STUB: returns wrong value for RED phase
  return { stub: true };
}
