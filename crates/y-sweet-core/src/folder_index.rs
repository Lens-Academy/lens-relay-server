use dashmap::DashMap;
use std::sync::Arc;

/// In-memory index mapping document UUIDs to their parent folder IDs.
///
/// This is needed because content doc IDs (e.g., "relay-id-doc-uuid") don't
/// contain the folder ID. When a content doc is updated, we need to know
/// which folder's backlinks_v0 to update.
#[derive(Clone)]
pub struct FolderIndex {
    // doc_uuid -> folder_id
    index: Arc<DashMap<String, String>>,
}

impl FolderIndex {
    pub fn new() -> Self {
        Self {
            index: Arc::new(DashMap::new()),
        }
    }

    /// Register a document as belonging to a folder.
    pub fn register(&self, doc_uuid: &str, folder_id: &str) {
        self.index.insert(doc_uuid.to_string(), folder_id.to_string());
    }

    /// Unregister a document (when deleted from folder).
    pub fn unregister(&self, doc_uuid: &str) {
        self.index.remove(doc_uuid);
    }

    /// Look up which folder a document belongs to.
    pub fn get_folder(&self, doc_uuid: &str) -> Option<String> {
        self.index.get(doc_uuid).map(|r| r.value().clone())
    }

    /// Get all documents in a folder.
    pub fn get_docs_in_folder(&self, folder_id: &str) -> Vec<String> {
        self.index
            .iter()
            .filter(|entry| entry.value() == folder_id)
            .map(|entry| entry.key().clone())
            .collect()
    }
}

impl Default for FolderIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_and_retrieves_folder() {
        let index = FolderIndex::new();
        index.register("doc-123", "folder-abc");

        assert_eq!(index.get_folder("doc-123"), Some("folder-abc".to_string()));
    }

    #[test]
    fn returns_none_for_unknown_doc() {
        let index = FolderIndex::new();

        assert_eq!(index.get_folder("unknown"), None);
    }

    #[test]
    fn unregisters_doc() {
        let index = FolderIndex::new();
        index.register("doc-123", "folder-abc");
        index.unregister("doc-123");

        assert_eq!(index.get_folder("doc-123"), None);
    }

    #[test]
    fn gets_all_docs_in_folder() {
        let index = FolderIndex::new();
        index.register("doc-1", "folder-a");
        index.register("doc-2", "folder-a");
        index.register("doc-3", "folder-b");

        let mut docs = index.get_docs_in_folder("folder-a");
        docs.sort();

        assert_eq!(docs, vec!["doc-1", "doc-2"]);
    }
}
