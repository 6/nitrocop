use crate::cop::shared::node_type::EMBEDDED_STATEMENTS_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Checks for whitespace within string interpolations.
///
/// RuboCop does not check spacing for interpolations inside multiline strings
/// (backslash-newline continuation), even when the interpolation itself is on
/// a single line. For example, in `"foo \\\n#{bar}"`, the `#{bar}` is not
/// flagged by RuboCop even with `EnforcedStyle: space`. This is because RuboCop's
/// `Interpolation` mixin considers the interpolation part of a multiline string
/// context. We replicate this behavior by skipping interpolations where the byte
/// immediately before `#{` is a newline, or where a space before `#{` is preceded
/// by a backslash-newline sequence (line continuation).
pub struct SpaceInsideStringInterpolation;

impl Cop for SpaceInsideStringInterpolation {
    fn name(&self) -> &'static str {
        "Layout/SpaceInsideStringInterpolation"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[EMBEDDED_STATEMENTS_NODE]
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
        mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let style = config.get_str("EnforcedStyle", "no_space");

        // EmbeddedStatementsNode represents `#{ ... }` inside strings
        let embedded = match node.as_embedded_statements_node() {
            Some(e) => e,
            None => return,
        };

        let open_loc = embedded.opening_loc();
        let close_loc = embedded.closing_loc();

        let (open_line, _) = source.offset_to_line_col(open_loc.start_offset());
        let (close_line, _) = source.offset_to_line_col(close_loc.start_offset());

        // Skip multiline interpolations
        if open_line != close_line {
            return;
        }

        let bytes = source.as_bytes();
        let open_start = open_loc.start_offset(); // position of `#` in `#{`
        let open_end = open_loc.end_offset(); // position after `#{`
        let close_start = close_loc.start_offset(); // position of `}`

        // Skip interpolations in multiline strings (backslash-newline continuation).
        // If the character before `#{` is a newline, it's a line continuation.
        // If there's a space before `#{` followed by backslash-newline before that, it's also
        // a continuation (e.g., `"foo \\\n#{bar}"`).
        if open_start > 0 {
            let byte_before = bytes[open_start - 1];
            if byte_before == b'\n' {
                // Newline immediately before `#{` - skip (line continuation)
                return;
            }
            if byte_before == b' ' && open_start > 2 {
                // Space before `#{` - check if preceded by backslash-newline
                if bytes[open_start - 2] == b'\\' && bytes[open_start - 3] == b'\n' {
                    return;
                }
            }
        }

        // Skip empty interpolation
        if close_start <= open_end {
            return;
        }

        let space_after_open = bytes.get(open_end) == Some(&b' ');
        let space_before_close = close_start > 0 && bytes.get(close_start - 1) == Some(&b' ');

        match style {
            "space" => {
                // Require spaces
                if !space_after_open {
                    let (line, col) = source.offset_to_line_col(open_end);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        col,
                        "Missing space inside string interpolation.".to_string(),
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
                if !space_before_close {
                    let (line, col) = source.offset_to_line_col(close_start);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        col,
                        "Missing space inside string interpolation.".to_string(),
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
            _ => {
                // "no_space" (default) — flag spaces
                if space_after_open {
                    let (line, col) = source.offset_to_line_col(open_end);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        col,
                        "Space inside string interpolation detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: open_end,
                            end: open_end + 1,
                            replacement: String::new(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
                if space_before_close {
                    let (line, col) = source.offset_to_line_col(close_start - 1);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        col,
                        "Space inside string interpolation detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: close_start - 1,
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(
        SpaceInsideStringInterpolation,
        "cops/layout/space_inside_string_interpolation"
    );
    crate::cop_autocorrect_fixture_tests!(
        SpaceInsideStringInterpolation,
        "cops/layout/space_inside_string_interpolation"
    );

    fn space_config() -> CopConfig {
        let mut config = CopConfig::default();
        config.options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("space".to_string()),
        );
        config
    }

    #[test]
    fn space_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &SpaceInsideStringInterpolation,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/space_inside_string_interpolation/space_no_offense.rb"
            ),
            space_config(),
        );
    }
}
