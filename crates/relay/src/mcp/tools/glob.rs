use crate::server::Server;
use glob_match::glob_match;
use serde_json::Value;
use std::sync::Arc;

/// Execute the `glob` tool: pattern match against document paths.
pub fn execute(server: &Arc<Server>, arguments: &Value) -> Result<String, String> {
    let pattern = arguments
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: pattern".to_string())?;

    let path_scope = arguments.get("path").and_then(|v| v.as_str());

    let all_paths = server.doc_resolver().all_paths();

    let mut matched: Vec<String> = all_paths
        .into_iter()
        .filter(|p| {
            // If a path scope is given, only include paths under that folder
            if let Some(scope) = path_scope {
                let prefix = if scope.ends_with('/') {
                    scope.to_string()
                } else {
                    format!("{}/", scope)
                };
                if !p.starts_with(&prefix) {
                    return false;
                }
            }
            glob_match(pattern, p)
        })
        .collect();

    matched.sort();

    if matched.is_empty() {
        Ok("No matches found.".to_string())
    } else {
        Ok(matched.join("\n"))
    }
}
