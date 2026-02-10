pub mod get_links;
pub mod glob;
pub mod read;

use crate::server::Server;
use serde_json::{json, Value};
use std::sync::Arc;

/// Return tool definitions for MCP tools/list response.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "read",
            "description": "Reads a document from the knowledge base. Returns content with line numbers (cat -n format). Supports partial reads via offset and limit.",
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
    ]
}

/// Dispatch a tool call to the correct handler and wrap result in MCP CallToolResult format.
pub fn dispatch_tool(server: &Arc<Server>, name: &str, arguments: &Value) -> Value {
    match name {
        "read" => match read::execute(server, arguments) {
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
