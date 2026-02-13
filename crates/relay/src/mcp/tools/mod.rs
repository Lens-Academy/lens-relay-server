pub mod critic_diff;
pub mod edit;
pub mod get_links;
pub mod glob;
pub mod grep;
pub mod read;

use crate::server::Server;
use serde_json::{json, Value};
use std::sync::Arc;

/// Return tool definitions for MCP tools/list response.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "read",
            "description": "Reads a document from the knowledge base. Returns content with line numbers (cat -n format). Supports partial reads via offset and limit. The response includes a [session: ...] value — pass this to the edit tool's session_id parameter when editing.",
            "inputSchema": {
                "type": "object",
                "required": ["file_path"],
                "additionalProperties": false,
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to the document (e.g. 'Lens/Photosynthesis.md')"
                    },
                    "offset": {
                        "type": "number",
                        "description": "The line number to start reading from. Only provide if the document is too large to read at once"
                    },
                    "limit": {
                        "type": "number",
                        "description": "The number of lines to read. Only provide if the document is too large to read at once."
                    }
                }
            }
        }),
        json!({
            "name": "glob",
            "description": "Fast document pattern matching. Returns matching document paths sorted alphabetically. Use to discover documents in the knowledge base.",
            "inputSchema": {
                "type": "object",
                "required": ["pattern"],
                "additionalProperties": false,
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "The glob pattern to match documents against (e.g. '**/*.md', 'Lens/*.md', 'Lens Edu/**')"
                    },
                    "path": {
                        "type": "string",
                        "description": "Folder to scope the search to (e.g. 'Lens', 'Lens Edu'). If not specified, searches all folders."
                    }
                }
            }
        }),
        json!({
            "name": "get_links",
            "description": "Get backlinks and forward links for a document. Returns document paths that link TO this document (backlinks) and paths this document links TO (forward links).",
            "inputSchema": {
                "type": "object",
                "required": ["file_path"],
                "additionalProperties": false,
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to the document (e.g. 'Lens/Photosynthesis.md')"
                    }
                }
            }
        }),
        json!({
            "name": "grep",
            "description": "Search document contents using regex patterns. Returns matching lines with context. Mirrors ripgrep output format.",
            "inputSchema": {
                "type": "object",
                "required": ["pattern"],
                "additionalProperties": false,
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "The regular expression pattern to search for in document contents"
                    },
                    "path": {
                        "type": "string",
                        "description": "Folder to scope the search to (e.g. 'Lens', 'Lens Edu'). If not specified, searches all folders."
                    },
                    "output_mode": {
                        "type": "string",
                        "enum": ["content", "files_with_matches", "count"],
                        "description": "Output mode: 'content' shows matching lines, 'files_with_matches' shows file paths (default), 'count' shows match counts."
                    },
                    "-i": {
                        "type": "boolean",
                        "description": "Case insensitive search"
                    },
                    "-C": {
                        "type": "number",
                        "description": "Number of lines to show before and after each match"
                    },
                    "-A": {
                        "type": "number",
                        "description": "Number of lines to show after each match"
                    },
                    "-B": {
                        "type": "number",
                        "description": "Number of lines to show before each match"
                    },
                    "head_limit": {
                        "type": "number",
                        "description": "Limit output to first N entries. In files_with_matches/count mode limits files, in content mode limits output lines."
                    }
                }
            }
        }),
        json!({
            "name": "edit",
            "description": "Edit a document by replacing old_string with new_string. The change is wrapped in CriticMarkup ({--old--}{++new++}) for human review. You must read the document first.",
            "inputSchema": {
                "type": "object",
                "required": ["file_path", "old_string", "new_string", "session_id"],
                "additionalProperties": false,
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to the document (e.g. 'Lens/Photosynthesis.md')"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "The exact text to find and replace. Must match exactly and be unique in the document."
                    },
                    "new_string": {
                        "type": "string",
                        "description": "The replacement text. Empty string for deletion."
                    },
                    "session_id": {
                        "type": "string",
                        "description": "The session value from the read tool's response. Required to verify the document was read before editing."
                    }
                }
            }
        }),
    ]
}

/// Dispatch a tool call to the correct handler and wrap result in MCP CallToolResult format.
pub fn dispatch_tool(server: &Arc<Server>, session_id: &str, name: &str, arguments: &Value) -> Value {
    // Lazy rebuild: if the resolver has no entries but docs exist, trigger a rebuild.
    // This handles the case where docs were created after server startup (e.g. local dev).
    if server.doc_resolver().all_paths().is_empty() {
        server.doc_resolver().rebuild(server.docs());
    }

    match name {
        "read" => match read::execute(server, session_id, arguments) {
            Ok(text) => tool_success(&text),
            Err(msg) => tool_error(&msg),
        },
        "glob" => match glob::execute(server, arguments) {
            Ok(text) => tool_success(&text),
            Err(msg) => tool_error(&msg),
        },
        "get_links" => match get_links::execute(server, arguments) {
            Ok(text) => tool_success(&text),
            Err(msg) => tool_error(&msg),
        },
        "grep" => match grep::execute(server, arguments) {
            Ok(text) => tool_success(&text),
            Err(msg) => tool_error(&msg),
        },
        "edit" => match edit::execute(server, arguments) {
            Ok(text) => tool_success(&text),
            Err(msg) => tool_error(&msg),
        },
        _ => tool_error(&format!("Unknown tool: {}", name)),
    }
}

/// Wrap successful tool output in MCP CallToolResult format.
fn tool_success(text: &str) -> Value {
    json!({
        "content": [
            {
                "type": "text",
                "text": text
            }
        ],
        "isError": false
    })
}

/// Wrap tool error in MCP CallToolResult format (tool-level error, not protocol error).
fn tool_error(msg: &str) -> Value {
    json!({
        "content": [
            {
                "type": "text",
                "text": msg
            }
        ],
        "isError": true
    })
}
