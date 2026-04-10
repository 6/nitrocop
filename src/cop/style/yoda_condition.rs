use crate::cop::shared::method_dispatch_predicates;
use crate::cop::shared::node_type::{
    CALL_NODE, FALSE_NODE, FLOAT_NODE, INTEGER_NODE, NIL_NODE, STRING_NODE, SYMBOL_NODE, TRUE_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Corpus investigation (FP=7, FN=27):
/// - FP: RationalNode (3r) and ImaginaryNode (1i) were not recognized as literals.
///   ArrayNode with all-literal elements (e.g. [[1,2],[3,4]]) also not recognized.
///   Both sides being constant means no Yoda offense — fixed by adding these to
///   is_constant_portion().
/// - FN: XStringNode (backtick `cmd`), InterpolatedSymbolNode (:"#{x}="),
///   InterpolatedXStringNode, and RegularExpressionNode/InterpolatedRegularExpressionNode
///   are all treated as literal? by RuboCop's Parser gem but were missing from
///   is_constant_portion(). Added all of these node types.
///
/// Corpus investigation round 2 (FP=3, FN=5):
/// - FP: InterpolatedStringNode (dstr) is considered literal? in Parser gem but was
///   missing from is_constant_portion(). When dstr appears on RHS (e.g. `0 != "...#{x}"`),
///   both sides should be constant → no offense. Fixed by adding InterpolatedStringNode.
/// - FP: RuboCop has an `interpolation?` check that skips offenses when LHS is a dstr
///   or interpolated regexp, even when RHS is non-constant. Added is_interpolation() check.
/// - FN: Hash literals with all-literal keys/values (e.g. `{"foo" => ["bar"]} == params`)
///   were not recognized as constant portions. Added hash support.
///
/// Corpus investigation round 3 (FP=62, FN=10):
/// - FP/FN: RuboCop's Parser AST treats array and hash syntax as `literal?` even when
///   elements or values are variables/expressions. nitrocop required descendants to be
///   literal, which produced false positives like `%w(admin password) == [u, p]` and
///   `ConstPathRef <= { scope: (ConstPathRef | Const), name: Const }`, and missed offenses
///   like `[query] == found`. Fixed by treating Prism ArrayNode/HashNode/KeywordHashNode
///   as constant portions by node type.
/// - FN: RuboCop treats `__FILE__` as a constant portion, except for the explicit
///   `__FILE__ == $0` / `$PROGRAM_NAME` exemption. Added SourceFileNode support and the
///   matching exemption.
///
/// Variant style divergence (`EnforcedStyle: require_for_all_comparison_operators`):
/// - FP: safe-navigation operator sends like `foo&.>(0)` are Prism `CallNode`s, but
///   RuboCop only hooks regular `send`, not `csend`, so they must be skipped.
/// - FN: RuboCop uses `first_argument` for dotted operator methods such as
///   `(2**500).<(1,2)`, so extra invalid arguments must not suppress the offense.
pub struct YodaCondition;

/// RuboCop's `constant_portion?` checks `node.literal? || node.const_type?`.
/// This means constants like `CONST` and `Foo::BAR` are treated as
/// "constant portions" for Yoda condition detection, just like literals.
///
/// In Parser gem, `literal?` returns true for: int, float, str, sym, nil, true,
/// false, rational, complex/imaginary, regexp, xstr (backtick), dsym (interpolated
/// symbol), dstr, dregexp, array, hash, and `__FILE__`.
fn is_constant_portion(node: &ruby_prism::Node<'_>) -> bool {
    node.as_integer_node().is_some()
        || node.as_float_node().is_some()
        || node.as_string_node().is_some()
        || node.as_symbol_node().is_some()
        || node.as_nil_node().is_some()
        || node.as_true_node().is_some()
        || node.as_false_node().is_some()
        || node.as_constant_read_node().is_some()
        || node.as_constant_path_node().is_some()
        || node.as_rational_node().is_some()
        || node.as_imaginary_node().is_some()
        || node.as_x_string_node().is_some()
        || node.as_interpolated_x_string_node().is_some()
        || node.as_interpolated_symbol_node().is_some()
        || node.as_regular_expression_node().is_some()
        || node.as_interpolated_regular_expression_node().is_some()
        || node.as_interpolated_string_node().is_some()
        || node.as_source_file_node().is_some()
        || is_literal_array(node)
        || is_literal_hash(node)
}

/// Parser gem treats array syntax itself as `literal?`, even when its elements are
/// variables or calls.
fn is_literal_array(node: &ruby_prism::Node<'_>) -> bool {
    node.as_array_node().is_some()
}

/// Parser gem treats hash syntax itself as `literal?`, even when its keys or values
/// are variables or calls.
fn is_literal_hash(node: &ruby_prism::Node<'_>) -> bool {
    node.as_hash_node().is_some() || node.as_keyword_hash_node().is_some()
}

/// Check if a node is an interpolated string (dstr) or interpolated regexp,
/// matching RuboCop's `interpolation?` method. When the LHS has interpolation,
/// the Yoda condition check is skipped entirely.
fn is_interpolation(node: &ruby_prism::Node<'_>) -> bool {
    node.as_interpolated_string_node().is_some()
        || (node.as_interpolated_regular_expression_node().is_some())
}

fn is_program_name(node: &ruby_prism::Node<'_>) -> bool {
    node.as_global_variable_read_node()
        .map(|gvar| matches!(gvar.location().as_slice(), b"$0" | b"$PROGRAM_NAME"))
        .unwrap_or(false)
}

fn is_source_file_equal_program_name(
    receiver: &ruby_prism::Node<'_>,
    argument: &ruby_prism::Node<'_>,
) -> bool {
    receiver.as_source_file_node().is_some() && is_program_name(argument)
}

impl Cop for YodaCondition {
    fn name(&self) -> &'static str {
        "Style/YodaCondition"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            CALL_NODE,
            FALSE_NODE,
            FLOAT_NODE,
            INTEGER_NODE,
            NIL_NODE,
            STRING_NODE,
            SYMBOL_NODE,
            TRUE_NODE,
        ]
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
        let enforced_style = config.get_str("EnforcedStyle", "forbid_for_all_comparison_operators");
        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let name = call.name().as_slice();

        let is_equality = name == b"==" || name == b"!=";
        let is_comparison =
            is_equality || name == b"<" || name == b">" || name == b"<=" || name == b">=";

        if !is_comparison {
            return;
        }

        if method_dispatch_predicates::is_safe_navigation(&call) {
            return;
        }

        // For *_equality_operators_only styles, skip non-equality operators
        let equality_only = enforced_style == "forbid_for_equality_operators_only"
            || enforced_style == "require_for_equality_operators_only";
        if equality_only && !is_equality {
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

        let Some(argument) = args.arguments().iter().next() else {
            return;
        };

        let require_yoda = enforced_style == "require_for_all_comparison_operators"
            || enforced_style == "require_for_equality_operators_only";

        if is_source_file_equal_program_name(&receiver, &argument) {
            return;
        }

        let lhs_constant = is_constant_portion(&receiver);
        let rhs_constant = is_constant_portion(&argument);

        // Both constant or both non-constant: not a Yoda issue
        if lhs_constant == rhs_constant {
            return;
        }

        // RuboCop skips when LHS has interpolation (dstr or interpolated regexp)
        if is_interpolation(&receiver) {
            return;
        }

        if require_yoda {
            // Require Yoda: flag when literal is on the RIGHT (non-Yoda)
            if !lhs_constant && rhs_constant {
                let loc = call.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Prefer Yoda conditions.".to_string(),
                ));
            }
        } else {
            // Forbid Yoda: flag when literal is on the LEFT (Yoda)
            if lhs_constant && !rhs_constant {
                let loc = call.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Prefer non-Yoda conditions.".to_string(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{
        assert_cop_no_offenses_full_with_config, assert_cop_offenses_full_with_config, run_cop_full,
    };

    crate::cop_fixture_tests!(YodaCondition, "cops/style/yoda_condition");

    fn require_all_comparison_config() -> CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".into(),
            serde_yml::Value::String("require_for_all_comparison_operators".into()),
        );
        CopConfig {
            options,
            ..CopConfig::default()
        }
    }

    #[test]
    fn require_all_comparison_operators_offense_fixture() {
        assert_cop_offenses_full_with_config(
            &YodaCondition,
            include_bytes!(
                "../../../tests/fixtures/cops/style/yoda_condition/require_for_all_comparison_operators_offense.rb"
            ),
            require_all_comparison_config(),
        );
    }

    #[test]
    fn require_all_comparison_operators_no_offense_fixture() {
        assert_cop_no_offenses_full_with_config(
            &YodaCondition,
            include_bytes!(
                "../../../tests/fixtures/cops/style/yoda_condition/require_for_all_comparison_operators_no_offense.rb"
            ),
            require_all_comparison_config(),
        );
    }

    #[test]
    fn both_literals_not_flagged() {
        let source = b"1 == 1\n";
        let diags = run_cop_full(&YodaCondition, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn nil_on_left_is_flagged() {
        let source = b"nil == x\n";
        let diags = run_cop_full(&YodaCondition, source);
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn require_yoda_style() {
        use crate::testutil::run_cop_full_with_config;

        let config = require_all_comparison_config();
        // Non-Yoda should be flagged
        let source = b"x == 1\n";
        let diags = run_cop_full_with_config(&YodaCondition, source, config.clone());
        assert_eq!(diags.len(), 1, "Should flag non-Yoda with require style");
        assert!(diags[0].message.contains("Prefer Yoda"));

        // Yoda should be allowed
        let source2 = b"1 == x\n";
        let diags2 = run_cop_full_with_config(&YodaCondition, source2, config);
        assert!(
            diags2.is_empty(),
            "Should allow Yoda conditions with require style"
        );
    }

    #[test]
    fn require_yoda_style_flags_dot_operator_call() {
        use crate::testutil::run_cop_full_with_config;

        let diags = run_cop_full_with_config(
            &YodaCondition,
            b"count.>(0)\n",
            require_all_comparison_config(),
        );
        assert_eq!(
            diags.len(),
            1,
            "Should still flag regular dotted operator calls"
        );
    }

    #[test]
    fn forbid_equality_only_skips_comparisons() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("forbid_for_equality_operators_only".into()),
            )]),
            ..CopConfig::default()
        };
        // `1 == x` should be flagged (equality Yoda)
        let source = b"1 == x\n";
        let diags = run_cop_full_with_config(&YodaCondition, source, config.clone());
        assert_eq!(diags.len(), 1, "Should flag equality Yoda");

        // `1 < x` should NOT be flagged (comparison, not equality)
        let source2 = b"1 < x\n";
        let diags2 = run_cop_full_with_config(&YodaCondition, source2, config);
        assert!(
            diags2.is_empty(),
            "Should skip non-equality comparison operators"
        );
    }

    #[test]
    fn forbid_all_flags_comparison_operators() {
        // Default: forbid_for_all_comparison_operators
        // `1 < x` should be flagged (Yoda with comparison)
        let source = b"1 < x\n";
        let diags = run_cop_full(&YodaCondition, source);
        assert_eq!(
            diags.len(),
            1,
            "Should flag comparison Yoda with default style"
        );
    }

    #[test]
    fn source_file_equal_program_name_is_not_flagged() {
        let source = b"__FILE__ == $0\n__FILE__ != $PROGRAM_NAME\n";
        let diags = run_cop_full(&YodaCondition, source);
        assert!(
            diags.is_empty(),
            "Should allow RuboCop's __FILE__ program-name exemption"
        );
    }
}
