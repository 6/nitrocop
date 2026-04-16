use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// ## Corpus investigation
///
/// FN fix: was using `is_code()` to skip non-code regions, which excluded
/// `=begin`/`=end` multi-line comment blocks. RuboCop only skips string
/// literals (via `string_literal_ranges`), not comments. Changed to
/// `is_not_string()` to match RuboCop's behavior. This fixed 225 FN across
/// 8 corpus repos (WhatWeb: 136, greasyfork: 58, others: 31).
///
/// ## Corpus investigation (2026-03-17, FN=73)
///
/// 73 FN on heredoc closing delimiters with tab indentation (e.g., `\tSQL`).
/// Root cause: CodeMap maps heredoc ranges including the closing delimiter,
/// so `is_not_string()` returned false and the line was skipped. In Parser
/// gem, the closing delimiter is a separate `:tSTRING_END` token NOT
/// included in `string_literal_ranges`, so RuboCop checks its indentation.
/// Fix: detect heredoc closing delimiter lines (inside heredoc range,
/// content is just an identifier) and still check their indentation.
///
/// ## Corpus investigation (2026-03-17, FP=69)
///
/// 69 FP on tab-indented heredoc content lines (not the closing delimiter).
/// Root cause: `is_heredoc_closing_delimiter()` was using content-pattern
/// matching (whitespace + identifier), which matched short content lines
/// like `y`, `end`, `SQL` etc. inside heredoc bodies. These were incorrectly
/// treated as closing delimiters and flagged.
/// Fix: replaced pattern-matching heuristic with positional check — a line
/// is a closing delimiter only if it's the LAST line within its heredoc range
/// (i.e., the next line's start offset falls outside the heredoc range).
/// Added `CodeMap::heredoc_range_end()` to support this check.
///
/// ## Corpus investigation (2026-03-31, FP=4)
///
/// 4 FP on multiline regular interpolated strings (`"...#{ ... }..."`).
/// Root cause: `CodeMap` intentionally treats `#{}` bodies as code, but
/// RuboCop suppresses `Layout/IndentationStyle` when the matched leading
/// whitespace is contained by the enclosing non-heredoc `dstr` expression.
/// That means tab-indented lines inside interpolation bodies, and on the line
/// starting with the closing `}`, should NOT be flagged.
/// Fix: collect non-heredoc `InterpolatedStringNode` expression ranges in this
/// cop and skip indentation checks when the leading whitespace falls entirely
/// inside one of those ranges.
///
/// ## Corpus investigation (2026-04-08, tabs variant: FP=1165, FN=15918)
///
/// Three root causes for the tabs variant divergence:
///
/// 1. **FP on squiggly heredoc interpolation content**: For `<<~` heredocs with
///    `#{}` interpolation, Prism strips common indent from parts, so the
///    CodeMap's heredoc body range starts AFTER the leading whitespace. The cop
///    incorrectly flagged the whitespace as "space in indentation" because
///    `is_not_string(line_start)` returned true.
///    Fix: after the existing skip logic, check if the first non-whitespace byte
///    is inside a heredoc body (via `code_map.is_heredoc(line_start + indent_end)`).
///
/// 2. **FN on xstring (backtick) content**: RuboCop's `string_literal_ranges`
///    only collects `:str`/`:dstr`, NOT `:xstr`. But nitrocop's CodeMap included
///    `XStringNode`/`InterpolatedXStringNode` in `string_ranges`, causing the cop
///    to skip xstring content.
///    Fix: added `CodeMap::is_xstring()` and `xstring_ranges` field, then added
///    `!code_map.is_xstring()` overrides in the skip logic (same pattern as
///    `is_regex`).
///
/// 3. **FN on stacked heredoc closing delimiters**: When two heredocs are opened
///    on the same line (`method(<<-A, <<-B)`), their ranges are adjacent and
///    `merge_ranges` combines them. `heredoc_range_end()` then returns the END
///    of the merged range, so the first closing delimiter's `next_line_start >=
///    range_end` check fails.
///    Fix: added `raw_heredoc_ranges` (sorted but unmerged) to CodeMap and
///    switched `heredoc_range_end()` to use it.
///
/// ## Corpus investigation (2026-04-16, tabs variant: FP=19, FN=10)
///
/// Nested heredoc closing delimiters inside an enclosing heredoc interpolation
/// diverged in both directions under `EnforcedStyle: tabs`.
///
/// 1. **FP on inner closing delimiters**: lines like the inner `HTML` in
///    `<<~OUTER ... #{<<~INNER ... INNER} ... OUTER` were flagged even though
///    RuboCop suppresses them because their leading spaces are still inside the
///    outer heredoc body range.
///
/// 2. **FN on outer closing delimiters**: the matching outer `OUTER` line was
///    sometimes missed because `CodeMap::heredoc_range_end()` used a binary
///    search that assumed raw heredoc ranges never overlap. Nested heredocs
///    violate that assumption, so the lookup could resolve the wrong range.
///
/// Fix: make `heredoc_range_end()` return the OUTERMOST raw heredoc containing
/// the line start. That mirrors RuboCop's `string_literal_ranges`: inner
/// closing delimiters stay suppressed by the outer heredoc body, while the
/// outer closing delimiter is still checked.
pub struct IndentationStyle;

impl Cop for IndentationStyle {
    fn name(&self) -> &'static str {
        "Layout/IndentationStyle"
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        code_map: &CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let style = config.get_str("EnforcedStyle", "spaces");
        let indent_width = config.get_usize("IndentationWidth", 2);
        let interpolated_string_ranges = regular_interpolated_string_ranges(parse_result);

        let mut offset = 0;

        for (i, line) in source.lines().enumerate() {
            let line_num = i + 1;
            let line_start = offset;
            // Advance offset past this line and its newline
            offset += line.len() + 1; // +1 for the '\n' delimiter
            let indent_end = line
                .iter()
                .take_while(|&&b| b == b' ' || b == b'\t')
                .count();
            let is_heredoc_closing = is_heredoc_closing_delimiter(line, code_map, line_start);
            let in_interpolated_string = indent_end > 0
                && range_contained_in_any(
                    &interpolated_string_ranges,
                    line_start,
                    line_start + indent_end,
                );

            // Skip lines whose indentation starts in a string/heredoc region.
            // RuboCop checks indentation in comments (including =begin/=end blocks)
            // but skips string literals, so use is_not_string() instead of is_code().
            // Exception 1: heredoc closing delimiters (e.g., `\tSQL`) are NOT skipped.
            // In Parser gem, the closing delimiter is a separate :tSTRING_END token
            // outside the string_literal_range, so RuboCop checks its indentation.
            // Exception 2: regex literals are NOT skipped. RuboCop's
            // string_literal_ranges only covers :str/:dstr nodes, not :regexp.
            // Exception 3: xstring (backtick) literals are NOT skipped. RuboCop's
            // string_literal_ranges only covers :str/:dstr, not :xstr.
            if (!code_map.is_not_string(line_start) || in_interpolated_string)
                && !code_map.is_regex(line_start)
                && !code_map.is_xstring(line_start)
                && !is_heredoc_closing
            {
                continue;
            }

            // For <<~ (squiggly) heredocs with interpolation, Prism strips
            // common indent from parts, so the CodeMap's heredoc body range may
            // start AFTER the leading whitespace. Check if the first non-whitespace
            // byte is inside a str/dstr heredoc body (not regex or xstring).
            if indent_end > 0
                && indent_end < line.len()
                && !is_heredoc_closing
                && code_map.is_heredoc(line_start + indent_end)
                && !code_map.is_regex(line_start + indent_end)
                && !code_map.is_xstring(line_start + indent_end)
            {
                continue;
            }

            if style == "spaces" {
                // Flag tabs in indentation
                let indent = &line[..indent_end];
                if indent.contains(&b'\t') {
                    let tab_col = indent.iter().position(|&b| b == b'\t').unwrap_or(0);
                    let tab_offset = line_start + tab_col;
                    // Double-check the specific tab character is not in a string literal.
                    // Exceptions: heredoc closing delimiters, regex, and xstring
                    // literals are checked even though they're inside ranges in
                    // the CodeMap.
                    if code_map.is_not_string(tab_offset)
                        || code_map.is_regex(tab_offset)
                        || code_map.is_xstring(tab_offset)
                        || is_heredoc_closing
                    {
                        let mut diag = self.diagnostic(
                            source,
                            line_num,
                            tab_col,
                            "Tab detected in indentation.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            // Calculate visual width of the mixed indent region
                            let visual_width = indent.iter().fold(0usize, |w, &b| {
                                if b == b'\t' {
                                    (w / indent_width + 1) * indent_width
                                } else {
                                    w + 1
                                }
                            });
                            corr.push(crate::correction::Correction {
                                start: line_start,
                                end: line_start + indent_end,
                                replacement: " ".repeat(visual_width),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                }
            } else {
                // "tabs" — flag spaces in indentation
                let indent = &line[..indent_end];
                if indent.contains(&b' ') {
                    let space_col = indent.iter().position(|&b| b == b' ').unwrap_or(0);
                    let space_offset = line_start + space_col;
                    if code_map.is_not_string(space_offset)
                        || code_map.is_regex(space_offset)
                        || code_map.is_xstring(space_offset)
                        || is_heredoc_closing
                    {
                        let mut diag = self.diagnostic(
                            source,
                            line_num,
                            space_col,
                            "Space detected in indentation.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            // Count leading spaces and convert to tabs
                            let space_count = indent.iter().filter(|&&b| b == b' ').count();
                            let tab_count = indent.iter().filter(|&&b| b == b'\t').count();
                            let total_tabs = tab_count + space_count / indent_width;
                            let remaining_spaces = space_count % indent_width;
                            let mut replacement = "\t".repeat(total_tabs);
                            replacement.push_str(&" ".repeat(remaining_spaces));
                            corr.push(crate::correction::Correction {
                                start: line_start,
                                end: line_start + indent_end,
                                replacement,
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                }
            }
        }
    }
}

#[derive(Default)]
struct InterpolatedStringRangeCollector {
    ranges: Vec<(usize, usize)>,
}

impl<'pr> Visit<'pr> for InterpolatedStringRangeCollector {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        if let Some(string) = node.as_interpolated_string_node() {
            if let Some(opening) = string.opening_loc() {
                if !opening.as_slice().starts_with(b"<<") {
                    let loc = node.location();
                    self.ranges.push((loc.start_offset(), loc.end_offset()));
                }
            }
        }
    }
}

fn regular_interpolated_string_ranges(
    parse_result: &ruby_prism::ParseResult<'_>,
) -> Vec<(usize, usize)> {
    let mut collector = InterpolatedStringRangeCollector::default();
    collector.visit(&parse_result.node());
    collector.ranges.sort_unstable();
    collector.ranges
}

fn range_contained_in_any(ranges: &[(usize, usize)], start: usize, end: usize) -> bool {
    ranges
        .iter()
        .any(|&(range_start, range_end)| start >= range_start && end <= range_end)
}

/// Check if a line is a heredoc closing delimiter.
/// The closing delimiter is the last line within a heredoc range. We detect this
/// by checking whether the next line's start offset falls outside the heredoc range.
/// This is more reliable than pattern-matching on content, which can false-positive
/// on short content lines like `y` or `end` that look like identifiers.
///
/// In Parser gem, the closing delimiter is a `:tSTRING_END` token and is NOT
/// included in `string_literal_ranges`, so RuboCop checks its indentation.
fn is_heredoc_closing_delimiter(line: &[u8], code_map: &CodeMap, line_start: usize) -> bool {
    // Must be inside a heredoc range
    let range_end = match code_map.heredoc_range_end(line_start) {
        Some(end) => end,
        None => return false,
    };

    // The closing delimiter line is the last line in the heredoc range.
    // The next line starts at line_start + line.len() + 1 (for the newline).
    // If that offset is >= the heredoc range end, this is the closing delimiter.
    let next_line_start = line_start + line.len() + 1;
    next_line_start >= range_end
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(IndentationStyle, "cops/layout/indentation_style");
    crate::cop_autocorrect_fixture_tests!(IndentationStyle, "cops/layout/indentation_style");

    #[test]
    fn tabs_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &IndentationStyle,
            include_bytes!("../../../tests/fixtures/cops/layout/indentation_style/tabs_offense.rb"),
            tabs_config(),
        );
    }

    #[test]
    fn tabs_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &IndentationStyle,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/indentation_style/tabs_no_offense.rb"
            ),
            tabs_config(),
        );
    }

    fn tabs_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("tabs".into()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn tabs_variant_squiggly_heredoc_interpolation_not_flagged() {
        // FP fix: <<~ heredoc with interpolation — leading whitespace is outside
        // the CodeMap body range because Prism strips common indent from parts
        let source = b"expect(x).to eq(y), <<~SPEC_FAILURE\n  \x23{model} text\n  expected: \x23{x}\nSPEC_FAILURE\n";
        let diags =
            crate::testutil::run_cop_full_with_config(&IndentationStyle, source, tabs_config());
        // Lines 2 and 3 are heredoc content — should not be flagged
        let flagged_lines: Vec<usize> = diags.iter().map(|d| d.location.line).collect();
        assert!(
            !flagged_lines.contains(&2),
            "Heredoc body line 2 should not be flagged: {:?}",
            diags
        );
        assert!(
            !flagged_lines.contains(&3),
            "Heredoc body line 3 should not be flagged: {:?}",
            diags
        );
    }

    #[test]
    fn tabs_variant_xstring_content_flagged() {
        // FN fix: RuboCop does NOT skip xstring (backtick) content
        let source = b"x = `\n  command\n`\n";
        let diags =
            crate::testutil::run_cop_full_with_config(&IndentationStyle, source, tabs_config());
        let flagged_lines: Vec<usize> = diags.iter().map(|d| d.location.line).collect();
        assert!(
            flagged_lines.contains(&2),
            "xstring content should be flagged: {:?}",
            diags
        );
    }

    #[test]
    fn tabs_variant_heredoc_xstring_content_flagged() {
        // FN fix: RuboCop does NOT skip heredoc xstring content
        let source = b"x = <<~`CMD`\n  echo hello\nCMD\n";
        let diags =
            crate::testutil::run_cop_full_with_config(&IndentationStyle, source, tabs_config());
        let flagged_lines: Vec<usize> = diags.iter().map(|d| d.location.line).collect();
        assert!(
            flagged_lines.contains(&2),
            "heredoc xstring content should be flagged: {:?}",
            diags
        );
    }

    #[test]
    fn tabs_variant_stacked_heredoc_closing_delimiter() {
        // FN fix: stacked heredoc closing delimiters with space indentation
        let source = b"method(<<-A, <<-B)\n  body a\n  A\n  body b\n  B\n";
        let diags =
            crate::testutil::run_cop_full_with_config(&IndentationStyle, source, tabs_config());
        let flagged_lines: Vec<usize> = diags.iter().map(|d| d.location.line).collect();
        assert!(
            flagged_lines.contains(&3),
            "Closing delimiter A should be flagged: {:?}",
            diags
        );
        assert!(
            flagged_lines.contains(&5),
            "Closing delimiter B should be flagged: {:?}",
            diags
        );
    }

    #[test]
    fn tabs_variant_heredoc_closing_delimiter_spaces() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("tabs".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"x = <<-INPUT\n  text here\n  INPUT\n";
        let diags = crate::testutil::run_cop_full_with_config(&IndentationStyle, source, config);
        for d in &diags {
            eprintln!(
                "DIAG: line={} col={} msg={}",
                d.location.line, d.location.column, d.message
            );
        }
        let closing_line_flagged = diags.iter().any(|d| d.location.line == 3);
        assert!(
            closing_line_flagged,
            "Should flag spaces in heredoc closing delimiter: {:?}",
            diags
        );
    }

    #[test]
    fn heredoc_closing_tag_tab() {
        // Tab-indented heredoc closing tag should be flagged
        let source = b"execute <<-SQL\n\tSELECT * FROM users\n\tSQL\n";
        let diags = crate::testutil::run_cop_full(&IndentationStyle, source);
        assert!(
            !diags.is_empty(),
            "Should flag tab in heredoc closing tag indentation"
        );
        assert_eq!(
            diags.len(),
            1,
            "Only the closing tag tab, not heredoc content: {:?}",
            diags
        );
    }

    #[test]
    fn heredoc_squiggly_content_tabs_not_flagged() {
        // Tab-indented heredoc content in a <<~ heredoc should NOT be flagged.
        // This reproduces the phlex FP pattern where a tab-indented file uses
        // <<~RUBY heredocs and the content lines have tab indentation.
        let source = b"\t\timg: <<~RUBY,\n\t\t\tif true\n\t\t\t\ty\n\t\t\tend\n\t\tRUBY\n";
        let diags = crate::testutil::run_cop_full(&IndentationStyle, source);
        // The opening line ("\t\timg: <<~RUBY,") has a tab indent in code — flagged.
        // The closing delimiter ("\t\tRUBY") is a heredoc closing tag — flagged.
        // The content lines ("\t\t\tif true", etc.) are inside the heredoc — NOT flagged.
        let flagged_lines: Vec<usize> = diags.iter().map(|d| d.location.line).collect();
        assert!(
            !flagged_lines.contains(&2),
            "Heredoc content line 2 should not be flagged: {:?}",
            diags
        );
        assert!(
            !flagged_lines.contains(&3),
            "Heredoc content line 3 should not be flagged: {:?}",
            diags
        );
        assert!(
            !flagged_lines.contains(&4),
            "Heredoc content line 4 should not be flagged: {:?}",
            diags
        );
    }

    #[test]
    fn heredoc_interpolated_content_tabs_not_flagged() {
        // Interpolated heredoc content should not be flagged either.
        let source = b"\t\tx = <<~RUBY\n\t\t\tval = #{foo}\n\t\tRUBY\n";
        let diags = crate::testutil::run_cop_full(&IndentationStyle, source);
        let flagged_lines: Vec<usize> = diags.iter().map(|d| d.location.line).collect();
        assert!(
            !flagged_lines.contains(&2),
            "Interpolated heredoc content line 2 should not be flagged: {:?}",
            diags
        );
    }

    #[test]
    fn regex_mixed_tab_in_indent() {
        // Reproduces the rouge-ruby/rouge FN: tab mixed with spaces in a multi-line regex
        let source = b"KEYWORDS = /( bool       | byte       | complex64\n             | complex128 | error      | float32\n      \t                       | float64    | int8       | int16\n             | int32      | int64      | int\n             | rune       | string     | uint8\n             | uint16     | uint32     | uint64\n             | uintptr    | uint\n      \t                       )\\b/x\n";
        let diags = crate::testutil::run_cop_full(&IndentationStyle, source);
        let flagged_lines: Vec<usize> = diags.iter().map(|d| d.location.line).collect();
        assert!(
            flagged_lines.contains(&3),
            "Line 3 (mixed tab) should be flagged: {:?}",
            diags
        );
        assert!(
            flagged_lines.contains(&8),
            "Line 8 (mixed tab) should be flagged: {:?}",
            diags
        );
    }

    #[test]
    fn autocorrect_tab_to_spaces() {
        let input = b"\tx = 1\n";
        let (_diags, corrections) = crate::testutil::run_cop_autocorrect(&IndentationStyle, input);
        assert!(!corrections.is_empty());
        let cs = crate::correction::CorrectionSet::from_vec(corrections);
        let corrected = cs.apply(input);
        assert_eq!(corrected, b"  x = 1\n");
    }

    #[test]
    fn autocorrect_spaces_to_tab() {
        use std::collections::HashMap;
        let config = crate::cop::CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("tabs".into()),
            )]),
            ..crate::cop::CopConfig::default()
        };
        let input = b"  x = 1\n";
        let (_diags, corrections) =
            crate::testutil::run_cop_autocorrect_with_config(&IndentationStyle, input, config);
        assert!(!corrections.is_empty());
        let cs = crate::correction::CorrectionSet::from_vec(corrections);
        let corrected = cs.apply(input);
        assert_eq!(corrected, b"\tx = 1\n");
    }
}
