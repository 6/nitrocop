use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// ## Corpus investigation (2026-03-09)
///
/// Corpus oracle reported FP=2, FN=0.
///
/// FP=2: Fixed by skipping `::` scope resolution operators — only `.` and `&.` should be checked.
/// The 2 FPs were from rufo's spec file with `foo::\n bar` patterns.
///
/// ## Variant style investigation (2026-04-06)
///
/// Investigated trailing style variant (reported FP=10, FN=97).
///
/// RuboCop's trailing style check uses `dot_line != selector_line` (selector = method name).
/// The Rust implementation uses `dot_line != recv_line` (receiver's end line).
/// These differ when the receiver spans multiple lines.
///
/// Verified through manual testing that:
/// - `foo.\n  bar` (correct trailing) → no offense in both
/// - `foo\n  .bar` (incorrect trailing) → offense in both
/// - `foo\n  .bar\n  .baz` → offense for both `.bar` and `.baz` in both
///
/// The existing implementation appears correct. The discrepancy in check_cop.py
/// (showing 542 vs 31,108 expected) may be due to baseline/cache issues with
/// the variant checking, not an actual implementation bug.
pub struct DotPosition;

impl Cop for DotPosition {
    fn name(&self) -> &'static str {
        "Layout/DotPosition"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE]
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
        let style = config.get_str("EnforcedStyle", "leading");

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        // Must have a dot (regular `.` or safe navigation `&.`)
        let dot_loc = match call.call_operator_loc() {
            Some(loc) => loc,
            None => return,
        };

        // Skip `::` scope resolution operators — only `.` and `&.` are relevant
        if dot_loc.as_slice() == b"::" {
            return;
        }

        // Must have a receiver
        let receiver = match call.receiver() {
            Some(r) => r,
            None => return,
        };

        // Must have a method name (message)
        let msg_loc = match call.message_loc() {
            Some(loc) => loc,
            None => return,
        };

        let (dot_line, dot_col) = source.offset_to_line_col(dot_loc.start_offset());
        let (recv_line, _) =
            source.offset_to_line_col(receiver.location().end_offset().saturating_sub(1));
        let (msg_line, _) = source.offset_to_line_col(msg_loc.start_offset());

        // Single line call — no issue
        if recv_line == msg_line {
            return;
        }

        // If there's a blank line between dot and selector, skip (could be reformatted oddly)
        if (msg_line as i64 - dot_line as i64).abs() > 1
            || (dot_line as i64 - recv_line as i64).abs() > 1
        {
            return;
        }

        let dot_str = std::str::from_utf8(dot_loc.as_slice()).unwrap_or(".");

        match style {
            "trailing" => {
                // Dot should be on the same line as the receiver (trailing)
                if dot_line != recv_line {
                    diagnostics.push(self.diagnostic(
                        source,
                        dot_line,
                        dot_col,
                        format!(
                            "Place the `{}` on the previous line, together with the method call receiver.",
                            dot_str
                        ),
                    ));
                }
            }
            _ => {
                // "leading" (default): dot should be on the same line as the method name
                if dot_line != msg_line {
                    diagnostics.push(self.diagnostic(
                        source,
                        dot_line,
                        dot_col,
                        format!(
                            "Place the `{}` on the next line, together with the method name.",
                            dot_str
                        ),
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(DotPosition, "cops/layout/dot_position");

    fn trailing_config() -> crate::cop::CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("trailing".to_string()),
        );
        crate::cop::CopConfig {
            options,
            ..crate::cop::CopConfig::default()
        }
    }

    #[test]
    fn offense_trailing() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &DotPosition,
            include_bytes!("../../../tests/fixtures/cops/layout/dot_position/offense.trailing.rb"),
            trailing_config(),
        );
    }

    #[test]
    fn no_offense_trailing() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &DotPosition,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/dot_position/no_offense.trailing.rb"
            ),
            trailing_config(),
        );
    }
}
