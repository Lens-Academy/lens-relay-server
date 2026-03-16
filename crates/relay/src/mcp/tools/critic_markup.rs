use serde_json::Value;
use similar::{ChangeTag, TextDiff};

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

const CRITIC_DELIMITERS: &[&str] = &[
    "{--", "--}", "{++", "++}", "{~~", "~~}", "{==", "==}", "{>>", "<<}",
];

/// Check if text contains CriticMarkup delimiters. Returns an error message if found.
pub fn reject_if_contains_markup(text: &str, param_name: &str) -> Result<(), String> {
    for delim in CRITIC_DELIMITERS {
        if text.contains(delim) {
            return Err(format!(
                "Error: {} contains CriticMarkup syntax '{}'. Do not include CriticMarkup in your input — the system handles suggestion wrapping automatically.",
                param_name, delim
            ));
        }
    }
    Ok(())
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

    // Helper: check if position is start of a ``` fence line
    let is_fence_start = |pos: usize| -> bool {
        if pos + 2 >= len {
            return false;
        }
        bytes[pos] == b'`' && bytes[pos + 1] == b'`' && bytes[pos + 2] == b'`'
    };

    // At the start of the string, check for fence
    if is_fence_start(0) {
        in_code_block = true;
    }

    while i < len {
        // After a newline, check if the next line starts a code fence
        if bytes[i] == b'\n' && i + 1 < len && is_fence_start(i + 1) {
            plain.push('\n');
            i += 1;
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
                if let Some(close) = find_closing(raw, i + 3, "--}") {
                    let del_inner = &raw[i + 3..close];
                    let (del_content, del_author, del_ts) = extract_metadata(del_inner);
                    let after_del = close + 3;

                    // Check for adjacent {++...++}
                    if after_del + 2 < len
                        && bytes[after_del] == b'{'
                        && bytes[after_del + 1] == b'+'
                        && bytes[after_del + 2] == b'+'
                    {
                        if let Some(ins_close) = find_closing(raw, after_del + 3, "++}") {
                            let ins_inner = &raw[after_del + 3..ins_close];
                            let (ins_content, ins_author, ins_ts) = extract_metadata(ins_inner);

                            if !plain.is_empty() {
                                spans.push(Span::Plain(std::mem::take(&mut plain)));
                            }

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

                    // Standalone deletion
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
                plain.push('{');
                i += 1;
                continue;
            } else if bytes[i + 1] == b'+' && bytes[i + 2] == b'+' {
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

#[derive(Debug, Clone, PartialEq)]
pub struct CoveredSpan {
    pub span_index: usize,
    /// Byte offset within this span's accepted-view contribution where coverage starts
    pub start_within: usize,
    /// Byte length of coverage within this span's accepted-view contribution
    pub len_within: usize,
}

/// Given a byte offset and length in the accepted view, return which spans are covered.
pub fn spans_covering_accepted_range(
    spans: &[Span],
    accepted_offset: usize,
    accepted_len: usize,
) -> Vec<CoveredSpan> {
    if accepted_len == 0 {
        return vec![];
    }

    let range_end = accepted_offset + accepted_len;
    let mut result = Vec::new();
    let mut cursor = 0usize;

    for (idx, span) in spans.iter().enumerate() {
        let contribution = match span {
            Span::Plain(text) => text.len(),
            Span::Suggestion { inserted, .. } => inserted.len(),
        };

        let span_end = cursor + contribution;

        if contribution > 0 && cursor < range_end && span_end > accepted_offset {
            let overlap_start = accepted_offset.max(cursor);
            let overlap_end = range_end.min(span_end);
            let start_within = overlap_start - cursor;
            let len_within = overlap_end - overlap_start;
            result.push(CoveredSpan {
                span_index: idx,
                start_within,
                len_within,
            });
        }

        cursor = span_end;
    }

    result
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

// --- Smart Merge ---

/// Compute the raw byte length of a span in the raw document.
fn span_raw_length(raw: &str, span: &Span, raw_start: usize) -> usize {
    match span {
        Span::Plain(text) => text.len(),
        Span::Suggestion { .. } => {
            let bytes = raw.as_bytes();
            if raw_start >= raw.len() {
                return 0;
            }
            if raw_start + 2 < raw.len()
                && bytes[raw_start] == b'{'
                && bytes[raw_start + 1] == b'-'
                && bytes[raw_start + 2] == b'-'
            {
                if let Some(del_close) = find_closing(raw, raw_start + 3, "--}") {
                    let after_del = del_close + 3;
                    if after_del + 2 < raw.len()
                        && bytes[after_del] == b'{'
                        && bytes[after_del + 1] == b'+'
                        && bytes[after_del + 2] == b'+'
                    {
                        if let Some(ins_close) = find_closing(raw, after_del + 3, "++}") {
                            return ins_close + 3 - raw_start;
                        }
                    }
                    return after_del - raw_start;
                }
            } else if raw_start + 2 < raw.len()
                && bytes[raw_start] == b'{'
                && bytes[raw_start + 1] == b'+'
                && bytes[raw_start + 2] == b'+'
            {
                if let Some(close) = find_closing(raw, raw_start + 3, "++}") {
                    return close + 3 - raw_start;
                }
            }
            0
        }
    }
}

/// Compute raw byte start positions for each span.
fn compute_raw_positions(raw: &str, spans: &[Span]) -> Vec<usize> {
    let mut positions = Vec::with_capacity(spans.len());
    let mut pos = 0usize;
    for span in spans {
        positions.push(pos);
        pos += span_raw_length(raw, span, pos);
    }
    positions
}

#[derive(Debug, Clone, PartialEq)]
pub struct MergeResult {
    pub raw_offset: usize,
    pub raw_len: usize,
    pub replacement: String,
}

/// A segment in the covered range, tracking accepted/base/raw text.
struct MergeSegment {
    accepted_text: String,
    base_text: String,
    raw_text: String,
    is_suggestion: bool,
}

pub fn merge_edit(
    raw: &str,
    old_string: &str,
    new_string: &str,
    author: &str,
    timestamp: u64,
) -> Result<MergeResult, String> {
    let spans = parse(raw);
    let accepted = accepted_view(&spans);

    // Find old_string in accepted view (uniqueness check)
    let matches: Vec<usize> = accepted.match_indices(old_string).map(|(i, _)| i).collect();
    if matches.is_empty() {
        return Err("old_string not found in accepted view".into());
    }
    if matches.len() > 1 {
        return Err(format!(
            "old_string not unique ({} occurrences)",
            matches.len()
        ));
    }
    let match_offset = matches[0];

    // No-op
    if old_string == new_string {
        return Ok(MergeResult {
            raw_offset: 0,
            raw_len: 0,
            replacement: String::new(),
        });
    }

    let raw_positions = compute_raw_positions(raw, &spans);
    let covered = spans_covering_accepted_range(&spans, match_offset, old_string.len());

    if covered.is_empty() {
        return Err("No spans cover the matched range".into());
    }

    let first_covered = &covered[0];
    let last_covered = &covered[covered.len() - 1];
    let first_span_idx = first_covered.span_index;
    let last_span_idx = last_covered.span_index;

    // For Suggestion spans at boundaries: include the full span (even if partially covered).
    // For Plain spans at boundaries: only include the covered portion.
    let raw_start = match &spans[first_span_idx] {
        Span::Plain(_) => raw_positions[first_span_idx] + first_covered.start_within,
        _ => raw_positions[first_span_idx],
    };

    let last_raw_pos = raw_positions[last_span_idx];
    let last_raw_len = span_raw_length(raw, &spans[last_span_idx], last_raw_pos);
    let raw_end = match &spans[last_span_idx] {
        Span::Plain(text) => {
            if first_span_idx == last_span_idx {
                raw_positions[last_span_idx] + first_covered.start_within + first_covered.len_within
            } else {
                raw_positions[last_span_idx] + last_covered.start_within + last_covered.len_within
            }
        }
        _ => last_raw_pos + last_raw_len,
    };

    let raw_offset = raw_start;
    let raw_len = raw_end - raw_start;

    // Expand old/new strings to include the full accepted text of partially covered suggestions.
    let mut prefix_extra = String::new();
    let mut suffix_extra = String::new();

    if let Span::Suggestion { inserted, .. } = &spans[first_span_idx] {
        if first_covered.start_within > 0 {
            prefix_extra = inserted[..first_covered.start_within].to_string();
        }
    }

    if let Span::Suggestion { inserted, .. } = &spans[last_span_idx] {
        let cov_end = last_covered.start_within + last_covered.len_within;
        if cov_end < inserted.len() {
            suffix_extra = inserted[cov_end..].to_string();
        }
    }

    let expanded_old = format!("{}{}{}", prefix_extra, old_string, suffix_extra);
    let expanded_new = format!("{}{}{}", prefix_extra, new_string, suffix_extra);

    // Build segments for the covered range.
    let mut segments: Vec<MergeSegment> = Vec::new();

    for span_idx in first_span_idx..=last_span_idx {
        let span = &spans[span_idx];
        let span_raw_pos = raw_positions[span_idx];
        let span_raw_len = span_raw_length(raw, span, span_raw_pos);
        let span_raw = &raw[span_raw_pos..span_raw_pos + span_raw_len];

        match span {
            Span::Plain(text) => {
                let start = if span_idx == first_span_idx {
                    first_covered.start_within
                } else {
                    0
                };
                let end = if span_idx == last_span_idx {
                    if span_idx == first_span_idx {
                        first_covered.start_within + first_covered.len_within
                    } else {
                        last_covered.start_within + last_covered.len_within
                    }
                } else {
                    text.len()
                };

                let slice = &text[start..end];
                segments.push(MergeSegment {
                    accepted_text: slice.to_string(),
                    base_text: slice.to_string(),
                    raw_text: slice.to_string(),
                    is_suggestion: false,
                });
            }
            Span::Suggestion {
                deleted, inserted, ..
            } => {
                segments.push(MergeSegment {
                    accepted_text: inserted.clone(),
                    base_text: deleted.clone(),
                    raw_text: span_raw.to_string(),
                    is_suggestion: true,
                });
            }
        }
    }

    // Word-level diff with single-space absorption on expanded strings.
    let meta = format!(r#"{{"author":"{}","timestamp":{}}}@@"#, author, timestamp);
    let diff = TextDiff::from_words(&expanded_old, &expanded_new);
    let changes: Vec<_> = diff.iter_all_changes().collect();

    // Build diff regions: each region has old_start, old_len, new_text, is_change.
    struct DiffRegion {
        old_start: usize,
        old_len: usize,
        new_text: String,
        is_change: bool,
    }

    let mut regions: Vec<DiffRegion> = Vec::new();
    let mut ci = 0;
    let mut old_cursor = 0usize;

    while ci < changes.len() {
        if changes[ci].tag() == ChangeTag::Equal {
            let eq_val = changes[ci].value();
            let is_trivial = eq_val.chars().all(|c| c == ' ');

            if is_trivial && !regions.is_empty() && ci + 1 < changes.len() {
                let prev_is_change = matches!(regions.last(), Some(r) if r.is_change);
                let next_is_change = changes[ci + 1].tag() != ChangeTag::Equal;
                if prev_is_change && next_is_change {
                    if let Some(last) = regions.last_mut() {
                        last.old_len += eq_val.len();
                        last.new_text.push_str(eq_val);
                    }
                    old_cursor += eq_val.len();
                    ci += 1;
                    continue;
                }
            }

            regions.push(DiffRegion {
                old_start: old_cursor,
                old_len: eq_val.len(),
                new_text: eq_val.to_string(),
                is_change: false,
            });
            old_cursor += eq_val.len();
            ci += 1;
        } else {
            let mut del_text = String::new();
            let mut ins_text = String::new();

            loop {
                if ci >= changes.len() {
                    break;
                }
                match changes[ci].tag() {
                    ChangeTag::Delete => {
                        del_text.push_str(changes[ci].value());
                        ci += 1;
                    }
                    ChangeTag::Insert => {
                        ins_text.push_str(changes[ci].value());
                        ci += 1;
                    }
                    ChangeTag::Equal => {
                        let eq_val = changes[ci].value();
                        let is_trivial = eq_val.chars().all(|c| c == ' ');
                        if is_trivial
                            && ci + 1 < changes.len()
                            && changes[ci + 1].tag() != ChangeTag::Equal
                        {
                            del_text.push_str(eq_val);
                            ins_text.push_str(eq_val);
                            ci += 1;
                        } else {
                            break;
                        }
                    }
                }
            }

            regions.push(DiffRegion {
                old_start: old_cursor,
                old_len: del_text.len(),
                new_text: ins_text,
                is_change: true,
            });
            old_cursor += del_text.len();
        }
    }

    // Consolidate regions: merge change regions with adjacent equal regions
    // that fall within a touched suggestion. This ensures suggestions are
    // handled atomically.
    //
    // Strategy: for each suggestion segment that's touched by a change,
    // expand the change to cover the entire suggestion. This may merge
    // adjacent equal regions into the change.

    // Compute segment positions
    let mut seg_positions: Vec<(usize, usize)> = Vec::new(); // (start, end) in expanded_old
    {
        let mut pos = 0usize;
        for seg in &segments {
            let end = pos + seg.accepted_text.len();
            seg_positions.push((pos, end));
            pos = end;
        }
    }

    // Find touched suggestion ranges
    let mut touched_ranges: Vec<(usize, usize)> = Vec::new();
    for (si, seg) in segments.iter().enumerate() {
        if !seg.is_suggestion {
            continue;
        }
        let (s_start, s_end) = seg_positions[si];
        let is_touched = regions
            .iter()
            .any(|r| r.is_change && r.old_start < s_end && (r.old_start + r.old_len) > s_start);
        if is_touched {
            touched_ranges.push((s_start, s_end));
        }
    }

    // Merge regions: consolidate any change region + adjacent equal regions
    // that overlap a touched suggestion into one change region.
    // We also merge adjacent change regions (which can happen after expansion).
    let mut merged_regions: Vec<DiffRegion> = Vec::new();

    for r in &regions {
        let r_start = r.old_start;
        let r_end = r.old_start + r.old_len;

        // Check if this region (equal or change) is within a touched suggestion
        let in_touched = touched_ranges
            .iter()
            .any(|&(ts, te)| r_start < te && r_end > ts);

        if r.is_change || in_touched {
            // This should be part of a change region.
            // Try to merge with the last merged region if it's also a change.
            if let Some(last) = merged_regions.last_mut() {
                if !last.is_change {
                    // Previous was equal, can't merge
                    merged_regions.push(DiffRegion {
                        old_start: r_start,
                        old_len: r.old_len,
                        new_text: r.new_text.clone(),
                        is_change: true,
                    });
                } else if last.old_start + last.old_len == r_start {
                    // Adjacent to previous change region, merge
                    last.old_len += r.old_len;
                    last.new_text.push_str(&r.new_text);
                } else {
                    merged_regions.push(DiffRegion {
                        old_start: r_start,
                        old_len: r.old_len,
                        new_text: r.new_text.clone(),
                        is_change: true,
                    });
                }
            } else {
                merged_regions.push(DiffRegion {
                    old_start: r_start,
                    old_len: r.old_len,
                    new_text: r.new_text.clone(),
                    is_change: true,
                });
            }
        } else {
            // Pure equal region not in a touched suggestion
            merged_regions.push(DiffRegion {
                old_start: r_start,
                old_len: r.old_len,
                new_text: r.new_text.clone(),
                is_change: false,
            });
        }
    }

    // Now walk merged regions and build replacement.
    let mut replacement = String::new();
    let mut emitted_suggestions: std::collections::HashSet<usize> =
        std::collections::HashSet::new();

    for r in &merged_regions {
        if !r.is_change {
            // Equal region: emit raw text from segments
            replacement.push_str(&collect_raw_for_equal(
                &segments,
                &seg_positions,
                r.old_start,
                r.old_len,
                &mut emitted_suggestions,
            ));
        } else {
            // Change region: collect base text, emit CriticMarkup
            let base = collect_base_for_change(&segments, &seg_positions, r.old_start, r.old_len);
            if !base.is_empty() {
                replacement.push_str(&format!("{{--{}{}--}}", meta, base));
            }
            if !r.new_text.is_empty() {
                replacement.push_str(&format!("{{++{}{}++}}", meta, r.new_text));
            }
        }
    }

    Ok(MergeResult {
        raw_offset,
        raw_len,
        replacement,
    })
}

/// For an equal diff region, emit the original raw text from segments.
/// `emitted_suggestions` tracks which suggestion segments have already been
/// emitted (by index) to avoid double-emitting when multiple small equal
/// regions cover parts of the same untouched suggestion.
fn collect_raw_for_equal(
    segments: &[MergeSegment],
    seg_positions: &[(usize, usize)],
    offset: usize,
    len: usize,
    emitted_suggestions: &mut std::collections::HashSet<usize>,
) -> String {
    if len == 0 {
        return String::new();
    }
    let mut result = String::new();
    let end = offset + len;

    for (si, seg) in segments.iter().enumerate() {
        let (seg_start, seg_end) = seg_positions[si];
        if seg_start >= end {
            break;
        }
        if seg_end <= offset {
            continue;
        }

        let ov_start = offset.max(seg_start) - seg_start;
        let ov_end = end.min(seg_end) - seg_start;

        if !seg.is_suggestion {
            result.push_str(&seg.raw_text[ov_start..ov_end]);
        } else if emitted_suggestions.contains(&si) {
            // Already emitted this suggestion's raw markup from a previous
            // equal region — skip to avoid double-emit.
        } else {
            // Untouched suggestion: emit full raw markup on first encounter.
            // Multiple word-level equal regions may cover parts of this
            // suggestion; we emit it once and skip subsequent overlaps.
            result.push_str(&seg.raw_text);
            emitted_suggestions.insert(si);
        }
    }

    result
}

/// Truncate text to "first three words ... last three words" if >10 words.
fn truncate_side(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= 10 {
        return text.to_string();
    }
    format!(
        "{} ... {}",
        words[..3].join(" "),
        words[words.len() - 3..].join(" ")
    )
}

/// Format a timestamp as relative time.
fn format_relative_time(timestamp_ms: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let diff_secs = now.saturating_sub(timestamp_ms) / 1000;
    if diff_secs < 60 {
        format!("{}s ago", diff_secs)
    } else if diff_secs < 3600 {
        format!("{}m ago", diff_secs / 60)
    } else if diff_secs < 86400 {
        format!("{}h ago", diff_secs / 3600)
    } else if diff_secs < 604800 {
        format!("{}d ago", diff_secs / 86400)
    } else {
        format!("{}w ago", diff_secs / 604800)
    }
}

/// Render a CriticMarkup suggestion span as compact markup without metadata,
/// applying truncation to long sides.
fn render_suggestion_compact(span: &Span) -> String {
    match span {
        Span::Plain(_) => String::new(),
        Span::Suggestion {
            deleted, inserted, ..
        } => {
            let del_part = if deleted.is_empty() {
                String::new()
            } else {
                format!("{{--{}--}}", truncate_side(deleted))
            };
            let ins_part = if inserted.is_empty() {
                String::new()
            } else {
                format!("{{++{}++}}", truncate_side(inserted))
            };
            format!("{}{}", del_part, ins_part)
        }
    }
}

/// Render the pending suggestions footer for the read tool.
/// Returns None if there are no suggestions.
pub fn render_pending_summary(spans: &[Span], accepted_content: &str) -> Option<String> {
    // Check if any Suggestion spans exist
    let has_suggestions = spans.iter().any(|s| matches!(s, Span::Suggestion { .. }));
    if !has_suggestions {
        return None;
    }

    // Build a list of (line_number, compact_markup, author, timestamp) for each suggestion.
    // We walk spans and track line position in the accepted view.
    let mut line_number = 1usize; // 1-based
    let mut entries: Vec<(usize, String, String, Option<u64>)> = Vec::new();

    for span in spans {
        match span {
            Span::Plain(text) => {
                for ch in text.chars() {
                    if ch == '\n' {
                        line_number += 1;
                    }
                }
            }
            Span::Suggestion {
                inserted,
                author,
                timestamp,
                ..
            } => {
                let compact = render_suggestion_compact(span);
                entries.push((line_number, compact, author.clone(), *timestamp));
                // Advance line counter by newlines in inserted text (accepted view contribution)
                for ch in inserted.chars() {
                    if ch == '\n' {
                        line_number += 1;
                    }
                }
            }
        }
    }

    let _ = accepted_content; // used for context, not needed for line counting above

    let mut out = String::from("[Pending suggestions]\n");
    for (line, markup, author, ts) in entries {
        let ts_str = if let Some(ms) = ts {
            format!(" {}", format_relative_time(ms))
        } else {
            String::new()
        };
        out.push_str(&format!(
            "  L{}: {} ({}){}\\n",
            line, markup, author, ts_str
        ));
    }

    Some(out)
}

/// For a change diff region, collect base text from segments.
fn collect_base_for_change(
    segments: &[MergeSegment],
    seg_positions: &[(usize, usize)],
    offset: usize,
    len: usize,
) -> String {
    if len == 0 {
        return String::new();
    }
    let mut result = String::new();
    let end = offset + len;

    for (si, seg) in segments.iter().enumerate() {
        let (seg_start, seg_end) = seg_positions[si];
        if seg_start >= end {
            break;
        }
        if seg_end <= offset {
            continue;
        }

        let ov_start = offset.max(seg_start) - seg_start;
        let ov_end = end.min(seg_end) - seg_start;

        if !seg.is_suggestion {
            result.push_str(&seg.base_text[ov_start..ov_end]);
        } else {
            // Suggestion: use FULL base (deleted) text
            result.push_str(&seg.base_text);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_metadata(raw: &str) -> String {
        let re = regex::Regex::new(r#"\{"author":"[^"]*","timestamp":\d+(?:,[^}]*)?\}@@"#).unwrap();
        re.replace_all(raw, "").to_string()
    }

    fn apply_merge(raw: &str, edit: &MergeResult) -> String {
        if edit.raw_len == 0 && edit.replacement.is_empty() {
            return raw.to_string();
        }
        let mut result = String::from(raw);
        result.replace_range(
            edit.raw_offset..edit.raw_offset + edit.raw_len,
            &edit.replacement,
        );
        result
    }

    // --- Group A: parse() + accepted_view() + base_view() ---

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
        assert_eq!(
            spans,
            vec![
                Span::Plain("The ".into()),
                Span::Suggestion {
                    deleted: "quick".into(),
                    inserted: "fast".into(),
                    author: "Unknown".into(),
                    timestamp: None,
                },
                Span::Plain(" brown fox.".into()),
            ]
        );
        assert_eq!(accepted_view(&spans), "The fast brown fox.");
        assert_eq!(base_view(&spans), "The quick brown fox.");
    }

    #[test]
    fn a03_substitution_with_metadata() {
        let raw = r#"The {--{"author":"AI","timestamp":1700000000000}@@quick--}{++{"author":"AI","timestamp":1700000000000}@@fast++} brown fox."#;
        let spans = parse(raw);
        assert_eq!(
            spans,
            vec![
                Span::Plain("The ".into()),
                Span::Suggestion {
                    deleted: "quick".into(),
                    inserted: "fast".into(),
                    author: "AI".into(),
                    timestamp: Some(1700000000000),
                },
                Span::Plain(" brown fox.".into()),
            ]
        );
        assert_eq!(accepted_view(&spans), "The fast brown fox.");
        assert_eq!(base_view(&spans), "The quick brown fox.");
    }

    #[test]
    fn a04_standalone_deletion() {
        let spans = parse("Hello {--beautiful --}world.");
        assert_eq!(
            spans,
            vec![
                Span::Plain("Hello ".into()),
                Span::Suggestion {
                    deleted: "beautiful ".into(),
                    inserted: "".into(),
                    author: "Unknown".into(),
                    timestamp: None,
                },
                Span::Plain("world.".into()),
            ]
        );
        assert_eq!(accepted_view(&spans), "Hello world.");
        assert_eq!(base_view(&spans), "Hello beautiful world.");
    }

    #[test]
    fn a05_standalone_insertion() {
        let spans = parse("Hello {++beautiful ++}world.");
        assert_eq!(
            spans,
            vec![
                Span::Plain("Hello ".into()),
                Span::Suggestion {
                    deleted: "".into(),
                    inserted: "beautiful ".into(),
                    author: "Unknown".into(),
                    timestamp: None,
                },
                Span::Plain("world.".into()),
            ]
        );
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
        assert_eq!(
            accepted_view(&spans),
            "Line one.\nReplaced lines.\nLine four."
        );
        assert_eq!(
            base_view(&spans),
            "Line one.\nLine two.\nLine three.\nLine four."
        );
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
            Span::Suggestion {
                author, timestamp, ..
            } => {
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
        assert_eq!(
            accepted_view(&spans),
            "```\n{--code1--}\n```\nactual\n```\n{++code2++}\n```"
        );
        assert_eq!(
            base_view(&spans),
            "```\n{--code1--}\n```\nreal\n```\n{++code2++}\n```"
        );
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

    // --- Group D: spans_covering_accepted_range() ---

    #[test]
    fn d01_plain_text_identity() {
        let spans = parse("Hello world.");
        let covered = spans_covering_accepted_range(&spans, 6, 5);
        assert_eq!(covered.len(), 1);
        assert_eq!(covered[0].span_index, 0);
    }

    #[test]
    fn d02_offset_in_suggestion_inserted() {
        let spans = parse("Say {--hello--}{++goodbye++} today.");
        let covered = spans_covering_accepted_range(&spans, 4, 7);
        assert_eq!(covered.len(), 1);
        assert_eq!(covered[0].span_index, 1);
    }

    #[test]
    fn d03_offset_spanning_plain_and_suggestion() {
        let spans = parse("The {--quick--}{++fast++} brown.");
        let covered = spans_covering_accepted_range(&spans, 2, 7);
        assert_eq!(covered.len(), 3);
        assert_eq!(covered[0].span_index, 0);
        assert_eq!(covered[1].span_index, 1);
        assert_eq!(covered[2].span_index, 2);
    }

    #[test]
    fn d04_offset_in_plain_between_suggestions() {
        let spans = parse("{--a--}{++x++} middle {--b--}{++y++}");
        let covered = spans_covering_accepted_range(&spans, 2, 6);
        assert_eq!(covered.len(), 1);
        assert_eq!(covered[0].span_index, 1);
    }

    #[test]
    fn d05_zero_length_range() {
        let spans = parse("Hello {--world--}{++earth++} today.");
        let covered = spans_covering_accepted_range(&spans, 11, 0);
        assert_eq!(covered.len(), 0);
    }

    #[test]
    fn d06_range_covering_entire_document() {
        let spans = parse("The {--quick--}{++fast++} brown {--fox--}{++cat++}.");
        let covered = spans_covering_accepted_range(&spans, 0, 19);
        assert_eq!(covered.len(), 5);
    }

    #[test]
    fn d07_standalone_deletion_contributes_zero_chars() {
        let spans = parse("Hello {--beautiful --}world.");
        let covered = spans_covering_accepted_range(&spans, 6, 5);
        assert_eq!(covered.len(), 1);
        assert_eq!(covered[0].span_index, 2);
    }

    #[test]
    fn d08_exact_span_boundary() {
        let spans = parse("ABC{--DEF--}{++GHI++}JKL");
        let covered = spans_covering_accepted_range(&spans, 3, 3);
        assert_eq!(covered.len(), 1);
        assert_eq!(covered[0].span_index, 1);
    }

    // --- Group B: merge_edit() ---

    #[test]
    fn b01_plain_text_no_suggestions() {
        let raw = "The quick brown fox jumps over the lazy dog.";
        let edit = merge_edit(raw, "quick brown", "slow red", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(
            accepted_view(&spans),
            "The slow red fox jumps over the lazy dog."
        );
    }

    #[test]
    fn b02_edit_not_touching_suggestions() {
        let raw = "The {--quick--}{++fast++} brown fox jumps over the lazy dog.";
        let edit = merge_edit(raw, "lazy dog", "happy cat", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(
            accepted_view(&spans),
            "The fast brown fox jumps over the happy cat."
        );
        assert_eq!(
            base_view(&spans),
            "The quick brown fox jumps over the lazy dog."
        );
    }

    #[test]
    fn b03_supersede_entire_suggestion() {
        let raw = "The {--quick--}{++fast++} brown fox.";
        let edit = merge_edit(raw, "fast", "speedy", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "The speedy brown fox.");
        assert_eq!(base_view(&spans), "The quick brown fox.");
    }

    #[test]
    fn b04_supersede_and_extend_right() {
        let raw = "The {--quick--}{++fast++} brown fox.";
        let edit = merge_edit(raw, "fast brown", "speedy red", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "The speedy red fox.");
        assert_eq!(base_view(&spans), "The quick brown fox.");
    }

    #[test]
    fn b05_supersede_and_extend_left() {
        let raw = "The {--quick--}{++fast++} brown fox.";
        let edit = merge_edit(raw, "The fast", "A speedy", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "A speedy brown fox.");
        assert_eq!(base_view(&spans), "The quick brown fox.");
    }

    #[test]
    fn b06_supersede_and_extend_both() {
        let raw = "Hello {--world--}{++earth++} today.";
        let edit = merge_edit(
            raw,
            "Hello earth today",
            "Greetings mars now",
            "AI",
            1700000000000,
        )
        .unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "Greetings mars now.");
        assert_eq!(base_view(&spans), "Hello world today.");
    }

    #[test]
    fn b07_span_multiple_suggestions() {
        let raw = "The {--quick--}{++fast++} brown {--fox--}{++cat++} jumps.";
        let edit =
            merge_edit(raw, "fast brown cat", "speedy red dog", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "The speedy red dog jumps.");
        assert_eq!(base_view(&spans), "The quick brown fox jumps.");
    }

    #[test]
    fn b08_two_suggestions_only_one_touched() {
        let raw = "The {--quick--}{++fast++} brown fox {--jumps--}{++leaps++} over.";
        let edit = merge_edit(raw, "fast", "speedy", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "The speedy brown fox leaps over.");
        assert_eq!(base_view(&spans), "The quick brown fox jumps over.");
    }

    #[test]
    fn b09_sequential_supersede() {
        let raw = r#"Say {--{"author":"AI","timestamp":1700000000000}@@hello--}{++{"author":"AI","timestamp":1700000000000}@@world++} today."#;
        let edit = merge_edit(raw, "world", "earth", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "Say earth today.");
        assert_eq!(base_view(&spans), "Say hello today.");
    }

    #[test]
    fn b10_ai_supersedes_human_suggestion() {
        let raw = r#"The {--{"author":"Human","timestamp":1700000000000}@@quick--}{++{"author":"Human","timestamp":1700000000000}@@fast++} brown fox."#;
        let edit = merge_edit(raw, "fast", "speedy", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "The speedy brown fox.");
        assert_eq!(base_view(&spans), "The quick brown fox.");
    }

    #[test]
    fn b11_whole_doc_replace_few_changes() {
        let raw = "Alpha {--beta--}{++gamma++} delta epsilon {--zeta--}{++eta++} theta.";
        let edit = merge_edit(
            raw,
            "Alpha gamma delta epsilon eta theta.",
            "Alpha gamma CHANGED epsilon eta MODIFIED.",
            "AI",
            1700000000000,
        )
        .unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(
            accepted_view(&spans),
            "Alpha gamma CHANGED epsilon eta MODIFIED."
        );
        assert_eq!(base_view(&spans), "Alpha beta delta epsilon zeta theta.");
    }

    #[test]
    fn b12_whole_doc_replace_everything() {
        let raw = "The {--quick--}{++fast++} brown fox.";
        let edit = merge_edit(
            raw,
            "The fast brown fox.",
            "Completely different content here.",
            "AI",
            1700000000000,
        )
        .unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "Completely different content here.");
        assert_eq!(base_view(&spans), "The quick brown fox.");
    }

    #[test]
    fn b13_standalone_deletion_not_overlapped() {
        let raw = "Hello {--beautiful --}world today.";
        let edit = merge_edit(raw, "world today", "earth now", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "Hello earth now.");
        // Standalone deletion preserved
        assert!(result.contains("{--beautiful --}"));
    }

    #[test]
    fn b14_overlaps_standalone_insertion() {
        let raw = "Hello {++beautiful ++}world today.";
        let edit = merge_edit(
            raw,
            "beautiful world",
            "wonderful planet",
            "AI",
            1700000000000,
        )
        .unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "Hello wonderful planet today.");
        assert_eq!(base_view(&spans), "Hello world today.");
    }

    #[test]
    fn b15_edit_within_multiword_suggestion() {
        let raw = "Say {--hello world--}{++goodbye earth++} now.";
        let edit = merge_edit(raw, "goodbye", "farewell", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "Say farewell earth now.");
        assert_eq!(base_view(&spans), "Say hello world now.");
    }

    #[test]
    fn b16_pure_deletion() {
        let raw = "Keep {--old--}{++current++} remove this end.";
        let edit = merge_edit(raw, "remove this", "", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "Keep current  end.");
        // base_view includes the deleted plain text "remove this" because it's tracked as a suggestion
        assert_eq!(base_view(&spans), "Keep old remove this end.");
    }

    #[test]
    fn b17_pure_insertion() {
        let raw = "Hello world.";
        let edit = merge_edit(
            raw,
            "Hello world",
            "Hello beautiful world",
            "AI",
            1700000000000,
        )
        .unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "Hello beautiful world.");
    }

    #[test]
    fn b18_edit_at_document_start() {
        let raw = "{--Old--}{++New++} start of document.";
        let edit = merge_edit(raw, "New start", "Fresh beginning", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "Fresh beginning of document.");
        assert_eq!(base_view(&spans), "Old start of document.");
    }

    #[test]
    fn b19_edit_at_document_end() {
        let raw = "Start of document {--ending--}{++conclusion++}.";
        let edit = merge_edit(raw, "conclusion.", "finale!", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "Start of document finale!");
        assert_eq!(base_view(&spans), "Start of document ending.");
    }

    #[test]
    fn b20_three_adjacent_suggestions() {
        let raw = "{--a--}{++x++}{--b--}{++y++}{--c--}{++z++}";
        let edit = merge_edit(raw, "xyz", "123", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "123");
        assert_eq!(base_view(&spans), "abc");
    }

    #[test]
    fn b21_insert_newline_paragraph_break() {
        let raw = "First sentence. Second sentence.";
        let edit = merge_edit(
            raw,
            "First sentence. Second sentence.",
            "First sentence.\n\nSecond sentence.",
            "AI",
            1700000000000,
        )
        .unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "First sentence.\n\nSecond sentence.");
    }

    #[test]
    fn b22_remove_newline_join_paragraphs() {
        let raw = "First paragraph.\n\nSecond paragraph.";
        let edit = merge_edit(
            raw,
            "First paragraph.\n\nSecond paragraph.",
            "First paragraph. Second paragraph.",
            "AI",
            1700000000000,
        )
        .unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "First paragraph. Second paragraph.");
    }

    #[test]
    fn b23_noop_edit_preserves_existing_suggestions() {
        let raw = "The {--quick--}{++fast++} brown fox.";
        let edit = merge_edit(
            raw,
            "fast brown fox.",
            "fast brown fox.",
            "AI",
            1700000000000,
        )
        .unwrap();
        let result = apply_merge(raw, &edit);
        assert_eq!(result, raw);
    }

    #[test]
    fn b24_edit_adjacent_before_suggestion() {
        let raw = "Hello {--world--}{++earth++} today.";
        let edit = merge_edit(raw, "Hello", "Greetings", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "Greetings earth today.");
        assert_eq!(base_view(&spans), "Hello world today.");
    }

    #[test]
    fn b25_edit_adjacent_after_suggestion() {
        let raw = "Hello {--world--}{++earth++} today.";
        let edit = merge_edit(raw, "today", "now", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "Hello earth now.");
        assert_eq!(base_view(&spans), "Hello world today.");
    }

    #[test]
    fn b26_diff_regions_overlap_different_suggestions() {
        let raw = "Alpha {--beta--}{++gamma++} delta epsilon {--zeta--}{++eta++} theta.";
        let edit = merge_edit(
            raw,
            "Alpha gamma delta epsilon eta theta.",
            "Alpha GAMMA delta epsilon ETA theta.",
            "AI",
            1700000000000,
        )
        .unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(
            accepted_view(&spans),
            "Alpha GAMMA delta epsilon ETA theta."
        );
        assert_eq!(base_view(&spans), "Alpha beta delta epsilon zeta theta.");
    }

    #[test]
    fn b27_ai_writes_criticmarkup_as_literal_content() {
        let raw = "CriticMarkup uses special syntax.";
        let edit = merge_edit(
            raw,
            "CriticMarkup uses special syntax.",
            "CriticMarkup uses {--deleted--} and {++inserted++} syntax.",
            "AI",
            1700000000000,
        )
        .unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        // The accepted view should contain the literal delimiters as content
        assert_eq!(
            accepted_view(&spans),
            "CriticMarkup uses {--deleted--} and {++inserted++} syntax."
        );
    }

    #[test]
    fn b28_document_entirely_suggestions() {
        let raw = "{--old beginning--}{++new beginning++}{--old middle--}{++new middle++}{--old end--}{++new end++}";
        let edit = merge_edit(raw, "new beginning", "fresh start", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "fresh startnew middlenew end");
        assert_eq!(base_view(&spans), "old beginningold middleold end");
    }

    #[test]
    fn b29_three_separate_change_regions() {
        let raw = "The quick brown fox jumps over the lazy dog near the old barn.";
        let edit = merge_edit(
            raw,
            "The quick brown fox jumps over the lazy dog near the old barn.",
            "The slow brown fox jumps over the happy dog near the new barn.",
            "AI",
            1700000000000,
        )
        .unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(
            accepted_view(&spans),
            "The slow brown fox jumps over the happy dog near the new barn."
        );
        let suggestion_count = spans
            .iter()
            .filter(|s| matches!(s, Span::Suggestion { .. }))
            .count();
        assert!(
            suggestion_count >= 3,
            "Expected at least 3 suggestions, got {}",
            suggestion_count
        );
    }

    #[test]
    fn b30_old_string_spans_suggestion_and_plain_text_duplicate() {
        let raw = "Say {--goodbye--}{++hello++} and then say hello again.";
        let edit = merge_edit(
            raw,
            "hello and then say hello",
            "greetings and then say farewell",
            "AI",
            1700000000000,
        )
        .unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(
            accepted_view(&spans),
            "Say greetings and then say farewell again."
        );
        assert_eq!(base_view(&spans), "Say goodbye and then say hello again.");
    }

    #[test]
    fn b31_edit_starts_at_exact_suggestion_boundary() {
        let raw = "Say {--hello--}{++goodbye++} friend.";
        let edit =
            merge_edit(raw, "goodbye friend", "farewell buddy", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "Say farewell buddy.");
        assert_eq!(base_view(&spans), "Say hello friend.");
    }

    #[test]
    fn b32_edit_ends_at_exact_suggestion_boundary() {
        let raw = "Say {--hello--}{++goodbye++} friend.";
        let edit = merge_edit(raw, "Say goodbye", "Tell farewell", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);
        assert_eq!(accepted_view(&spans), "Tell farewell friend.");
        assert_eq!(base_view(&spans), "Say hello friend.");
    }

    // --- Exact markup tests ---

    #[test]
    fn b03_exact_markup() {
        let raw = "The {--quick--}{++fast++} brown fox.";
        let edit = merge_edit(raw, "fast", "speedy", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let clean = strip_metadata(&result);
        assert_eq!(clean, "The {--quick--}{++speedy++} brown fox.");
    }

    #[test]
    fn b04_exact_markup() {
        let raw = "The {--quick--}{++fast++} brown fox.";
        let edit = merge_edit(raw, "fast brown", "speedy red", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let clean = strip_metadata(&result);
        assert_eq!(clean, "The {--quick brown--}{++speedy red++} fox.");
    }

    #[test]
    fn b07_exact_markup() {
        let raw = "The {--quick--}{++fast++} brown {--fox--}{++cat++} jumps.";
        let edit =
            merge_edit(raw, "fast brown cat", "speedy red dog", "AI", 1700000000000).unwrap();
        let result = apply_merge(raw, &edit);
        let clean = strip_metadata(&result);
        assert_eq!(
            clean,
            "The {--quick brown fox--}{++speedy red dog++} jumps."
        );
    }

    // --- Bug regression: append to end of suggestion loses wrapping ---

    #[test]
    fn b33_append_to_end_of_suggestion_preserves_wrapping() {
        // Bug: when entire doc is a suggestion and you edit the end to append content,
        // the original suggestion content gets "promoted" to plain text (loses wrapping)
        // and only the new appended text gets wrapped.
        let raw = "{++{\"author\":\"AI\",\"timestamp\":1700000000000}@@# Test File\n\nSome content.\n\nCreated: 2026-03-16++}";
        let edit = merge_edit(
            raw,
            "Created: 2026-03-16",
            "Created: 2026-03-16\n\n## More Testing\n\nAppended content here.",
            "AI",
            1700000060000,
        )
        .unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);

        // Accepted view should show all content
        assert_eq!(
            accepted_view(&spans),
            "# Test File\n\nSome content.\n\nCreated: 2026-03-16\n\n## More Testing\n\nAppended content here."
        );

        // Base view: the ORIGINAL base was empty (standalone insertion).
        // The unchanged prefix "# Test File\n\nSome content.\n\n" is still inside
        // the original suggestion, so base for that part is "" (empty).
        // The "Created: 2026-03-16" part was edited — its base is also "" (from the
        // original insertion's deleted side). The new appended text has no base.
        // So base_view should be empty string (everything is insertions).
        assert_eq!(base_view(&spans), "");

        // The original content that wasn't edited should still be inside a suggestion,
        // NOT promoted to plain text.
        let plain_count = spans.iter().filter(|s| matches!(s, Span::Plain(_))).count();
        assert_eq!(
            plain_count, 0,
            "No plain text should exist — everything should be in suggestions. Got: {:?}",
            spans
        );
    }

    #[test]
    fn b34_prepend_to_start_of_suggestion_preserves_wrapping() {
        // Mirror of b33: edit matches the beginning of a suggestion and prepends content.
        let raw = "{++{\"author\":\"AI\",\"timestamp\":1700000000000}@@# Test File\n\nSome content here.++}";
        let edit = merge_edit(
            raw,
            "# Test File",
            "# Introduction\n\n# Test File",
            "AI",
            1700000060000,
        )
        .unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);

        assert_eq!(
            accepted_view(&spans),
            "# Introduction\n\n# Test File\n\nSome content here."
        );
        // Base was empty (standalone insertion), should still be empty
        assert_eq!(base_view(&spans), "");
        // No plain text — everything in suggestions
        let plain_count = spans.iter().filter(|s| matches!(s, Span::Plain(_))).count();
        assert_eq!(
            plain_count, 0,
            "No plain text should exist. Got: {:?}",
            spans
        );
    }

    #[test]
    fn b35_edit_word_in_middle_of_suggestion_preserves_wrapping() {
        // Edit one word inside a suggestion. The prefix and suffix of the suggestion
        // are in equal regions and must preserve their CriticMarkup wrapping.
        let raw = "{++{\"author\":\"AI\",\"timestamp\":1700000000000}@@The quick brown fox jumps over the lazy dog.++}";
        let edit = merge_edit(
            raw,
            "The quick brown fox jumps over the lazy dog.",
            "The quick brown fox leaps over the lazy dog.",
            "AI",
            1700000060000,
        )
        .unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);

        assert_eq!(
            accepted_view(&spans),
            "The quick brown fox leaps over the lazy dog."
        );
        assert_eq!(base_view(&spans), "");
        let plain_count = spans.iter().filter(|s| matches!(s, Span::Plain(_))).count();
        assert_eq!(
            plain_count, 0,
            "No plain text should exist. Got: {:?}",
            spans
        );
    }

    #[test]
    fn b36_two_untouched_suggestions_in_covered_range() {
        // Edit plain text between two suggestions. Both suggestions must preserve wrapping.
        let raw = "Start {--old1--}{++new1++} middle text {--old2--}{++new2++} end.";
        let edit = merge_edit(
            raw,
            "new1 middle text new2",
            "new1 CHANGED text new2",
            "AI",
            1700000000000,
        )
        .unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);

        assert_eq!(accepted_view(&spans), "Start new1 CHANGED text new2 end.");
        // "middle" was plain text changed to "CHANGED", so base = "middle"
        assert_eq!(base_view(&spans), "Start old1 middle text old2 end.");
        // Both original suggestions should still be present (not promoted to plain)
        assert!(
            result.contains("{--old1--}") || result.contains("old1--}"),
            "First suggestion should be preserved in raw: {}",
            result
        );
        assert!(
            result.contains("{--old2--}") || result.contains("old2--}"),
            "Second suggestion should be preserved in raw: {}",
            result
        );
    }

    #[test]
    fn b37_create_then_edit_word_in_middle() {
        // Simulates: LLM creates doc (all content in {++...++}), then edits one word.
        // This is the most common LLM workflow that triggered the original bug.
        let raw = "{++{\"author\":\"AI\",\"timestamp\":1700000000000}@@Photosynthesis is the process by which green plants convert sunlight into chemical energy for growth.++}";
        let edit = merge_edit(
            raw,
            "Photosynthesis is the process by which green plants convert sunlight into chemical energy for growth.",
            "Photosynthesis is the process by which all plants convert sunlight into stored energy for growth.",
            "AI",
            1700000060000,
        )
        .unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);

        assert_eq!(
            accepted_view(&spans),
            "Photosynthesis is the process by which all plants convert sunlight into stored energy for growth."
        );
        // Base was empty (standalone insertion), should still be empty
        assert_eq!(base_view(&spans), "");
        let plain_count = spans.iter().filter(|s| matches!(s, Span::Plain(_))).count();
        assert_eq!(
            plain_count, 0,
            "No plain text after editing a created doc. Got: {:?}",
            spans
        );
    }

    #[test]
    fn b38_edit_straddling_suggestion_and_plain_text() {
        // old_string starts in plain text and ends inside a suggestion's inserted text.
        let raw = "Hello world {--old--}{++new++} rest of doc.";
        let edit = merge_edit(
            raw,
            "world new rest",
            "planet new remainder",
            "AI",
            1700000000000,
        )
        .unwrap();
        let result = apply_merge(raw, &edit);
        let spans = parse(&result);

        assert_eq!(accepted_view(&spans), "Hello planet new remainder of doc.");
        // "world" was plain text (base = "world"), "new" maps to suggestion (base = "old"),
        // "rest" was plain text (base = "rest")
        // "world"→"planet" change: base = "world"
        // "new" is in equal region: suggestion preserved, base = "old"
        // "rest"→"remainder" change: base = "rest"
        assert_eq!(base_view(&spans), "Hello world old rest of doc.");
    }

    #[test]
    #[test]
    fn b39_content_with_criticmarkup_delimiters_in_created_doc() {
        // KNOWN LIMITATION: When AI creates a doc containing literal CriticMarkup
        // delimiters, and the content is wrapped in {++...++}, the parser's
        // find_closing() matches the inner ++} before the outer one.
        //
        // Example: {++meta@@Use {--old--} and {++new++} text.++}
        // Parser finds ++} at "new++}" instead of "text.++}"
        //
        // This is inherent to using simple string search for closing delimiters.
        // A nesting-aware parser would fix this, but it's a rare edge case
        // (LLM writing about CriticMarkup syntax in a newly created doc).
        //
        // For now, document the limitation rather than adding parser complexity.
        let raw = "{++{\"author\":\"AI\",\"timestamp\":1700000000000}@@Use {--old--} for deletions and {++new++} for insertions.++}";
        let spans = parse(raw);

        // Due to the limitation, the parser sees:
        // - {++...@@Use {--old--} for deletions and {++new++} as a suggestion
        //   (closes at the inner ++})
        // - " for insertions.++}" as plain text
        // This is not ideal but documented behavior.
        assert_eq!(
            accepted_view(&spans),
            "Use {--old--} for deletions and {++new for insertions.++}"
        );
    }

    // --- Input validation: reject CriticMarkup in AI input ---

    #[test]
    fn reject_criticmarkup_in_old_string() {
        assert!(reject_if_contains_markup("normal text", "old_string").is_ok());
        assert!(reject_if_contains_markup("{--deleted--}", "old_string").is_err());
        assert!(reject_if_contains_markup("{++inserted++}", "old_string").is_err());
        assert!(
            reject_if_contains_markup("text with {~~sub~>rep~~} inside", "old_string").is_err()
        );
    }

    #[test]
    fn reject_criticmarkup_in_new_string() {
        assert!(reject_if_contains_markup("normal replacement", "new_string").is_ok());
        assert!(reject_if_contains_markup("has {--markup--} inside", "new_string").is_err());
    }

    #[test]
    fn reject_criticmarkup_in_content() {
        assert!(reject_if_contains_markup("# Normal Title\n\nContent here.", "content").is_ok());
        assert!(reject_if_contains_markup("Has {++suggestion++} inside", "content").is_err());
    }

    // --- Group C: render_pending_summary() ---

    #[test]
    fn c01_no_suggestions_no_footer() {
        let raw = "Line one.\nLine two.\nLine three.";
        let spans = parse(raw);
        let accepted = accepted_view(&spans);
        assert!(render_pending_summary(&spans, &accepted).is_none());
    }

    #[test]
    fn c02_single_suggestion_footer() {
        let raw = "The {--quick--}{++fast++} brown fox.\nLine two.\nLine three.";
        let spans = parse(raw);
        let accepted = accepted_view(&spans);
        let footer = render_pending_summary(&spans, &accepted).unwrap();
        assert!(footer.contains("[Pending suggestions]"));
        assert!(footer.contains("{--quick--}{++fast++}"));
    }

    #[test]
    fn c03_multiple_lines_affected() {
        let raw = "The {--quick--}{++fast++} brown fox.\nA normal line.\n{++New ++}content here.";
        let spans = parse(raw);
        let accepted = accepted_view(&spans);
        let footer = render_pending_summary(&spans, &accepted).unwrap();
        assert!(footer.contains("{--quick--}{++fast++}"));
        assert!(footer.contains("{++New ++}"));
        assert!(!footer.contains("normal line"));
    }

    #[test]
    fn c04_truncation_long_deletion() {
        let raw = "{--one two three four five six seven eight nine ten eleven twelve--}{++replacement++} end.";
        let spans = parse(raw);
        let accepted = accepted_view(&spans);
        let footer = render_pending_summary(&spans, &accepted).unwrap();
        assert!(footer.contains("one two three ... ten eleven twelve"));
    }

    #[test]
    fn c05_truncation_long_insertion() {
        let raw =
            "{--old--}{++one two three four five six seven eight nine ten eleven twelve++} end.";
        let spans = parse(raw);
        let accepted = accepted_view(&spans);
        let footer = render_pending_summary(&spans, &accepted).unwrap();
        assert!(footer.contains("one two three ... ten eleven twelve"));
    }

    #[test]
    fn c06_short_sides_not_truncated() {
        let raw = "{--one two three four five--}{++six seven eight nine ten++} end.";
        let spans = parse(raw);
        let accepted = accepted_view(&spans);
        let footer = render_pending_summary(&spans, &accepted).unwrap();
        assert!(footer.contains("{--one two three four five--}{++six seven eight nine ten++}"));
    }

    #[test]
    fn c07_metadata_stripped_in_footer() {
        let raw = r#"The {--{"author":"AI","timestamp":1700000000000}@@quick--}{++{"author":"AI","timestamp":1700000000000}@@fast++} fox."#;
        let spans = parse(raw);
        let accepted = accepted_view(&spans);
        let footer = render_pending_summary(&spans, &accepted).unwrap();
        assert!(footer.contains("{--quick--}{++fast++}"));
        assert!(footer.contains("AI"));
        assert!(!footer.contains("@@"));
        assert!(!footer.contains("\"timestamp\""));
    }

    #[test]
    fn c11_multiple_suggestions_same_line() {
        let raw = "The {--quick--}{++fast++} brown {--fox--}{++cat++} jumps.";
        let spans = parse(raw);
        let accepted = accepted_view(&spans);
        let footer = render_pending_summary(&spans, &accepted).unwrap();
        assert!(footer.contains("{--quick--}{++fast++}"));
        assert!(footer.contains("{--fox--}{++cat++}"));
    }

    #[test]
    fn c12_unknown_author_no_timestamp() {
        let raw = "The {--quick--}{++fast++} fox.";
        let spans = parse(raw);
        let accepted = accepted_view(&spans);
        let footer = render_pending_summary(&spans, &accepted).unwrap();
        assert!(footer.contains("Unknown"));
        assert!(!footer.contains("ago"));
    }
}
