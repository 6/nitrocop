use crate::cop::shared::node_type::{
    INTERPOLATED_STRING_NODE, INTERPOLATED_X_STRING_NODE, STRING_NODE, X_STRING_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Fixed: the `is_uppercase` / `is_lowercase` checks previously required every
/// byte to be `[A-Z_0-9]` or `[a-z_0-9]`, which caused false positives for
/// quoted heredoc delimiters containing non-alphanumeric characters (e.g.,
/// `'end-of-data'`, `"begin;"`) under the `lowercase` EnforcedStyle variant.
/// The fix mirrors Ruby's `String#upcase`/`String#downcase` behavior: non-alphabetic
/// characters are ignored in the case check since case conversion leaves them unchanged.
pub struct HeredocDelimiterCase;

impl Cop for HeredocDelimiterCase {
    fn name(&self) -> &'static str {
        "Naming/HeredocDelimiterCase"
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            INTERPOLATED_STRING_NODE,
            STRING_NODE,
            INTERPOLATED_X_STRING_NODE,
            X_STRING_NODE,
        ]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let enforced_style = config.get_str("EnforcedStyle", "uppercase");

        // Extract opening and closing locations based on node type.
        let (opening_start, opening_end, closing_loc) =
            if let Some(interp) = node.as_interpolated_string_node() {
                let open = match interp.opening_loc() {
                    Some(loc) => loc,
                    None => return,
                };
                let close = interp.closing_loc();
                (open.start_offset(), open.end_offset(), close)
            } else if let Some(s) = node.as_string_node() {
                let open = match s.opening_loc() {
                    Some(loc) => loc,
                    None => return,
                };
                let close = s.closing_loc();
                (open.start_offset(), open.end_offset(), close)
            } else if let Some(x) = node.as_interpolated_x_string_node() {
                let open = x.opening_loc();
                let close = x.closing_loc();
                (open.start_offset(), open.end_offset(), Some(close))
            } else if let Some(x) = node.as_x_string_node() {
                let open = x.opening_loc();
                let close = x.closing_loc();
                (open.start_offset(), open.end_offset(), Some(close))
            } else {
                return;
            };

        let bytes = source.as_bytes();
        let opening = &bytes[opening_start..opening_end];

        // Must be a heredoc (starts with <<)
        if !opening.starts_with(b"<<") {
            return;
        }

        // Extract delimiter name (skip <<, ~, -, and quotes)
        let after_arrows = &opening[2..];
        let prefix_len = if after_arrows.starts_with(b"~") || after_arrows.starts_with(b"-") {
            1
        } else {
            0
        };
        let after_prefix = &after_arrows[prefix_len..];

        let (delimiter, delim_offset_in_opening, _is_quoted) = if after_prefix.starts_with(b"'")
            || after_prefix.starts_with(b"\"")
            || after_prefix.starts_with(b"`")
        {
            let quote = after_prefix[0];
            let end = after_prefix[1..]
                .iter()
                .position(|&b| b == quote)
                .unwrap_or(after_prefix.len() - 1);
            (
                &after_prefix[1..1 + end],
                2 + prefix_len + 1, // <<, ~/-?, quote
                true,
            )
        } else {
            // Unquoted delimiter: take only word characters (alphanumeric + underscore)
            let end = after_prefix
                .iter()
                .position(|b| !b.is_ascii_alphanumeric() && *b != b'_')
                .unwrap_or(after_prefix.len());
            if end == 0 {
                return;
            }
            (
                &after_prefix[..end],
                2 + prefix_len, // <<, ~/-?
                false,          // not quoted
            )
        };

        if delimiter.is_empty() {
            return;
        }

        // Skip delimiters with no alphabetic characters
        if !delimiter.iter().any(|b| b.is_ascii_alphabetic()) {
            return;
        }

        // Match Ruby's String#upcase / String#downcase behavior: non-alphabetic
        // characters are unchanged by case conversion, so they never affect the
        // "is already correct case?" check.
        let is_uppercase = delimiter
            .iter()
            .all(|b| !b.is_ascii_alphabetic() || b.is_ascii_uppercase());
        let is_lowercase = delimiter
            .iter()
            .all(|b| !b.is_ascii_alphabetic() || b.is_ascii_lowercase());

        let offense = match enforced_style {
            "uppercase" => !is_uppercase,
            "lowercase" => !is_lowercase,
            _ => false,
        };

        if offense {
            let closing_start = closing_loc.as_ref().map(|l| l.start_offset());
            let offset = closing_start.unwrap_or(opening_start + 2);
            let (line, column) = source.offset_to_line_col(offset);
            let mut diag = self.diagnostic(
                source,
                line,
                column,
                format!("Use {enforced_style} heredoc delimiters."),
            );

            if let Some(ref mut corr) = corrections {
                let transformed: String = match enforced_style {
                    "uppercase" => delimiter
                        .iter()
                        .map(|&b| b.to_ascii_uppercase() as char)
                        .collect(),
                    "lowercase" => delimiter
                        .iter()
                        .map(|&b| b.to_ascii_lowercase() as char)
                        .collect(),
                    _ => return,
                };

                // Replace delimiter in opening
                let open_delim_start = opening_start + delim_offset_in_opening;
                let open_delim_end = open_delim_start + delimiter.len();
                corr.push(crate::correction::Correction {
                    start: open_delim_start,
                    end: open_delim_end,
                    replacement: transformed.clone(),
                    cop_name: self.name(),
                    cop_index: 0,
                });

                // Replace delimiter in closing
                if let Some(close_loc) = &closing_loc {
                    let close_bytes = &bytes[close_loc.start_offset()..close_loc.end_offset()];
                    // The closing delimiter may have leading whitespace (for <<~)
                    // and a trailing newline. Find the actual delimiter text within it.
                    let close_str = std::str::from_utf8(close_bytes).unwrap_or("");
                    let trimmed = close_str.trim();
                    if let Some(pos) = close_str.find(trimmed) {
                        let close_delim_start = close_loc.start_offset() + pos;
                        let close_delim_end = close_delim_start + trimmed.len();
                        // Don't add if it would overlap with the opening correction
                        if close_delim_start >= open_delim_end {
                            corr.push(crate::correction::Correction {
                                start: close_delim_start,
                                end: close_delim_end,
                                replacement: transformed,
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                        }
                    }
                }

                diag.corrected = true;
            }

            diagnostics.push(diag);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(HeredocDelimiterCase, "cops/naming/heredoc_delimiter_case");
    crate::cop_autocorrect_fixture_tests!(
        HeredocDelimiterCase,
        "cops/naming/heredoc_delimiter_case"
    );

    #[test]
    fn autocorrect_lowercase_to_uppercase() {
        let input = b"x = <<~sql\n  SELECT 1\nsql\n";
        let (diags, corrections) =
            crate::testutil::run_cop_autocorrect(&HeredocDelimiterCase, input);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].corrected);
        let cs = crate::correction::CorrectionSet::from_vec(corrections);
        let corrected = cs.apply(input);
        assert_eq!(corrected, b"x = <<~SQL\n  SELECT 1\nSQL\n");
    }

    #[test]
    fn autocorrect_quoted_delimiter() {
        let input = b"x = <<~'sql'\n  SELECT 1\nsql\n";
        let (diags, corrections) =
            crate::testutil::run_cop_autocorrect(&HeredocDelimiterCase, input);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].corrected);
        let cs = crate::correction::CorrectionSet::from_vec(corrections);
        let corrected = cs.apply(input);
        assert_eq!(corrected, b"x = <<~'SQL'\n  SELECT 1\nSQL\n");
    }

    #[test]
    fn autocorrect_mixed_case() {
        let input = b"x = <<-\"Sql\"\n  foo\nSql\n";
        let (diags, corrections) =
            crate::testutil::run_cop_autocorrect(&HeredocDelimiterCase, input);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].corrected);
        let cs = crate::correction::CorrectionSet::from_vec(corrections);
        let corrected = cs.apply(input);
        assert_eq!(corrected, b"x = <<-\"SQL\"\n  foo\nSQL\n");
    }

    #[test]
    fn heredoc_inside_string_interpolation() {
        // Heredocs used inside string interpolation like:
        //   "#{<<-"begin;"}\n#{<<-'end;'}"
        // Should still be detected as case offenses.
        let input = "x = \"#{<<-\"begin;\"}\\n#{<<-'end;'}\"\nbegin;\n  something\nend;\n";
        let diags = crate::testutil::run_cop_full(&HeredocDelimiterCase, input.as_bytes());
        assert!(
            diags.len() >= 2,
            "should flag both 'begin;' and 'end;' heredoc delimiters, got {}",
            diags.len()
        );
    }

    #[test]
    fn lowercase_style_non_alnum_delimiter_no_offense() {
        // Delimiters like 'end-of-data' that are already lowercase should not be flagged
        // when EnforcedStyle is lowercase. Non-alphabetic characters (e.g., '-', ';', '.')
        // should not affect the case check, matching Ruby's String#downcase behavior.
        use std::collections::HashMap;
        let mut options = HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("lowercase".to_string()),
        );
        let config = CopConfig {
            options,
            ..CopConfig::default()
        };
        let input = b"x = <<~'end-of-data'\n  hello\nend-of-data\n";
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &HeredocDelimiterCase,
            input,
            config,
        );
    }

    #[test]
    fn lowercase_style_uppercase_non_alnum_delimiter_offense() {
        // Delimiters like 'END-OF-DATA' should still be flagged when EnforcedStyle is lowercase
        use std::collections::HashMap;
        let mut options = HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("lowercase".to_string()),
        );
        let config = CopConfig {
            options,
            ..CopConfig::default()
        };
        let input = b"x = <<~'END-OF-DATA'\n  hello\nEND-OF-DATA\n";
        let diags = crate::testutil::run_cop_full_with_config(&HeredocDelimiterCase, input, config);
        assert_eq!(
            diags.len(),
            1,
            "should flag uppercase delimiter with non-alnum chars"
        );
    }

    #[test]
    fn backtick_heredoc_xstring_node() {
        // Backtick heredocs without interpolation use XStringNode, not InterpolatedXStringNode.
        // Must be handled to avoid FN.
        let input = b"x = <<~`cmd`\n  echo hello\ncmd\n";
        let diags = crate::testutil::run_cop_full(&HeredocDelimiterCase, input);
        assert_eq!(
            diags.len(),
            1,
            "should flag lowercase backtick heredoc delimiter with default uppercase style"
        );
    }

    #[test]
    fn heredoc_inside_interpolation_exact_corpus_pattern() {
        // Exact pattern from ruby/logger corpus: heredoc with ; in delimiter
        let input = concat!(
            "stderr = run_children(2, [logfile], \"#{<<-\"begin;\"}\\n#{<<-'end;'}\")\n",
            "begin;\n",
            "  logger = Logger.new(ARGV[0], 4, 10)\n",
            "  10.times do\n",
            "    logger.info '0' * 15\n",
            "  end\n",
            "end;\n",
        );
        let diags = crate::testutil::run_cop_full(&HeredocDelimiterCase, input.as_bytes());
        assert!(
            diags.len() >= 2,
            "should flag 'begin;' and 'end;' heredoc delimiters, got {}",
            diags.len()
        );
        // RuboCop reports at the closing delimiter line:
        // begin; is at line 2, end; is at line 7
        let lines: Vec<usize> = diags.iter().map(|d| d.location.line).collect();
        assert!(
            lines.contains(&2),
            "should have offense at line 2 (begin; closing), got {:?}",
            lines
        );
        assert!(
            lines.contains(&7),
            "should have offense at line 7 (end; closing), got {:?}",
            lines
        );
    }
}
