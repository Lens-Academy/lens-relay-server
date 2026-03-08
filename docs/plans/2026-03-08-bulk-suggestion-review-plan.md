# Bulk Suggestion Review — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a `/review` page to lens-editor that surfaces all CriticMarkup suggestions across a folder via a new server endpoint, with accept/reject controls.

**Architecture:** Server-side on-demand scan (no persistent index). New Rust endpoint scans folder docs for CriticMarkup regex, returns JSON including the raw markup string for each suggestion. Frontend renders grouped suggestions and uses Y.Doc WebSocket connections for accept/reject actions.

**Tech Stack:** Rust (Axum, yrs), TypeScript (React, React Router, yjs, CodeMirror state reuse)

---

## Task 1: Rust CriticMarkup Scanner (Pure Function)

**Files:**
- Create: `crates/y-sweet-core/src/critic_scanner.rs`
- Modify: `crates/y-sweet-core/src/lib.rs` (add `pub mod critic_scanner;`)

This is a pure function with no server dependencies — takes a string, returns parsed suggestions.

**Step 1: Write the failing test**

In `crates/y-sweet-core/src/critic_scanner.rs`:

```rust
use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionType {
    Addition,
    Deletion,
    Substitution,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    #[serde(rename = "type")]
    pub suggestion_type: SuggestionType,
    pub content: String,
    pub old_content: Option<String>,
    pub new_content: Option<String>,
    pub author: Option<String>,
    pub timestamp: Option<u64>,
    pub from: usize,
    pub to: usize,
    /// The raw CriticMarkup string as it appears in the document (e.g. `{++meta@@text++}`).
    /// Used by the frontend to locate and replace the suggestion without reconstructing it.
    pub raw_markup: String,
    pub context_before: String,
    pub context_after: String,
}

pub fn scan_suggestions(text: &str) -> Vec<Suggestion> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_addition() {
        let text = r#"Hello {++{"author":"AI","timestamp":1709900000000}@@world++} end"#;
        let results = scan_suggestions(text);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].suggestion_type, SuggestionType::Addition);
        assert_eq!(results[0].content, "world");
        assert_eq!(results[0].author.as_deref(), Some("AI"));
        assert_eq!(results[0].timestamp, Some(1709900000000));
        assert_eq!(results[0].context_before, "Hello ");
        assert_eq!(results[0].context_after, " end");
        assert_eq!(
            results[0].raw_markup,
            r#"{++{"author":"AI","timestamp":1709900000000}@@world++}"#
        );
    }

    #[test]
    fn test_scan_deletion() {
        let text = r#"Keep {--{"author":"AI","timestamp":1000}@@removed--} this"#;
        let results = scan_suggestions(text);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].suggestion_type, SuggestionType::Deletion);
        assert_eq!(results[0].content, "removed");
        assert_eq!(
            results[0].raw_markup,
            r#"{--{"author":"AI","timestamp":1000}@@removed--}"#
        );
    }

    #[test]
    fn test_scan_substitution() {
        let text = r#"Say {~~{"author":"AI","timestamp":2000}@@hello~>goodbye~~} now"#;
        let results = scan_suggestions(text);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].suggestion_type, SuggestionType::Substitution);
        assert_eq!(results[0].old_content.as_deref(), Some("hello"));
        assert_eq!(results[0].new_content.as_deref(), Some("goodbye"));
        assert_eq!(
            results[0].raw_markup,
            r#"{~~{"author":"AI","timestamp":2000}@@hello~>goodbye~~}"#
        );
    }

    #[test]
    fn test_scan_no_metadata() {
        let text = "Hello {++plain addition++} end";
        let results = scan_suggestions(text);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "plain addition");
        assert!(results[0].author.is_none());
        assert!(results[0].timestamp.is_none());
        assert_eq!(results[0].raw_markup, "{++plain addition++}");
    }

    #[test]
    fn test_scan_multiple() {
        let text = r#"{++{"author":"AI","timestamp":1000}@@added++} middle {--{"author":"Bob","timestamp":2000}@@deleted--}"#;
        let results = scan_suggestions(text);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].suggestion_type, SuggestionType::Addition);
        assert_eq!(results[1].suggestion_type, SuggestionType::Deletion);
    }

    #[test]
    fn test_scan_empty() {
        let results = scan_suggestions("No suggestions here");
        assert!(results.is_empty());
    }

    #[test]
    fn test_context_truncation() {
        // Context should be truncated to ~50 chars
        let long_before = "a".repeat(100);
        let text = format!("{} {{++{{\"author\":\"AI\",\"timestamp\":1000}}@@added++}} after", long_before);
        let results = scan_suggestions(&text);
        assert_eq!(results.len(), 1);
        assert!(results[0].context_before.len() <= 60); // some leeway for word boundary
    }
}
```

**Step 2: Run test to verify it fails**

Run: `CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo test --manifest-path=crates/Cargo.toml -p y-sweet-core critic_scanner -- --nocapture`

Expected: FAIL — `todo!()` panics or compile errors

**Step 3: Implement `scan_suggestions`**

Replace the `todo!()` with the implementation. Pattern-match the JS parser in `lens-editor/src/lib/criticmarkup-parser.ts`:

```rust
static ADDITION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)\{\+\+(.*?)\+\+\}").unwrap()
});
static DELETION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)\{--(.*?)--\}").unwrap()
});
static SUBSTITUTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)\{~~(.*?)~>(.*?)~~\}").unwrap()
});

const CONTEXT_CHARS: usize = 50;

fn extract_context(text: &str, from: usize, to: usize) -> (String, String) {
    let before_start = from.saturating_sub(CONTEXT_CHARS);
    let after_end = (to + CONTEXT_CHARS).min(text.len());
    let context_before = &text[before_start..from];
    let context_after = &text[to..after_end];
    (context_before.to_string(), context_after.to_string())
}

fn extract_metadata(raw: &str) -> (Option<String>, Option<u64>, &str) {
    if let Some(sep_pos) = raw.find("@@") {
        let meta_str = &raw[..sep_pos];
        if meta_str.starts_with('{') && meta_str.ends_with('}') {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(meta_str) {
                let author = json.get("author").and_then(|v| v.as_str()).map(|s| s.to_string());
                let timestamp = json.get("timestamp").and_then(|v| v.as_u64());
                let content = &raw[sep_pos + 2..];
                return (author, timestamp, content);
            }
        }
    }
    (None, None, raw)
}

pub fn scan_suggestions(text: &str) -> Vec<Suggestion> {
    let mut suggestions = Vec::new();

    for m in ADDITION_RE.find_iter(text) {
        let raw_markup = m.as_str().to_string();
        let raw = &text[m.start() + 3..m.end() - 3]; // strip {++ and ++}
        let (author, timestamp, content) = extract_metadata(raw);
        let (ctx_before, ctx_after) = extract_context(text, m.start(), m.end());
        suggestions.push(Suggestion {
            suggestion_type: SuggestionType::Addition,
            content: content.to_string(),
            old_content: None,
            new_content: None,
            author,
            timestamp,
            from: m.start(),
            to: m.end(),
            raw_markup,
            context_before: ctx_before,
            context_after: ctx_after,
        });
    }

    for m in DELETION_RE.find_iter(text) {
        let raw_markup = m.as_str().to_string();
        let raw = &text[m.start() + 3..m.end() - 3]; // strip {-- and --}
        let (author, timestamp, content) = extract_metadata(raw);
        let (ctx_before, ctx_after) = extract_context(text, m.start(), m.end());
        suggestions.push(Suggestion {
            suggestion_type: SuggestionType::Deletion,
            content: content.to_string(),
            old_content: None,
            new_content: None,
            author,
            timestamp,
            from: m.start(),
            to: m.end(),
            raw_markup,
            context_before: ctx_before,
            context_after: ctx_after,
        });
    }

    for caps in SUBSTITUTION_RE.captures_iter(text) {
        let m = caps.get(0).unwrap();
        let raw_markup = m.as_str().to_string();
        let raw_old = caps.get(1).unwrap().as_str();
        let new_content = caps.get(2).unwrap().as_str();
        let (author, timestamp, old_content) = extract_metadata(raw_old);
        let (ctx_before, ctx_after) = extract_context(text, m.start(), m.end());
        suggestions.push(Suggestion {
            suggestion_type: SuggestionType::Substitution,
            content: format!("{}→{}", old_content, new_content),
            old_content: Some(old_content.to_string()),
            new_content: Some(new_content.to_string()),
            author,
            timestamp,
            from: m.start(),
            to: m.end(),
            raw_markup,
            context_before: ctx_before,
            context_after: ctx_after,
        });
    }

    suggestions.sort_by_key(|s| s.from);
    suggestions
}
```

**Step 4: Register the module**

In `crates/y-sweet-core/src/lib.rs`, add:
```rust
pub mod critic_scanner;
```

**Step 5: Run tests to verify they pass**

Run: `CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo test --manifest-path=crates/Cargo.toml -p y-sweet-core critic_scanner -- --nocapture`

Expected: All 7 tests PASS

**Step 6: Commit**

```bash
jj new -m "feat: add CriticMarkup suggestion scanner (pure function)"
# (code is auto-committed by jj)
```

---

## Task 2: Rust `/suggestions` Endpoint

**Files:**
- Modify: `crates/relay/src/server.rs` (add route + handler)
- Create: `crates/relay/tests/suggestions_endpoint_test.rs`

Follows the pattern of `handle_search` — reads folder docs, iterates content docs, scans each.

**Step 1: Write the failing integration test**

Create `crates/relay/tests/suggestions_endpoint_test.rs`. Follow the pattern from `crates/relay/src/mcp/tools/grep.rs` tests which set up Y.Docs with folder metadata and content, then call functions that scan them. Since the endpoint handler depends on the full server, extract the core logic into a testable function.

```rust
// crates/relay/tests/suggestions_endpoint_test.rs
// This test validates the scan logic via the public scan_suggestions function
// and the endpoint response shape. Full HTTP integration test requires a running
// server, so we test the building blocks in isolation.

use y_sweet_core::critic_scanner::{scan_suggestions, SuggestionType};

#[test]
fn test_endpoint_response_shape() {
    // Verify the scanner returns data that serializes to the expected JSON shape
    let text = r#"Hello {++{"author":"AI","timestamp":1000}@@world++} end"#;
    let suggestions = scan_suggestions(text);
    let json = serde_json::json!({
        "files": [{
            "path": "Notes/Test.md",
            "doc_id": "relay-id-test-uuid",
            "suggestions": suggestions,
        }]
    });
    let response: serde_json::Value = json;
    let files = response["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["path"], "Notes/Test.md");
    let sug = &files[0]["suggestions"][0];
    assert_eq!(sug["type"], "addition");
    assert_eq!(sug["content"], "world");
    assert_eq!(sug["author"], "AI");
    assert_eq!(sug["timestamp"], 1000);
    assert!(sug["from"].is_number());
    assert!(sug["to"].is_number());
    assert!(sug["raw_markup"].is_string());
    assert!(sug["context_before"].is_string());
    assert!(sug["context_after"].is_string());
}

#[test]
fn test_empty_folder_returns_no_files() {
    let suggestions = scan_suggestions("No CriticMarkup here.");
    assert!(suggestions.is_empty());
}

#[test]
fn test_null_fields_serialized() {
    // Verify optional fields serialize as null (not omitted)
    let text = "Hello {++plain text++} end";
    let suggestions = scan_suggestions(text);
    let json = serde_json::to_value(&suggestions[0]).unwrap();
    assert!(json.get("author").is_some(), "author field should be present");
    assert!(json.get("timestamp").is_some(), "timestamp field should be present");
    assert!(json["author"].is_null());
    assert!(json["timestamp"].is_null());
}
```

**Step 2: Run test to verify it fails**

Run: `CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo test --manifest-path=crates/Cargo.toml -p relay suggestions_endpoint -- --nocapture`

Expected: FAIL — test file references the serialization shape; the `test_null_fields_serialized` test will fail because the current `Suggestion` struct uses `skip_serializing_if = "Option::is_none"` which omits null fields.

**Step 3: Fix serialization — remove `skip_serializing_if`**

In `crates/y-sweet-core/src/critic_scanner.rs`, remove all `#[serde(skip_serializing_if = "Option::is_none")]` annotations from the `Suggestion` struct so optional fields serialize as `null` (matching the design doc's API contract).

**Step 4: Write the endpoint handler**

Add to `server.rs` imports:
```rust
use y_sweet_core::critic_scanner;
```

Add route in the Router chain:
```rust
.route("/suggestions", get(handle_suggestions))
```

Add query params struct:
```rust
#[derive(serde::Deserialize)]
struct SuggestionsQuery {
    folder_id: String,
}
```

Add handler:
```rust
async fn handle_suggestions(
    auth_header: Option<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>,
    State(server_state): State<Arc<Server>>,
    Query(params): Query<SuggestionsQuery>,
) -> Result<Json<Value>, AppError> {
    server_state.check_auth(auth_header)?;

    let folder_id = &params.folder_id;

    // Load the folder doc and get content UUIDs from filemeta_v0
    server_state.ensure_doc_loaded(folder_id).await
        .map_err(|e| AppError(StatusCode::NOT_FOUND, format!("Folder not found: {}", e)))?;

    let content_uuids = link_indexer::is_folder_doc(folder_id, &server_state.docs)
        .ok_or_else(|| AppError(StatusCode::NOT_FOUND, "Not a folder document".into()))?;

    // Get path mapping from filemeta_v0
    let path_map = {
        let doc_ref = server_state.docs.get(folder_id)
            .ok_or_else(|| AppError(StatusCode::NOT_FOUND, "Folder doc not loaded".into()))?;
        let awareness = doc_ref.awareness();
        let guard = awareness.read().unwrap_or_else(|e| e.into_inner());
        let txn = guard.doc.transact();
        let filemeta = txn.get_map("filemeta_v0")
            .ok_or_else(|| AppError(StatusCode::NOT_FOUND, "No filemeta_v0".into()))?;
        let mut map = std::collections::HashMap::new();
        for (path, value) in filemeta.iter(&txn) {
            if let Some(id) = link_indexer::extract_id_from_filemeta_entry(&value, &txn) {
                map.insert(id, path.to_string());
            }
        }
        map
    };

    // relay_id = first 36 chars of compound folder_id
    let relay_id = &folder_id[..36];

    let mut files = Vec::new();

    for content_uuid in &content_uuids {
        let doc_id = format!("{}-{}", relay_id, content_uuid);
        let path = path_map.get(content_uuid).cloned().unwrap_or_else(|| content_uuid.clone());

        // Load doc content
        if server_state.ensure_doc_loaded(&doc_id).await.is_err() {
            continue;
        }
        let content = {
            let Some(doc_ref) = server_state.docs.get(&doc_id) else { continue };
            let awareness = doc_ref.awareness();
            let guard = awareness.read().unwrap_or_else(|e| e.into_inner());
            let txn = guard.doc.transact();
            match txn.get_text("contents") {
                Some(text) => text.get_string(&txn),
                None => continue,
            }
        };

        let suggestions = critic_scanner::scan_suggestions(&content);
        if suggestions.is_empty() {
            continue;
        }

        files.push(serde_json::json!({
            "path": path,
            "doc_id": doc_id,
            "suggestions": suggestions,
        }));
    }

    Ok(Json(serde_json::json!({ "files": files })))
}
```

**Step 5: Run tests to verify they pass**

Run: `CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo test --manifest-path=crates/Cargo.toml -p relay suggestions_endpoint -- --nocapture`

Expected: All 3 tests PASS

**Step 6: Verify it compiles**

Run: `CARGO_TARGET_DIR=~/code/lens-relay/.cargo-target cargo check --manifest-path=crates/Cargo.toml`

Expected: Compiles without errors

**Step 7: Commit**

```bash
jj new -m "feat: add /suggestions endpoint for folder-wide CriticMarkup scan"
```

---

## Task 3: Frontend API Client Hook

**Files:**
- Create: `lens-editor/src/hooks/useSuggestions.test.ts`
- Create: `lens-editor/src/hooks/useSuggestions.ts`

**Step 1: Write the failing test**

```typescript
// lens-editor/src/hooks/useSuggestions.test.ts
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';
import { useSuggestions } from './useSuggestions';

// Mock authFetch at the module boundary
vi.mock('../lib/auth', () => ({
  authFetch: vi.fn(),
}));

import { authFetch } from '../lib/auth';
const mockAuthFetch = vi.mocked(authFetch);

describe('useSuggestions', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('starts in loading state', () => {
    mockAuthFetch.mockReturnValue(new Promise(() => {})); // never resolves
    const { result } = renderHook(() => useSuggestions(['folder-1']));
    expect(result.current.loading).toBe(true);
    expect(result.current.data).toEqual([]);
    expect(result.current.error).toBeNull();
  });

  it('fetches suggestions for a single folder', async () => {
    mockAuthFetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        files: [{ path: 'Notes/Test.md', doc_id: 'doc-1', suggestions: [{ type: 'addition', content: 'hello' }] }],
      }),
    } as Response);

    const { result } = renderHook(() => useSuggestions(['folder-1']));
    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.data).toHaveLength(1);
    expect(result.current.data[0].path).toBe('Notes/Test.md');
    expect(mockAuthFetch).toHaveBeenCalledWith('/suggestions?folder_id=folder-1');
  });

  it('aggregates suggestions across multiple folders', async () => {
    mockAuthFetch
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({
          files: [{ path: 'A.md', doc_id: 'doc-a', suggestions: [] }],
        }),
      } as Response)
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({
          files: [{ path: 'B.md', doc_id: 'doc-b', suggestions: [] }],
        }),
      } as Response);

    const { result } = renderHook(() => useSuggestions(['folder-1', 'folder-2']));
    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.data).toHaveLength(2);
    expect(mockAuthFetch).toHaveBeenCalledTimes(2);
  });

  it('sets error when fetch fails', async () => {
    mockAuthFetch.mockResolvedValueOnce({
      ok: false,
      json: async () => ({}),
    } as Response);

    const { result } = renderHook(() => useSuggestions(['folder-1']));
    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.error).toBeTruthy();
    expect(result.current.data).toEqual([]);
  });

  it('refresh re-fetches data', async () => {
    mockAuthFetch.mockResolvedValue({
      ok: true,
      json: async () => ({ files: [] }),
    } as Response);

    const { result } = renderHook(() => useSuggestions(['folder-1']));
    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(mockAuthFetch).toHaveBeenCalledTimes(1);
    await result.current.refresh();
    expect(mockAuthFetch).toHaveBeenCalledTimes(2);
  });
});
```

**Step 2: Run test to verify it fails**

Run: `cd lens-editor && npx vitest run src/hooks/useSuggestions.test.ts`

Expected: FAIL — `useSuggestions` module doesn't exist

**Step 3: Implement the hook**

```typescript
// lens-editor/src/hooks/useSuggestions.ts
import { useState, useEffect, useCallback } from 'react';
import { authFetch } from '../lib/auth';

export interface SuggestionItem {
  type: 'addition' | 'deletion' | 'substitution';
  content: string;
  old_content: string | null;
  new_content: string | null;
  author: string | null;
  timestamp: number | null;
  from: number;
  to: number;
  raw_markup: string;
  context_before: string;
  context_after: string;
}

export interface FileSuggestions {
  path: string;
  doc_id: string;
  suggestions: SuggestionItem[];
}

export interface SuggestionsResponse {
  files: FileSuggestions[];
}

export function useSuggestions(folderIds: string[]) {
  const [data, setData] = useState<FileSuggestions[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const allFiles: FileSuggestions[] = [];
      for (const folderId of folderIds) {
        const res = await authFetch(`/suggestions?folder_id=${encodeURIComponent(folderId)}`);
        if (!res.ok) throw new Error(`Failed to fetch suggestions for ${folderId}`);
        const json: SuggestionsResponse = await res.json();
        allFiles.push(...json.files);
      }
      setData(allFiles);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Unknown error');
    } finally {
      setLoading(false);
    }
  }, [folderIds]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return { data, loading, error, refresh };
}
```

**Step 4: Run tests to verify they pass**

Run: `cd lens-editor && npx vitest run src/hooks/useSuggestions.test.ts`

Expected: All 5 tests PASS

**Step 5: Commit**

```bash
jj new -m "feat: add useSuggestions hook for fetching folder suggestions"
```

---

## Task 4: Review Page Component

**Files:**
- Create: `lens-editor/src/components/ReviewPage/ReviewPage.test.tsx`
- Create: `lens-editor/src/components/ReviewPage/ReviewPage.tsx`
- Modify: `lens-editor/src/App.tsx` (add route)

**Step 1: Write the failing tests**

```typescript
// lens-editor/src/components/ReviewPage/ReviewPage.test.tsx
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { ReviewPage } from './ReviewPage';

const mockRefresh = vi.fn();

// Default mock: loaded data with one file
function mockLoaded() {
  vi.doMock('../../hooks/useSuggestions', () => ({
    useSuggestions: () => ({
      data: [
        {
          path: 'Notes/Test.md',
          doc_id: 'relay-id-doc-uuid',
          suggestions: [
            {
              type: 'addition' as const,
              content: 'new text',
              old_content: null,
              new_content: null,
              author: 'AI',
              timestamp: 1709900000000,
              from: 10,
              to: 50,
              raw_markup: '{++{"author":"AI","timestamp":1709900000000}@@new text++}',
              context_before: 'before ',
              context_after: ' after',
            },
          ],
        },
      ],
      loading: false,
      error: null,
      refresh: mockRefresh,
    }),
  }));
}

function mockLoading() {
  vi.doMock('../../hooks/useSuggestions', () => ({
    useSuggestions: () => ({
      data: [],
      loading: true,
      error: null,
      refresh: mockRefresh,
    }),
  }));
}

function mockError() {
  vi.doMock('../../hooks/useSuggestions', () => ({
    useSuggestions: () => ({
      data: [],
      loading: false,
      error: 'Network error',
      refresh: mockRefresh,
    }),
  }));
}

function mockEmpty() {
  vi.doMock('../../hooks/useSuggestions', () => ({
    useSuggestions: () => ({
      data: [],
      loading: false,
      error: null,
      refresh: mockRefresh,
    }),
  }));
}

describe('ReviewPage', () => {
  beforeEach(() => {
    vi.resetModules();
    mockRefresh.mockClear();
  });

  describe('with loaded data', () => {
    beforeEach(() => mockLoaded());

    it('renders file with suggestion count', async () => {
      const { ReviewPage } = await import('./ReviewPage');
      render(<MemoryRouter><ReviewPage folderIds={['test-folder']} /></MemoryRouter>);
      expect(screen.getByText('Notes/Test.md')).toBeTruthy();
      expect(screen.getByText(/1 suggestion/)).toBeTruthy();
    });

    it('shows suggestion content when file is expanded', async () => {
      const { ReviewPage } = await import('./ReviewPage');
      render(<MemoryRouter><ReviewPage folderIds={['test-folder']} /></MemoryRouter>);
      // Click to expand
      fireEvent.click(screen.getByText('Notes/Test.md'));
      expect(screen.getByText('new text')).toBeTruthy();
    });

    it('shows author badge', async () => {
      const { ReviewPage } = await import('./ReviewPage');
      render(<MemoryRouter><ReviewPage folderIds={['test-folder']} /></MemoryRouter>);
      fireEvent.click(screen.getByText('Notes/Test.md'));
      expect(screen.getByText('AI')).toBeTruthy();
    });

    it('toggles file expansion on click', async () => {
      const { ReviewPage } = await import('./ReviewPage');
      render(<MemoryRouter><ReviewPage folderIds={['test-folder']} /></MemoryRouter>);
      const fileHeader = screen.getByText('Notes/Test.md');
      // Initially collapsed — no suggestion content visible
      expect(screen.queryByText('new text')).toBeNull();
      // Expand
      fireEvent.click(fileHeader);
      expect(screen.getByText('new text')).toBeTruthy();
      // Collapse
      fireEvent.click(fileHeader);
      expect(screen.queryByText('new text')).toBeNull();
    });
  });

  describe('loading state', () => {
    beforeEach(() => mockLoading());

    it('shows loading message', async () => {
      const { ReviewPage } = await import('./ReviewPage');
      render(<MemoryRouter><ReviewPage folderIds={['test-folder']} /></MemoryRouter>);
      expect(screen.getByText(/Scanning documents/)).toBeTruthy();
    });
  });

  describe('error state', () => {
    beforeEach(() => mockError());

    it('shows error message', async () => {
      const { ReviewPage } = await import('./ReviewPage');
      render(<MemoryRouter><ReviewPage folderIds={['test-folder']} /></MemoryRouter>);
      expect(screen.getByText(/Network error/)).toBeTruthy();
    });
  });

  describe('empty state', () => {
    beforeEach(() => mockEmpty());

    it('shows no-suggestions message', async () => {
      const { ReviewPage } = await import('./ReviewPage');
      render(<MemoryRouter><ReviewPage folderIds={['test-folder']} /></MemoryRouter>);
      expect(screen.getByText(/No pending suggestions/)).toBeTruthy();
    });
  });
});
```

**Step 2: Run test to verify it fails**

Run: `cd lens-editor && npx vitest run src/components/ReviewPage/ReviewPage.test.tsx`

Expected: FAIL — component doesn't exist

**Step 3: Implement ReviewPage component**

```typescript
// lens-editor/src/components/ReviewPage/ReviewPage.tsx
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useSuggestions, type FileSuggestions, type SuggestionItem } from '../../hooks/useSuggestions';

interface ReviewPageProps {
  folderIds: string[];
  relayId?: string;
  onAction?: (docId: string, suggestion: SuggestionItem, action: 'accept' | 'reject') => Promise<void>;
  onAcceptAllFile?: (file: FileSuggestions) => Promise<void>;
  onRejectAllFile?: (file: FileSuggestions) => Promise<void>;
  onAcceptAll?: () => Promise<void>;
  onRejectAll?: () => Promise<void>;
}

export function ReviewPage({ folderIds, relayId, onAction, onAcceptAllFile, onRejectAllFile, onAcceptAll, onRejectAll }: ReviewPageProps) {
  const { data, loading, error, refresh } = useSuggestions(folderIds);
  const [expandedFiles, setExpandedFiles] = useState<Set<string>>(new Set());
  const navigate = useNavigate();

  const toggleFile = (docId: string) => {
    setExpandedFiles(prev => {
      const next = new Set(prev);
      if (next.has(docId)) next.delete(docId);
      else next.add(docId);
      return next;
    });
  };

  const totalSuggestions = data.reduce((sum, f) => sum + f.suggestions.length, 0);

  const navigateToSuggestion = (docId: string, from: number) => {
    // Extract UUID from compound doc ID (last 36 chars), take first 8 for short URL
    const uuid = docId.slice(-36);
    const shortUuid = uuid.slice(0, 8);
    navigate(`/${shortUuid}?pos=${from}`);
  };

  if (loading) {
    return <div className="p-8 text-gray-500">Scanning documents for suggestions...</div>;
  }

  if (error) {
    return <div className="p-8 text-red-600">Error: {error}</div>;
  }

  if (data.length === 0) {
    return (
      <div className="p-8 text-center text-gray-500">
        <p className="text-lg">No pending suggestions</p>
        <p className="text-sm mt-2">All documents are clean.</p>
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto">
      <div className="max-w-4xl mx-auto p-6">
        <div className="flex items-center justify-between mb-6">
          <div>
            <h1 className="text-xl font-semibold text-gray-900">Review Suggestions</h1>
            <p className="text-sm text-gray-500 mt-1">
              {totalSuggestions} suggestion{totalSuggestions !== 1 ? 's' : ''} across {data.length} file{data.length !== 1 ? 's' : ''}
            </p>
          </div>
          <div className="flex gap-2">
            {onAcceptAll && (
              <button
                onClick={onAcceptAll}
                className="px-3 py-1.5 text-sm bg-green-600 text-white rounded hover:bg-green-700"
              >
                Accept All
              </button>
            )}
            {onRejectAll && (
              <button
                onClick={onRejectAll}
                className="px-3 py-1.5 text-sm bg-red-600 text-white rounded hover:bg-red-700"
              >
                Reject All
              </button>
            )}
            <button
              onClick={refresh}
              className="px-3 py-1.5 text-sm border border-gray-300 rounded hover:bg-gray-50"
            >
              Refresh
            </button>
          </div>
        </div>

        <div className="space-y-2">
          {data.map(file => (
            <FileSection
              key={file.doc_id}
              file={file}
              expanded={expandedFiles.has(file.doc_id)}
              onToggle={() => toggleFile(file.doc_id)}
              onAction={onAction}
              onAcceptAllFile={onAcceptAllFile}
              onRejectAllFile={onRejectAllFile}
              onNavigate={navigateToSuggestion}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

function FileSection({ file, expanded, onToggle, onAction, onAcceptAllFile, onRejectAllFile, onNavigate }: {
  file: FileSuggestions;
  expanded: boolean;
  onToggle: () => void;
  onAction?: (docId: string, suggestion: SuggestionItem, action: 'accept' | 'reject') => Promise<void>;
  onAcceptAllFile?: (file: FileSuggestions) => Promise<void>;
  onRejectAllFile?: (file: FileSuggestions) => Promise<void>;
  onNavigate: (docId: string, from: number) => void;
}) {
  return (
    <div className="border border-gray-200 rounded-lg overflow-hidden">
      <div className="flex items-center justify-between px-4 py-3 bg-gray-50 hover:bg-gray-100 transition-colors">
        <button
          onClick={onToggle}
          className="flex items-center gap-3 flex-1"
        >
          <span className="text-xs text-gray-400">{expanded ? '▼' : '▶'}</span>
          <span className="font-medium text-gray-800">{file.path}</span>
          <span className="text-xs text-gray-500 bg-gray-200 px-2 py-0.5 rounded-full">
            {file.suggestions.length} suggestion{file.suggestions.length !== 1 ? 's' : ''}
          </span>
        </button>
        {expanded && (
          <div className="flex gap-1 ml-2">
            {onAcceptAllFile && (
              <button
                onClick={() => onAcceptAllFile(file)}
                title="Accept all in file"
                className="px-2 py-1 text-xs text-green-700 hover:bg-green-50 rounded"
              >
                Accept All
              </button>
            )}
            {onRejectAllFile && (
              <button
                onClick={() => onRejectAllFile(file)}
                title="Reject all in file"
                className="px-2 py-1 text-xs text-red-700 hover:bg-red-50 rounded"
              >
                Reject All
              </button>
            )}
          </div>
        )}
      </div>
      {expanded && (
        <div className="divide-y divide-gray-100">
          {file.suggestions.map((s, i) => (
            <SuggestionRow
              key={i}
              suggestion={s}
              onAccept={onAction ? () => onAction(file.doc_id, s, 'accept') : undefined}
              onReject={onAction ? () => onAction(file.doc_id, s, 'reject') : undefined}
              onNavigate={() => onNavigate(file.doc_id, s.from)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function SuggestionRow({ suggestion, onAccept, onReject, onNavigate }: {
  suggestion: SuggestionItem;
  onAccept?: () => void;
  onReject?: () => void;
  onNavigate: () => void;
}) {
  const typeColors = {
    addition: 'bg-green-100 text-green-800',
    deletion: 'bg-red-100 text-red-800',
    substitution: 'bg-yellow-100 text-yellow-800',
  };

  return (
    <div className="px-4 py-3 flex items-start gap-3">
      <span className={`text-xs px-2 py-0.5 rounded font-medium ${typeColors[suggestion.type]}`}>
        {suggestion.type}
      </span>
      <button
        onClick={onNavigate}
        className="flex-1 min-w-0 text-left hover:bg-gray-50 rounded px-1 -mx-1"
        title="Open in editor"
      >
        <div className="font-mono text-sm">
          <span className="text-gray-400">{suggestion.context_before}</span>
          {suggestion.type === 'substitution' ? (
            <>
              <span className="bg-red-100 line-through">{suggestion.old_content}</span>
              <span className="bg-green-100">{suggestion.new_content}</span>
            </>
          ) : suggestion.type === 'deletion' ? (
            <span className="bg-red-100 line-through">{suggestion.content}</span>
          ) : (
            <span className="bg-green-100">{suggestion.content}</span>
          )}
          <span className="text-gray-400">{suggestion.context_after}</span>
        </div>
        <div className="flex items-center gap-2 mt-1 text-xs text-gray-400">
          {suggestion.author && <span className="bg-gray-100 px-1.5 py-0.5 rounded">{suggestion.author}</span>}
          {suggestion.timestamp && (
            <span>{new Date(suggestion.timestamp).toLocaleString()}</span>
          )}
        </div>
      </button>
      <div className="flex gap-1">
        {onAccept && (
          <button
            onClick={onAccept}
            title="Accept"
            className="p-1 text-green-600 hover:bg-green-50 rounded"
          >
            ✓
          </button>
        )}
        {onReject && (
          <button
            onClick={onReject}
            title="Reject"
            className="p-1 text-red-600 hover:bg-red-50 rounded"
          >
            ✗
          </button>
        )}
      </div>
    </div>
  );
}
```

**Step 4: Run tests to verify they pass**

Run: `cd lens-editor && npx vitest run src/components/ReviewPage/ReviewPage.test.tsx`

Expected: All 7 tests PASS

**Step 5: Add route in App.tsx**

In `AuthenticatedApp` component, add the `/review` route **before** the `/:docUuid/*` route (more specific routes first):

```tsx
// Inside the <Routes> block (around line 313-316):
<Route path="/review" element={<ReviewPage folderIds={FOLDERS.map(f => `${RELAY_ID}-${f.id}`)} relayId={RELAY_ID} />} />
<Route path="/:docUuid/*" element={<DocumentView />} />
```

Add import at top of App.tsx:
```tsx
import { ReviewPage } from './components/ReviewPage/ReviewPage';
```

**Step 6: Verify the app compiles**

Run: `cd lens-editor && npx tsc --noEmit`

Expected: No type errors

**Step 7: Commit**

```bash
jj new -m "feat: add ReviewPage component with /review route"
```

---

## Task 5: Accept/Reject Pure Functions (Tested)

**Files:**
- Create: `lens-editor/src/lib/suggestion-actions.test.ts`
- Create: `lens-editor/src/lib/suggestion-actions.ts`

These are pure functions that operate on Y.Doc text. Separated from the connection hook for testability.

**Step 1: Write the failing tests**

```typescript
// lens-editor/src/lib/suggestion-actions.test.ts
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
    // Markup is at position 10, but suggestion.from says 5 (stale)
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
```

**Step 2: Run test to verify it fails**

Run: `cd lens-editor && npx vitest run src/lib/suggestion-actions.test.ts`

Expected: FAIL — module doesn't exist

**Step 3: Implement the pure functions**

```typescript
// lens-editor/src/lib/suggestion-actions.ts
import * as Y from 'yjs';
import type { SuggestionItem } from '../hooks/useSuggestions';

/**
 * Apply accept/reject to a suggestion in a Y.Doc.
 * Uses `raw_markup` from the server to find the exact string (avoids reconstruction fragility).
 * Searches near `suggestion.from` first, then falls back to searching the entire doc.
 */
export function applySuggestionAction(
  doc: Y.Doc,
  suggestion: SuggestionItem,
  action: 'accept' | 'reject',
) {
  const text = doc.getText('contents');
  const content = text.toString();

  const markup = suggestion.raw_markup;
  // Search near the expected position first (within 200 chars), then fall back to full search
  let idx = content.indexOf(markup, Math.max(0, suggestion.from - 200));
  if (idx === -1) {
    idx = content.indexOf(markup);
  }
  if (idx === -1) {
    throw new Error('Suggestion no longer found in document');
  }

  const replacement = action === 'accept'
    ? getAcceptText(suggestion)
    : getRejectText(suggestion);

  doc.transact(() => {
    text.delete(idx, markup.length);
    if (replacement) {
      text.insert(idx, replacement);
    }
  });
}

export function getAcceptText(s: SuggestionItem): string {
  switch (s.type) {
    case 'addition': return s.content;
    case 'deletion': return '';
    case 'substitution': return s.new_content ?? '';
  }
}

export function getRejectText(s: SuggestionItem): string {
  switch (s.type) {
    case 'addition': return '';
    case 'deletion': return s.content;
    case 'substitution': return s.old_content ?? '';
  }
}
```

**Step 4: Run tests to verify they pass**

Run: `cd lens-editor && npx vitest run src/lib/suggestion-actions.test.ts`

Expected: All 11 tests PASS

**Step 5: Commit**

```bash
jj new -m "feat: add suggestion accept/reject pure functions with tests"
```

---

## Task 6: Wire Up Accept/Reject on Review Page

**Files:**
- Create: `lens-editor/src/hooks/useDocConnection.ts`
- Modify: `lens-editor/src/components/ReviewPage/ReviewPage.tsx` (wire callbacks)

**Step 1: Write the doc connection hook**

```typescript
// lens-editor/src/hooks/useDocConnection.ts
import { useRef, useCallback } from 'react';
import * as Y from 'yjs';
import { WebsocketProvider } from 'y-websocket';
import { getRelayWsUrl } from '../lib/auth';

/**
 * Manages temporary Y.Doc connections for applying suggestion actions
 * from the review page (outside the normal editor context).
 */
export function useDocConnection() {
  const connections = useRef<Map<string, { doc: Y.Doc; provider: WebsocketProvider }>>(new Map());

  const getOrConnect = useCallback(async (docId: string): Promise<Y.Doc> => {
    const existing = connections.current.get(docId);
    if (existing) return existing.doc;

    const doc = new Y.Doc();
    const wsUrl = getRelayWsUrl();
    const provider = new WebsocketProvider(wsUrl, docId, doc);

    await new Promise<void>((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error('Connection timeout')), 10000);
      provider.on('sync', (synced: boolean) => {
        if (synced) {
          clearTimeout(timeout);
          resolve();
        }
      });
    });

    connections.current.set(docId, { doc, provider });
    return doc;
  }, []);

  const disconnect = useCallback((docId: string) => {
    const conn = connections.current.get(docId);
    if (conn) {
      conn.provider.destroy();
      conn.doc.destroy();
      connections.current.delete(docId);
    }
  }, []);

  const disconnectAll = useCallback(() => {
    for (const [id] of connections.current) {
      disconnect(id);
    }
  }, [disconnect]);

  return { getOrConnect, disconnect, disconnectAll };
}
```

**Step 2: Create a wrapper component that wires everything together**

Create a container component (or update the route in `App.tsx`) that wraps `ReviewPage` with the action handlers:

```typescript
// In App.tsx or a new ReviewPageContainer.tsx — add this wrapper:
import { useDocConnection } from '../../hooks/useDocConnection';
import { applySuggestionAction } from '../../lib/suggestion-actions';
import type { SuggestionItem, FileSuggestions } from '../../hooks/useSuggestions';

function ReviewPageWithActions({ folderIds, relayId }: { folderIds: string[]; relayId: string }) {
  const { getOrConnect, disconnectAll } = useDocConnection();
  const [, forceRefresh] = useState(0);

  useEffect(() => disconnectAll, [disconnectAll]);

  const handleAction = async (docId: string, suggestion: SuggestionItem, action: 'accept' | 'reject') => {
    const doc = await getOrConnect(docId);
    applySuggestionAction(doc, suggestion, action);
  };

  const handleAcceptAllFile = async (file: FileSuggestions) => {
    const doc = await getOrConnect(file.doc_id);
    // Process from last to first to preserve positions
    const sorted = [...file.suggestions].sort((a, b) => b.from - a.from);
    for (const s of sorted) {
      applySuggestionAction(doc, s, 'accept');
    }
  };

  const handleRejectAllFile = async (file: FileSuggestions) => {
    const doc = await getOrConnect(file.doc_id);
    const sorted = [...file.suggestions].sort((a, b) => b.from - a.from);
    for (const s of sorted) {
      applySuggestionAction(doc, s, 'reject');
    }
  };

  return (
    <ReviewPage
      folderIds={folderIds}
      relayId={relayId}
      onAction={handleAction}
      onAcceptAllFile={handleAcceptAllFile}
      onRejectAllFile={handleRejectAllFile}
    />
  );
}
```

Update the route in `App.tsx` to use this wrapper.

**Step 3: Verify it compiles**

Run: `cd lens-editor && npx tsc --noEmit`

Expected: No type errors

**Step 4: Manual test**

1. Start relay server: `cd lens-editor && npm run relay:start`
2. Set up test data: `cd lens-editor && npm run relay:setup`
3. Create a suggestion via MCP or editor
4. Navigate to `/review` in browser
5. Verify suggestions appear, accept/reject works per suggestion and per file

**Step 5: Commit**

```bash
jj new -m "feat: wire up accept/reject actions on review page via Y.Doc connections"
```

---

## Task 7: Navigation Link to Review Page

**Files:**
- Modify: `lens-editor/src/components/Sidebar/Sidebar.test.tsx`
- Modify: `lens-editor/src/components/Sidebar/Sidebar.tsx`

**Step 1: Write the failing test**

Add to the existing `Sidebar.test.tsx`:

```typescript
it('renders Review link', () => {
  // render Sidebar with necessary providers
  expect(screen.getByText('Review')).toBeTruthy();
});
```

**Step 2: Run test to verify it fails**

Run: `cd lens-editor && npx vitest run src/components/Sidebar/Sidebar.test.tsx`

Expected: FAIL — no "Review" link in sidebar

**Step 3: Add the Review link to the Sidebar**

Add a link/button in the Sidebar that navigates to `/review`:

```tsx
<Link to="/review" className="...">Review</Link>
```

**Step 4: Run tests to verify they pass**

Run: `cd lens-editor && npx vitest run src/components/Sidebar/Sidebar.test.tsx`

Expected: PASS

**Step 5: Commit**

```bash
jj new -m "feat: add Review link to sidebar navigation"
```

---

## Implementation Notes

### Auth Pattern
The `/suggestions` endpoint uses the same `check_auth` pattern as other endpoints. The frontend `authFetch` helper (in `lib/auth.ts`) handles bearer token injection.

### `getRelayWsUrl` Helper
Check if `lib/auth.ts` already exports a WebSocket URL builder. If not, derive from the existing relay URL pattern — the WebSocket URL is the same as the HTTP URL but with `ws://` scheme and the doc ID appended.

### Raw Markup for Reliable Matching
The server returns `raw_markup` — the exact CriticMarkup string as it appears in the document. The frontend uses this for `indexOf` matching instead of reconstructing the markup (which would be fragile if JSON key ordering differs between server and client).

### Position Drift
When accepting/rejecting from the review page, the positions returned by the scan may be stale (another user may have edited). The `applySuggestionAction` function handles this by re-searching for the `raw_markup` string near the expected position, falling back to a full document search.

### Bulk Accept/Reject Order
When applying "accept all" on a file, process suggestions from last to first (by position) so earlier positions aren't invalidated by text length changes.

### Serialization Contract
Optional fields (`author`, `timestamp`, `old_content`, `new_content`) serialize as `null` (not omitted) to match the design doc API contract. Frontend types use `| null`.

### Click-to-Navigate
Each suggestion row is a clickable button that navigates to `/:shortUuid?pos=N`. The editor can read the `pos` query param to scroll to the suggestion position (may need a small addition to `DocumentView` to handle `?pos=` on mount).

### Future Enhancements (Not in Scope)
- Persistent suggestion index (upgrade from on-demand scan)
- Suggestion count badges in file list
- Filter by author/date on review page
- Keyboard navigation (j/k to move between suggestions, a/r to accept/reject)
- Global "Accept All" / "Reject All" (deferred — requires connecting to every doc; add when needed)
