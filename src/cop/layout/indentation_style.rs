use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::codemap::CodeMap;
use crate::parse::directives::normalize_directive_cop_name;
use crate::parse::source::SourceFile;
use regex::Regex;
use ruby_prism::Visit;
use std::sync::LazyLock;

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
/// ## Variant `tabs`: remaining 19 FP / 10 FN (DO NOT touch `heredoc_range_end` or
/// `raw_heredoc_ranges` to fix these — three separate agent attempts at this have
/// all produced +20,000+ FP regressions by inverting the skip-zone logic)
///
/// Closed PRs with identical catastrophic regression pattern:
/// - #2141 (Apr 16): +20,758 FP from `.max(end)` expanding skip zone
/// - #2159 (Apr 16): same +20,758 FP via "outermost heredoc" loop change
/// - #2183 (Apr 16): same +20,758 FP via different heredoc range tweak
///
/// What DOES NOT work: changing `heredoc_range_end()` from binary-search to any
/// kind of "outermost enclosing" lookup. The lookups consistently invert, so
/// nested-heredoc closing delimiters inside outer heredoc bodies get flagged
/// AND legitimate indent lines get suppressed across the entire heredoc span.
///
/// What a correct fix probably requires: a separate per-line "inside-heredoc-body"
/// check that walks the nesting chain from innermost to outermost without altering
/// `raw_heredoc_ranges` semantics. Verify with RuboCop on files like
/// `thoughtbot__shoulda-matchers__f147e7b:lib/shoulda/matchers/rails_shim.rb:160-164`
/// (nested heredoc interpolation) before touching `codemap.rs`.
///
/// ## Variant `tabs`: remaining 19 FP / 10 FN (2026-04-16, fixed)
///
/// Root cause: the `tabs` path was using `CodeMap`'s broader string/heredoc ranges,
/// then overriding them with `is_heredoc_closing_delimiter()`. That diverged from
/// RuboCop in two opposite ways:
///
/// 1. Nested heredoc closers inside an OUTER heredoc body were falsely flagged,
///    because the inner closing delimiter bypassed the broader skip even though
///    RuboCop still sees the matched indentation range as contained in the outer
///    heredoc body.
/// 2. Outer closing delimiters in nested/stacked heredocs were missed, because the
///    raw-range lookup was only suitable for the earlier closing-delimiter heuristic,
///    not for RuboCop's actual range-containment check.
///
/// Fix: keep the existing `spaces` implementation unchanged, but for `tabs` build
/// RuboCop-style ignored ranges directly from Prism AST nodes:
/// - regular `str`/`dstr`: full expression range
/// - heredocs: body only, from the line start of the FIRST actual content/interpolation
///   part to the line start of the closing delimiter
/// - `__END__` data section: skipped like before
///
/// This matches RuboCop for nested, squiggly, and stacked heredocs without changing
/// `CodeMap` or `heredoc_range_end()` semantics, and keeps default-style behavior
/// untouched.
///
/// Follow-up (same variant): two remaining mismatches were outside the heredoc-body
/// range math itself:
/// - Prism reports `data_loc` even when a file starts with `__END__`, but RuboCop
///   still checks indentation after a top-level leading `__END__`. Only skip the
///   data section when some non-whitespace content precedes it.
/// - YARD/example comment lines like `#   # rubocop:disable all` suppress that line
///   in RuboCop even though nitrocop's general directive parser intentionally ignores
///   them as block directives. For `tabs`, suppress that single line locally before
///   reporting an indentation offense.
pub struct IndentationStyle;

static NESTED_COMMENT_DIRECTIVE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#\s*(?:rubocop|nitrocop)\s*:\s*(disable|todo)\s+(.+)").unwrap());

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
        let interpolated_string_ranges = if style == "spaces" {
            regular_interpolated_string_ranges(parse_result)
        } else {
            Vec::new()
        };
        let rubocop_string_ranges = if style == "tabs" {
            rubocop_string_literal_ranges(source.as_bytes(), parse_result)
        } else {
            Vec::new()
        };
        let tabs_nested_directive_lines = if style == "tabs" {
            tabs_nested_directive_comment_lines(source)
        } else {
            Vec::new()
        };

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
            if style == "spaces" {
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
                if let Some(space_col) = indent.iter().position(|&b| b == b' ') {
                    if tabs_nested_directive_lines.contains(&line_num) {
                        continue;
                    }

                    // Match RuboCop's /\A\s* +/ skip semantics: suppress only when
                    // the full matched indentation range is contained in a :str/:dstr
                    // range (including outer heredoc bodies), not merely when a single
                    // space falls inside the broader CodeMap string range.
                    let match_end = indent.iter().rposition(|&b| b == b' ').unwrap_or(0) + 1;
                    if range_contained_in_any(
                        &rubocop_string_ranges,
                        line_start,
                        line_start + match_end,
                    ) {
                        continue;
                    }

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

#[derive(Default)]
struct RubocopStringRangeCollector<'a> {
    source: &'a [u8],
    ranges: Vec<(usize, usize)>,
}

impl<'a, 'pr> Visit<'pr> for RubocopStringRangeCollector<'a> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.collect(&node);
    }

    fn visit_leaf_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.collect(&node);
    }
}

impl RubocopStringRangeCollector<'_> {
    fn collect(&mut self, node: &ruby_prism::Node<'_>) {
        if let Some(string) = node.as_string_node() {
            let Some(opening) = string.opening_loc() else {
                return;
            };
            if opening.as_slice().starts_with(b"<<") {
                if let Some(close) = string.closing_loc() {
                    if let Some(range) = heredoc_body_range(
                        self.source,
                        string.content_loc().start_offset(),
                        close.start_offset(),
                    ) {
                        self.ranges.push(range);
                    }
                }
            } else {
                let loc = node.location();
                self.ranges.push((loc.start_offset(), loc.end_offset()));
            }
            return;
        }

        let Some(string) = node.as_interpolated_string_node() else {
            return;
        };
        let Some(opening) = string.opening_loc() else {
            return;
        };

        if opening.as_slice().starts_with(b"<<") {
            if let Some(close) = string.closing_loc() {
                if let Some(first_part) = string.parts().iter().next() {
                    if let Some(range) = heredoc_body_range(
                        self.source,
                        first_part.location().start_offset(),
                        close.start_offset(),
                    ) {
                        self.ranges.push(range);
                    }
                }
            }
        } else {
            let loc = node.location();
            self.ranges.push((loc.start_offset(), loc.end_offset()));
        }
    }
}

fn rubocop_string_literal_ranges(
    source: &[u8],
    parse_result: &ruby_prism::ParseResult<'_>,
) -> Vec<(usize, usize)> {
    let mut collector = RubocopStringRangeCollector {
        source,
        ranges: Vec::new(),
    };
    collector.visit(&parse_result.node());
    if let Some(data_loc) = parse_result.data_loc() {
        // RuboCop only treats __END__ as a skipped data section when some
        // non-whitespace content precedes it. A file that starts with __END__
        // is still checked line-by-line under EnforcedStyle: tabs.
        if source[..data_loc.start_offset()]
            .iter()
            .any(|b| !b.is_ascii_whitespace())
        {
            collector
                .ranges
                .push((data_loc.start_offset(), data_loc.end_offset()));
        }
    }
    collector.ranges.sort_unstable();
    collector.ranges
}

fn heredoc_body_range(
    source: &[u8],
    first_content_start: usize,
    closing_start: usize,
) -> Option<(usize, usize)> {
    let body_start = line_start(source, first_content_start);
    let body_end = line_start(source, closing_start);
    (body_start < body_end).then_some((body_start, body_end))
}

fn line_start(source: &[u8], offset: usize) -> usize {
    source[..offset]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |pos| pos + 1)
}

fn tabs_nested_directive_comment_lines(source: &SourceFile) -> Vec<usize> {
    let mut lines = Vec::new();

    for (i, line) in source.lines().enumerate() {
        if nested_tabs_directive_comment_applies(line) {
            lines.push(i + 1);
        }
    }

    lines
}

fn nested_tabs_directive_comment_applies(line: &[u8]) -> bool {
    let indent_end = line
        .iter()
        .take_while(|&&b| b == b' ' || b == b'\t')
        .count();
    if indent_end >= line.len() || line[indent_end] != b'#' {
        return false;
    }

    let Ok(comment) = std::str::from_utf8(&line[indent_end..]) else {
        return false;
    };
    let Some(caps) = NESTED_COMMENT_DIRECTIVE_RE.captures(&comment[1..]) else {
        return false;
    };

    let cop_list_raw = caps.get(2).map_or("", |m| m.as_str());
    let cop_list = cop_list_raw.split("--").next().unwrap_or(cop_list_raw);
    cop_list.split(',').any(|cop| {
        matches!(
            normalize_directive_cop_name(cop.trim()).as_str(),
            "all"
                | "Layout"
                | "Layout/IndentationStyle"
                | "IndentationStyle"
                | "Layout/Tab"
                | "Tab"
        )
    })
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
    fn tabs_variant_nested_disable_comment_line_not_flagged() {
        let source = b"def foo\n  #   # rubocop:disable all\nend\n";
        let diags =
            crate::testutil::run_cop_full_with_config(&IndentationStyle, source, tabs_config());
        let flagged_lines: Vec<usize> = diags.iter().map(|d| d.location.line).collect();
        assert!(
            !flagged_lines.contains(&2),
            "nested disable comment line should not be flagged: {:?}",
            diags
        );
    }

    #[test]
    fn tabs_variant_nested_enable_comment_line_flagged() {
        let source = b"def foo\n  #   # rubocop:enable all\nend\n";
        let diags =
            crate::testutil::run_cop_full_with_config(&IndentationStyle, source, tabs_config());
        let flagged_lines: Vec<usize> = diags.iter().map(|d| d.location.line).collect();
        assert!(
            flagged_lines.contains(&2),
            "nested enable comment line should still be flagged: {:?}",
            diags
        );
    }

    #[test]
    fn tabs_variant_leading_end_marker_still_checked() {
        let source = b"__END__\n  data\n";
        let diags =
            crate::testutil::run_cop_full_with_config(&IndentationStyle, source, tabs_config());
        let flagged_lines: Vec<usize> = diags.iter().map(|d| d.location.line).collect();
        assert!(
            flagged_lines.contains(&2),
            "leading __END__ should not suppress indentation checks: {:?}",
            diags
        );
    }

    #[test]
    fn tabs_variant_data_section_after_nonblank_content_skipped() {
        let source = b"# comment\n__END__\n  data\n";
        let diags =
            crate::testutil::run_cop_full_with_config(&IndentationStyle, source, tabs_config());
        let flagged_lines: Vec<usize> = diags.iter().map(|d| d.location.line).collect();
        assert!(
            !flagged_lines.contains(&3),
            "__END__ after nonblank content should still suppress data section: {:?}",
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
