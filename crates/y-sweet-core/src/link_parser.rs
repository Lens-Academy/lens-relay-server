#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_simple_wikilink() {
        let result = extract_wikilinks("[[Note]]");
        assert_eq!(result, vec!["Note"]);
    }

    #[test]
    fn returns_empty_for_no_links() {
        let result = extract_wikilinks("plain text");
        assert_eq!(result, Vec::<String>::new());
    }

    #[test]
    fn extracts_multiple_wikilinks() {
        let result = extract_wikilinks("[[One]] and [[Two]]");
        assert_eq!(result, vec!["One", "Two"]);
    }

    #[test]
    fn strips_anchor_from_link() {
        let result = extract_wikilinks("[[Note#Section]]");
        assert_eq!(result, vec!["Note"]);
    }

    #[test]
    fn strips_alias_from_link() {
        let result = extract_wikilinks("[[Note|Display Text]]");
        assert_eq!(result, vec!["Note"]);
    }

    #[test]
    fn handles_anchor_and_alias() {
        let result = extract_wikilinks("[[Note#Section|Display]]");
        assert_eq!(result, vec!["Note"]);
    }

    #[test]
    fn ignores_empty_brackets() {
        let result = extract_wikilinks("[[]]");
        assert_eq!(result, Vec::<String>::new());
    }

    #[test]
    fn ignores_links_in_code_blocks() {
        let markdown = "```\n[[CodeLink]]\n```\nOutside [[RealLink]]";
        let result = extract_wikilinks(markdown);
        assert_eq!(result, vec!["RealLink"]);
    }

    #[test]
    fn ignores_links_in_inline_code() {
        let result = extract_wikilinks("See `[[Fake]]` but [[Real]]");
        assert_eq!(result, vec!["Real"]);
    }
}

/// Extract wikilink targets from markdown text.
/// Returns page names only (strips anchors and aliases).
/// Ignores links inside code blocks and inline code.
pub fn extract_wikilinks(markdown: &str) -> Vec<String> {
    // TODO: Implement
    vec![]
}
