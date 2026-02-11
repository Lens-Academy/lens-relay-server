use crate::server::Server;
use serde_json::Value;
use std::sync::Arc;
use yrs::{GetString, ReadTxn, Transact};

/// Execute the `read` tool: read document content in cat -n format.
pub fn execute(server: &Arc<Server>, session_id: &str, arguments: &Value) -> Result<String, String> {
    let file_path = arguments
        .get("file_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: file_path".to_string())?;

    let offset = arguments
        .get("offset")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(0);

    let limit = arguments
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(2000);

    let doc_info = server
        .doc_resolver()
        .resolve_path(file_path)
        .ok_or_else(|| format!("Error: Document not found: {}", file_path))?;

    // Read Y.Doc content into an owned String, then drop all guards
    let content = {
        let doc_ref = server
            .docs()
            .get(&doc_info.doc_id)
            .ok_or_else(|| format!("Error: Document data not loaded: {}", file_path))?;
        let awareness = doc_ref.awareness();
        let guard = awareness.read().unwrap();
        let txn = guard.doc.transact();
        match txn.get_text("contents") {
            Some(text) => text.get_string(&txn),
            None => String::new(),
        }
        // guard, awareness, doc_ref all dropped here
    };

    // Record this doc as read in the session (for read-before-edit enforcement)
    if let Some(mut session) = server.mcp_sessions.get_session_mut(session_id) {
        session.read_docs.insert(doc_info.doc_id.clone());
    }

    Ok(format_cat_n(&content, offset, limit))
}

/// Format content as cat -n output with 6-char right-aligned line numbers.
fn format_cat_n(content: &str, offset: usize, limit: usize) -> String {
    // offset is 1-indexed (line number to start from), 0 means start from beginning
    let skip = if offset > 0 {
        offset.saturating_sub(1)
    } else {
        0
    };

    content
        .lines()
        .enumerate()
        .skip(skip)
        .take(limit)
        .map(|(i, line)| {
            let line_num = i + 1; // 1-indexed
            let truncated = if line.len() > 2000 {
                &line[..2000]
            } else {
                line
            };
            format!("{:>6}\t{}", line_num, truncated)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_cat_n_basic() {
        let content = "line one\nline two\nline three";
        let result = format_cat_n(content, 0, 2000);
        assert_eq!(
            result,
            "     1\tline one\n     2\tline two\n     3\tline three"
        );
    }

    #[test]
    fn format_cat_n_with_offset() {
        let content = "a\nb\nc\nd";
        let result = format_cat_n(content, 3, 2000);
        // offset=3 means start from line 3
        assert_eq!(result, "     3\tc\n     4\td");
    }

    #[test]
    fn format_cat_n_with_limit() {
        let content = "a\nb\nc\nd";
        let result = format_cat_n(content, 0, 2);
        assert_eq!(result, "     1\ta\n     2\tb");
    }

    #[test]
    fn format_cat_n_with_offset_and_limit() {
        let content = "a\nb\nc\nd\ne";
        let result = format_cat_n(content, 2, 2);
        // offset=2 means start from line 2, limit=2
        assert_eq!(result, "     2\tb\n     3\tc");
    }

    #[test]
    fn format_cat_n_empty_content() {
        let result = format_cat_n("", 0, 2000);
        // Empty string has no lines
        assert_eq!(result, "");
    }

    #[test]
    fn format_cat_n_truncates_long_lines() {
        let long_line = "x".repeat(3000);
        let result = format_cat_n(&long_line, 0, 2000);
        // Should truncate to 2000 chars
        let expected = format!("     1\t{}", "x".repeat(2000));
        assert_eq!(result, expected);
    }
}
