use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

fn collect_regular_interpolated_string_ranges(
    parse_result: &ruby_prism::ParseResult<'_>,
) -> Vec<(usize, usize)> {
    struct Collector {
        ranges: Vec<(usize, usize)>,
    }

    impl<'pr> Visit<'pr> for Collector {
        fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
            self.collect(&node);
        }

        fn visit_leaf_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
            self.collect(&node);
        }
    }

    impl Collector {
        fn collect(&mut self, node: &ruby_prism::Node<'_>) {
            let Some(interpolated) = node.as_interpolated_string_node() else {
                return;
            };
            let Some(opening) = interpolated.opening_loc() else {
                return;
            };
            if opening.as_slice().starts_with(b"<<") {
                return;
            }

            let location = node.location();
            self.ranges
                .push((location.start_offset(), location.end_offset()));
        }
    }

    let mut collector = Collector { ranges: Vec::new() };
    collector.visit(&parse_result.node());
    collector.ranges
}

fn range_within_ranges(ranges: &[(usize, usize)], start: usize, end: usize) -> bool {
    ranges
        .iter()
        .any(|&(range_start, range_end)| start >= range_start && end <= range_end)
}

/// Corpus investigation (2026-03-08, reverted):
/// A previous fix skipped offenses when a closing string delimiter preceded
/// the backslash, based on the incorrect assumption that RuboCop ignores these
/// via dstr expression ranges. In fact, RuboCop flags `"text"\` as an offense
/// in `space` style because the Parser gem's dstr node for implicit string
/// concatenation does NOT have `loc.begin` set, so it is not added to
/// `ignored_literal_ranges`. The skip logic was removed because it caused
/// 1,535 FNs (88.5% miss rate) by suppressing offenses on lines where a
/// string happened to precede the backslash continuation.
///
/// Corpus investigation (2026-03-25):
/// FP=37 all from CRLF files. RuboCop's `\\$` regex does not match `\<CR>`
/// because `$` matches before `\n`, not before `\r\n`. The fix: skip lines
/// where the backslash is followed by `\r` (CRLF line endings).
///
/// FN=16: 15 were already fixed in the current code (the code_map correctly
/// identifies line-continuation backslashes as code). 1 FN from a symbol
/// literal `:"a\\\nb"` — RuboCop flags it because `:sym` nodes are not in
/// `ignored_literal_ranges`, but the code_map marks it as non-code. To match
/// RuboCop, we use `is_heredoc()` instead of `is_code()` for the primary
/// skip check, and add a separate comment check, so that symbol interiors
/// are not incorrectly skipped.
///
/// Variant divergence (2026-04-09, `EnforcedStyle=no_space`):
/// OpenProject had 1 FP on `"..." \` inside `#{...}` of an enclosing regular
/// interpolated string. RuboCop ignores the full offense range because it is
/// contained by the outer `dstr` expression range (the string has `loc.begin`),
/// even though the backslash itself is in code. Match that range-based skip
/// without suppressing standalone continuations like `if 2 + 2  \`.
pub struct LineContinuationSpacing;

impl Cop for LineContinuationSpacing {
    fn name(&self) -> &'static str {
        "Layout/LineContinuationSpacing"
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        code_map: &CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let style = config.get_str("EnforcedStyle", "space");

        let content = source.as_bytes();
        let lines: Vec<&[u8]> = source.lines().collect();
        let interpolated_string_ranges = collect_regular_interpolated_string_ranges(parse_result);

        // Precompute byte offset of each line start
        let mut line_starts: Vec<usize> = Vec::with_capacity(lines.len());
        let mut offset = 0usize;
        for (i, line) in lines.iter().enumerate() {
            line_starts.push(offset);
            offset += line.len();
            if i < lines.len() - 1 || (offset < content.len() && content[offset] == b'\n') {
                offset += 1;
            }
        }

        for (i, &line) in lines.iter().enumerate() {
            // Strip trailing \r for content inspection, but remember if CRLF
            let has_cr = line.last() == Some(&b'\r');
            let trimmed_end = if has_cr {
                &line[..line.len() - 1]
            } else {
                line
            };

            if !trimmed_end.ends_with(b"\\") {
                continue;
            }

            // CRLF compatibility: RuboCop's regex `\\$` does not match `\<CR>`
            // because `$` matches before `\n`, not before `\r\n`. Skip CRLF
            // lines to avoid false positives on Windows-style line endings.
            if has_cr {
                continue;
            }

            let backslash_pos = trimmed_end.len() - 1;
            let offense_in_regular_interpolated_string = |start: usize, end: usize| {
                range_within_ranges(
                    &interpolated_string_ranges,
                    line_starts[i] + start,
                    line_starts[i] + end,
                )
            };

            // Skip backslashes inside heredoc bodies, comments, and the
            // __END__ data section. We intentionally do NOT use the broad
            // `code_map.is_code()` check here because it also covers symbol
            // interiors — RuboCop's `ignored_literal_ranges` does not include
            // `:sym` nodes, so symbol interiors should not be skipped.
            let backslash_offset = line_starts[i] + backslash_pos;
            if code_map.is_heredoc(backslash_offset) {
                continue;
            }
            // Comment or __END__ data section: non-code AND not inside a string/symbol
            if !code_map.is_code(backslash_offset) && code_map.is_not_string(backslash_offset) {
                continue;
            }
            // Inside a multiline string literal (not a symbol): the backslash
            // is part of the string content, not a line continuation.
            // Check: non-code, inside string_ranges, but NOT inside a symbol.
            // We detect symbols by checking if the non-code region starts with `:"`.
            if !code_map.is_code(backslash_offset)
                && !code_map.is_not_string(backslash_offset)
                && !code_map.is_heredoc(backslash_offset)
            {
                // Inside a string-like region (string, symbol, regexp, xstring).
                // RuboCop skips strings/regexps/xstrings (they have loc.begin in
                // ignored_literal_ranges) but NOT symbols. To distinguish, we check
                // if the region is a regex (skip) or look at source context.
                if code_map.is_regex(backslash_offset) {
                    continue;
                }
                // For non-regex string-like regions: skip unless it's a quoted
                // symbol (:"..." or :'...'). RuboCop's ignored_literal_ranges
                // iterates :str/:dstr/:array but NOT :sym/:dsym, so symbol
                // interiors are not skipped.
                let mut region_start = backslash_offset;
                while region_start > 0 && !code_map.is_code(region_start - 1) {
                    region_start -= 1;
                }
                let is_quoted_symbol = content[region_start] == b':'
                    && region_start + 1 < content.len()
                    && (content[region_start + 1] == b'"' || content[region_start + 1] == b'\'');
                if !is_quoted_symbol {
                    continue;
                }
            }

            match style {
                "space" => {
                    // Should have exactly one space before the backslash
                    if backslash_pos == 0 {
                        continue;
                    }
                    let before = trimmed_end[backslash_pos - 1];
                    if before != b' ' && before != b'\t' {
                        // No space before backslash
                        if offense_in_regular_interpolated_string(backslash_pos, backslash_pos + 1)
                        {
                            continue;
                        }
                        let line_num = i + 1;
                        diagnostics.push(self.diagnostic(
                            source,
                            line_num,
                            backslash_pos,
                            "Use one space before backslash.".to_string(),
                        ));
                    } else if backslash_pos >= 2
                        && (trimmed_end[backslash_pos - 2] == b' '
                            || trimmed_end[backslash_pos - 2] == b'\t')
                    {
                        // Multiple whitespace characters before backslash
                        let line_num = i + 1;
                        // Find start of whitespace
                        let mut space_start = backslash_pos - 1;
                        while space_start > 0
                            && (trimmed_end[space_start - 1] == b' '
                                || trimmed_end[space_start - 1] == b'\t')
                        {
                            space_start -= 1;
                        }
                        if offense_in_regular_interpolated_string(space_start, backslash_pos + 1) {
                            continue;
                        }
                        diagnostics.push(self.diagnostic(
                            source,
                            line_num,
                            space_start,
                            "Use one space before backslash.".to_string(),
                        ));
                    }
                }
                "no_space" => {
                    // Should have no space before the backslash
                    if backslash_pos > 0
                        && (trimmed_end[backslash_pos - 1] == b' '
                            || trimmed_end[backslash_pos - 1] == b'\t')
                    {
                        let line_num = i + 1;
                        let mut space_start = backslash_pos - 1;
                        while space_start > 0
                            && (trimmed_end[space_start - 1] == b' '
                                || trimmed_end[space_start - 1] == b'\t')
                        {
                            space_start -= 1;
                        }
                        if offense_in_regular_interpolated_string(space_start, backslash_pos + 1) {
                            continue;
                        }
                        diagnostics.push(self.diagnostic(
                            source,
                            line_num,
                            space_start,
                            "No space before backslash.".to_string(),
                        ));
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(
        LineContinuationSpacing,
        "cops/layout/line_continuation_spacing"
    );

    fn no_space_config() -> CopConfig {
        let mut config = CopConfig::default();
        config.options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::from("no_space"),
        );
        config
    }

    #[test]
    fn crlf_backslash_not_flagged() {
        // RuboCop's regex \\$ doesn't match \<CR> so CRLF lines are skipped
        let source = b"x = \"hello\"\\\r\n\"world\"\r\n";
        let diags = crate::testutil::run_cop_full_internal(
            &LineContinuationSpacing,
            source,
            CopConfig::default(),
            "test.rb",
        );
        assert!(
            diags.is_empty(),
            "CRLF backslash should not be flagged (RuboCop compat): got {} diagnostics",
            diags.len()
        );
    }

    #[test]
    fn crlf_no_space_style_not_flagged() {
        // no_space style should also skip CRLF lines
        let source = b"x = 1 \\\r\n  + 2\r\n";
        let diags = crate::testutil::run_cop_full_internal(
            &LineContinuationSpacing,
            source,
            no_space_config(),
            "test.rb",
        );
        assert!(
            diags.is_empty(),
            "CRLF backslash should not be flagged in no_space style: got {} diagnostics",
            diags.len()
        );
    }

    #[test]
    fn no_space_interpolated_outer_string_continuation_no_offense_fixture() {
        let fixture = include_bytes!(
            "../../../tests/fixtures/cops/layout/line_continuation_spacing/no_offense.no_space.rb"
        );
        let fixture = std::str::from_utf8(fixture).expect("fixture must be valid UTF-8");
        let source = fixture
            .strip_prefix("# nitrocop-config: EnforcedStyle: no_space\n")
            .expect("fixture should start with no_space config directive");

        crate::testutil::assert_cop_no_offenses_full_with_config(
            &LineContinuationSpacing,
            source.as_bytes(),
            no_space_config(),
        );
    }

    #[test]
    fn no_space_general_continuation_still_flagged() {
        let source = b"if 2 + 2  \\\n  == 4\n  foo\nend\n";
        let diags = crate::testutil::run_cop_full_internal(
            &LineContinuationSpacing,
            source,
            no_space_config(),
            "test.rb",
        );

        assert_eq!(
            diags.len(),
            1,
            "general no_space case should still be flagged"
        );
        assert_eq!(diags[0].location.line, 1);
        assert_eq!(diags[0].location.column, 8);
        assert_eq!(diags[0].message, "No space before backslash.");
    }

    #[test]
    fn symbol_interior_flagged() {
        // RuboCop does not add :sym nodes to ignored_literal_ranges,
        // so backslashes inside symbols are flagged
        let source = b":\"a\\\\\nb\"\n";
        let diags = crate::testutil::run_cop_full_internal(
            &LineContinuationSpacing,
            source,
            CopConfig::default(),
            "test.rb",
        );
        assert_eq!(
            diags.len(),
            1,
            "Should flag backslash inside symbol (RuboCop compat)"
        );
    }

    #[test]
    fn chained_method_backslash_flagged() {
        // Backslash with no space in chained method calls
        let source = b"Dir.entries(dir)\\\n        .select {|s| s }\n";
        let diags = crate::testutil::run_cop_full_internal(
            &LineContinuationSpacing,
            source,
            CopConfig::default(),
            "test.rb",
        );
        assert!(
            !diags.is_empty(),
            "Should flag backslash with no space after )"
        );
    }

    #[test]
    fn string_then_unless_backslash_flagged() {
        // Backslash after closing quote with postfix unless
        let source = b"raise ArgumentError, \"not directory: #{dir}\"\\\n  unless cond\n";
        let diags = crate::testutil::run_cop_full_internal(
            &LineContinuationSpacing,
            source,
            CopConfig::default(),
            "test.rb",
        );
        assert!(
            !diags.is_empty(),
            "Should flag backslash with no space after closing quote"
        );
    }
}
