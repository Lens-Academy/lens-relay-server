use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Span {
    Plain(String),
    Suggestion {
        deleted: String,
        inserted: String,
        author: String,
        timestamp: Option<u64>,
    },
}

/// Extract metadata (author, timestamp) from the inner content of a critic markup block.
/// The format is: `{"author":"AI","timestamp":1700000000000}@@actual content`
/// Only splits on `@@` if the prefix is valid JSON containing an "author" field.
/// Returns (content, author, timestamp).
fn extract_metadata(inner: &str) -> (&str, String, Option<u64>) {
    if let Some(at_pos) = inner.find("@@") {
        let candidate = &inner[..at_pos];
        if let Ok(obj) = serde_json::from_str::<Value>(candidate) {
            if let Some(author) = obj.get("author").and_then(|a| a.as_str()) {
                let content = &inner[at_pos + 2..];
                let timestamp = obj.get("timestamp").and_then(|t| t.as_u64());
                return (content, author.to_string(), timestamp);
            }
        }
    }
    (inner, "Unknown".to_string(), None)
}

pub fn parse(raw: &str) -> Vec<Span> {
    if raw.is_empty() {
        return vec![];
    }

    let mut spans: Vec<Span> = Vec::new();
    let mut plain = String::new();
    let bytes = raw.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut in_code_block = false;

    // We need to track code fences by checking line beginnings.
    // Pre-check: is position i at the start of a line beginning with ```?
    // We track code block state as we encounter newlines.

    // Helper: check if position is start of a ``` fence line
    let is_fence_start = |pos: usize| -> bool {
        // pos must be at start of line (pos == 0 or bytes[pos-1] == '\n')
        if pos + 2 >= len {
            return false;
        }
        bytes[pos] == b'`' && bytes[pos + 1] == b'`' && bytes[pos + 2] == b'`'
    };

    // We need to process line by line for code fence detection, but character by character
    // for delimiter detection. Let's do a single pass.

    // At the start of the string, check for fence
    if is_fence_start(0) {
        in_code_block = true;
    }

    while i < len {
        // After a newline, check if the next line starts a code fence
        if bytes[i] == b'\n' && i + 1 < len && is_fence_start(i + 1) {
            // Toggle code block state when we reach that fence line
            // But we add the newline first, then toggle when we process the fence
            plain.push('\n');
            i += 1;
            // Now we're at the start of a ``` line
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block {
            plain.push(bytes[i] as char);
            i += 1;
            continue;
        }

        // Check for {-- or {++ delimiters
        if i + 2 < len && bytes[i] == b'{' {
            if bytes[i + 1] == b'-' && bytes[i + 2] == b'-' {
                // Found {-- : look for --}
                if let Some(close) = find_closing(raw, i + 3, "--}") {
                    let del_inner = &raw[i + 3..close];
                    let (del_content, del_author, del_ts) = extract_metadata(del_inner);
                    let after_del = close + 3; // position after --}

                    // Check for adjacent {++...++}
                    if after_del + 2 < len
                        && bytes[after_del] == b'{'
                        && bytes[after_del + 1] == b'+'
                        && bytes[after_del + 2] == b'+'
                    {
                        if let Some(ins_close) = find_closing(raw, after_del + 3, "++}") {
                            let ins_inner = &raw[after_del + 3..ins_close];
                            let (ins_content, ins_author, ins_ts) = extract_metadata(ins_inner);

                            // Flush plain text
                            if !plain.is_empty() {
                                spans.push(Span::Plain(std::mem::take(&mut plain)));
                            }

                            // Use deletion's metadata, fall back to insertion's
                            let author = if del_author != "Unknown" {
                                del_author
                            } else {
                                ins_author
                            };
                            let timestamp = del_ts.or(ins_ts);

                            spans.push(Span::Suggestion {
                                deleted: del_content.to_string(),
                                inserted: ins_content.to_string(),
                                author,
                                timestamp,
                            });
                            i = ins_close + 3;
                            continue;
                        }
                    }

                    // Standalone deletion (no adjacent {++...++})
                    if !plain.is_empty() {
                        spans.push(Span::Plain(std::mem::take(&mut plain)));
                    }
                    spans.push(Span::Suggestion {
                        deleted: del_content.to_string(),
                        inserted: String::new(),
                        author: del_author,
                        timestamp: del_ts,
                    });
                    i = after_del;
                    continue;
                }
                // Unclosed {-- : treat as plain text
                plain.push('{');
                i += 1;
                continue;
            } else if bytes[i + 1] == b'+' && bytes[i + 2] == b'+' {
                // Found {++ without preceding {--: standalone insertion
                if let Some(close) = find_closing(raw, i + 3, "++}") {
                    let ins_inner = &raw[i + 3..close];
                    let (ins_content, ins_author, ins_ts) = extract_metadata(ins_inner);

                    if !plain.is_empty() {
                        spans.push(Span::Plain(std::mem::take(&mut plain)));
                    }
                    spans.push(Span::Suggestion {
                        deleted: String::new(),
                        inserted: ins_content.to_string(),
                        author: ins_author,
                        timestamp: ins_ts,
                    });
                    i = close + 3;
                    continue;
                }
                // Unclosed {++ : treat as plain text
                plain.push('{');
                i += 1;
                continue;
            }
        }

        plain.push(raw[i..].chars().next().unwrap());
        i += 1;
    }

    if !plain.is_empty() {
        spans.push(Span::Plain(plain));
    }

    spans
}

/// Find the closing delimiter starting from `start` position in `raw`.
/// For `--}`, we need to find the *first* occurrence that properly closes the block.
/// For deletion blocks, the content may contain `{++...++}` (test a15), so we need
/// to find the first `--}` that isn't inside a nested insertion marker.
fn find_closing(raw: &str, start: usize, delimiter: &str) -> Option<usize> {
    raw[start..].find(delimiter).map(|offset| start + offset)
}

pub fn accepted_view(spans: &[Span]) -> String {
    let mut out = String::new();
    for span in spans {
        match span {
            Span::Plain(text) => out.push_str(text),
            Span::Suggestion { inserted, .. } => out.push_str(inserted),
        }
    }
    out
}

pub fn base_view(spans: &[Span]) -> String {
    let mut out = String::new();
    for span in spans {
        match span {
            Span::Plain(text) => out.push_str(text),
            Span::Suggestion { deleted, .. } => out.push_str(deleted),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a01_plain_text_only() {
        let spans = parse("The quick brown fox.");
        assert_eq!(spans, vec![Span::Plain("The quick brown fox.".into())]);
        assert_eq!(accepted_view(&spans), "The quick brown fox.");
        assert_eq!(base_view(&spans), "The quick brown fox.");
    }

    #[test]
    fn a02_simple_substitution() {
        let spans = parse("The {--quick--}{++fast++} brown fox.");
        assert_eq!(spans, vec![
            Span::Plain("The ".into()),
            Span::Suggestion {
                deleted: "quick".into(), inserted: "fast".into(),
                author: "Unknown".into(), timestamp: None,
            },
            Span::Plain(" brown fox.".into()),
        ]);
        assert_eq!(accepted_view(&spans), "The fast brown fox.");
        assert_eq!(base_view(&spans), "The quick brown fox.");
    }

    #[test]
    fn a03_substitution_with_metadata() {
        let raw = r#"The {--{"author":"AI","timestamp":1700000000000}@@quick--}{++{"author":"AI","timestamp":1700000000000}@@fast++} brown fox."#;
        let spans = parse(raw);
        assert_eq!(spans, vec![
            Span::Plain("The ".into()),
            Span::Suggestion {
                deleted: "quick".into(), inserted: "fast".into(),
                author: "AI".into(), timestamp: Some(1700000000000),
            },
            Span::Plain(" brown fox.".into()),
        ]);
        assert_eq!(accepted_view(&spans), "The fast brown fox.");
        assert_eq!(base_view(&spans), "The quick brown fox.");
    }

    #[test]
    fn a04_standalone_deletion() {
        let spans = parse("Hello {--beautiful --}world.");
        assert_eq!(spans, vec![
            Span::Plain("Hello ".into()),
            Span::Suggestion {
                deleted: "beautiful ".into(), inserted: "".into(),
                author: "Unknown".into(), timestamp: None,
            },
            Span::Plain("world.".into()),
        ]);
        assert_eq!(accepted_view(&spans), "Hello world.");
        assert_eq!(base_view(&spans), "Hello beautiful world.");
    }

    #[test]
    fn a05_standalone_insertion() {
        let spans = parse("Hello {++beautiful ++}world.");
        assert_eq!(spans, vec![
            Span::Plain("Hello ".into()),
            Span::Suggestion {
                deleted: "".into(), inserted: "beautiful ".into(),
                author: "Unknown".into(), timestamp: None,
            },
            Span::Plain("world.".into()),
        ]);
        assert_eq!(accepted_view(&spans), "Hello beautiful world.");
        assert_eq!(base_view(&spans), "Hello world.");
    }

    #[test]
    fn a06_multiple_suggestions() {
        let raw = "The {--quick--}{++fast++} brown fox {--jumps--}{++leaps++} over.";
        let spans = parse(raw);
        assert_eq!(accepted_view(&spans), "The fast brown fox leaps over.");
        assert_eq!(base_view(&spans), "The quick brown fox jumps over.");
        assert_eq!(spans.len(), 5);
    }

    #[test]
    fn a07_multiline_suggestion() {
        let raw = "Line one.\n{--Line two.\nLine three.--}{++Replaced lines.++}\nLine four.";
        let spans = parse(raw);
        assert_eq!(accepted_view(&spans), "Line one.\nReplaced lines.\nLine four.");
        assert_eq!(base_view(&spans), "Line one.\nLine two.\nLine three.\nLine four.");
    }

    #[test]
    fn a08_unclosed_deletion_treated_as_plain_text() {
        let raw = "Hello {--world and goodbye.";
        let spans = parse(raw);
        assert_eq!(spans, vec![Span::Plain(raw.into())]);
    }

    #[test]
    fn a09_unclosed_insertion_treated_as_plain_text() {
        let raw = "Hello {++world and goodbye.";
        let spans = parse(raw);
        assert_eq!(spans, vec![Span::Plain(raw.into())]);
    }

    #[test]
    fn a10_fenced_code_block_not_parsed() {
        let raw = "Before.\n```\n{--this is code--}{++not markup++}\n```\nAfter.";
        let spans = parse(raw);
        assert_eq!(spans, vec![Span::Plain(raw.into())]);
        assert_eq!(accepted_view(&spans), raw);
    }

    #[test]
    fn a11_adjacent_suggestions_different_authors() {
        let raw = r#"{--{"author":"Human","timestamp":1700000000000}@@old1--}{++{"author":"Human","timestamp":1700000000000}@@new1++}{--{"author":"AI","timestamp":1700000060000}@@old2--}{++{"author":"AI","timestamp":1700000060000}@@new2++}"#;
        let spans = parse(raw);
        assert_eq!(accepted_view(&spans), "new1new2");
        assert_eq!(base_view(&spans), "old1old2");
        assert_eq!(spans.len(), 2);
    }

    #[test]
    fn a12_deletion_with_metadata() {
        let raw = r#"Keep {--{"author":"AI","timestamp":1700000000000}@@remove this--} text."#;
        let spans = parse(raw);
        assert_eq!(accepted_view(&spans), "Keep  text.");
        assert_eq!(base_view(&spans), "Keep remove this text.");
    }

    #[test]
    fn a13_empty_document() {
        let spans = parse("");
        assert_eq!(spans, vec![]);
        assert_eq!(accepted_view(&spans), "");
        assert_eq!(base_view(&spans), "");
    }

    #[test]
    fn a14_inline_code_delimiters_still_parsed() {
        let raw = "Use `{--old--}` or {--real--}{++actual++} markup.";
        let spans = parse(raw);
        assert_eq!(accepted_view(&spans), "Use `` or actual markup.");
        assert_eq!(base_view(&spans), "Use `old` or real markup.");
    }

    #[test]
    fn a15_delimiter_text_inside_suggestion_content() {
        let raw = "Use {--{++old syntax++}--}{++the new syntax++} here.";
        let spans = parse(raw);
        assert_eq!(accepted_view(&spans), "Use the new syntax here.");
        assert_eq!(base_view(&spans), "Use {++old syntax++} here.");
    }

    #[test]
    fn a16_extra_json_fields_in_metadata_ignored() {
        let raw = r#"The {--{"author":"AI","timestamp":1700000000000,"model":"claude-3"}@@quick--}{++{"author":"AI","timestamp":1700000000000,"model":"claude-3"}@@fast++} fox."#;
        let spans = parse(raw);
        assert_eq!(accepted_view(&spans), "The fast fox.");
        match &spans[1] {
            Span::Suggestion { author, timestamp, .. } => {
                assert_eq!(author, "AI");
                assert_eq!(*timestamp, Some(1700000000000));
            }
            _ => panic!("Expected suggestion at index 1"),
        }
    }

    #[test]
    fn a17_code_blocks_with_markup_between() {
        let raw = "```\n{--code1--}\n```\n{--real--}{++actual++}\n```\n{++code2++}\n```";
        let spans = parse(raw);
        assert_eq!(accepted_view(&spans), "```\n{--code1--}\n```\nactual\n```\n{++code2++}\n```");
        assert_eq!(base_view(&spans), "```\n{--code1--}\n```\nreal\n```\n{++code2++}\n```");
    }

    #[test]
    fn a18_entire_document_is_one_suggestion() {
        let raw = "{--old document content--}{++new document content++}";
        let spans = parse(raw);
        assert_eq!(spans.len(), 1);
        assert_eq!(accepted_view(&spans), "new document content");
        assert_eq!(base_view(&spans), "old document content");
    }

    #[test]
    fn a19_at_sign_in_content_not_metadata() {
        let raw = "The {--user@@example.com--}{++admin@@example.com++} address.";
        let spans = parse(raw);
        assert_eq!(accepted_view(&spans), "The admin@@example.com address.");
        assert_eq!(base_view(&spans), "The user@@example.com address.");
    }
}
