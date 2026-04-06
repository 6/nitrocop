use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Checks for division with integers coerced to floats.
///
/// Per RuboCop's behavior, this cop exempts cases where the `to_f` receiver is
/// a regexp match result (`$1`, `$2`, etc. or `Regexp.last_match(n)`), because
/// these are already string representations of matched values and calling `.to_f`
/// on them is acceptable.
///
/// This exemption applies to all `EnforcedStyle` variants (`single_coerce`,
/// `left_coerce`, `right_coerce`, `fdiv`).
pub struct FloatDivision;

impl FloatDivision {
    fn is_to_f_call(node: &ruby_prism::Node<'_>) -> bool {
        if let Some(call) = node.as_call_node() {
            if call.name().as_slice() == b"to_f" && call.receiver().is_some() {
                // Make sure it has no arguments (not an implicit receiver call)
                if call.arguments().is_none() {
                    return true;
                }
            }
        }
        false
    }

    /// Checks if the node is a regexp nth_ref (e.g., $1, $2) or
    /// Regexp.last_match(int) result.
    fn is_regexp_match_result(node: &ruby_prism::Node<'_>) -> bool {
        // Check for NumberedReferenceReadNode (e.g., $1, $2, etc.)
        if node.as_numbered_reference_read_node().is_some() {
            return true;
        }

        // Check for Regexp.last_match(int) or ::Regexp.last_match(int)
        if let Some(call) = node.as_call_node() {
            if call.name().as_slice() == b"last_match" {
                // Check receiver is Regexp or ::Regexp (constant)
                if let Some(receiver) = call.receiver() {
                    // Check simple constant like Regexp
                    if let Some(const_node) = receiver.as_constant_read_node() {
                        if const_node.name().as_slice() == b"Regexp" {
                            // Has exactly one integer argument
                            return Self::call_has_single_integer_arg(&call);
                        }
                    }
                    // Check qualified constant like ::Regexp
                    if let Some(path_node) = receiver.as_constant_path_node() {
                        // path_node.name() returns Option<ConstantId> — the final segment
                        // For ::Regexp, parent is None; for Foo::Regexp, parent is Some
                        if path_node.name().is_some_and(|n| n.as_slice() == b"Regexp")
                            && path_node.parent().is_none()
                        {
                            return Self::call_has_single_integer_arg(&call);
                        }
                    }
                }
            }
        }

        false
    }

    /// Returns true if the call has exactly one integer argument.
    fn call_has_single_integer_arg(call: &ruby_prism::CallNode<'_>) -> bool {
        if let Some(args) = call.arguments() {
            if args.arguments().len() == 1 {
                if let Some(first_arg) = args.arguments().first() {
                    return first_arg.as_integer_node().is_some();
                }
            }
        }
        false
    }

    /// Returns true if the to_f call's receiver is a regexp match result.
    /// This is used to exempt cases like `$1.to_f / b` or `Regexp.last_match(1).to_f / b`.
    fn to_f_receiver_is_regexp_match(to_f_call: &ruby_prism::Node<'_>) -> bool {
        if let Some(call) = to_f_call.as_call_node() {
            if let Some(receiver) = call.receiver() {
                return Self::is_regexp_match_result(&receiver);
            }
        }
        false
    }
}

impl Cop for FloatDivision {
    fn name(&self) -> &'static str {
        "Style/FloatDivision"
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
        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        if call.name().as_slice() != b"/" {
            return;
        }

        let receiver = match call.receiver() {
            Some(r) => r,
            None => return,
        };

        let args = match call.arguments() {
            Some(a) => a,
            None => return,
        };

        let arg_list: Vec<_> = args.arguments().iter().collect();
        if arg_list.len() != 1 {
            return;
        }

        let left_is_to_f = Self::is_to_f_call(&receiver);
        let right_is_to_f = Self::is_to_f_call(&arg_list[0]);

        if !left_is_to_f && !right_is_to_f {
            return;
        }

        // Skip if either side's to_f receiver is a regexp match result.
        // e.g., $1.to_f / b or a / $1.to_f should not be flagged.
        if left_is_to_f && Self::to_f_receiver_is_regexp_match(&receiver) {
            return;
        }
        if right_is_to_f && Self::to_f_receiver_is_regexp_match(&arg_list[0]) {
            return;
        }

        let style = config.get_str("EnforcedStyle", "single_coerce");

        let loc = call.location();
        let (line, column) = source.offset_to_line_col(loc.start_offset());

        match style {
            "single_coerce" => {
                if left_is_to_f && right_is_to_f {
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Prefer using `.to_f` on one side only.".to_string(),
                    ));
                }
            }
            "left_coerce" => {
                if right_is_to_f {
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Prefer using `.to_f` on the left side.".to_string(),
                    ));
                }
            }
            "right_coerce" => {
                if left_is_to_f {
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Prefer using `.to_f` on the right side.".to_string(),
                    ));
                }
            }
            "fdiv" => {
                if left_is_to_f || right_is_to_f {
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Prefer using `fdiv` for float divisions.".to_string(),
                    ));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style_config(style: &str) -> CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String(style.to_string()),
        );
        CopConfig {
            options,
            ..CopConfig::default()
        }
    }

    crate::cop_fixture_tests!(FloatDivision, "cops/style/float_division");

    #[test]
    fn right_coerce_no_offense() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &FloatDivision,
            include_bytes!(
                "../../../tests/fixtures/cops/style/float_division/right_coerce_no_offense.rb"
            ),
            style_config("right_coerce"),
        );
    }

    #[test]
    fn right_coerce_offense() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &FloatDivision,
            include_bytes!(
                "../../../tests/fixtures/cops/style/float_division/right_coerce_offense.rb"
            ),
            style_config("right_coerce"),
        );
    }

    #[test]
    fn fdiv_no_offense() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &FloatDivision,
            include_bytes!("../../../tests/fixtures/cops/style/float_division/fdiv_no_offense.rb"),
            style_config("fdiv"),
        );
    }

    #[test]
    fn fdiv_offense() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &FloatDivision,
            include_bytes!("../../../tests/fixtures/cops/style/float_division/fdiv_offense.rb"),
            style_config("fdiv"),
        );
    }
}
