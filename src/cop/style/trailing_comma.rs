//! Shared infrastructure for the three TrailingComma cops:
//! - Style/TrailingCommaInArguments
//! - Style/TrailingCommaInArrayLiteral
//! - Style/TrailingCommaInHashLiteral
//!
//! Mirrors RuboCop's `TrailingComma` mixin. All three cops share the same
//! heredoc-aware comma detection, multiline style enforcement, and
//! `no_elements_on_same_line` checks. The only differences are which node
//! types they inspect and the wording of diagnostic messages.
//!
//! Note: Style/TrailingCommaInBlockArgs is intentionally excluded — it checks
//! for useless trailing commas in block parameter lists, which is a
//! fundamentally different concern with no shared logic.

use crate::cop::shared::util::has_trailing_comma;
use crate::parse::source::SourceFile;

// ── Heredoc detection ─────────────────────────────────────────────────

/// Returns true if a node is or contains a heredoc.
///
/// Handles:
/// - Direct heredoc string/interpolated-string nodes
/// - Method calls on heredocs (e.g., `<<~SQL.strip.chomp`)
/// - Assoc pair nodes (hash key-value pairs)
/// - KeywordHashNode elements
/// - HashNode elements
/// - ArrayNode elements (nested sub-arrays containing heredocs)
/// - Call arguments containing heredocs
pub fn is_heredoc_node(node: &ruby_prism::Node<'_>) -> bool {
    // Check pair nodes (hash key-value pairs)
    if let Some(pair) = node.as_assoc_node() {
        return is_heredoc_node(&pair.value());
    }

    // Keyword hash expansion
    if let Some(kw_hash) = node.as_keyword_hash_node() {
        return kw_hash.elements().iter().any(|elem| is_heredoc_node(&elem));
    }

    // Hash values
    if let Some(hash) = node.as_hash_node() {
        return hash
            .elements()
            .iter()
            .any(|element| is_heredoc_node(&element));
    }

    // Direct interpolated string heredoc
    if let Some(s) = node.as_interpolated_string_node() {
        if let Some(open) = s.opening_loc() {
            if open.as_slice().starts_with(b"<<") {
                return true;
            }
        }
    }

    // Direct string heredoc
    if let Some(s) = node.as_string_node() {
        if let Some(open) = s.opening_loc() {
            if open.as_slice().starts_with(b"<<") {
                return true;
            }
        }
    }

    // Method calls on heredocs (e.g., <<~SQL.strip.chomp)
    if let Some(call) = node.as_call_node() {
        if let Some(recv) = call.receiver() {
            if is_heredoc_node(&recv) {
                return true;
            }
        }
        // Check arguments for heredocs (e.g., method(<<RUBY))
        if let Some(args) = call.arguments() {
            if args.arguments().iter().any(|a| is_heredoc_node(&a)) {
                return true;
            }
        }
    }

    // Check sub-arrays that may contain heredoc elements
    if let Some(arr) = node.as_array_node() {
        return arr.elements().iter().any(|e| is_heredoc_node(&e));
    }

    false
}

// ── Trailing comma detection ──────────────────────────────────────────

/// Check for a trailing comma between `last_end` and `closing_start`,
/// choosing the heredoc-safe or standard approach based on `has_heredoc`.
pub fn detect_trailing_comma(
    bytes: &[u8],
    last_end: usize,
    closing_start: usize,
    has_heredoc: bool,
) -> bool {
    if has_heredoc {
        has_trailing_comma_no_newline(bytes, last_end, closing_start)
    } else {
        has_trailing_comma(bytes, last_end, closing_start)
    }
}

/// Like `has_trailing_comma` but stops at the first newline. Used when
/// heredocs are present: only match commas on the same line as the last
/// element's end offset (the heredoc opening tag line), never inside
/// heredoc content on subsequent lines. Matches RuboCop's `/\A[^\S\n]*,/`
/// regex used in `comma_offset` when `any_heredoc?` is true.
fn has_trailing_comma_no_newline(
    source_bytes: &[u8],
    last_element_end: usize,
    closing_start: usize,
) -> bool {
    if last_element_end >= closing_start || closing_start > source_bytes.len() {
        return false;
    }
    let region = &source_bytes[last_element_end..closing_start];
    for &b in region {
        match b {
            b'\n' | b'\r' => return false,
            b',' => return true,
            b' ' | b'\t' => {}
            _ => return false,
        }
    }
    false
}

/// Find the absolute byte offset of a trailing comma between `start` and
/// `end`. When `stop_at_newline` is true (heredoc mode), stops searching
/// at the first newline to avoid entering heredoc content.
pub fn find_trailing_comma_offset(
    bytes: &[u8],
    start: usize,
    end: usize,
    stop_at_newline: bool,
) -> Option<usize> {
    if start >= end || end > bytes.len() {
        return None;
    }

    for (idx, &b) in bytes[start..end].iter().enumerate() {
        if stop_at_newline && matches!(b, b'\n' | b'\r') {
            return None;
        }
        if b == b',' {
            return Some(start + idx);
        }
    }

    None
}

// ── Element line analysis ─────────────────────────────────────────────

/// Returns true if no two consecutive items (including the closing bracket)
/// are on the same line. Matches RuboCop's `no_elements_on_same_line?`.
pub fn no_elements_on_same_line(
    source: &SourceFile,
    element_locations: &[(usize, usize)],
    closing_start: usize,
) -> bool {
    for pair in element_locations.windows(2) {
        let end_line = source.offset_to_line_col(pair[0].1).0;
        let start_line = source.offset_to_line_col(pair[1].0).0;
        if end_line == start_line {
            return false;
        }
    }
    if let Some(last) = element_locations.last() {
        let last_end_line = source.offset_to_line_col(last.1).0;
        let close_line = source.offset_to_line_col(closing_start).0;
        if last_end_line == close_line {
            return false;
        }
    }
    true
}

/// Returns true if the last item immediately precedes a newline (possibly with
/// an optional comma and inline comment in between). Matches RuboCop's
/// `last_item_precedes_newline?` for the `diff_comma` style.
pub fn last_item_precedes_newline(bytes: &[u8], last_end: usize, closing_start: usize) -> bool {
    let region = &bytes[last_end..closing_start];
    let mut i = 0;
    // Skip optional comma
    if i < region.len() && region[i] == b',' {
        i += 1;
    }
    // Skip spaces/tabs (but not newlines)
    while i < region.len() && (region[i] == b' ' || region[i] == b'\t') {
        i += 1;
    }
    // Skip optional comment
    if i < region.len() && region[i] == b'#' {
        while i < region.len() && region[i] != b'\n' {
            i += 1;
        }
    }
    // Must end with a newline
    i < region.len() && region[i] == b'\n'
}

/// Collect element (start_offset, end_offset) pairs from call arguments.
///
/// RuboCop only promotes a braceless hash argument into its individual assoc
/// elements when that hash argument is itself multiline. A single-line keyword
/// hash inside an otherwise multiline call still counts as one argument for the
/// `comma` style.
pub fn effective_element_locations<'a>(
    source: &SourceFile,
    elements: impl Iterator<Item = ruby_prism::Node<'a>>,
) -> Vec<(usize, usize)> {
    let mut locations = Vec::new();
    for elem in elements {
        if let Some(kw_hash) = elem.as_keyword_hash_node() {
            let kw_loc = kw_hash.location();
            let kw_start_line = source.offset_to_line_col(kw_loc.start_offset()).0;
            let kw_end_line = source.offset_to_line_col(kw_loc.end_offset()).0;
            if kw_start_line != kw_end_line {
                for child in kw_hash.elements().iter() {
                    let loc = child.location();
                    locations.push((loc.start_offset(), loc.end_offset()));
                }
            } else {
                locations.push((kw_loc.start_offset(), kw_loc.end_offset()));
            }
        } else {
            let loc = elem.location();
            locations.push((loc.start_offset(), loc.end_offset()));
        }
    }
    locations
}

/// Check if a byte range contains only whitespace and exactly one comma.
/// Returns false if there are other non-whitespace characters (e.g., heredoc
/// content). Comments (starting with #) are treated as whitespace.
///
/// Used by TrailingCommaInArguments for the non-heredoc path where comments
/// may appear between the last arg and closing paren.
pub fn is_only_whitespace_and_comma(bytes: &[u8]) -> bool {
    let mut found_comma = false;
    let mut in_comment = false;
    for &b in bytes {
        if in_comment {
            if b == b'\n' {
                in_comment = false;
            }
            continue;
        }
        match b {
            b',' => {
                if found_comma {
                    return false; // Multiple commas
                }
                found_comma = true;
            }
            b'#' => {
                in_comment = true;
            }
            b' ' | b'\t' | b'\n' | b'\r' => {}
            _ => return false,
        }
    }
    found_comma
}
