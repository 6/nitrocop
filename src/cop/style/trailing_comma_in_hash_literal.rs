use crate::cop::shared::node_type::HASH_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

use super::trailing_comma;

/// Checks for trailing commas in hash literals.
///
/// ## Heredoc handling (2026-03)
///
/// Prism reports a hash pair's `end_offset()` at the heredoc opening token
/// (for example `<<~RUBY.chomp`), not at the closing heredoc terminator. A
/// previous FP fix tried to avoid scanning heredoc bodies by starting at the
/// closing `}` line whenever a hash contained a heredoc, but that skipped the
/// real trailing comma on the heredoc opening line:
///
/// `key: <<~RUBY,`
///
/// Fix: keep scanning from the last element end offset, but stop at the first
/// newline when a heredoc is present. This matches RuboCop's heredoc-specific
/// `/\A[^\S\n]*,/` check, so commas on the heredoc opening line are found
/// without treating commas inside heredoc bodies as trailing hash commas.
///
/// Nested hash values also need heredoc recursion. Without that, an outer hash
/// whose last value is another hash containing a heredoc still scans through
/// the nested heredoc body and can mistake commas in embedded Ruby for a
/// trailing comma on the outer hash.
///
/// ## Variant divergence (2026-04)
///
/// The remaining `diff_comma` corpus drift came from Windows line endings.
/// Those repos use `\r\n`, but the newline predicate only accepted `\n`, so a
/// last item followed by `,\r\n` was treated as an illegal trailing comma and a
/// missing comma before `\r\n}` was missed entirely. The corpus examples from
/// timetrap and canine reproduced both sides of that split.
///
/// Fix: keep RuboCop's `diff_comma` behavior, but accept either `\n` or
/// `\r\n` as the newline immediately following the last item. Also fall back to
/// `EnforcedStyle` when `EnforcedStyleForMultiline` is absent so the local
/// variant harness can still exercise the non-default style through a generic
/// override.
pub struct TrailingCommaInHashLiteral;

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

impl Cop for TrailingCommaInHashLiteral {
    fn name(&self) -> &'static str {
        "Style/TrailingCommaInHashLiteral"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[HASH_NODE]
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
        // Note: keyword_hash_node (keyword args like `foo(a: 1)`) intentionally not
        // handled — this cop only applies to trailing commas in hash literals.
        let hash_node = match node.as_hash_node() {
            Some(h) => h,
            None => return,
        };

        let closing_loc = hash_node.closing_loc();
        let elements: Vec<ruby_prism::Node<'_>> = hash_node.elements().iter().collect();
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

        // Multiline check: the hash node spans multiple lines. For single-element
        // hashes, use the allowed_multiline_argument exception (closing bracket on
        // the same line as element end means not multiline).
        let open_line = source
            .offset_to_line_col(hash_node.opening_loc().start_offset())
            .0;
        let close_line = source.offset_to_line_col(closing_start).0;
        let is_multiline = if elements.len() == 1 {
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
                            "Avoid comma after the last item of a hash, unless each item is on its own line.".to_string(),
                        ));
                    }
                } else if !has_comma && should_have {
                    let (line, column) = source.offset_to_line_col(last_end);
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Put a comma after the last item of a multiline hash.".to_string(),
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
                            "Avoid comma after the last item of a hash, unless items are split onto multiple lines.".to_string(),
                        ));
                    }
                } else if !has_comma && is_multiline {
                    let (line, column) = source.offset_to_line_col(last_end);
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Put a comma after the last item of a multiline hash.".to_string(),
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
                            "Avoid comma after the last item of a hash, unless that item immediately precedes a newline.".to_string(),
                        ));
                    }
                } else if !has_comma && last_precedes_newline {
                    let (line, column) = source.offset_to_line_col(last_end);
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Put a comma after the last item of a multiline hash.".to_string(),
                    ));
                }
            }
            _ => {
                // no_comma: flag trailing commas
                if has_comma {
                    if let Some(abs_offset) = find_comma_offset() {
                        let (line, column) = source.offset_to_line_col(abs_offset);
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Avoid comma after the last item of a hash.".to_string(),
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

    crate::cop_fixture_tests!(
        TrailingCommaInHashLiteral,
        "cops/style/trailing_comma_in_hash_literal"
    );

    fn multiline_config(style: &str) -> crate::cop::CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyleForMultiline".to_string(),
            serde_yml::Value::String(style.to_string()),
        );
        crate::cop::CopConfig {
            options,
            ..crate::cop::CopConfig::default()
        }
    }

    fn alias_style_config(style: &str) -> crate::cop::CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String(style.to_string()),
        );
        crate::cop::CopConfig {
            options,
            ..crate::cop::CopConfig::default()
        }
    }

    #[test]
    fn offense_comma() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &TrailingCommaInHashLiteral,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_hash_literal/offense.comma.rb"
            ),
            multiline_config("comma"),
        );
    }

    #[test]
    fn no_offense_comma() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &TrailingCommaInHashLiteral,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_hash_literal/no_offense.comma.rb"
            ),
            multiline_config("comma"),
        );
    }

    #[test]
    fn offense_consistent_comma() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &TrailingCommaInHashLiteral,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_hash_literal/offense.consistent_comma.rb"
            ),
            multiline_config("consistent_comma"),
        );
    }

    #[test]
    fn offense_diff_comma() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &TrailingCommaInHashLiteral,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_hash_literal/offense.diff_comma.rb"
            ),
            multiline_config("diff_comma"),
        );
    }

    #[test]
    fn no_offense_diff_comma() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &TrailingCommaInHashLiteral,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_hash_literal/no_offense.diff_comma.rb"
            ),
            multiline_config("diff_comma"),
        );
    }

    #[test]
    fn offense_diff_comma_via_enforced_style_alias() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &TrailingCommaInHashLiteral,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_hash_literal/offense.diff_comma.rb"
            ),
            alias_style_config("diff_comma"),
        );
    }

    #[test]
    fn no_offense_diff_comma_via_enforced_style_alias() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &TrailingCommaInHashLiteral,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_hash_literal/no_offense.diff_comma.rb"
            ),
            alias_style_config("diff_comma"),
        );
    }

    #[test]
    fn diff_comma_accepts_crlf_trailing_comma_before_newline() {
        let source = b"CodeblockDelimiters = {\r\n  '{'     => '}',\r\n  'begin' => 'end',\r\n  'do'    => 'end',\r\n}\r\n";
        let diags = crate::testutil::run_cop_full_with_config(
            &TrailingCommaInHashLiteral,
            source,
            multiline_config("diff_comma"),
        );
        assert!(
            diags.is_empty(),
            "CRLF diff_comma hash should accept trailing comma before newline"
        );
    }

    #[test]
    fn diff_comma_flags_missing_comma_before_crlf_newline() {
        let source = b"auth_data = {\r\n  provider: auth.provider,\r\n  uid: auth.uid,\r\n  auth: auth_hash.to_json,\r\n  expires_at: expires_at,\r\n  access_token: auth.credentials.token,\r\n  access_token_secret: auth.credentials.secret\r\n}\r\n";
        let diags = crate::testutil::run_cop_full_with_config(
            &TrailingCommaInHashLiteral,
            source,
            multiline_config("diff_comma"),
        );
        assert_eq!(
            diags.len(),
            1,
            "CRLF diff_comma hash should require the trailing comma"
        );
        assert_eq!(diags[0].location.line, 7);
        assert_eq!(diags[0].location.column, 46);
        assert_eq!(
            diags[0].message,
            "Put a comma after the last item of a multiline hash."
        );
    }
}
