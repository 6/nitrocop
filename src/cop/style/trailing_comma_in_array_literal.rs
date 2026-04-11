use crate::cop::shared::node_type::ARRAY_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

use super::trailing_comma;

/// Style/TrailingCommaInArrayLiteral — checks trailing commas in array literals.
///
/// ## Heredoc handling
/// When arrays contain heredoc elements, Prism's `location().end_offset()` for
/// the last element points to the end of the heredoc opening tag (e.g., after
/// `<<~STR.chomp`), NOT the closing terminator. The heredoc body and terminator
/// sit between `last_end` and `closing_start` (`]`).
///
/// **Root cause of FPs:** Previous approach scanned from start of `]`'s line,
/// which could pick up commas inside heredoc content or terminators.
///
/// **Root cause of FNs:** Previous approach for multiline heredoc arrays scanned
/// from start of `]`'s line, missing the trailing comma on the heredoc opening
/// line (e.g., `<<~STR.chomp,`).
///
/// **Fix:** When heredocs are present, scan from `last_end` but stop at the
/// first newline (matching RuboCop's `/\A[^\S\n]*,/` regex). This finds commas
/// on the heredoc opening line without entering heredoc content.
///
/// ## Nested heredoc FPs (2026-03)
/// When the last element of an outer array is a sub-array containing a heredoc
/// (e.g., `["foo.rb", <<-EOS]`), the heredoc body sits between the sub-array's
/// `end_offset()` and the outer `]`. The `any_heredoc` check must recurse into
/// sub-arrays to detect these nested heredocs, otherwise heredoc content gets
/// scanned for commas producing false positives. Seen in zeitwerk, rufo, thredded.
///
/// ## Variant divergence (2026-04)
/// The remaining `diff_comma` corpus drift came from two array-only issues:
/// the variant harness can arrive through the generic `EnforcedStyle` key, and
/// Windows line endings use `\r\n` instead of `\n`. That meant the `diff_comma`
/// branch sometimes fell back to `no_comma`, and even when the style was set
/// explicitly it treated `,\r\n]` as an illegal trailing comma while missing
/// required commas before `\r\n]`.
///
/// Fix: fall back to `EnforcedStyle` when `EnforcedStyleForMultiline` is
/// absent, and accept either `\n` or `\r\n` as the newline immediately after
/// the last item. This matches RuboCop on the timetrap, solargraph, and betty
/// corpus examples without broadening the rule for `...,]` on the same line.
pub struct TrailingCommaInArrayLiteral;

fn last_item_precedes_newline(bytes: &[u8], last_end: usize, closing_start: usize) -> bool {
    let region = &bytes[last_end..closing_start];
    let mut i = 0;

    if i < region.len() && region[i] == b',' {
        i += 1;
    }

    while i < region.len() && matches!(region[i], b' ' | b'\t') {
        i += 1;
    }

    if i < region.len() && region[i] == b'#' {
        while i < region.len() && !matches!(region[i], b'\n' | b'\r') {
            i += 1;
        }
    }

    matches!(region.get(i), Some(b'\n'))
        || (matches!(region.get(i), Some(b'\r')) && matches!(region.get(i + 1), Some(b'\n')))
}

impl Cop for TrailingCommaInArrayLiteral {
    fn name(&self) -> &'static str {
        "Style/TrailingCommaInArrayLiteral"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[ARRAY_NODE]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let array_node = match node.as_array_node() {
            Some(a) => a,
            None => return,
        };

        // Skip %w(), %W(), %i(), %I() word/symbol arrays — they don't use commas
        if let Some(opening) = array_node.opening_loc() {
            if source.as_bytes().get(opening.start_offset()) == Some(&b'%') {
                return;
            }
        }

        let closing_loc = match array_node.closing_loc() {
            Some(loc) => loc,
            None => return,
        };

        let elements: Vec<ruby_prism::Node<'_>> = array_node.elements().iter().collect();
        let last_elem = match elements.last() {
            Some(e) => e,
            None => return,
        };

        let last_end = last_elem.location().end_offset();
        let closing_start = closing_loc.start_offset();
        let bytes = source.as_bytes();

        let has_heredoc = elements.iter().any(|e| trailing_comma::is_heredoc_node(e));
        let has_comma =
            trailing_comma::detect_trailing_comma(bytes, last_end, closing_start, has_heredoc);

        let style = {
            let alias_style = config.get_str("EnforcedStyle", "no_comma");
            config.get_str("EnforcedStyleForMultiline", alias_style)
        };

        // Check if array is multiline: the opening `[` and closing `]` are on different lines.
        let open_line = if let Some(opening) = array_node.opening_loc() {
            source.offset_to_line_col(opening.start_offset()).0
        } else {
            return;
        };
        let close_line = source.offset_to_line_col(closing_start).0;
        let is_multiline = if elements.len() == 1 {
            // Single element: only consider multiline if closing bracket is on a different line
            // than the end of the element (allowed_multiline_argument check)
            let last_line = source.offset_to_line_col(last_end).0;
            close_line > last_line
        } else {
            close_line > open_line
        };

        // Helper: find the absolute offset of the trailing comma for diagnostics.
        let find_comma_offset = || {
            trailing_comma::find_trailing_comma_offset(bytes, last_end, closing_start, has_heredoc)
        };

        match style {
            "comma" => {
                let elem_locs: Vec<(usize, usize)> = elements
                    .iter()
                    .map(|e| (e.location().start_offset(), e.location().end_offset()))
                    .collect();
                let each_on_own_line =
                    trailing_comma::no_elements_on_same_line(source, &elem_locs, closing_start);
                let should_have = is_multiline && each_on_own_line;
                if has_comma && !should_have {
                    if let Some(abs_offset) = find_comma_offset() {
                        let (line, column) = source.offset_to_line_col(abs_offset);
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Avoid comma after the last item of an array, unless each item is on its own line.".to_string(),
                        ));
                    }
                } else if !has_comma && should_have {
                    let (line, column) = source.offset_to_line_col(last_end);
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Put a comma after the last item of a multiline array.".to_string(),
                    ));
                }
            }
            "consistent_comma" => {
                if has_comma && !is_multiline {
                    if let Some(abs_offset) = find_comma_offset() {
                        let (line, column) = source.offset_to_line_col(abs_offset);
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Avoid comma after the last item of an array, unless items are split onto multiple lines.".to_string(),
                        ));
                    }
                } else if !has_comma && is_multiline {
                    let (line, column) = source.offset_to_line_col(last_end);
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Put a comma after the last item of a multiline array.".to_string(),
                    ));
                }
            }
            "diff_comma" => {
                let last_precedes_newline =
                    is_multiline && last_item_precedes_newline(bytes, last_end, closing_start);
                if has_comma && !last_precedes_newline {
                    if let Some(abs_offset) = find_comma_offset() {
                        let (line, column) = source.offset_to_line_col(abs_offset);
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Avoid comma after the last item of an array, unless that item immediately precedes a newline.".to_string(),
                        ));
                    }
                } else if !has_comma && last_precedes_newline {
                    let (line, column) = source.offset_to_line_col(last_end);
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Put a comma after the last item of a multiline array.".to_string(),
                    ));
                }
            }
            _ => {
                if has_comma {
                    if let Some(abs_offset) = find_comma_offset() {
                        let (line, column) = source.offset_to_line_col(abs_offset);
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Avoid comma after the last item of an array.".to_string(),
                        ));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cop::CopConfig;
    use crate::testutil::{
        assert_cop_no_offenses_full_with_config, assert_cop_offenses_full_with_config,
    };
    use std::collections::HashMap;

    crate::cop_fixture_tests!(
        TrailingCommaInArrayLiteral,
        "cops/style/trailing_comma_in_array_literal"
    );

    fn multiline_config(style: &str) -> CopConfig {
        let mut options = HashMap::new();
        options.insert(
            "EnforcedStyleForMultiline".to_string(),
            serde_yml::Value::String(style.to_string()),
        );
        CopConfig {
            options,
            ..CopConfig::default()
        }
    }

    fn comma_config() -> CopConfig {
        multiline_config("comma")
    }

    fn diff_comma_config() -> CopConfig {
        multiline_config("diff_comma")
    }

    #[test]
    fn comma_style_multiline_elements_on_same_line_no_offense() {
        // Multiline array with elements sharing lines should NOT be flagged
        let fixture = b"x = [\n  Foo, Bar, Baz,\n  Qux\n]\n";
        assert_cop_no_offenses_full_with_config(
            &TrailingCommaInArrayLiteral,
            fixture,
            comma_config(),
        );
    }

    #[test]
    fn comma_style_multiline_each_on_own_line_missing_comma_offense() {
        // Multiline array with each element on its own line, missing trailing comma
        let fixture = b"# nitrocop-expect: 4:3 Style/TrailingCommaInArrayLiteral: Put a comma after the last item of a multiline array.\nx = [\n  1,\n  2,\n  3\n]\n";
        assert_cop_offenses_full_with_config(&TrailingCommaInArrayLiteral, fixture, comma_config());
    }

    #[test]
    fn comma_style_single_line_trailing_comma_offense() {
        // Single-line array with trailing comma should be flagged even in comma style
        let fixture = b"[1, 2, 3,]\n        ^ Style/TrailingCommaInArrayLiteral: Avoid comma after the last item of an array, unless each item is on its own line.\n";
        assert_cop_offenses_full_with_config(&TrailingCommaInArrayLiteral, fixture, comma_config());
    }

    #[test]
    fn comma_style_multiline_each_on_own_line_with_comma_no_offense() {
        // Multiline array with each element on its own line AND trailing comma is fine
        let fixture = b"x = [\n  1,\n  2,\n  3,\n]\n";
        assert_cop_no_offenses_full_with_config(
            &TrailingCommaInArrayLiteral,
            fixture,
            comma_config(),
        );
    }

    fn alias_style_config(style: &str) -> CopConfig {
        let mut options = HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String(style.to_string()),
        );
        CopConfig {
            options,
            ..CopConfig::default()
        }
    }

    #[test]
    fn diff_comma_style_single_line_trailing_comma_offense() {
        let fixture = b"[1, 2, 3,]\n        ^ Style/TrailingCommaInArrayLiteral: Avoid comma after the last item of an array, unless that item immediately precedes a newline.\n";
        assert_cop_offenses_full_with_config(
            &TrailingCommaInArrayLiteral,
            fixture,
            diff_comma_config(),
        );
    }

    #[test]
    fn diff_comma_style_multiline_last_on_own_line_missing_comma_offense() {
        // Last element is followed by newline — should require comma
        let fixture = b"# nitrocop-expect: 3:3 Style/TrailingCommaInArrayLiteral: Put a comma after the last item of a multiline array.\nx = [\n  1,\n  2\n]\n";
        assert_cop_offenses_full_with_config(
            &TrailingCommaInArrayLiteral,
            fixture,
            diff_comma_config(),
        );
    }

    #[test]
    fn diff_comma_style_multiline_with_comma_no_offense() {
        // Last element has trailing comma and precedes newline — fine
        let fixture = b"x = [\n  1,\n  2,\n]\n";
        assert_cop_no_offenses_full_with_config(
            &TrailingCommaInArrayLiteral,
            fixture,
            diff_comma_config(),
        );
    }

    #[test]
    fn diff_comma_style_multiline_elements_sharing_lines_with_comma_no_offense() {
        // Multiple elements per line, last element precedes newline, has comma
        let fixture = b"x = [\n  1, 2,\n  3,\n]\n";
        assert_cop_no_offenses_full_with_config(
            &TrailingCommaInArrayLiteral,
            fixture,
            diff_comma_config(),
        );
    }

    #[test]
    fn diff_comma_style_closing_on_same_line_trailing_comma_offense() {
        // Closing bracket on same line as last element — comma is unwanted
        let fixture = b"[1, 2,\n     3,]\n      ^ Style/TrailingCommaInArrayLiteral: Avoid comma after the last item of an array, unless that item immediately precedes a newline.\n";
        assert_cop_offenses_full_with_config(
            &TrailingCommaInArrayLiteral,
            fixture,
            diff_comma_config(),
        );
    }

    #[test]
    fn diff_comma_style_single_line_no_comma_no_offense() {
        let fixture = b"[1, 2, 3]\n";
        assert_cop_no_offenses_full_with_config(
            &TrailingCommaInArrayLiteral,
            fixture,
            diff_comma_config(),
        );
    }

    #[test]
    fn offense_comma_fixture() {
        assert_cop_offenses_full_with_config(
            &TrailingCommaInArrayLiteral,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_array_literal/offense.comma.rb"
            ),
            comma_config(),
        );
    }

    #[test]
    fn no_offense_comma_fixture() {
        assert_cop_no_offenses_full_with_config(
            &TrailingCommaInArrayLiteral,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_array_literal/no_offense.comma.rb"
            ),
            comma_config(),
        );
    }

    #[test]
    fn offense_diff_comma_fixture() {
        assert_cop_offenses_full_with_config(
            &TrailingCommaInArrayLiteral,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_array_literal/offense.diff_comma.rb"
            ),
            diff_comma_config(),
        );
    }

    #[test]
    fn no_offense_diff_comma_fixture() {
        assert_cop_no_offenses_full_with_config(
            &TrailingCommaInArrayLiteral,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_array_literal/no_offense.diff_comma.rb"
            ),
            diff_comma_config(),
        );
    }

    #[test]
    fn offense_diff_comma_fixture_via_enforced_style_alias() {
        assert_cop_offenses_full_with_config(
            &TrailingCommaInArrayLiteral,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_array_literal/offense.diff_comma.rb"
            ),
            alias_style_config("diff_comma"),
        );
    }

    #[test]
    fn no_offense_diff_comma_fixture_via_enforced_style_alias() {
        assert_cop_no_offenses_full_with_config(
            &TrailingCommaInArrayLiteral,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_array_literal/no_offense.diff_comma.rb"
            ),
            alias_style_config("diff_comma"),
        );
    }

    #[test]
    fn diff_comma_accepts_crlf_trailing_comma_before_newline() {
        let source = b"multiple_functions = [\r\n  :scanVariable   => [],\r\n  :scanQuotelike  => [],\r\n  :scanCodeblock  => [],\r\n]\r\n";
        let diags = crate::testutil::run_cop_full_with_config(
            &TrailingCommaInArrayLiteral,
            source,
            diff_comma_config(),
        );
        assert!(
            diags.is_empty(),
            "CRLF diff_comma array should accept trailing comma before newline"
        );
    }

    #[test]
    fn diff_comma_flags_missing_comma_before_crlf_newline() {
        let source = b"files = {\r\n  examples: [\r\n    \"- betty copy folder my_songs/ to backup/\",\r\n    \"- betty move folder my_songs/ to backup/\",\r\n    \"- betty delete file junk.txt\",\r\n    \"- betty remove file junk.txt\",\r\n    \"- betty delete folder logs/\",\r\n    \"- betty remove folder logs/\",\r\n    \"- betty cleanup folder logs/\",\r\n    \"- betty force cleanup folder logs/\"\r\n  ]\r\n}\r\n";
        let diags = crate::testutil::run_cop_full_with_config(
            &TrailingCommaInArrayLiteral,
            source,
            diff_comma_config(),
        );
        assert_eq!(
            diags.len(),
            1,
            "CRLF diff_comma array should require the trailing comma"
        );
        assert_eq!(diags[0].location.line, 10);
        assert_eq!(diags[0].location.column, 40);
        assert_eq!(
            diags[0].message,
            "Put a comma after the last item of a multiline array."
        );
    }
}
