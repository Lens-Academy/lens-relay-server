/**
 * Live Preview Extension for CodeMirror 6
 *
 * Implements Obsidian-style inline rendering where markdown syntax hides
 * when cursor moves away and reveals when editing.
 *
 * Key features:
 * - Headings (H1-H6) display with progressively smaller font sizes
 * - # markers hidden when cursor not on heading line
 * - Bold/italic text shows formatted when cursor moves away
 * - Asterisks/underscores hidden when cursor not on that text
 * - Links render as clickable text with external link icon
 * - Inline code shows with distinct background styling
 * - Bullet list markers replaced with dot (•) widget
 * - Checklists rendered as interactive checkboxes with toggle
 * - Completed tasks shown with strikethrough
 */

import {
  ViewPlugin,
  ViewUpdate,
  EditorView,
  Decoration,
  drawSelection,
  WidgetType,
} from '@codemirror/view';
import { criticMarkupCompartment, criticMarkupPlugin, criticMarkupSourcePlugin } from './criticmarkup';
import type { DecorationSet } from '@codemirror/view';
import { syntaxTree } from '@codemirror/language';
import { RangeSetBuilder, Compartment, EditorSelection, StateEffect } from '@codemirror/state';

// CSS classes for heading sizes
const HEADING_CLASSES: Record<string, string> = {
  ATXHeading1: 'cm-heading-1',
  ATXHeading2: 'cm-heading-2',
  ATXHeading3: 'cm-heading-3',
  ATXHeading4: 'cm-heading-4',
  ATXHeading5: 'cm-heading-5',
  ATXHeading6: 'cm-heading-6',
};

// Hidden syntax class
const HIDDEN_CLASS = 'cm-hidden-syntax';

// Emphasis/strong classes
const EMPHASIS_CLASS = 'cm-emphasis';
const STRONG_CLASS = 'cm-strong';

// Inline code class
const INLINE_CODE_CLASS = 'cm-inline-code';

/**
 * WikilinkContext for navigation callbacks
 * Set via livePreview() function parameter
 */
export interface WikilinkContext {
  onClick: (pageName: string) => void;
  onOpenNewTab?: (pageName: string) => void;
  isResolved: (pageName: string) => boolean;
}

// Module-scoped context (set by livePreview factory)
let wikilinkContext: WikilinkContext | null = null;

/**
 * StateEffect dispatched when wikilink metadata changes (e.g., file renames).
 * Triggers decoration rebuild so widget resolution state updates.
 */
export const wikilinkMetadataChanged = StateEffect.define<void>();

/**
 * WikilinkWidget - Renders wikilinks as clickable internal links
 * Uses module-scoped wikilinkContext for navigation and resolution checking
 */
class WikilinkWidget extends WidgetType {
  pageName: string;
  resolved: boolean;

  constructor(pageName: string, resolved: boolean) {
    super();
    this.pageName = pageName;
    this.resolved = resolved;
  }

  toDOM(): HTMLElement {
    const span = document.createElement('span');
    span.className = 'cm-wikilink-widget';

    // Add unresolved class if document doesn't exist
    if (!this.resolved) {
      span.classList.add('unresolved');
    }

    span.textContent = this.pageName;
    span.style.cursor = 'pointer';
    span.onclick = (e) => {
      e.preventDefault();
      if (!wikilinkContext) return;
      if (e.ctrlKey || e.metaKey) {
        wikilinkContext.onOpenNewTab?.(this.pageName);
      } else {
        wikilinkContext.onClick(this.pageName);
      }
    };
    span.onmousedown = (e) => { if (e.button === 1) e.preventDefault(); };
    span.onauxclick = (e) => {
      if (e.button === 1) {
        e.preventDefault();
        wikilinkContext?.onOpenNewTab?.(this.pageName);
      }
    };
    return span;
  }

  eq(other: WikilinkWidget): boolean {
    return this.pageName === other.pageName && this.resolved === other.resolved;
  }
}

/**
 * LinkWidget - Renders links as clickable text with external link icon
 */
class LinkWidget extends WidgetType {
  private text: string;
  private url: string;

  constructor(text: string, url: string) {
    super();
    this.text = text;
    this.url = url;
  }

  toDOM(): HTMLElement {
    const span = document.createElement('span');
    span.className = 'cm-link-widget';
    span.textContent = this.text;

    const icon = document.createElement('span');
    icon.className = 'cm-link-icon';
    span.appendChild(icon);

    span.style.cursor = 'pointer';
    span.onclick = (e) => {
      e.preventDefault();
      // Prepend https:// if URL doesn't have a protocol
      let url = this.url;
      if (!/^https?:\/\//i.test(url)) {
        url = 'https://' + url;
      }
      window.open(url, '_blank');
    };

    return span;
  }

  eq(other: LinkWidget): boolean {
    return this.text === other.text && this.url === other.url;
  }
}

/**
 * BulletWidget - Renders bullet list markers as a dot character
 */
class BulletWidget extends WidgetType {
  toDOM(): HTMLElement {
    const span = document.createElement('span');
    span.className = 'cm-bullet';
    span.textContent = '\u2022';
    return span;
  }

  eq(_other: BulletWidget): boolean {
    return true;
  }
}

/**
 * CheckboxWidget - Renders checklist markers as interactive checkboxes.
 * Clicking toggles [ ] <-> [x] in the document.
 */
class CheckboxWidget extends WidgetType {
  private checked: boolean;
  private view: EditorView;
  private markerFrom: number;
  private markerTo: number;

  constructor(checked: boolean, view: EditorView, markerFrom: number, markerTo: number) {
    super();
    this.checked = checked;
    this.view = view;
    this.markerFrom = markerFrom;
    this.markerTo = markerTo;
  }

  toDOM(): HTMLElement {
    const input = document.createElement('input');
    input.type = 'checkbox';
    input.className = 'cm-checkbox';
    input.checked = this.checked;
    input.onclick = (e) => {
      e.preventDefault();
      const newText = this.checked ? '[ ]' : '[x]';
      this.view.dispatch({
        changes: { from: this.markerFrom, to: this.markerTo, insert: newText },
      });
    };
    return input;
  }

  eq(other: CheckboxWidget): boolean {
    return this.checked === other.checked
      && this.markerFrom === other.markerFrom
      && this.markerTo === other.markerTo;
  }
}

/**
 * Check if any selection range intersects with the given range
 */
function selectionIntersects(
  selection: EditorSelection,
  from: number,
  to: number
): boolean {
  return selection.ranges.some((range) => range.to >= from && range.from <= to);
}

/**
 * Check if cursor is on the same line as the given position
 */
function cursorOnLine(view: EditorView, pos: number): boolean {
  const cursorLine = view.state.doc.lineAt(view.state.selection.main.head).number;
  const targetLine = view.state.doc.lineAt(pos).number;
  return cursorLine === targetLine;
}

/**
 * ViewPlugin that builds decorations based on cursor position
 */
const livePreviewPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;

    constructor(view: EditorView) {
      this.decorations = this.buildDecorations(view);
    }

    update(update: ViewUpdate) {
      // Rebuild on doc change, viewport change, selection change, OR metadata change
      if (
        update.docChanged ||
        update.viewportChanged ||
        update.selectionSet ||
        update.transactions.some(tr => tr.effects.some(e => e.is(wikilinkMetadataChanged)))
      ) {
        this.decorations = this.buildDecorations(update.view);
      }
    }

    buildDecorations(view: EditorView): DecorationSet {
      const builder = new RangeSetBuilder<Decoration>();
      const selection = view.state.selection;

      // Track decorations to sort them (required for RangeSetBuilder)
      const decorations: Array<{ from: number; to: number; deco: Decoration }> =
        [];

      // Iterate syntax tree within visible ranges only (performance)
      for (const { from, to } of view.visibleRanges) {
        syntaxTree(view.state).iterate({
          from,
          to,
          enter(node) {
            // Headings: ALWAYS apply heading class for font sizing
            // # markers are hidden separately based on cursor position (HeaderMark below)
            if (node.name in HEADING_CLASSES) {
              decorations.push({
                from: node.from,
                to: node.to,
                deco: Decoration.mark({ class: HEADING_CLASSES[node.name] }),
              });
            }

            // HeaderMark (# characters): hide when cursor not on line
            if (node.name === 'HeaderMark') {
              if (!cursorOnLine(view, node.from)) {
                const line = view.state.doc.lineAt(node.from);
                // Hide # and trailing space
                const end = Math.min(node.to + 1, line.to);
                decorations.push({
                  from: node.from,
                  to: end,
                  deco: Decoration.mark({ class: HIDDEN_CLASS }),
                });
              }
            }

            // Emphasis (italic): element-based reveal
            if (node.name === 'Emphasis') {
              if (!selectionIntersects(selection, node.from, node.to)) {
                decorations.push({
                  from: node.from,
                  to: node.to,
                  deco: Decoration.mark({ class: EMPHASIS_CLASS }),
                });
              }
            }

            // StrongEmphasis (bold): element-based reveal
            if (node.name === 'StrongEmphasis') {
              if (!selectionIntersects(selection, node.from, node.to)) {
                decorations.push({
                  from: node.from,
                  to: node.to,
                  deco: Decoration.mark({ class: STRONG_CLASS }),
                });
              }
            }

            // EmphasisMark (* or _ characters): hide when cursor not on element
            if (node.name === 'EmphasisMark') {
              // Get the parent node to check if cursor intersects the whole emphasis element
              const parent = node.node.parent;
              if (parent) {
                if (!selectionIntersects(selection, parent.from, parent.to)) {
                  decorations.push({
                    from: node.from,
                    to: node.to,
                    deco: Decoration.mark({ class: HIDDEN_CLASS }),
                  });
                }
              }
            }

            // Link: replace with clickable widget when cursor not on link
            // Link node contains: LinkMark `[`, link text, LinkMark `]`, LinkMark `(`, URL, LinkMark `)`
            if (node.name === 'Link') {
              if (!selectionIntersects(selection, node.from, node.to)) {
                // Extract link text and URL from the Link node's content
                const content = view.state.doc.sliceString(node.from, node.to);
                const textMatch = content.match(/^\[([^\]]*)\]/);
                const urlMatch = content.match(/\]\(([^)]*)\)$/);

                if (textMatch && urlMatch) {
                  const linkText = textMatch[1];
                  const linkUrl = urlMatch[1];

                  // Replace entire link with widget
                  decorations.push({
                    from: node.from,
                    to: node.to,
                    deco: Decoration.replace({
                      widget: new LinkWidget(linkText, linkUrl),
                    }),
                  });
                }
              }
            }

            // Wikilink: replace with clickable widget when cursor not on link
            if (node.name === 'Wikilink') {
              if (!selectionIntersects(selection, node.from, node.to)) {
                // Extract page name from WikilinkContent child (works for both [[page]] and ![[page]])
                const contentNode = node.node.getChild('WikilinkContent');
                if (!contentNode) return;
                const raw = view.state.doc.sliceString(contentNode.from, contentNode.to);
                const pipeIndex = raw.indexOf('|');
                const content = pipeIndex !== -1 ? raw.substring(0, pipeIndex) : raw;

                const resolved = wikilinkContext ? wikilinkContext.isResolved(content) : true;
                decorations.push({
                  from: node.from,
                  to: node.to,
                  deco: Decoration.replace({
                    widget: new WikilinkWidget(content, resolved),
                  }),
                });
                // Skip children (WikilinkMark) - replaced by widget
                return false;
              }
            }

            // FencedCode: line decorations for background + hide fences when cursor outside
            if (node.name === 'FencedCode') {
              const cursorInside = selectionIntersects(selection, node.from, node.to);

              // Add cm-code-block line class to every line in the fenced code range
              const startLine = view.state.doc.lineAt(node.from).number;
              const endLine = view.state.doc.lineAt(node.to).number;
              for (let ln = startLine; ln <= endLine; ln++) {
                const line = view.state.doc.line(ln);
                decorations.push({
                  from: line.from,
                  to: line.from,
                  deco: Decoration.line({ class: 'cm-code-block' }),
                });
              }

              // Hide fence markers and language info when cursor is outside
              if (!cursorInside) {
                // Hide opening fence line content (``` + optional language)
                const openLine = view.state.doc.lineAt(node.from);
                if (openLine.from < openLine.to) {
                  decorations.push({
                    from: openLine.from,
                    to: openLine.to,
                    deco: Decoration.mark({ class: HIDDEN_CLASS }),
                  });
                }

                // Hide closing fence line content (```)
                const closeLine = view.state.doc.lineAt(node.to);
                if (closeLine.from < closeLine.to && closeLine.number !== openLine.number) {
                  decorations.push({
                    from: closeLine.from,
                    to: closeLine.to,
                    deco: Decoration.mark({ class: HIDDEN_CLASS }),
                  });
                }
              }

              // Skip child iteration (CodeMark/CodeInfo/CodeText handled above)
              return false;
            }

            // InlineCode: always style, hide backticks only when cursor outside
            if (node.name === 'InlineCode') {
              decorations.push({
                from: node.from,
                to: node.to,
                deco: Decoration.mark({ class: INLINE_CODE_CLASS }),
              });
            }

            // CodeMark (backtick characters): hide when cursor not on inline code
            if (node.name === 'CodeMark') {
              // Get the parent node (InlineCode) to check if cursor intersects
              const parent = node.node.parent;
              if (parent) {
                if (!selectionIntersects(selection, parent.from, parent.to)) {
                  decorations.push({
                    from: node.from,
                    to: node.to,
                    deco: Decoration.mark({ class: HIDDEN_CLASS }),
                  });
                }
              }
            }

            // ListMark in bullet lists: replace with dot widget when cursor not touching
            if (node.name === 'ListMark') {
              // Only handle bullet lists, not ordered lists
              const parent = node.node.parent; // ListItem
              const grandparent = parent?.parent; // BulletList or OrderedList
              if (grandparent && grandparent.name === 'BulletList') {
                // Skip if this is a task list item (has Task child — handled by checklist code)
                const listItem = parent;
                let isTask = false;
                if (listItem) {
                  for (let child = listItem.firstChild; child; child = child.nextSibling) {
                    if (child.name === 'Task') { isTask = true; break; }
                  }
                }
                if (!isTask && !selectionIntersects(selection, node.from, node.to)) {
                  decorations.push({
                    from: node.from,
                    to: node.to,
                    deco: Decoration.replace({
                      widget: new BulletWidget(),
                    }),
                  });
                }
              }
            }

            // TaskMarker: replace list marker + task marker with checkbox widget
            if (node.name === 'TaskMarker') {
              // Find the ListMark sibling (the `- ` part)
              const task = node.node.parent; // Task node
              const listItem = task?.parent; // ListItem node
              let listMark: { from: number; to: number } | null = null;
              if (listItem) {
                for (let child = listItem.firstChild; child; child = child.nextSibling) {
                  if (child.name === 'ListMark') {
                    listMark = { from: child.from, to: child.to };
                    break;
                  }
                }
              }

              const replaceFrom = listMark ? listMark.from : node.from;
              // Include trailing space after ] in the replacement range
              const replaceTo = Math.min(node.to + 1, view.state.doc.lineAt(node.from).to);

              // Cursor proximity: reveal raw when cursor touches the marker chars.
              // node.to is the position right after ], which counts as "touching".
              // The trailing space (node.to + 1) does NOT trigger reveal.
              if (!selectionIntersects(selection, replaceFrom, node.to)) {
                const markerText = view.state.doc.sliceString(node.from, node.to);
                const isChecked = markerText !== '[ ]';

                decorations.push({
                  from: replaceFrom,
                  to: replaceTo,
                  deco: Decoration.replace({
                    widget: new CheckboxWidget(isChecked, view, node.from, node.to),
                  }),
                });

                // Strikethrough for completed tasks
                if (isChecked) {
                  const lineEnd = view.state.doc.lineAt(node.from).to;
                  if (replaceTo < lineEnd) {
                    decorations.push({
                      from: replaceTo,
                      to: lineEnd,
                      deco: Decoration.mark({ class: 'cm-task-completed' }),
                    });
                  }
                }
              }
            }
          },
        });
      }

      // Sort decorations by position (required for RangeSetBuilder)
      decorations.sort((a, b) => a.from - b.from || a.to - b.to);

      // Add to builder in sorted order
      for (const d of decorations) {
        builder.add(d.from, d.to, d.deco);
      }

      return builder.finish();
    }
  },
  {
    decorations: (v) => v.decorations,
  }
);

/**
 * Compartment for toggling live preview on/off (source mode toggle)
 */
export const livePreviewCompartment = new Compartment();

/**
 * Theme for live preview (empty since styles are in index.css,
 * but kept as a placeholder for consistency with the compartment pattern)
 */
const livePreviewTheme = EditorView.theme({});

/**
 * Live preview extension with all necessary components
 *
 * Includes:
 * - drawSelection() for proper cursor rendering with hidden content
 * - ViewPlugin for selection-aware decorations
 *
 * @param context - Optional WikilinkContext for navigation callbacks
 */
export function livePreview(context?: WikilinkContext) {
  if (context) {
    wikilinkContext = context;
  }
  return [
    drawSelection(), // Required for proper cursor with hidden content
    livePreviewCompartment.of([livePreviewPlugin, livePreviewTheme]),
  ];
}

/**
 * Update the wikilink context without recreating the extension.
 * Call this when metadata changes to update navigation and resolution.
 */
export function updateWikilinkContext(context: WikilinkContext | undefined) {
  wikilinkContext = context ?? null;
}

/**
 * Source-mode heading plugin — applies heading size classes
 * but keeps # markers visible (no hidden-syntax decorations).
 */
const sourceHeadingPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;

    constructor(view: EditorView) {
      this.decorations = this.buildDecorations(view);
    }

    update(update: ViewUpdate) {
      if (update.docChanged || update.viewportChanged) {
        this.decorations = this.buildDecorations(update.view);
      }
    }

    buildDecorations(view: EditorView): DecorationSet {
      const builder = new RangeSetBuilder<Decoration>();
      const decorations: Array<{ from: number; to: number; deco: Decoration }> = [];

      for (const { from, to } of view.visibleRanges) {
        syntaxTree(view.state).iterate({
          from,
          to,
          enter(node) {
            if (node.name in HEADING_CLASSES) {
              decorations.push({
                from: node.from,
                to: node.to,
                deco: Decoration.mark({ class: HEADING_CLASSES[node.name] }),
              });
            }
          },
        });
      }

      decorations.sort((a, b) => a.from - b.from || a.to - b.to);
      for (const d of decorations) {
        builder.add(d.from, d.to, d.deco);
      }
      return builder.finish();
    }
  },
  {
    decorations: (v) => v.decorations,
  }
);

/**
 * Toggle between live preview mode and source mode
 * @param view - The EditorView instance
 * @param sourceMode - true to show source (raw markdown), false for live preview
 */
export function toggleSourceMode(view: EditorView, sourceMode: boolean) {
  view.dispatch({
    effects: [
      livePreviewCompartment.reconfigure(
        sourceMode ? [sourceHeadingPlugin, livePreviewTheme] : [livePreviewPlugin, livePreviewTheme]
      ),
      criticMarkupCompartment.reconfigure(
        sourceMode ? criticMarkupSourcePlugin : criticMarkupPlugin
      ),
    ],
  });
}
