import { describe, it, expect, afterEach, vi } from 'vitest';
import {
  createTestEditor,
  moveCursor,
  hasClass,
  countClass,
} from '../../../test/codemirror-helpers';
import { updateWikilinkContext, wikilinkMetadataChanged } from './livePreview';
import { resolvePageName } from '../../../lib/document-resolver';
import type { FolderMetadata } from '../../../hooks/useFolderMetadata';

describe('livePreview - emphasis markers', () => {
  let cleanup: () => void;

  afterEach(() => {
    if (cleanup) cleanup();
  });

  it('applies cm-emphasis class to italic text when cursor outside', () => {
    const { view, cleanup: c } = createTestEditor('*italic* end', 10);
    cleanup = c;

    expect(hasClass(view, 'cm-emphasis')).toBe(true);
  });

  it('applies cm-strong class to bold text when cursor outside', () => {
    const { view, cleanup: c } = createTestEditor('**bold** end', 12);
    cleanup = c;

    expect(hasClass(view, 'cm-strong')).toBe(true);
  });

  it('hides emphasis markers when cursor is outside element', () => {
    const { view, cleanup: c } = createTestEditor('*italic* end', 10);
    cleanup = c;

    // The * markers should have cm-hidden-syntax class
    expect(hasClass(view, 'cm-hidden-syntax')).toBe(true);
  });

  it('shows emphasis markers when cursor is inside element', () => {
    // Cursor at position 3 = inside "italic"
    const { view, cleanup: c } = createTestEditor('*italic* end', 3);
    cleanup = c;

    // When cursor is inside, markers should NOT be hidden
    // The cm-emphasis class should NOT be applied (raw text visible)
    expect(hasClass(view, 'cm-emphasis')).toBe(false);
  });

  it('updates decorations when cursor moves in and out', () => {
    const { view, cleanup: c } = createTestEditor('*italic* text', 12);
    cleanup = c;

    // Initially outside: emphasis styled, markers hidden
    expect(hasClass(view, 'cm-emphasis')).toBe(true);

    // Move cursor inside
    moveCursor(view, 3);

    // Now inside: raw markdown visible, no emphasis class
    expect(hasClass(view, 'cm-emphasis')).toBe(false);

    // Move cursor back outside
    moveCursor(view, 12);

    // Outside again: emphasis styled
    expect(hasClass(view, 'cm-emphasis')).toBe(true);
  });
});

describe('livePreview - heading markers', () => {
  let cleanup: () => void;

  afterEach(() => {
    if (cleanup) cleanup();
  });

  it('applies heading class for h1 when cursor on different line', () => {
    const content = '# Heading\n\nParagraph';
    const { view, cleanup: c } = createTestEditor(content, 20);
    cleanup = c;

    expect(hasClass(view, 'cm-heading-1')).toBe(true);
  });

  it('hides # marker when cursor is on different line', () => {
    const content = '# Heading\n\nParagraph';
    const { view, cleanup: c } = createTestEditor(content, 20);
    cleanup = c;

    expect(hasClass(view, 'cm-hidden-syntax')).toBe(true);
  });

  it('shows # marker when cursor is on heading line', () => {
    const content = '# Heading\n\nParagraph';
    // Cursor on heading line (position 3 = in "Heading")
    const { view, cleanup: c } = createTestEditor(content, 3);
    cleanup = c;

    // # should NOT be hidden when cursor on same line
    // Check that there's no hidden-syntax class on the # mark
    const hiddenCount = countClass(view, 'cm-hidden-syntax');
    // The # mark should be visible, so hidden count should be 0 for header marks
    expect(hiddenCount).toBe(0);
  });

  it('applies correct heading classes for h1 through h6', () => {
    const content = `# H1
## H2
### H3
#### H4
##### H5
###### H6`;

    // Cursor at end
    const { view, cleanup: c } = createTestEditor(content, content.length);
    cleanup = c;

    expect(hasClass(view, 'cm-heading-1')).toBe(true);
    expect(hasClass(view, 'cm-heading-2')).toBe(true);
    expect(hasClass(view, 'cm-heading-3')).toBe(true);
    expect(hasClass(view, 'cm-heading-4')).toBe(true);
    expect(hasClass(view, 'cm-heading-5')).toBe(true);
    expect(hasClass(view, 'cm-heading-6')).toBe(true);
  });
});

describe('livePreview - wikilinks', () => {
  let cleanup: () => void;

  // Test metadata with real document entries for resolution testing
  const testMetadata: FolderMetadata = {
    '/My Page.md': { id: 'doc-1', type: 'markdown', version: 0 },
    '/Existing Page.md': { id: 'doc-2', type: 'markdown', version: 0 },
  };

  // Create context with real resolution logic
  const createRealContext = () => ({
    onClick: () => {},
    isResolved: (pageName: string) => resolvePageName(pageName, testMetadata) !== null,
  });

  afterEach(() => {
    if (cleanup) cleanup();
  });

  it('replaces wikilink with widget when cursor is outside', () => {
    const content = '[[Page Name]] more';
    const { view, cleanup: c } = createTestEditor(content, 18);
    cleanup = c;

    expect(hasClass(view, 'cm-wikilink-widget')).toBe(true);
  });

  it('shows raw [[ ]] when cursor is inside wikilink', () => {
    const content = '[[Page Name]] more';
    // Cursor inside wikilink (position 5)
    const { view, cleanup: c } = createTestEditor(content, 5);
    cleanup = c;

    // Widget should NOT be present when cursor inside
    expect(hasClass(view, 'cm-wikilink-widget')).toBe(false);
  });

  it('widget displays page name text', () => {
    const content = '[[My Page]] end';

    const { view, cleanup: c } = createTestEditor(content, 15, createRealContext());
    cleanup = c;

    const widgets = view.contentDOM.querySelectorAll('.cm-wikilink-widget');
    expect(widgets.length).toBe(1);
    expect(widgets[0].textContent).toBe('My Page');
  });

  it('marks unresolved links with unresolved class', () => {
    const content = '[[NonExistent]] more text';

    // NonExistent is not in testMetadata, so isResolved will return false
    const { view, cleanup: c } = createTestEditor(content, 25, createRealContext());
    cleanup = c;

    expect(hasClass(view, 'unresolved')).toBe(true);
  });

  it('does not mark resolved links with unresolved class', () => {
    const content = '[[Existing Page]] more text';

    // Existing Page is in testMetadata, so isResolved will return true
    const { view, cleanup: c } = createTestEditor(content, 27, createRealContext());
    cleanup = c;

    const widget = view.contentDOM.querySelector('.cm-wikilink-widget');
    expect(widget).not.toBeNull();
    expect(widget!.classList.contains('unresolved')).toBe(false);
  });

  it('replaces ![[Page]] embed with widget when cursor outside', () => {
    const content = '![[Page Name]] more';
    const { view, cleanup: c } = createTestEditor(content, 19);
    cleanup = c;
    expect(hasClass(view, 'cm-wikilink-widget')).toBe(true);
  });

  it('widget displays page name for embed syntax', () => {
    const content = '![[My Page]] end';
    const { view, cleanup: c } = createTestEditor(content, 16, createRealContext());
    cleanup = c;
    const widgets = view.contentDOM.querySelectorAll('.cm-wikilink-widget');
    expect(widgets.length).toBe(1);
    expect(widgets[0].textContent).toBe('My Page');
  });

  it('calls onOpenNewTab on ctrl+click', () => {
    const onClick = vi.fn();
    const onOpenNewTab = vi.fn();
    const { view, cleanup: c } = createTestEditor('[[My Page]] end', 15, {
      onClick,
      onOpenNewTab,
      isResolved: () => true,
    });
    cleanup = c;

    const widget = view.contentDOM.querySelector('.cm-wikilink-widget') as HTMLElement;
    expect(widget).not.toBeNull();
    widget.dispatchEvent(new MouseEvent('click', { bubbles: true, ctrlKey: true }));
    expect(onOpenNewTab).toHaveBeenCalledWith('My Page');
    expect(onClick).not.toHaveBeenCalled();
  });

  it('calls onOpenNewTab on meta+click', () => {
    const onClick = vi.fn();
    const onOpenNewTab = vi.fn();
    const { view, cleanup: c } = createTestEditor('[[My Page]] end', 15, {
      onClick,
      onOpenNewTab,
      isResolved: () => true,
    });
    cleanup = c;

    const widget = view.contentDOM.querySelector('.cm-wikilink-widget') as HTMLElement;
    expect(widget).not.toBeNull();
    widget.dispatchEvent(new MouseEvent('click', { bubbles: true, metaKey: true }));
    expect(onOpenNewTab).toHaveBeenCalledWith('My Page');
    expect(onClick).not.toHaveBeenCalled();
  });

  it('calls onOpenNewTab on middle-click', () => {
    const onClick = vi.fn();
    const onOpenNewTab = vi.fn();
    const { view, cleanup: c } = createTestEditor('[[My Page]] end', 15, {
      onClick,
      onOpenNewTab,
      isResolved: () => true,
    });
    cleanup = c;

    const widget = view.contentDOM.querySelector('.cm-wikilink-widget') as HTMLElement;
    expect(widget).not.toBeNull();
    widget.dispatchEvent(new MouseEvent('auxclick', { bubbles: true, button: 1 }));
    expect(onOpenNewTab).toHaveBeenCalledWith('My Page');
    expect(onClick).not.toHaveBeenCalled();
  });

  it('calls onClick on plain click (no modifiers)', () => {
    const onClick = vi.fn();
    const onOpenNewTab = vi.fn();
    const { view, cleanup: c } = createTestEditor('[[My Page]] end', 15, {
      onClick,
      onOpenNewTab,
      isResolved: () => true,
    });
    cleanup = c;

    const widget = view.contentDOM.querySelector('.cm-wikilink-widget') as HTMLElement;
    expect(widget).not.toBeNull();
    widget.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    expect(onClick).toHaveBeenCalledWith('My Page');
    expect(onOpenNewTab).not.toHaveBeenCalled();
  });

  it('updates widget resolved state when metadata changes', () => {
    const content = '[[Target Page]] end';

    // Start with isResolved returning false
    const { view, cleanup: c } = createTestEditor(content, 19, {
      onClick: () => {},
      isResolved: () => false,
    });
    cleanup = c;

    // Widget should be unresolved
    const widget = view.contentDOM.querySelector('.cm-wikilink-widget');
    expect(widget).not.toBeNull();
    expect(widget!.classList.contains('unresolved')).toBe(true);

    // Update context so isResolved returns true
    updateWikilinkContext({
      onClick: () => {},
      isResolved: () => true,
    });

    // Dispatch the metadata changed effect to trigger rebuild
    view.dispatch({
      effects: wikilinkMetadataChanged.of(undefined),
    });

    // Widget should now be resolved (no unresolved class)
    const updatedWidget = view.contentDOM.querySelector('.cm-wikilink-widget');
    expect(updatedWidget).not.toBeNull();
    expect(updatedWidget!.classList.contains('unresolved')).toBe(false);
  });
});

describe('livePreview - markdown links', () => {
  let cleanup: () => void;

  afterEach(() => {
    if (cleanup) cleanup();
  });

  it('replaces [text](url) with widget when cursor is outside', () => {
    const content = '[Example](https://example.com) more';
    const { view, cleanup: c } = createTestEditor(content, 35);
    cleanup = c;

    expect(hasClass(view, 'cm-link-widget')).toBe(true);
  });

  it('shows raw markdown when cursor is inside link', () => {
    const content = '[Example](https://example.com) more';
    // Cursor inside link text
    const { view, cleanup: c } = createTestEditor(content, 5);
    cleanup = c;

    expect(hasClass(view, 'cm-link-widget')).toBe(false);
  });

  it('widget displays link text', () => {
    const content = '[Click Here](url) end';
    const { view, cleanup: c } = createTestEditor(content, 20);
    cleanup = c;

    const widgets = view.contentDOM.querySelectorAll('.cm-link-widget');
    expect(widgets.length).toBe(1);
    expect(widgets[0].textContent).toContain('Click Here');
  });
});

describe('livePreview - autolinks (bare URLs)', () => {
  let cleanup: () => void;

  afterEach(() => {
    if (cleanup) cleanup();
  });

  it('replaces bare URL with link widget when cursor is outside', () => {
    const content = 'Visit https://example.com for more';
    const { view, cleanup: c } = createTestEditor(content, 0);
    cleanup = c;

    expect(hasClass(view, 'cm-link-widget')).toBe(true);
  });

  it('shows raw URL when cursor is on autolink', () => {
    const content = 'Visit https://example.com for more';
    // Cursor inside the URL
    const { view, cleanup: c } = createTestEditor(content, 15);
    cleanup = c;

    expect(hasClass(view, 'cm-link-widget')).toBe(false);
  });

  it('replaces angle-bracket autolink with widget when cursor outside', () => {
    const content = 'See <https://example.com> here';
    const { view, cleanup: c } = createTestEditor(content, 0);
    cleanup = c;

    expect(hasClass(view, 'cm-link-widget')).toBe(true);
  });

  it('widget displays URL as text', () => {
    const content = 'Visit https://example.com end';
    const { view, cleanup: c } = createTestEditor(content, content.length);
    cleanup = c;

    const widgets = view.contentDOM.querySelectorAll('.cm-link-widget');
    expect(widgets.length).toBe(1);
    expect(widgets[0].textContent).toContain('https://example.com');
  });
});

describe('livePreview - inline code', () => {
  let cleanup: () => void;

  afterEach(() => {
    if (cleanup) cleanup();
  });

  it('applies inline-code class when cursor is outside', () => {
    const content = 'Use `code` here';
    const { view, cleanup: c } = createTestEditor(content, 15);
    cleanup = c;

    expect(hasClass(view, 'cm-inline-code')).toBe(true);
  });

  it('hides backticks when cursor is outside', () => {
    const content = 'Use `code` here';
    const { view, cleanup: c } = createTestEditor(content, 15);
    cleanup = c;

    expect(hasClass(view, 'cm-hidden-syntax')).toBe(true);
  });

  it('shows backticks when cursor is inside code', () => {
    const content = 'Use `code` here';
    // Cursor inside code
    const { view, cleanup: c } = createTestEditor(content, 6);
    cleanup = c;

    // Inline code styling should persist even when cursor is inside
    expect(hasClass(view, 'cm-inline-code')).toBe(true);
  });
});

describe('livePreview - bullet lists', () => {
  let cleanup: () => void;

  afterEach(() => {
    if (cleanup) cleanup();
  });

  it('replaces bullet marker with dot widget when cursor outside', () => {
    const content = '- item one\n\nParagraph';
    const { view, cleanup: c } = createTestEditor(content, 20);
    cleanup = c;

    expect(hasClass(view, 'cm-bullet')).toBe(true);
  });

  it('shows raw - marker when cursor touches marker', () => {
    // Cursor at position 0 = on the `-` character
    const content = '- item one\n\nParagraph';
    const { view, cleanup: c } = createTestEditor(content, 0);
    cleanup = c;

    expect(hasClass(view, 'cm-bullet')).toBe(false);
  });

  it('updates when cursor moves in and out of marker', () => {
    const content = '- item one\n\nParagraph';
    const { view, cleanup: c } = createTestEditor(content, 20);
    cleanup = c;

    // Initially outside: bullet widget shown
    expect(hasClass(view, 'cm-bullet')).toBe(true);

    // Move cursor onto marker
    moveCursor(view, 0);
    expect(hasClass(view, 'cm-bullet')).toBe(false);

    // Move cursor back outside
    moveCursor(view, 20);
    expect(hasClass(view, 'cm-bullet')).toBe(true);
  });

  it('does not replace ordered list markers', () => {
    const content = '1. first item\n\nParagraph';
    const { view, cleanup: c } = createTestEditor(content, 22);
    cleanup = c;

    expect(hasClass(view, 'cm-bullet')).toBe(false);
  });

  it('handles nested bullet lists', () => {
    const content = '- outer\n  - inner\n\nParagraph';
    const { view, cleanup: c } = createTestEditor(content, 27);
    cleanup = c;

    // Both bullets should be rendered
    expect(countClass(view, 'cm-bullet')).toBe(2);
  });

  it('replaces * and + bullet markers with dot widget', () => {
    const content = '* star item\n+ plus item\n\nParagraph';
    const { view, cleanup: c } = createTestEditor(content, 30);
    cleanup = c;

    expect(countClass(view, 'cm-bullet')).toBe(2);
  });

  it('does not replace bullet marker on task list items', () => {
    // Task list items are handled by the checklist code, not bullet code
    const content = '- [ ] task item\n\nParagraph';
    const { view, cleanup: c } = createTestEditor(content, 25);
    cleanup = c;

    expect(hasClass(view, 'cm-bullet')).toBe(false);
  });
});

describe('livePreview - fenced code blocks', () => {
  let cleanup: () => void;

  afterEach(() => {
    if (cleanup) cleanup();
  });

  it('applies cm-code-block line class to code lines when cursor outside', () => {
    const content = 'before\n```\ncode line\n```\nafter';
    const { view, cleanup: c } = createTestEditor(content, 0);
    cleanup = c;

    expect(hasClass(view, 'cm-code-block')).toBe(true);
  });

  it('applies cm-code-block line class when cursor inside too', () => {
    const content = 'before\n```\ncode line\n```\nafter';
    // Cursor inside the code block on "code line"
    const { view, cleanup: c } = createTestEditor(content, 12);
    cleanup = c;

    expect(hasClass(view, 'cm-code-block')).toBe(true);
  });

  it('hides opening fence markers when cursor outside', () => {
    const content = 'before\n```\ncode line\n```\nafter';
    const { view, cleanup: c } = createTestEditor(content, 0);
    cleanup = c;

    expect(hasClass(view, 'cm-hidden-syntax')).toBe(true);
  });

  it('hides closing fence markers when cursor outside', () => {
    const content = 'before\n```\ncode line\n```\nafter';
    const { view, cleanup: c } = createTestEditor(content, content.length);
    cleanup = c;

    // Both opening and closing fences hidden = at least 2 hidden elements
    expect(countClass(view, 'cm-hidden-syntax')).toBeGreaterThanOrEqual(2);
  });

  it('hides language info when cursor outside', () => {
    const content = 'before\n```javascript\ncode line\n```\nafter';
    const { view, cleanup: c } = createTestEditor(content, 0);
    cleanup = c;

    // Language info + fence markers should all be hidden
    expect(hasClass(view, 'cm-hidden-syntax')).toBe(true);
  });

  it('shows fence markers when cursor inside code block', () => {
    const content = 'before\n```\ncode line\n```\nafter';
    // Cursor on "code line"
    const { view, cleanup: c } = createTestEditor(content, 12);
    cleanup = c;

    // Fence markers should NOT be hidden
    expect(countClass(view, 'cm-hidden-syntax')).toBe(0);
  });

  it('cursor moving in/out toggles fence visibility', () => {
    const content = 'before\n```\ncode line\n```\nafter';
    const { view, cleanup: c } = createTestEditor(content, 0);
    cleanup = c;

    // Initially outside: fences hidden
    expect(countClass(view, 'cm-hidden-syntax')).toBeGreaterThan(0);

    // Move cursor inside code block
    moveCursor(view, 12);
    expect(countClass(view, 'cm-hidden-syntax')).toBe(0);

    // Move cursor back outside
    moveCursor(view, 0);
    expect(countClass(view, 'cm-hidden-syntax')).toBeGreaterThan(0);
  });

  it('code block with no language info', () => {
    const content = 'before\n```\nplain code\n```\nafter';
    const { view, cleanup: c } = createTestEditor(content, 0);
    cleanup = c;

    expect(hasClass(view, 'cm-code-block')).toBe(true);
    expect(hasClass(view, 'cm-hidden-syntax')).toBe(true);
  });

  it('multiple code blocks styled independently', () => {
    const content = '```\nblock1\n```\ntext\n```\nblock2\n```';
    const { view, cleanup: c } = createTestEditor(content, content.indexOf('text'));
    cleanup = c;

    // Both blocks get cm-code-block lines
    // block1 has 3 lines, block2 has 3 lines = at least 6 code-block lines
    expect(countClass(view, 'cm-code-block')).toBeGreaterThanOrEqual(6);
  });

  it('inline code still works alongside fenced code blocks', () => {
    const content = 'Use `inline` here\n```\nfenced\n```\nmore';
    const { view, cleanup: c } = createTestEditor(content, content.length);
    cleanup = c;

    expect(hasClass(view, 'cm-inline-code')).toBe(true);
    expect(hasClass(view, 'cm-code-block')).toBe(true);
  });
});

describe('livePreview - inline images', () => {
  let cleanup: () => void;

  afterEach(() => {
    if (cleanup) cleanup();
  });

  it('replaces ![alt](url) with image widget when cursor outside', () => {
    const content = '![photo](https://example.com/img.png) more';
    const { view, cleanup: c } = createTestEditor(content, content.length);
    cleanup = c;

    expect(hasClass(view, 'cm-image-widget')).toBe(true);
  });

  it('shows raw markdown when cursor is inside image syntax', () => {
    const content = '![photo](https://example.com/img.png) more';
    // Cursor inside the image syntax
    const { view, cleanup: c } = createTestEditor(content, 5);
    cleanup = c;

    expect(hasClass(view, 'cm-image-widget')).toBe(false);
  });

  it('widget contains img element with correct src and alt', () => {
    const content = '![my alt](https://example.com/pic.jpg) end';
    const { view, cleanup: c } = createTestEditor(content, content.length);
    cleanup = c;

    const img = view.contentDOM.querySelector('.cm-image-preview') as HTMLImageElement | null;
    expect(img).not.toBeNull();
    expect(img!.src).toBe('https://example.com/pic.jpg');
    expect(img!.alt).toBe('my alt');
  });

  it('handles empty alt text ![](url)', () => {
    const content = '![](https://example.com/img.png) end';
    const { view, cleanup: c } = createTestEditor(content, content.length);
    cleanup = c;

    expect(hasClass(view, 'cm-image-widget')).toBe(true);
    const img = view.contentDOM.querySelector('.cm-image-preview') as HTMLImageElement | null;
    expect(img).not.toBeNull();
    expect(img!.alt).toBe('');
  });

  it('shows fallback for non-http URLs', () => {
    const content = '![alt](data:image/png;base64,abc) end';
    const { view, cleanup: c } = createTestEditor(content, content.length);
    cleanup = c;

    expect(hasClass(view, 'cm-image-widget')).toBe(true);
    expect(hasClass(view, 'cm-image-error')).toBe(true);
    // No actual img element should be rendered
    const img = view.contentDOM.querySelector('.cm-image-preview');
    expect(img).toBeNull();
  });

  it('does not interfere with regular links', () => {
    const content = '[text](https://example.com) end';
    const { view, cleanup: c } = createTestEditor(content, content.length);
    cleanup = c;

    expect(hasClass(view, 'cm-image-widget')).toBe(false);
    expect(hasClass(view, 'cm-link-widget')).toBe(true);
  });

  it('does not interfere with wikilink embeds', () => {
    const content = '![[My Page]] end';
    const { view, cleanup: c } = createTestEditor(content, content.length);
    cleanup = c;

    expect(hasClass(view, 'cm-image-widget')).toBe(false);
    expect(hasClass(view, 'cm-wikilink-widget')).toBe(true);
  });

  it('cursor move toggles between widget and raw markdown', () => {
    const content = '![photo](https://example.com/img.png) text';
    const { view, cleanup: c } = createTestEditor(content, content.length);
    cleanup = c;

    // Initially outside: image widget shown
    expect(hasClass(view, 'cm-image-widget')).toBe(true);

    // Move cursor inside image syntax
    moveCursor(view, 5);
    expect(hasClass(view, 'cm-image-widget')).toBe(false);

    // Move cursor back outside
    moveCursor(view, content.length);
    expect(hasClass(view, 'cm-image-widget')).toBe(true);
  });

  it('handles image with title ![alt](url "title")', () => {
    const content = '![photo](https://example.com/img.png "A title") end';
    const { view, cleanup: c } = createTestEditor(content, content.length);
    cleanup = c;

    expect(hasClass(view, 'cm-image-widget')).toBe(true);
    const img = view.contentDOM.querySelector('.cm-image-preview') as HTMLImageElement | null;
    expect(img).not.toBeNull();
    expect(img!.src).toBe('https://example.com/img.png');
  });
});

describe('livePreview - checklists', () => {
  let cleanup: () => void;

  afterEach(() => {
    if (cleanup) cleanup();
  });

  it('replaces unchecked task with checkbox widget when cursor outside', () => {
    const content = '- [ ] buy milk\n\nParagraph';
    const { view, cleanup: c } = createTestEditor(content, 24);
    cleanup = c;

    expect(hasClass(view, 'cm-checkbox')).toBe(true);
  });

  it('replaces checked task with checked checkbox when cursor outside', () => {
    const content = '- [x] buy milk\n\nParagraph';
    const { view, cleanup: c } = createTestEditor(content, 24);
    cleanup = c;

    const checkbox = view.contentDOM.querySelector('.cm-checkbox') as HTMLInputElement | null;
    expect(checkbox).not.toBeNull();
    expect(checkbox!.checked).toBe(true);
  });

  it('treats uppercase [X] as checked', () => {
    const content = '- [X] uppercase check\n\nParagraph';
    const { view, cleanup: c } = createTestEditor(content, 30);
    cleanup = c;

    const checkbox = view.contentDOM.querySelector('.cm-checkbox') as HTMLInputElement | null;
    expect(checkbox).not.toBeNull();
    expect(checkbox!.checked).toBe(true);
    expect(hasClass(view, 'cm-task-completed')).toBe(true);
  });

  it('shows raw [ ] when cursor touches checkbox marker', () => {
    const content = '- [ ] buy milk\n\nParagraph';
    // Cursor at position 2 = on the `[` character
    const { view, cleanup: c } = createTestEditor(content, 2);
    cleanup = c;

    expect(hasClass(view, 'cm-checkbox')).toBe(false);
  });

  it('reveals raw markdown when cursor is right after ] (position 5, touching)', () => {
    const content = '- [ ] buy milk\n\nParagraph';
    // Position 5 = right after `]`, directly touching the marker
    const { view, cleanup: c } = createTestEditor(content, 5);
    cleanup = c;

    expect(hasClass(view, 'cm-checkbox')).toBe(false);
  });

  it('keeps checkbox rendered when cursor is on trailing space (position 6, not touching)', () => {
    const content = '- [ ] buy milk\n\nParagraph';
    // Position 6 = after the space following `]`, one space away from marker
    const { view, cleanup: c } = createTestEditor(content, 6);
    cleanup = c;

    expect(hasClass(view, 'cm-checkbox')).toBe(true);
  });

  it('applies strikethrough to completed task text', () => {
    const content = '- [x] done task\n\nParagraph';
    const { view, cleanup: c } = createTestEditor(content, 25);
    cleanup = c;

    expect(hasClass(view, 'cm-task-completed')).toBe(true);
  });

  it('no strikethrough on unchecked task text', () => {
    const content = '- [ ] pending task\n\nParagraph';
    const { view, cleanup: c } = createTestEditor(content, 27);
    cleanup = c;

    expect(hasClass(view, 'cm-task-completed')).toBe(false);
  });

  it('checkbox click toggles [ ] to [x]', () => {
    const content = '- [ ] buy milk\n\nParagraph';
    const { view, cleanup: c } = createTestEditor(content, 24);
    cleanup = c;

    const checkbox = view.contentDOM.querySelector('.cm-checkbox') as HTMLInputElement;
    expect(checkbox).not.toBeNull();

    // Click the checkbox
    checkbox.click();

    // Document should now contain [x]
    expect(view.state.doc.toString()).toContain('- [x] buy milk');
  });

  it('checkbox click toggles [x] to [ ]', () => {
    const content = '- [x] buy milk\n\nParagraph';
    const { view, cleanup: c } = createTestEditor(content, 24);
    cleanup = c;

    const checkbox = view.contentDOM.querySelector('.cm-checkbox') as HTMLInputElement;
    expect(checkbox).not.toBeNull();

    // Click the checkbox
    checkbox.click();

    // Document should now contain [ ]
    expect(view.state.doc.toString()).toContain('- [ ] buy milk');
  });

  it('toggle preserves surrounding text', () => {
    const content = '- [ ] buy milk\n- [x] eggs\n\nEnd';
    const { view, cleanup: c } = createTestEditor(content, 30);
    cleanup = c;

    // Toggle the first checkbox
    const checkboxes = view.contentDOM.querySelectorAll('.cm-checkbox') as NodeListOf<HTMLInputElement>;
    expect(checkboxes.length).toBe(2);

    checkboxes[0].click();

    const doc = view.state.doc.toString();
    expect(doc).toContain('- [x] buy milk');
    expect(doc).toContain('- [x] eggs');
    expect(doc).toContain('End');
  });
});
