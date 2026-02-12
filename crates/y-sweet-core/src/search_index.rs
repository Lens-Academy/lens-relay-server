use anyhow::Result;
use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;
use tantivy::collector::TopDocs;
use tantivy::directory::{MmapDirectory, RamDirectory};
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, TextFieldIndexing, TextOptions, Value, STORED, STRING};
use tantivy::snippet::SnippetGenerator;
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};

/// A single search result with relevance score and snippet.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub doc_id: String,
    pub title: String,
    pub folder: String,
    pub snippet: String,
    pub score: f32,
}

/// Full-text search index backed by tantivy.
///
/// Provides BM25-ranked full-text search with snippet generation over documents
/// identified by unique `doc_id`. Thread-safe: the IndexWriter is wrapped in a Mutex.
pub struct SearchIndex {
    #[allow(dead_code)]
    index: Index,
    #[allow(dead_code)]
    schema: Schema,
    doc_id_field: Field,
    title_field: Field,
    body_field: Field,
    folder_field: Field,
    writer: Mutex<IndexWriter>,
    reader: IndexReader,
    query_parser: QueryParser,
}

impl SearchIndex {
    /// Create a new SearchIndex with MmapDirectory at the given path.
    pub fn new(path: &Path) -> Result<Self> {
        std::fs::create_dir_all(path)?;
        let dir = MmapDirectory::open(path)?;
        Self::build(dir)
    }

    /// Create a new SearchIndex backed by RAM (for tests).
    pub fn new_in_memory() -> Result<Self> {
        let dir = RamDirectory::create();
        Self::build(dir)
    }

    /// Internal constructor that works with any tantivy Directory.
    fn build<D: Into<Box<dyn tantivy::Directory>>>(dir: D) -> Result<Self> {
        let mut schema_builder = Schema::builder();

        // doc_id: STRING (indexed as single token for exact match) + STORED
        let doc_id_field = schema_builder.add_text_field("doc_id", STRING | STORED);

        // title: TEXT (tokenized for search) + STORED
        let text_options = TextOptions::default()
            .set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer("default")
                    .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions),
            )
            .set_stored();
        let title_field = schema_builder.add_text_field("title", text_options.clone());

        // body: TEXT + STORED (STORED is required for snippet generation)
        let body_field = schema_builder.add_text_field("body", text_options);

        // folder: STORED only (not searchable in v1)
        let folder_field = schema_builder.add_text_field("folder", STORED);

        let schema = schema_builder.build();

        let index = Index::open_or_create(dir, schema.clone())?;

        let writer: IndexWriter = index.writer(15_000_000)?; // 15MB budget

        let reader: IndexReader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        // QueryParser with AND semantics and title boost 2x
        let mut query_parser = QueryParser::for_index(&index, vec![title_field, body_field]);
        query_parser.set_conjunction_by_default();
        query_parser.set_field_boost(title_field, 2.0);

        Ok(SearchIndex {
            index,
            schema,
            doc_id_field,
            title_field,
            body_field,
            folder_field,
            writer: Mutex::new(writer),
            reader,
            query_parser,
        })
    }

    /// Add or update a document in the index.
    ///
    /// This is idempotent: if a document with the same `doc_id` already exists,
    /// it is deleted before the new version is added.
    pub fn add_document(
        &self,
        doc_id: &str,
        title: &str,
        body: &str,
        folder: &str,
    ) -> Result<()> {
        let mut writer = self.writer.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        // Delete existing document with same doc_id
        let term = Term::from_field_text(self.doc_id_field, doc_id);
        writer.delete_term(term);
        // Add the new document
        writer.add_document(doc!(
            self.doc_id_field => doc_id,
            self.title_field => title,
            self.body_field => body,
            self.folder_field => folder,
        ))?;
        writer.commit()?;
        // Reload the reader to pick up changes immediately
        self.reader.reload()?;
        Ok(())
    }

    /// Remove a document from the index by doc_id.
    pub fn remove_document(&self, doc_id: &str) -> Result<()> {
        let mut writer = self.writer.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        let term = Term::from_field_text(self.doc_id_field, doc_id);
        writer.delete_term(term);
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// Search the index and return ranked results with snippets.
    ///
    /// Returns an empty Vec for empty or whitespace-only queries.
    /// Uses `parse_query_lenient` to tolerate syntax errors in the query string.
    ///
    /// When Tantivy's snippet generator can't find highlights in the body
    /// (e.g. title-only matches), falls back to a manual substring search
    /// that extracts context around the first matching query term.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        // Guard: empty or whitespace-only queries return nothing
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let (parsed_query, _errors) = self.query_parser.parse_query_lenient(query);

        let searcher = self.reader.searcher();
        let top_docs = searcher.search(&parsed_query, &TopDocs::with_limit(limit))?;

        // Set up snippet generator for the body field
        let mut snippet_generator =
            SnippetGenerator::create(&searcher, &*parsed_query, self.body_field)?;
        snippet_generator.set_max_num_chars(200);

        // Extract query terms for fallback snippet generation
        let query_terms: Vec<String> = query
            .split_whitespace()
            .filter(|t| !t.is_empty())
            .map(|t| t.to_lowercase())
            .collect();

        let mut results = Vec::with_capacity(top_docs.len());
        for (score, doc_address) in top_docs {
            let retrieved: TantivyDocument = searcher.doc(doc_address)?;

            let doc_id = retrieved
                .get_first(self.doc_id_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = retrieved
                .get_first(self.title_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let folder = retrieved
                .get_first(self.folder_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let body = retrieved
                .get_first(self.body_field)
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let snippet = snippet_generator.snippet_from_doc(&retrieved);
            let snippet_html = if snippet.highlighted().is_empty() {
                // Tantivy found no highlights in the body (likely a title-only match).
                // Fall back to manual substring search in the body text.
                generate_fallback_snippet(body, &query_terms, 200)
            } else {
                render_snippet_with_mark(&snippet, body)
            };

            results.push(SearchResult {
                doc_id,
                title,
                folder,
                snippet: snippet_html,
                score,
            });
        }

        Ok(results)
    }
}

/// Render a tantivy Snippet using `<mark>` tags instead of the default `<b>` tags.
/// HTML-escapes non-highlighted text to prevent injection via `dangerouslySetInnerHTML`.
/// Adds "..." when the fragment is a subset of the full body text.
fn render_snippet_with_mark(snippet: &tantivy::snippet::Snippet, full_body: &str) -> String {
    let fragment = snippet.fragment();
    let highlighted = snippet.highlighted();

    if highlighted.is_empty() {
        return escape_html(fragment);
    }

    let mut result = String::new();
    let mut pos = 0;

    for range in highlighted {
        // Append escaped text before this highlight
        if range.start > pos {
            result.push_str(&escape_html(&fragment[pos..range.start]));
        }
        // Append highlighted text with <mark> tags (escaped inside)
        result.push_str("<mark>");
        result.push_str(&escape_html(&fragment[range.start..range.end]));
        result.push_str("</mark>");
        pos = range.end;
    }

    // Append remaining escaped text after last highlight
    if pos < fragment.len() {
        result.push_str(&escape_html(&fragment[pos..]));
    }

    // Add "..." indicators when the fragment is truncated
    if !fragment.is_empty() && fragment.len() < full_body.len() {
        let prefix_len = 20.min(fragment.len());
        let is_at_start = full_body.starts_with(&fragment[..prefix_len]);
        if !is_at_start {
            result = format!("...{}", result);
        }

        let suffix_len = 20.min(fragment.len());
        let is_at_end = full_body.ends_with(&fragment[fragment.len() - suffix_len..]);
        if !is_at_end {
            result.push_str("...");
        }
    }

    result
}

/// Escape HTML special characters to prevent XSS when rendering snippets.
fn escape_html(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            _ => result.push(ch),
        }
    }
    result
}

/// Generate a snippet by manually searching for query terms in the body text.
/// Used as a fallback when Tantivy's snippet generator can't find highlights
/// (e.g. when the document matched on title but not body).
///
/// Returns an HTML string with `<mark>` tags around matching terms,
/// or an empty string if no query terms appear in the body.
fn generate_fallback_snippet(body: &str, query_terms: &[String], max_chars: usize) -> String {
    if body.is_empty() || query_terms.is_empty() {
        return String::new();
    }

    // Find the byte position of the earliest matching query term (case-insensitive ASCII)
    let mut best_match: Option<(usize, usize)> = None; // (start, end) byte positions

    for term in query_terms {
        if let Some((start, end)) = find_ascii_ci(body, term) {
            match best_match {
                None => best_match = Some((start, end)),
                Some((existing_start, _)) if start < existing_start => {
                    best_match = Some((start, end));
                }
                _ => {}
            }
        }
    }

    let (match_start, _match_end) = match best_match {
        Some(m) => m,
        None => return String::new(), // No query terms found in body
    };

    // Extract a window of text centered on the match
    let half = max_chars / 2;

    // Find fragment start — snap to word boundary
    let raw_start = match_start.saturating_sub(half);
    let frag_start = if raw_start == 0 {
        0
    } else {
        // Snap forward to char boundary, then to next space
        let safe = snap_char_boundary_forward(body, raw_start);
        body[safe..].find(' ').map_or(safe, |off| safe + off + 1)
    };

    // Find fragment end — snap to word boundary
    let raw_end = (match_start + half).min(body.len());
    let frag_end = if raw_end >= body.len() {
        body.len()
    } else {
        let safe = snap_char_boundary_backward(body, raw_end);
        body[..safe].rfind(' ').unwrap_or(safe)
    };

    if frag_start >= frag_end {
        return String::new();
    }

    let fragment = &body[frag_start..frag_end];

    // Find all occurrences of all query terms in the fragment for highlighting
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for term in query_terms {
        ranges.extend(find_all_ascii_ci(fragment, term));
    }

    // Sort and merge overlapping ranges
    ranges.sort_by_key(|r| r.0);
    let merged = merge_ranges(&ranges);

    // Build HTML with ellipsis and highlights
    let mut html = String::new();
    if frag_start > 0 {
        html.push_str("...");
    }

    let mut pos = 0;
    for (start, end) in &merged {
        if *start > pos {
            html.push_str(&escape_html(&fragment[pos..*start]));
        }
        html.push_str("<mark>");
        html.push_str(&escape_html(&fragment[*start..*end]));
        html.push_str("</mark>");
        pos = *end;
    }
    if pos < fragment.len() {
        html.push_str(&escape_html(&fragment[pos..]));
    }

    if frag_end < body.len() {
        html.push_str("...");
    }

    html
}

/// Case-insensitive ASCII substring search. Returns byte position range in `haystack`.
fn find_ascii_ci(haystack: &str, needle: &str) -> Option<(usize, usize)> {
    let needle_bytes: Vec<u8> = needle.bytes().map(|b| b.to_ascii_lowercase()).collect();
    let needle_len = needle_bytes.len();
    if needle_len == 0 || needle_len > haystack.len() {
        return None;
    }
    let hay = haystack.as_bytes();
    for i in 0..=(hay.len() - needle_len) {
        if hay[i..i + needle_len]
            .iter()
            .map(|b| b.to_ascii_lowercase())
            .eq(needle_bytes.iter().copied())
        {
            return Some((i, i + needle_len));
        }
    }
    None
}

/// Find all non-overlapping case-insensitive ASCII matches.
fn find_all_ascii_ci(haystack: &str, needle: &str) -> Vec<(usize, usize)> {
    let needle_bytes: Vec<u8> = needle.bytes().map(|b| b.to_ascii_lowercase()).collect();
    let needle_len = needle_bytes.len();
    if needle_len == 0 || needle_len > haystack.len() {
        return Vec::new();
    }
    let hay = haystack.as_bytes();
    let mut results = Vec::new();
    let mut i = 0;
    while i + needle_len <= hay.len() {
        if hay[i..i + needle_len]
            .iter()
            .map(|b| b.to_ascii_lowercase())
            .eq(needle_bytes.iter().copied())
        {
            results.push((i, i + needle_len));
            i += needle_len; // skip past this match
        } else {
            i += 1;
        }
    }
    results
}

/// Snap a byte position forward to the nearest char boundary.
fn snap_char_boundary_forward(s: &str, pos: usize) -> usize {
    let mut p = pos.min(s.len());
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p
}

/// Snap a byte position backward to the nearest char boundary.
fn snap_char_boundary_backward(s: &str, pos: usize) -> usize {
    let mut p = pos.min(s.len());
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

/// Merge overlapping or adjacent byte ranges.
fn merge_ranges(ranges: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for &(start, end) in ranges {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_index() -> SearchIndex {
        SearchIndex::new_in_memory().expect("failed to create in-memory index")
    }

    #[test]
    fn empty_index_returns_empty_results() {
        let index = create_index();
        let results = index.search("anything", 10).unwrap();
        assert!(results.is_empty(), "expected no results from empty index");
    }

    #[test]
    fn search_by_title_finds_document() {
        let index = create_index();
        index
            .add_document("doc1", "Quantum Physics", "Introduction to quantum mechanics.", "Lens")
            .unwrap();
        let results = index.search("Quantum", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].doc_id, "doc1");
        assert_eq!(results[0].title, "Quantum Physics");
        assert!(results[0].score > 0.0, "score should be positive");
    }

    #[test]
    fn search_by_body_finds_document() {
        let index = create_index();
        index
            .add_document("doc1", "Physics Notes", "The Schrodinger equation is fundamental.", "Lens")
            .unwrap();
        let results = index.search("Schrodinger", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].doc_id, "doc1");
    }

    #[test]
    fn title_match_scores_higher_than_body_only_match() {
        let index = create_index();
        // doc1 has "gravity" in body only
        index
            .add_document(
                "doc1",
                "Physics Notes",
                "Gravity is a fundamental force of nature.",
                "Lens",
            )
            .unwrap();
        // doc2 has "gravity" in title
        index
            .add_document("doc2", "Gravity Explained", "An overview of forces.", "Lens")
            .unwrap();
        let results = index.search("gravity", 10).unwrap();
        assert!(results.len() >= 2, "expected at least 2 results");
        // The title match (doc2) should score higher
        assert_eq!(
            results[0].doc_id, "doc2",
            "title match should rank first, got {:?}",
            results.iter().map(|r| (&r.doc_id, r.score)).collect::<Vec<_>>()
        );
        assert!(
            results[0].score > results[1].score,
            "title match score ({}) should be higher than body-only score ({})",
            results[0].score,
            results[1].score
        );
    }

    #[test]
    fn snippet_contains_mark_tags() {
        let index = create_index();
        index
            .add_document(
                "doc1",
                "Photosynthesis",
                "Plants convert sunlight into energy through photosynthesis.",
                "Lens",
            )
            .unwrap();
        let results = index.search("photosynthesis", 10).unwrap();
        assert!(!results.is_empty(), "expected results");
        let snippet = &results[0].snippet;
        assert!(
            snippet.contains("<mark>") && snippet.contains("</mark>"),
            "snippet should contain <mark> tags, got: {snippet}"
        );
    }

    #[test]
    fn snippet_does_not_contain_bold_tags() {
        let index = create_index();
        index
            .add_document(
                "doc1",
                "Photosynthesis",
                "Plants convert sunlight into energy through photosynthesis.",
                "Lens",
            )
            .unwrap();
        let results = index.search("photosynthesis", 10).unwrap();
        assert!(!results.is_empty(), "expected results");
        let snippet = &results[0].snippet;
        assert!(
            !snippet.contains("<b>") && !snippet.contains("</b>"),
            "snippet should NOT contain <b> tags, got: {snippet}"
        );
    }

    #[test]
    fn update_document_replaces_old_content() {
        let index = create_index();
        index
            .add_document("doc1", "Original Title", "Original body content.", "Lens")
            .unwrap();
        // Update with new content
        index
            .add_document("doc1", "Updated Title", "Completely different body text.", "Lens")
            .unwrap();
        // Old content should not be found
        let old_results = index.search("Original", 10).unwrap();
        assert!(
            old_results.is_empty(),
            "old content should not be findable after update"
        );
        // New content should be found
        let new_results = index.search("Updated", 10).unwrap();
        assert_eq!(new_results.len(), 1);
        assert_eq!(new_results[0].doc_id, "doc1");
        assert_eq!(new_results[0].title, "Updated Title");
    }

    #[test]
    fn remove_document_makes_it_unsearchable() {
        let index = create_index();
        index
            .add_document("doc1", "Temporary Doc", "This will be removed.", "Lens")
            .unwrap();
        // Verify it exists
        let results = index.search("Temporary", 10).unwrap();
        assert_eq!(results.len(), 1);
        // Remove it
        index.remove_document("doc1").unwrap();
        // Should no longer be found
        let results = index.search("Temporary", 10).unwrap();
        assert!(
            results.is_empty(),
            "removed document should not appear in results"
        );
    }

    #[test]
    fn empty_query_returns_empty_results() {
        let index = create_index();
        index
            .add_document("doc1", "Some Doc", "Some content.", "Lens")
            .unwrap();
        let results = index.search("", 10).unwrap();
        assert!(results.is_empty(), "empty query should return no results");
    }

    #[test]
    fn whitespace_query_returns_empty_results() {
        let index = create_index();
        index
            .add_document("doc1", "Some Doc", "Some content.", "Lens")
            .unwrap();
        let results = index.search("   \t\n  ", 10).unwrap();
        assert!(
            results.is_empty(),
            "whitespace-only query should return no results"
        );
    }

    #[test]
    fn search_respects_limit() {
        let index = create_index();
        index
            .add_document("doc1", "Alpha", "Common search term here.", "Lens")
            .unwrap();
        index
            .add_document("doc2", "Beta", "Common search term here too.", "Lens")
            .unwrap();
        index
            .add_document("doc3", "Gamma", "Common search term again.", "Lens")
            .unwrap();
        let results = index.search("common", 1).unwrap();
        assert_eq!(
            results.len(),
            1,
            "should return at most 1 result when limit=1"
        );
    }

    #[test]
    fn phrase_search_works() {
        let index = create_index();
        index
            .add_document(
                "doc1",
                "Notes",
                "The quick brown fox jumps over the lazy dog.",
                "Lens",
            )
            .unwrap();
        index
            .add_document("doc2", "Other Notes", "The quick red car drives fast.", "Lens")
            .unwrap();
        // Phrase search should only match doc1
        let results = index.search("\"quick brown fox\"", 10).unwrap();
        assert_eq!(results.len(), 1, "phrase search should match exactly one doc");
        assert_eq!(results[0].doc_id, "doc1");
    }

    #[test]
    fn and_semantics_by_default() {
        let index = create_index();
        index
            .add_document("doc1", "Notes", "The cat sat on the mat.", "Lens")
            .unwrap();
        index
            .add_document("doc2", "Other", "The dog ran in the park.", "Lens")
            .unwrap();
        index
            .add_document("doc3", "Both", "The cat ran across the yard.", "Lens")
            .unwrap();
        // "cat ran" with AND semantics should only match doc3
        let results = index.search("cat ran", 10).unwrap();
        assert_eq!(
            results.len(),
            1,
            "AND semantics: only docs with both terms should match, got {} results",
            results.len()
        );
        assert_eq!(results[0].doc_id, "doc3");
    }

    #[test]
    fn lenient_parsing_handles_malformed_query() {
        let index = create_index();
        index
            .add_document("doc1", "Test Doc", "Some content for testing.", "Lens")
            .unwrap();
        // Malformed query should not error
        let result = index.search("test AND", 10);
        assert!(
            result.is_ok(),
            "malformed query should not error: {:?}",
            result.err()
        );
    }

    #[test]
    fn folder_is_stored_in_results() {
        let index = create_index();
        index
            .add_document("doc1", "Test", "Content here.", "Lens Edu")
            .unwrap();
        let results = index.search("Content", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].folder, "Lens Edu");
    }

    #[test]
    fn title_only_match_uses_fallback_snippet() {
        let index = create_index();
        // "quantum" appears in both title and body, but at different positions.
        // The body has "quantum" deep inside, not near the beginning.
        let body = "This is a long introduction about physics. ".repeat(10)
            + "The quantum realm is fascinating and complex. "
            + &"More text follows here with other topics. ".repeat(5);
        index
            .add_document("doc1", "Quantum Physics", &body, "Lens")
            .unwrap();
        let results = index.search("quantum", 10).unwrap();
        assert_eq!(results.len(), 1);
        let snippet = &results[0].snippet;
        // The snippet should contain the highlighted match, not just the beginning
        assert!(
            snippet.contains("<mark>"),
            "snippet should have highlights even for deep body match, got: {}",
            &snippet[..snippet.len().min(200)]
        );
    }

    #[test]
    fn title_only_match_no_body_occurrence_returns_empty_snippet() {
        let index = create_index();
        // "quantum" only in title, not in body at all
        index
            .add_document(
                "doc1",
                "Quantum Physics",
                "This document discusses forces and energy in nature.",
                "Lens",
            )
            .unwrap();
        let results = index.search("quantum", 10).unwrap();
        assert_eq!(results.len(), 1);
        // Should return empty snippet since "quantum" is nowhere in the body
        assert!(
            results[0].snippet.is_empty(),
            "snippet should be empty when term only in title, got: {}",
            results[0].snippet
        );
    }

    #[test]
    fn fallback_snippet_has_ellipsis_when_truncated() {
        // Test the fallback function directly — it's used when Tantivy has no highlights
        let body = "Beginning of the document with lots of introductory content. ".repeat(10)
            + "The target keyword appears here in the middle of the text. "
            + &"And then more content continues after the match. ".repeat(10);
        let terms = vec!["target".to_string()];
        let snippet = generate_fallback_snippet(&body, &terms, 200);
        assert!(
            snippet.contains("<mark>target</mark>"),
            "snippet should highlight the match: {}",
            snippet
        );
        assert!(
            snippet.starts_with("..."),
            "snippet should start with ... when match is not at beginning: {}",
            snippet
        );
        assert!(
            snippet.ends_with("..."),
            "snippet should end with ... when match is not at end: {}",
            snippet
        );
    }

    #[test]
    fn tantivy_snippet_has_ellipsis_for_body_match() {
        // When Tantivy finds the match in the body, it should also have "..." for context
        let index = create_index();
        let body = "Beginning of the document with lots of introductory content. ".repeat(10)
            + "The target keyword appears here in the middle of the text. "
            + &"And then more content continues after the match. ".repeat(10);
        index
            .add_document("doc1", "Target Document", &body, "Lens")
            .unwrap();
        let results = index.search("target", 10).unwrap();
        assert_eq!(results.len(), 1);
        let snippet = &results[0].snippet;
        assert!(
            snippet.contains("<mark>"),
            "snippet should highlight the match: {}",
            snippet
        );
        // Body match in the middle should have "..." on both sides
        assert!(
            snippet.starts_with("..."),
            "snippet should start with ... when match is not at beginning: {}",
            snippet
        );
    }

    #[test]
    fn snippet_escapes_html_in_body() {
        let index = create_index();
        index
            .add_document(
                "doc1",
                "HTML Test",
                "This has <script>alert('xss')</script> in the body for testing.",
                "Lens",
            )
            .unwrap();
        let results = index.search("testing", 10).unwrap();
        assert_eq!(results.len(), 1);
        let snippet = &results[0].snippet;
        // Should NOT contain raw HTML tags (except our <mark> tags)
        assert!(
            !snippet.contains("<script>"),
            "snippet should escape HTML: {}",
            snippet
        );
        assert!(
            snippet.contains("&lt;script&gt;"),
            "snippet should have escaped HTML entities: {}",
            snippet
        );
    }

    // Unit tests for helper functions

    #[test]
    fn find_ascii_ci_basic() {
        assert_eq!(find_ascii_ci("Hello World", "hello"), Some((0, 5)));
        assert_eq!(find_ascii_ci("Hello World", "WORLD"), Some((6, 11)));
        assert_eq!(find_ascii_ci("Hello World", "xyz"), None);
        assert_eq!(find_ascii_ci("", "test"), None);
        assert_eq!(find_ascii_ci("test", ""), None);
    }

    #[test]
    fn find_all_ascii_ci_basic() {
        let results = find_all_ascii_ci("Test the test of Testing", "test");
        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (0, 4));   // "Test"
        assert_eq!(results[1], (9, 13));  // "test"
        assert_eq!(results[2], (17, 21)); // "Test" in "Testing"
    }

    #[test]
    fn generate_fallback_snippet_basic() {
        let body = "The quick brown fox jumps over the lazy dog.";
        let terms = vec!["fox".to_string()];
        let result = generate_fallback_snippet(body, &terms, 200);
        assert!(result.contains("<mark>fox</mark>"), "got: {}", result);
    }

    #[test]
    fn generate_fallback_snippet_empty_body() {
        let result = generate_fallback_snippet("", &["test".to_string()], 200);
        assert!(result.is_empty());
    }

    #[test]
    fn generate_fallback_snippet_no_match() {
        let result = generate_fallback_snippet("Hello world", &["xyz".to_string()], 200);
        assert!(result.is_empty());
    }

    #[test]
    fn merge_ranges_overlapping() {
        let ranges = vec![(0, 5), (3, 8), (10, 15)];
        let merged = merge_ranges(&ranges);
        assert_eq!(merged, vec![(0, 8), (10, 15)]);
    }

    #[test]
    fn merge_ranges_adjacent() {
        let ranges = vec![(0, 5), (5, 10)];
        let merged = merge_ranges(&ranges);
        assert_eq!(merged, vec![(0, 10)]);
    }
}
