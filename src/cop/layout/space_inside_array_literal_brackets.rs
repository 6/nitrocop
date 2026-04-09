/// Layout/SpaceInsideArrayLiteralBrackets
///
/// Investigation notes (2026-03-24, FP=0 FN=58):
/// - The 58 FNs were caused by two issues:
///   1. Empty bracket detection only handled exactly 0 or 1 space between brackets.
///      RuboCop treats any bracket pair with only whitespace/newlines between them
///      as "empty", including `[   ]` and `[\n]`. Fixed by scanning for non-whitespace
///      between brackets.
///   2. Autocorrect for `no_space` style only removed a single space character.
///      When multiple spaces exist (e.g., `[  1, 2, 3   ]`), all contiguous spaces
///      adjacent to the bracket must be removed. Fixed by scanning for the full
///      whitespace run.
///   3. `ARRAY_PATTERN_NODE` (pattern matching `in [a, b]`) was not handled.
///      RuboCop aliases `on_array_pattern` to `on_array`. Added support.
/// - FP=6 fix (2026-04-02): RuboCop skips this cop entirely for array patterns
///   that end with a trailing comma (`in [a, ]`, `in Foo[ a, ]`), and it also
///   accepts multiline arrays with `[ <spaces>\n  # comment` after the opening
///   bracket. Fixed by detecting the trailing-comma array-pattern context and by
///   treating comment-on-next-line as an allowed multiline opening-bracket case.
/// - Compact variant FN=6885 fix (2026-04-09): RuboCop's `multi_dimensional_array?`
///   skips newline tokens when checking for adjacent brackets, so `[\n  [...]` is
///   treated as multi-dimensional and flagged for collapsing. Our byte-level scan
///   (`is_adjacent_bracket_forward/backward`) was stopping at newlines. Fixed by
///   including `\n`/`\r` in the whitespace set these functions skip. This also
///   catches element reference brackets across lines (e.g., `[\n  attr[:name]\n]`
///   where inner `]` is adjacent to outer `]`).
use crate::cop::shared::node_type::{ARRAY_NODE, ARRAY_PATTERN_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

pub struct SpaceInsideArrayLiteralBrackets;

impl Cop for SpaceInsideArrayLiteralBrackets {
    fn name(&self) -> &'static str {
        "Layout/SpaceInsideArrayLiteralBrackets"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[ARRAY_NODE, ARRAY_PATTERN_NODE]
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let (opening, closing, is_array_pattern) = if let Some(array) = node.as_array_node() {
            match (array.opening_loc(), array.closing_loc()) {
                (Some(o), Some(c)) => (o, c, false),
                _ => return, // Implicit array (no brackets)
            }
        } else if let Some(pattern) = node.as_array_pattern_node() {
            match (pattern.opening_loc(), pattern.closing_loc()) {
                (Some(o), Some(c)) => (o, c, true),
                _ => return,
            }
        } else {
            return;
        };

        // Only check [ ] arrays, not %w() etc.
        if opening.as_slice() != b"[" || closing.as_slice() != b"]" {
            return;
        }

        self.check_brackets(
            source,
            &opening,
            &closing,
            is_array_pattern,
            config,
            diagnostics,
            corrections,
        );
    }
}

impl SpaceInsideArrayLiteralBrackets {
    #[allow(clippy::too_many_arguments)]
    fn check_brackets(
        &self,
        source: &SourceFile,
        opening: &ruby_prism::Location<'_>,
        closing: &ruby_prism::Location<'_>,
        is_array_pattern: bool,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let bytes = source.as_bytes();
        let open_end = opening.end_offset();
        let close_start = closing.start_offset();

        if is_array_pattern && prev_non_whitespace(bytes, close_start) == Some(b',') {
            return;
        }

        let empty_style = config.get_str("EnforcedStyleForEmptyBrackets", "no_space");

        // Check if the array is empty: only whitespace/newlines between brackets
        let is_empty = is_only_whitespace(bytes, open_end, close_start);

        if is_empty {
            if close_start == open_end {
                // Truly empty: []
                if empty_style == "space" {
                    let (line, column) = source.offset_to_line_col(opening.start_offset());
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside empty array literal brackets missing.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: open_end,
                            end: open_end,
                            replacement: " ".to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
            } else {
                // Has whitespace between brackets: [ ], [   ], [\n]
                let is_single_space =
                    close_start == open_end + 1 && bytes.get(open_end) == Some(&b' ');
                match empty_style {
                    "no_space" => {
                        let (line, column) = source.offset_to_line_col(opening.start_offset());
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            "Space inside empty array literal brackets detected.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            corr.push(crate::correction::Correction {
                                start: open_end,
                                end: close_start,
                                replacement: String::new(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                    "space" if !is_single_space => {
                        // Multiple spaces or newline: correct to single space
                        let (line, column) = source.offset_to_line_col(opening.start_offset());
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            "Space inside empty array literal brackets missing.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            corr.push(crate::correction::Correction {
                                start: open_end,
                                end: close_start,
                                replacement: " ".to_string(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                    _ => {}
                }
            }
            return;
        }

        let enforced = config.get_str("EnforcedStyle", "no_space");

        // For multiline arrays, determine which bracket sides to skip.
        let (open_line, _) = source.offset_to_line_col(opening.start_offset());
        let (close_line, _) = source.offset_to_line_col(closing.start_offset());
        let is_multiline = open_line != close_line;

        let start_ok = if is_multiline {
            match enforced {
                "no_space" => {
                    next_to_comment(bytes, open_end)
                        || next_line_starts_with_comment(bytes, open_end)
                }
                _ => next_to_newline(bytes, open_end),
            }
        } else {
            false
        };

        let end_ok = if is_multiline {
            begins_its_line_raw(bytes, close_start)
        } else {
            false
        };

        let space_after_open = matches!(bytes.get(open_end), Some(b' ' | b'\t'));
        let space_before_close =
            close_start > 0 && matches!(bytes.get(close_start - 1), Some(b' ' | b'\t'));

        match enforced {
            "no_space" => {
                if !start_ok && space_after_open {
                    let space_end = scan_space_forward(bytes, open_end);
                    let (line, column) = source.offset_to_line_col(opening.start_offset());
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside array literal brackets detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: open_end,
                            end: space_end,
                            replacement: String::new(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
                if !end_ok && space_before_close {
                    let space_start = scan_space_backward(bytes, close_start);
                    let (line, column) = source.offset_to_line_col(closing.start_offset());
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside array literal brackets detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: space_start,
                            end: close_start,
                            replacement: String::new(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
            }
            "space" => {
                if !start_ok && !space_after_open {
                    let (line, column) = source.offset_to_line_col(opening.start_offset());
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside array literal brackets missing.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: open_end,
                            end: open_end,
                            replacement: " ".to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
                if !end_ok && !space_before_close {
                    let (line, column) = source.offset_to_line_col(closing.start_offset());
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside array literal brackets missing.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: close_start,
                            end: close_start,
                            replacement: " ".to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
            }
            "compact" => {
                let multi_dim_left = is_adjacent_bracket_forward(bytes, open_end);
                let multi_dim_right = is_adjacent_bracket_backward(bytes, close_start);

                // Left side: whitespace check includes newlines for compact collapse
                let ws_after_open =
                    matches!(bytes.get(open_end), Some(b' ' | b'\t' | b'\n' | b'\r'));

                if multi_dim_left && ws_after_open {
                    // Space (or newline) before nested [[ should be collapsed
                    let ws_end = scan_all_whitespace_forward(bytes, open_end);
                    let (line, column) = source.offset_to_line_col(opening.start_offset());
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside array literal brackets detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: open_end,
                            end: ws_end,
                            replacement: String::new(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                } else if !multi_dim_left && !start_ok && !space_after_open {
                    // Non-nested: require space (like space style)
                    let (line, column) = source.offset_to_line_col(opening.start_offset());
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside array literal brackets missing.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: open_end,
                            end: open_end,
                            replacement: " ".to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }

                // Right side: whitespace check includes newlines for compact collapse
                let ws_before_close = close_start > 0
                    && matches!(
                        bytes.get(close_start - 1),
                        Some(b' ' | b'\t' | b'\n' | b'\r')
                    );

                if multi_dim_right && ws_before_close {
                    // Space (or newline) after nested ]] should be collapsed
                    let ws_start = scan_all_whitespace_backward(bytes, close_start);
                    let (line, column) = source.offset_to_line_col(closing.start_offset());
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside array literal brackets detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: ws_start,
                            end: close_start,
                            replacement: String::new(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                } else if !multi_dim_right && !end_ok && !space_before_close {
                    // Non-nested: require space (like space style)
                    let (line, column) = source.offset_to_line_col(closing.start_offset());
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside array literal brackets missing.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: close_start,
                            end: close_start,
                            replacement: " ".to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
            }
            _ => {}
        }
    }
}

/// Check if bytes between `start` and `end` contain only whitespace (spaces, tabs, newlines).
fn is_only_whitespace(bytes: &[u8], start: usize, end: usize) -> bool {
    bytes[start..end]
        .iter()
        .all(|&b| b == b' ' || b == b'\t' || b == b'\n' || b == b'\r')
}

/// Find the previous non-whitespace byte before `pos`.
fn prev_non_whitespace(bytes: &[u8], pos: usize) -> Option<u8> {
    let mut i = pos;
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => continue,
            byte => return Some(byte),
        }
    }
    None
}

/// Scan forward from `pos` past contiguous spaces/tabs. Returns the offset after the run.
fn scan_space_forward(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
        i += 1;
    }
    i
}

/// Scan backward from `pos` past contiguous spaces/tabs. Returns the offset at the start of the run.
fn scan_space_backward(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos;
    while i > 0 && matches!(bytes[i - 1], b' ' | b'\t') {
        i -= 1;
    }
    i
}

/// Check if the next non-whitespace character after `pos` is on a different line.
fn next_to_newline(bytes: &[u8], pos: usize) -> bool {
    let mut i = pos;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' => i += 1,
            b'\n' | b'\r' => return true,
            _ => return false,
        }
    }
    true // end of file
}

/// Check if the next non-whitespace character after `pos` is a `#` comment.
fn next_to_comment(bytes: &[u8], pos: usize) -> bool {
    let mut i = pos;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' => i += 1,
            b'#' => return true,
            _ => return false,
        }
    }
    false
}

/// Check if optional spaces/tabs are followed by a newline whose next line starts with `#`.
fn next_line_starts_with_comment(bytes: &[u8], pos: usize) -> bool {
    let mut i = pos;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
        i += 1;
    }

    if i >= bytes.len() {
        return false;
    }

    match bytes[i] {
        b'\n' => i += 1,
        b'\r' => {
            i += 1;
            if i < bytes.len() && bytes[i] == b'\n' {
                i += 1;
            }
        }
        _ => return false,
    }

    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
        i += 1;
    }

    bytes.get(i) == Some(&b'#')
}

/// Check if the next non-whitespace character after `pos` is `[`.
/// Skips all whitespace including newlines — RuboCop's `multi_dimensional_array?`
/// skips newline tokens, so brackets on different lines are still considered adjacent.
fn is_adjacent_bracket_forward(bytes: &[u8], pos: usize) -> bool {
    let mut i = pos;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b'[' => return true,
            _ => return false,
        }
    }
    false
}

/// Check if the previous non-whitespace character before `pos` is `]`
/// from a real bracket array (not `%w[...]`, `%i[...]`, etc.).
/// Skips all whitespace including newlines — RuboCop's `multi_dimensional_array?`
/// skips newline tokens, so brackets on different lines are still considered adjacent.
fn is_adjacent_bracket_backward(bytes: &[u8], pos: usize) -> bool {
    let mut i = pos;
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => continue,
            b']' => {
                // Found `]` — find its matching `[` via bracket counting,
                // then check it's not a %w[/%i[/etc. delimiter.
                let mut depth: usize = 1;
                let mut j = i;
                while j > 0 && depth > 0 {
                    j -= 1;
                    match bytes[j] {
                        b']' => depth += 1,
                        b'[' => depth -= 1,
                        _ => {}
                    }
                }
                if depth == 0 && j > 0 && bytes[j] == b'[' {
                    // Check if preceded by %<letter> (e.g. %w[, %i[, %I[, %W[)
                    if j >= 2 && bytes[j - 2] == b'%' && bytes[j - 1].is_ascii_alphabetic() {
                        return false;
                    }
                }
                return depth == 0;
            }
            _ => return false,
        }
    }
    false
}

/// Scan forward from `pos` past all whitespace including newlines.
fn scan_all_whitespace_forward(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

/// Scan backward from `pos` past all whitespace including newlines.
fn scan_all_whitespace_backward(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos;
    while i > 0 && matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'\r') {
        i -= 1;
    }
    i
}

/// Check if the position is the first non-whitespace on its line (raw byte scan).
fn begins_its_line_raw(bytes: &[u8], pos: usize) -> bool {
    if pos == 0 {
        return true;
    }
    let mut i = pos - 1;
    loop {
        match bytes[i] {
            b' ' | b'\t' => {
                if i == 0 {
                    return true;
                }
                i -= 1;
            }
            b'\n' => return true,
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(
        SpaceInsideArrayLiteralBrackets,
        "cops/layout/space_inside_array_literal_brackets"
    );
    crate::cop_autocorrect_fixture_tests!(
        SpaceInsideArrayLiteralBrackets,
        "cops/layout/space_inside_array_literal_brackets"
    );

    #[test]
    fn empty_brackets_space_style_flags_no_space() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleForEmptyBrackets".into(),
                serde_yml::Value::String("space".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"x = []\n";
        let diags = run_cop_full_with_config(&SpaceInsideArrayLiteralBrackets, src, config);
        assert_eq!(
            diags.len(),
            1,
            "space style should flag empty [] without space"
        );
    }

    #[test]
    fn empty_brackets_no_space_is_default() {
        use crate::testutil::run_cop_full;

        let src = b"x = []\n";
        let diags = run_cop_full(&SpaceInsideArrayLiteralBrackets, src);
        assert!(diags.is_empty(), "Default no_space should accept []");
    }

    #[test]
    fn array_pattern_no_space_flags_spaces() {
        use crate::testutil::run_cop_full;

        let src = b"case foo\nin [ bar, baz ]\nend\n";
        let diags = run_cop_full(&SpaceInsideArrayLiteralBrackets, src);
        assert_eq!(
            diags.len(),
            2,
            "array pattern with spaces should be flagged"
        );
    }

    #[test]
    fn array_pattern_no_space_accepts_no_spaces() {
        use crate::testutil::run_cop_full;

        let src = b"case foo\nin [bar, baz]\nend\n";
        let diags = run_cop_full(&SpaceInsideArrayLiteralBrackets, src);
        assert!(
            diags.is_empty(),
            "array pattern without spaces should not be flagged"
        );
    }

    #[test]
    fn empty_brackets_multiple_spaces_no_space_style() {
        use crate::testutil::run_cop_full;

        let src = b"x = [     ]\n";
        let diags = run_cop_full(&SpaceInsideArrayLiteralBrackets, src);
        assert_eq!(
            diags.len(),
            1,
            "empty brackets with multiple spaces should be flagged"
        );
        assert!(diags[0].message.contains("empty"));
    }

    #[test]
    fn multiline_empty_brackets_no_space_style() {
        use crate::testutil::run_cop_full;

        let src = b"x = [\n]\n";
        let diags = run_cop_full(&SpaceInsideArrayLiteralBrackets, src);
        assert_eq!(diags.len(), 1, "multiline empty brackets should be flagged");
        assert!(diags[0].message.contains("empty"));
    }

    fn compact_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("compact".into()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn compact_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &SpaceInsideArrayLiteralBrackets,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/space_inside_array_literal_brackets/compact_offense.rb"
            ),
            compact_config(),
        );
    }

    #[test]
    fn compact_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &SpaceInsideArrayLiteralBrackets,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/space_inside_array_literal_brackets/compact_no_offense.rb"
            ),
            compact_config(),
        );
    }

    #[test]
    fn compact_multiline_collapse_across_lines() {
        use crate::testutil::run_cop_full_with_config;
        // RuboCop's multi_dimensional_array? skips newline tokens, so brackets
        // on different lines ARE considered adjacent and should be collapsed.
        // Opening [ on one line, inner [ on the next → collapse offense.
        let src = b"multiline = [\n  [ 1, 2, 3, 4 ],\n  [ 3, 4, 5, 6 ]]\n";
        let diags =
            run_cop_full_with_config(&SpaceInsideArrayLiteralBrackets, src, compact_config());
        assert!(
            diags.iter().any(|d| d.message.contains("detected")),
            "multiline [ \\n [ SHOULD collapse across lines"
        );
        // Closing ] on one line, outer ] on the next → collapse offense.
        let src = b"multiline = [[ 1, 2, 3, 4 ],\n  [ 3, 4, 5, 6 ]\n]\n";
        let diags =
            run_cop_full_with_config(&SpaceInsideArrayLiteralBrackets, src, compact_config());
        assert!(
            diags.iter().any(|d| d.message.contains("detected")),
            "multiline ] \\n ] SHOULD collapse across lines"
        );
    }
}
