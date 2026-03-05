use crate::server::Server;
use regex::RegexBuilder;
use serde_json::Value;
use std::sync::Arc;
use yrs::{GetString, ReadTxn, Transact};

/// Execute the `grep` tool: regex content search across Y.Docs.
pub async fn execute(server: &Arc<Server>, arguments: &Value) -> Result<String, String> {
    let pattern = arguments
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: pattern".to_string())?;

    let path_scope = arguments.get("path").and_then(|v| v.as_str());
    let output_mode = arguments
        .get("output_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("files_with_matches");
    let case_insensitive = arguments
        .get("-i")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let context_c = arguments
        .get("-C")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(0);
    let context_a = arguments
        .get("-A")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(context_c);
    let context_b = arguments
        .get("-B")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(context_c);
    let head_limit = arguments
        .get("head_limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(0);

    // Build regex
    let regex = RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|e| format!("Invalid regex pattern: {}", e))?;

    // Get all paths from resolver, sorted for deterministic output
    let mut all_paths = server.doc_resolver().all_paths();
    all_paths.sort();

    // Filter by path scope if provided
    if let Some(scope) = path_scope {
        let prefix = if scope.ends_with('/') {
            scope.to_string()
        } else {
            format!("{}/", scope)
        };
        all_paths.retain(|p| p.starts_with(&prefix) || p == scope);
    }

    let mut output_lines: Vec<String> = Vec::new();
    let mut file_count = 0;

    for path in &all_paths {
        // Apply head_limit for files_with_matches and count modes
        if head_limit > 0 && file_count >= head_limit && output_mode != "content" {
            break;
        }

        let doc_info = match server.doc_resolver().resolve_path(path) {
            Some(info) => info,
            None => continue,
        };

        let content = match read_doc_content(server, &doc_info.doc_id).await {
            Some(c) => c,
            None => continue,
        };

        let lines: Vec<&str> = content.lines().collect();
        let match_line_indices: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| regex.is_match(line))
            .map(|(i, _)| i)
            .collect();

        if match_line_indices.is_empty() {
            continue;
        }

        file_count += 1;

        // Apply head_limit for files_with_matches and count modes after incrementing
        if head_limit > 0 && file_count > head_limit && output_mode != "content" {
            break;
        }

        match output_mode {
            "files_with_matches" => {
                output_lines.push(path.clone());
            }
            "count" => {
                output_lines.push(format!("{}:{}", path, match_line_indices.len()));
            }
            "content" | _ => {
                // Build ranges with context, merging overlapping
                let ranges =
                    build_context_ranges(&match_line_indices, context_b, context_a, lines.len());

                let mut first_range = true;
                for range in &ranges {
                    // Add separator between non-adjacent groups (ripgrep convention)
                    if !first_range {
                        output_lines.push("--".to_string());
                    }
                    first_range = false;

                    for idx in range.start..=range.end {
                        let line_num = idx + 1; // 1-indexed
                        let is_match = match_line_indices.contains(&idx);
                        let separator = if is_match { ":" } else { "-" };
                        output_lines.push(format!(
                            "{}{}{}{}{}",
                            path, separator, line_num, separator, lines[idx]
                        ));
                    }
                }
            }
        }
    }

    // Apply head_limit for content mode (limit output lines)
    if head_limit > 0 && output_mode == "content" {
        output_lines.truncate(head_limit);
    }

    if output_lines.is_empty() {
        Ok("No matches found.".to_string())
    } else {
        Ok(output_lines.join("\n"))
    }
}

/// A range of line indices (inclusive) to display.
struct LineRange {
    start: usize,
    end: usize,
}

/// Build merged context ranges from match indices with before/after context.
fn build_context_ranges(
    match_indices: &[usize],
    before: usize,
    after: usize,
    total_lines: usize,
) -> Vec<LineRange> {
    if match_indices.is_empty() {
        return Vec::new();
    }

    let mut ranges: Vec<LineRange> = Vec::new();

    for &idx in match_indices {
        let start = idx.saturating_sub(before);
        let end = (idx + after).min(total_lines - 1);

        // Merge with previous range if overlapping or adjacent
        if let Some(last) = ranges.last_mut() {
            if start <= last.end + 1 {
                last.end = last.end.max(end);
                continue;
            }
        }

        ranges.push(LineRange { start, end });
    }

    ranges
}

/// Read Y.Doc text content for a given doc_id.
async fn read_doc_content(server: &Arc<Server>, doc_id: &str) -> Option<String> {
    server.ensure_doc_loaded(doc_id).await.ok()?;
    let doc_ref = server.docs().get(doc_id)?;
    let awareness = doc_ref.awareness();
    let guard = awareness.read().unwrap_or_else(|e| e.into_inner());
    let txn = guard.doc.transact();
    match txn.get_text("contents") {
        Some(text) => Some(text.get_string(&txn)),
        None => Some(String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use y_sweet_core::doc_resolver::DocumentResolver;
    use yrs::{Any, Doc, Map, Text, Transact, WriteTxn};

    // === Test Helpers ===

    const RELAY_ID: &str = "cb696037-0f72-4e93-8717-4e433129d789";
    const FOLDER0_UUID: &str = "aaaa0000-0000-0000-0000-000000000000";

    fn folder0_id() -> String {
        format!("{}-{}", RELAY_ID, FOLDER0_UUID)
    }

    fn set_folder_name(doc: &Doc, name: &str) {
        let mut txn = doc.transact_mut();
        let config = txn.get_or_insert_map("folder_config");
        config.insert(&mut txn, "name", Any::String(name.into()));
    }

    /// Create a folder Y.Doc with filemeta_v0 populated.
    /// entries: &[("/path.md", "uuid")]
    fn create_folder_doc(entries: &[(&str, &str)]) -> Doc {
        let doc = Doc::new();
        {
            let mut txn = doc.transact_mut();
            let filemeta = txn.get_or_insert_map("filemeta_v0");
            for (path, uuid) in entries {
                let mut map = HashMap::new();
                map.insert("id".to_string(), Any::String((*uuid).into()));
                map.insert("type".to_string(), Any::String("markdown".into()));
                map.insert("version".to_string(), Any::Number(0.0));
                filemeta.insert(&mut txn, *path, Any::Map(map.into()));
            }
        }
        doc
    }

    /// Create a content Y.Doc with TextRef("contents") populated.
    fn create_content_doc(content: &str) -> Doc {
        let doc = Doc::new();
        {
            let mut txn = doc.transact_mut();
            let text = txn.get_or_insert_text("contents");
            text.insert(&mut txn, 0, content);
        }
        doc
    }

    /// Helper to build a test server with docs and resolver populated.
    /// entries: &[("/path.md", "uuid", "content")]
    /// All docs go into folder0 (Lens/).
    async fn build_test_server(entries: &[(&str, &str, &str)]) -> Arc<Server> {
        let server = Server::new_for_test();

        // Create filemeta entries
        let filemeta_entries: Vec<(&str, &str)> = entries
            .iter()
            .map(|(path, uuid, _)| (*path, *uuid))
            .collect();
        let folder_doc = create_folder_doc(&filemeta_entries);
        set_folder_name(&folder_doc, "Lens");

        // Build resolver from folder doc
        let resolver = server.doc_resolver();
        resolver.update_folder_from_doc(&folder0_id(), &folder_doc);

        for (_, uuid, content) in entries {
            let doc_id = format!("{}-{}", RELAY_ID, uuid);
            let content_owned = content.to_string();
            let dwskv = y_sweet_core::doc_sync::DocWithSyncKv::new(&doc_id, None, || (), None)
                .await
                .expect("Failed to create test DocWithSyncKv");

            // Write content into the Y.Doc
            {
                let awareness = dwskv.awareness();
                let mut guard = awareness.write().unwrap();
                let mut txn = guard.doc.transact_mut();
                let text = txn.get_or_insert_text("contents");
                text.insert(&mut txn, 0, &content_owned);
            }

            server.docs().insert(doc_id, dwskv);
        }

        server
    }

    // === Grep Tests ===

    #[tokio::test]
    async fn grep_basic_match() {
        let server = build_test_server(&[(
            "/Photosynthesis.md",
            "uuid-photo",
            "# Photosynthesis\nPlants use sunlight.\nThis is important.",
        )])
        .await;

        let result = execute(
            &server,
            &json!({"pattern": "sunlight", "output_mode": "content"}),
        )
        .await
        .unwrap();

        assert!(
            result.contains("Lens/Photosynthesis.md:2:Plants use sunlight."),
            "Expected match with path:line:content, got: {}",
            result
        );
    }

    #[tokio::test]
    async fn grep_case_insensitive() {
        let server = build_test_server(&[(
            "/Test.md",
            "uuid-test",
            "Hello World\nhello world\nHELLO WORLD",
        )])
        .await;

        let result = execute(
            &server,
            &json!({"pattern": "hello", "-i": true, "output_mode": "content"}),
        )
        .await
        .unwrap();

        // Should match all three lines
        assert!(
            result.contains("Lens/Test.md:1:Hello World"),
            "Missing line 1 in: {}",
            result
        );
        assert!(
            result.contains("Lens/Test.md:2:hello world"),
            "Missing line 2 in: {}",
            result
        );
        assert!(
            result.contains("Lens/Test.md:3:HELLO WORLD"),
            "Missing line 3 in: {}",
            result
        );
    }

    #[tokio::test]
    async fn grep_files_with_matches_mode() {
        let server = build_test_server(&[
            ("/A.md", "uuid-a", "apple banana"),
            ("/B.md", "uuid-b", "cherry date"),
        ])
        .await;

        let result = execute(
            &server,
            &json!({"pattern": "apple", "output_mode": "files_with_matches"}),
        )
        .await
        .unwrap();

        assert_eq!(result.trim(), "Lens/A.md");
    }

    #[tokio::test]
    async fn grep_count_mode() {
        let server = build_test_server(&[(
            "/Multi.md",
            "uuid-multi",
            "apple\nbanana\napple pie\ncherry apple",
        )])
        .await;

        let result = execute(
            &server,
            &json!({"pattern": "apple", "output_mode": "count"}),
        )
        .await
        .unwrap();

        assert!(
            result.contains("Lens/Multi.md:3"),
            "Expected count of 3 matching lines, got: {}",
            result
        );
    }

    #[tokio::test]
    async fn grep_context_lines() {
        let server =
            build_test_server(&[("/Ctx.md", "uuid-ctx", "line1\nline2\nMATCH\nline4\nline5")])
                .await;

        let result = execute(
            &server,
            &json!({"pattern": "MATCH", "output_mode": "content", "-C": 1}),
        )
        .await
        .unwrap();

        assert!(
            result.contains("Lens/Ctx.md-2-line2"),
            "Missing before context: {}",
            result
        );
        assert!(
            result.contains("Lens/Ctx.md:3:MATCH"),
            "Missing match line: {}",
            result
        );
        assert!(
            result.contains("Lens/Ctx.md-4-line4"),
            "Missing after context: {}",
            result
        );
    }

    #[tokio::test]
    async fn grep_after_context() {
        let server = build_test_server(&[(
            "/After.md",
            "uuid-after",
            "before\nMATCH\nafter1\nafter2\nafter3",
        )])
        .await;

        let result = execute(
            &server,
            &json!({"pattern": "MATCH", "output_mode": "content", "-A": 2}),
        )
        .await
        .unwrap();

        assert!(
            result.contains("Lens/After.md:2:MATCH"),
            "Missing match: {}",
            result
        );
        assert!(
            result.contains("Lens/After.md-3-after1"),
            "Missing after1: {}",
            result
        );
        assert!(
            result.contains("Lens/After.md-4-after2"),
            "Missing after2: {}",
            result
        );
        assert!(
            !result.contains("after3"),
            "Should not include after3: {}",
            result
        );
    }

    #[tokio::test]
    async fn grep_before_context() {
        let server = build_test_server(&[(
            "/Before.md",
            "uuid-before",
            "before1\nbefore2\nMATCH\nafter",
        )])
        .await;

        let result = execute(
            &server,
            &json!({"pattern": "MATCH", "output_mode": "content", "-B": 1}),
        )
        .await
        .unwrap();

        assert!(
            result.contains("Lens/Before.md-2-before2"),
            "Missing before context: {}",
            result
        );
        assert!(
            result.contains("Lens/Before.md:3:MATCH"),
            "Missing match: {}",
            result
        );
        assert!(
            !result.contains("before1"),
            "Should not include before1: {}",
            result
        );
    }

    #[tokio::test]
    async fn grep_path_scope() {
        // Build server with two folders
        let server = Server::new_for_test();
        let folder0_uuid = "aaaa0000-0000-0000-0000-000000000000";
        let folder1_uuid = "bbbb0000-0000-0000-0000-000000000000";
        let folder0_doc_id = format!("{}-{}", RELAY_ID, folder0_uuid);
        let folder1_doc_id = format!("{}-{}", RELAY_ID, folder1_uuid);

        let folder0 = create_folder_doc(&[("/DocA.md", "uuid-a")]);
        set_folder_name(&folder0, "Lens");
        let folder1 = create_folder_doc(&[("/DocB.md", "uuid-b")]);
        set_folder_name(&folder1, "Lens Edu");

        let resolver = server.doc_resolver();
        resolver.update_folder_from_doc(&folder0_doc_id, &folder0);
        resolver.update_folder_from_doc(&folder1_doc_id, &folder1);

        for (uuid, content) in &[
            ("uuid-a", "target word here"),
            ("uuid-b", "target word there"),
        ] {
            let doc_id = format!("{}-{}", RELAY_ID, uuid);
            let content_owned = content.to_string();
            let dwskv = y_sweet_core::doc_sync::DocWithSyncKv::new(&doc_id, None, || (), None)
                .await
                .unwrap();
            {
                let awareness = dwskv.awareness();
                let mut guard = awareness.write().unwrap();
                let mut txn = guard.doc.transact_mut();
                let text = txn.get_or_insert_text("contents");
                text.insert(&mut txn, 0, &content_owned);
            }
            server.docs().insert(doc_id, dwskv);
        }

        // Search scoped to Lens/ only
        let result = execute(
            &server,
            &json!({"pattern": "target", "path": "Lens", "output_mode": "files_with_matches"}),
        )
        .await
        .unwrap();

        assert!(
            result.contains("Lens/DocA.md"),
            "Should include Lens doc: {}",
            result
        );
        assert!(
            !result.contains("Lens Edu"),
            "Should not include Lens Edu doc: {}",
            result
        );
    }

    #[tokio::test]
    async fn grep_no_matches() {
        let server = build_test_server(&[("/Doc.md", "uuid-doc", "nothing special here")]).await;

        let result = execute(
            &server,
            &json!({"pattern": "ZZZZNOTFOUND", "output_mode": "content"}),
        )
        .await
        .unwrap();

        assert_eq!(result, "No matches found.");
    }

    #[tokio::test]
    async fn grep_invalid_regex() {
        let server = build_test_server(&[("/Doc.md", "uuid-doc", "some content")]).await;

        let result = execute(
            &server,
            &json!({"pattern": "[invalid", "output_mode": "content"}),
        )
        .await;

        assert!(result.is_err(), "Invalid regex should return error");
        assert!(
            result.unwrap_err().contains("regex"),
            "Error should mention regex"
        );
    }

    #[tokio::test]
    async fn grep_head_limit() {
        let server = build_test_server(&[
            ("/A.md", "uuid-a", "target line"),
            ("/B.md", "uuid-b", "target line"),
            ("/C.md", "uuid-c", "target line"),
        ])
        .await;

        let result = execute(
            &server,
            &json!({"pattern": "target", "output_mode": "files_with_matches", "head_limit": 1}),
        )
        .await
        .unwrap();

        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(
            lines.len(),
            1,
            "head_limit=1 should return 1 file, got: {:?}",
            lines
        );
    }

    #[tokio::test]
    async fn grep_multiple_files() {
        let server = build_test_server(&[
            ("/Zebra.md", "uuid-z", "common word"),
            ("/Alpha.md", "uuid-a", "common word"),
        ])
        .await;

        let result = execute(
            &server,
            &json!({"pattern": "common", "output_mode": "files_with_matches"}),
        )
        .await
        .unwrap();

        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 2, "Should match 2 files");
        // Should be sorted alphabetically
        assert!(lines[0] < lines[1], "Results should be sorted: {:?}", lines);
    }
}
