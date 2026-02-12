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

    /// Add a document without committing. Call `flush()` after a batch.
    pub fn add_document_buffered(
        &self,
        doc_id: &str,
        title: &str,
        body: &str,
        folder: &str,
    ) -> Result<()> {
        let mut writer = self.writer.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        let term = Term::from_field_text(self.doc_id_field, doc_id);
        writer.delete_term(term);
        writer.add_document(doc!(
            self.doc_id_field => doc_id,
            self.title_field => title,
            self.body_field => body,
            self.folder_field => folder,
        ))?;
        Ok(())
    }

    /// Commit buffered changes and reload the reader.
    pub fn flush(&self) -> Result<()> {
        let mut writer = self.writer.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// Search the index and return ranked results with snippets.
    ///
    /// Returns an empty Vec for empty or whitespace-only queries.
    /// Uses `parse_query_lenient` to tolerate syntax errors in the query string.
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
        snippet_generator.set_max_num_chars(150);

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

            let snippet = snippet_generator.snippet_from_doc(&retrieved);
            let snippet_html = render_snippet_with_mark(&snippet);

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
fn render_snippet_with_mark(snippet: &tantivy::snippet::Snippet) -> String {
    let fragment = snippet.fragment();
    let highlighted = snippet.highlighted();

    if highlighted.is_empty() {
        return fragment.to_string();
    }

    let mut result = String::new();
    let mut pos = 0;

    for range in highlighted {
        // Append text before this highlight
        if range.start > pos {
            result.push_str(&fragment[pos..range.start]);
        }
        // Append highlighted text with <mark> tags
        result.push_str("<mark>");
        result.push_str(&fragment[range.start..range.end]);
        result.push_str("</mark>");
        pos = range.end;
    }

    // Append remaining text after last highlight
    if pos < fragment.len() {
        result.push_str(&fragment[pos..]);
    }

    result
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
}
