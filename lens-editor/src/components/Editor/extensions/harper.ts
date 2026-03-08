import { linter, type Diagnostic } from '@codemirror/lint';
import type { EditorView } from '@codemirror/view';
import { LocalLinter, BinaryModule, SuggestionKind } from 'harper.js';
import type { Linter, Lint } from 'harper.js';
import wasmUrl from 'harper.js/dist/harper_wasm_bg.wasm?url';

// Singleton linter instance — eagerly initialized on module load so WASM
// dictionary construction runs in parallel with page load / Y.js sync.
const harperReady: Promise<Linter> = (async () => {
  const binary = BinaryModule.create(wasmUrl);
  const linterInstance = new LocalLinter({ binary });
  await linterInstance.setup();
  return linterInstance;
})();

function getHarper(): Promise<Linter> {
  return harperReady;
}

/**
 * Whitelist-based masking: everything is blanked by default.
 * Only text after a `content::` line (until the next heading) is kept.
 * Preserves character offsets by replacing blanked chars with spaces.
 */
export function maskNonContent(text: string): string {
  const lines = text.split('\n');
  const masked: string[] = [];
  let inContent = false;

  for (const line of lines) {
    const trimmed = line.trimStart();

    // Heading ends a content region
    if (/^#{1,6}\s/.test(trimmed)) {
      inContent = false;
      masked.push(' '.repeat(line.length));
      continue;
    }

    // `content::` starts a content region (the key line itself is blanked)
    if (/^\s*content\s*::/.test(line)) {
      inContent = true;
      masked.push(' '.repeat(line.length));
      continue;
    }

    // Any other key:: line inside a content region ends it
    // Keys are single words (no spaces) followed by ::
    if (inContent && /^\s*\w+\s*::/.test(line)) {
      inContent = false;
      masked.push(' '.repeat(line.length));
      continue;
    }

    if (inContent) {
      masked.push(line);
    } else {
      masked.push(' '.repeat(line.length));
    }
  }

  return masked.join('\n');
}

function suggestionLabel(lint: Lint, suggestion: ReturnType<Lint['suggestions']>[number]): string {
  const kind = suggestion.kind();
  if (kind === SuggestionKind.Remove) return 'Remove';
  if (kind === SuggestionKind.InsertAfter) return `Insert "${suggestion.get_replacement_text()}"`;
  return suggestion.get_replacement_text();
}

async function checkText(view: EditorView): Promise<Diagnostic[]> {
  const text = view.state.doc.toString();
  if (text.trim().length === 0) return [];

  const masked = maskNonContent(text);
  if (masked.trim().length === 0) return [];

  const harper = await getHarper();
  const lints = await harper.lint(masked, { language: 'markdown' });

  const diagnostics: Diagnostic[] = [];

  for (const lint of lints) {
    const span = lint.span();
    const from = span.start;
    const to = span.end;

    if (from < 0 || to > text.length || from >= to) continue;

    const kind = lint.lint_kind();
    const severity = kind === 'Spelling' ? 'error' : 'warning';

    const suggestions = lint.suggestions();
    const actions = suggestions.slice(0, 5).map((s) => {
      const replacementText = s.kind() === SuggestionKind.Remove ? '' : s.get_replacement_text();
      const insertAfter = s.kind() === SuggestionKind.InsertAfter;
      return {
        name: suggestionLabel(lint, s),
        apply: (view: EditorView) => {
          if (insertAfter) {
            view.dispatch({ changes: { from: to, to, insert: replacementText } });
          } else {
            view.dispatch({ changes: { from, to, insert: replacementText } });
          }
        },
      };
    });

    diagnostics.push({
      from,
      to,
      severity,
      message: lint.message(),
      source: 'Harper',
      actions,
    });
  }

  return diagnostics;
}

/**
 * CodeMirror 6 extension that checks document text with Harper (WASM).
 * Non-content regions (frontmatter, headings, key:: fields) are masked
 * so only prose is linted. WASM linter runs in a web worker.
 */
export const harperLinter = linter(checkText, {
  delay: 1000,
});
