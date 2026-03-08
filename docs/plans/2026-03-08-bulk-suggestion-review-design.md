# Bulk Suggestion Review — Design

## Problem

Suggestions (CriticMarkup) are embedded inline in Y.Doc text. Currently, you can only see and act on suggestions by opening each document individually. When Claude (via MCP) makes suggestions across many files, there's no way to review them all at once.

## Solution

A new `/review` page in lens-editor that shows all pending suggestions across all documents in all folders, grouped by file, with accept/reject controls at individual, per-file, and global levels.

### Discovery: Server-Side On-Demand Scan

New REST endpoint `GET /suggestions?folder_id=...` that:
1. Finds all content doc UUIDs in the specified folder (via `filemeta_v0`)
2. Loads each doc and scans its `contents` Y.Text for CriticMarkup patterns
3. Returns a JSON response with suggestions grouped by document

No persistent index — just a one-shot scan using the same `read_doc_content` pattern as the grep tool.

### Action: Client-Side via Y.Doc Connections

Accept/reject requires modifying Y.Doc text, which must go through Y.js transactions. The review page connects to individual documents only when the user acts on a suggestion:
1. User clicks accept/reject
2. Client opens WebSocket to that doc (or reuses existing connection)
3. Applies the same accept/reject logic already in `criticmarkup-actions.ts`
4. Disconnects when done (or keeps alive for further actions on same doc)

### API Shape

```
GET /suggestions?folder_id={compound_folder_id}

Response:
{
  "files": [
    {
      "path": "Notes/Meeting.md",
      "doc_id": "cb696037-...-abc123",
      "suggestions": [
        {
          "type": "addition",
          "content": "new text here",
          "old_content": null,
          "new_content": null,
          "author": "AI",
          "timestamp": 1709900000000,
          "from": 142,
          "to": 198,
          "context_before": "some text before the suggestion",
          "context_after": "some text after the suggestion"
        }
      ]
    }
  ]
}
```

### Review Page UI

- Route: `/review` (outside the `/:docUuid/*` pattern)
- File list with suggestion counts, expandable to show inline suggestions
- Each suggestion shows: type badge, author, timestamp, content with surrounding context (rendered like the editor's CriticMarkup decorations)
- Actions: accept/reject per suggestion, "accept all" / "reject all" per file, global "accept all"
- Clicking a suggestion navigates to that position in the editor (existing `/:docUuid` route)

## Scope Summary

| Layer | Work | Estimate |
|-------|------|----------|
| Rust: CriticMarkup scanner | Regex parser + metadata extraction (~150 lines) | 1-2 days |
| Rust: `/suggestions` endpoint | Handler + folder iteration (follows grep.rs pattern) | 1 day |
| Frontend: API client | Fetch hook for suggestions endpoint | 0.5 day |
| Frontend: Review page + route | File list, suggestion display, expand/collapse | 2-3 days |
| Frontend: Accept/reject from review | Y.Doc connection manager, reuse existing actions | 1-2 days |
| **Total** | | **~5-8 days** |
