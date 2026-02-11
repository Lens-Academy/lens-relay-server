use similar::{ChangeTag, TextDiff};

/// Produce a CriticMarkup diff between `old` and `new` using word-level diffing.
///
/// Adjacent change regions separated by at most 1 unchanged token (whitespace
/// between words) are merged into a single CriticMarkup block. This keeps the
/// output compact when several consecutive words change.
///
/// If `meta_prefix` is provided, it is prepended inside each CriticMarkup block
/// before the content, matching the frontend format:
///   `{--{"author":"AI","timestamp":1234}@@old text--}`
///
/// - Equal text is emitted verbatim.
/// - Deletions produce `{--old--}`.
/// - Insertions produce `{++new++}`.
/// - Substitutions (adjacent delete + insert) produce `{--old--}{++new++}`.
pub fn smart_critic_markup(old: &str, new: &str, meta_prefix: Option<&str>) -> String {
    let diff = TextDiff::from_words(old, new);
    let changes: Vec<_> = diff.iter_all_changes().collect();

    let mut result = String::new();
    let mut i = 0;

    while i < changes.len() {
        let tag = changes[i].tag();

        if tag == ChangeTag::Equal {
            // Check whether this equal token is just whitespace separating two
            // change regions.  If so, absorb it into the surrounding change
            // rather than emitting it as-is, so we get one merged CM block.
            let eq_val = changes[i].value();
            let is_trivial_separator = eq_val.chars().all(|c| c == ' ');

            if is_trivial_separator && i > 0 && i + 1 < changes.len() {
                let prev_is_change = changes[i - 1].tag() != ChangeTag::Equal;
                let next_is_change = changes[i + 1].tag() != ChangeTag::Equal;

                if prev_is_change && next_is_change {
                    // We need to retroactively merge.  Rewind: pop the last
                    // emitted CM closing tag(s) so we can extend them.
                    // Actually, it's easier to use a two-pass approach.
                    // Let's fall through to a grouped approach below.
                    //
                    // Instead of doing complex rewind, let's just emit the
                    // equal text as-is for now and do grouping in a second pass.
                }
            }

            result.push_str(changes[i].value());
            i += 1;
            continue;
        }

        // We have a change region.  Collect all contiguous change tokens,
        // merging across single-space equal tokens.
        let mut del_parts = String::new();
        let mut ins_parts = String::new();

        loop {
            if i >= changes.len() {
                break;
            }

            match changes[i].tag() {
                ChangeTag::Delete => {
                    del_parts.push_str(changes[i].value());
                    i += 1;
                }
                ChangeTag::Insert => {
                    ins_parts.push_str(changes[i].value());
                    i += 1;
                }
                ChangeTag::Equal => {
                    // Check if this equal token is a trivial separator (spaces
                    // only) between two change regions.  If so, absorb it.
                    let eq_val = changes[i].value();
                    let is_trivial = eq_val.chars().all(|c| c == ' ');

                    if is_trivial && i + 1 < changes.len() && changes[i + 1].tag() != ChangeTag::Equal {
                        // Absorb this separator into both sides of the change.
                        del_parts.push_str(eq_val);
                        ins_parts.push_str(eq_val);
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
        }

        // Emit the accumulated change region.
        let prefix = meta_prefix.unwrap_or("");
        if !del_parts.is_empty() {
            result.push_str(&format!("{{--{}{}--}}", prefix, del_parts));
        }
        if !ins_parts.is_empty() {
            result.push_str(&format!("{{++{}{}++}}", prefix, ins_parts));
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // Core cases
    #[test]
    fn single_word_change_at_start() {
        assert_eq!(
            smart_critic_markup("You can explore", "People can explore", None),
            "{--You--}{++People++} can explore"
        );
    }

    #[test]
    fn single_word_change_at_end() {
        assert_eq!(
            smart_critic_markup("say hello", "say goodbye", None),
            "say {--hello--}{++goodbye++}"
        );
    }

    #[test]
    fn single_word_change_in_middle() {
        assert_eq!(
            smart_critic_markup("the big house", "the small house", None),
            "the {--big--}{++small++} house"
        );
    }

    // Merge behavior
    #[test]
    fn merge_adjacent_changes() {
        assert_eq!(
            smart_critic_markup("The quick brown fox", "A slow red fox", None),
            "{--The quick brown--}{++A slow red++} fox"
        );
    }

    #[test]
    fn separate_distant_changes() {
        assert_eq!(
            smart_critic_markup("I love cats and dogs", "I hate cats and puppies", None),
            "I {--love--}{++hate++} cats and {--dogs--}{++puppies++}"
        );
    }

    // Insertions & deletions
    #[test]
    fn pure_insertion() {
        assert_eq!(
            smart_critic_markup("hello world", "hello beautiful world", None),
            "hello {++beautiful ++}world"
        );
    }

    #[test]
    fn pure_deletion() {
        assert_eq!(
            smart_critic_markup("hello beautiful world", "hello world", None),
            "hello {--beautiful --}world"
        );
    }

    #[test]
    fn delete_everything() {
        assert_eq!(
            smart_critic_markup("delete everything", "", None),
            "{--delete everything--}"
        );
    }

    #[test]
    fn insert_into_empty() {
        assert_eq!(
            smart_critic_markup("", "brand new text", None),
            "{++brand new text++}"
        );
    }

    // Edge cases
    #[test]
    fn identical_strings() {
        assert_eq!(
            smart_critic_markup("same text", "same text", None),
            "same text"
        );
    }

    #[test]
    fn completely_different() {
        assert_eq!(
            smart_critic_markup("abc", "xyz", None),
            "{--abc--}{++xyz++}"
        );
    }

    #[test]
    fn multiline_change() {
        assert_eq!(
            smart_critic_markup("line one\nline two", "line one\nline three", None),
            "line one\nline {--two--}{++three++}"
        );
    }

    // Metadata
    #[test]
    fn with_metadata_prefix() {
        let meta = r#"{"author":"AI","timestamp":1707600000}@@"#;
        assert_eq!(
            smart_critic_markup("say hello", "say goodbye", Some(meta)),
            r#"say {--{"author":"AI","timestamp":1707600000}@@hello--}{++{"author":"AI","timestamp":1707600000}@@goodbye++}"#
        );
    }

    #[test]
    fn metadata_on_pure_insertion() {
        let meta = r#"{"author":"AI","timestamp":1707600000}@@"#;
        assert_eq!(
            smart_critic_markup("hello world", "hello beautiful world", Some(meta)),
            r#"hello {++{"author":"AI","timestamp":1707600000}@@beautiful ++}world"#
        );
    }

    #[test]
    fn metadata_on_pure_deletion() {
        let meta = r#"{"author":"AI","timestamp":1707600000}@@"#;
        assert_eq!(
            smart_critic_markup("hello beautiful world", "hello world", Some(meta)),
            r#"hello {--{"author":"AI","timestamp":1707600000}@@beautiful --}world"#
        );
    }
}
