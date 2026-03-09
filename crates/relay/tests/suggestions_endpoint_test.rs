use y_sweet_core::critic_scanner::scan_suggestions;

#[test]
fn test_endpoint_response_shape() {
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
    let text = "Hello {++plain text++} end";
    let suggestions = scan_suggestions(text);
    let json = serde_json::to_value(&suggestions[0]).unwrap();
    assert!(json.get("author").is_some(), "author field should be present");
    assert!(json.get("timestamp").is_some(), "timestamp field should be present");
    assert!(json["author"].is_null());
    assert!(json["timestamp"].is_null());
}
