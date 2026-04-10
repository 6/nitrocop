use crate::cop::shared::node_type::{
    CALL_NODE, CLASS_VARIABLE_READ_NODE, CLASS_VARIABLE_WRITE_NODE, CONSTANT_PATH_NODE,
    CONSTANT_READ_NODE, CONSTANT_WRITE_NODE, FALSE_NODE, GLOBAL_VARIABLE_READ_NODE,
    GLOBAL_VARIABLE_WRITE_NODE, IF_NODE, INSTANCE_VARIABLE_READ_NODE, INSTANCE_VARIABLE_WRITE_NODE,
    LOCAL_VARIABLE_READ_NODE, LOCAL_VARIABLE_WRITE_NODE, NIL_NODE, PARENTHESES_NODE, SELF_NODE,
    STATEMENTS_NODE, TRUE_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Mirrors RuboCop's narrow ternary-condition exemptions found in corpus work.
///
/// 2026-03-15: only `[]=` counts as a safe assignment in ternary conditions.
/// Broad setter-method exemptions fixed one FP but introduced new FNs.
///
/// 2026-03-31: parenthesized one-line pattern matching like
/// `(foo in bar) ? a : b` is accepted by RuboCop and must not be flagged.
///
/// 2026-04-09: fixed `require_parentheses_when_complex` variant (318 FN).
/// `is_complex_condition` was called on the ParenthesesNode wrapper itself
/// (always "complex") instead of unwrapping it to check the inner expression.
/// Added `complex_condition()` which mirrors RuboCop's `complex_condition?`
/// by recursing into `begin`-type (parenthesized) nodes before testing.
///
/// 2026-04-10: method calls with real attached blocks like `.any? { ... }`
/// are complex in RuboCop, but Prism still exposes them as `CallNode`.
/// Treating every non-operator call as simple caused variant-only divergence:
/// parenthesized block calls were false positives and unparenthesized block
/// calls were false negatives under `require_parentheses_when_complex`.
///
/// RuboCop's non-complex whitelist is also narrower than Ruby truthiness.
/// Literal predicates like `true`, `false`, `nil`, and `self` still count as
/// complex under `require_parentheses_when_complex`, so they must not be
/// treated like simple variable reads here.
pub struct TernaryParentheses;

/// Check if a parenthesized node contains a safe assignment (=) in ternary context.
fn is_ternary_safe_assignment(paren: &ruby_prism::ParenthesesNode<'_>) -> bool {
    let body = match paren.body() {
        Some(b) => b,
        None => return false,
    };
    if let Some(stmts) = body.as_statements_node() {
        let stmts_body = stmts.body();
        if stmts_body.len() == 1 {
            let inner = &stmts_body.iter().next().unwrap();
            return is_write_or_indexed_assign(inner);
        }
    }
    is_write_or_indexed_assign(&body)
}

/// Check if a parenthesized ternary condition is a one-line pattern match
/// (`(foo in bar)` / `(foo => bar)`), which RuboCop exempts.
fn is_parenthesized_one_line_pattern_matching(paren: &ruby_prism::ParenthesesNode<'_>) -> bool {
    let body = match paren.body() {
        Some(b) => b,
        None => return false,
    };
    if let Some(stmts) = body.as_statements_node() {
        let stmts_body = stmts.body();
        if stmts_body.len() != 1 {
            return false;
        }
        let inner = &stmts_body.iter().next().unwrap();
        return is_one_line_pattern_matching(inner);
    }
    is_one_line_pattern_matching(&body)
}

fn is_one_line_pattern_matching(node: &ruby_prism::Node<'_>) -> bool {
    node.as_match_predicate_node().is_some() || node.as_match_required_node().is_some()
}

/// Check if a node is a variable write or an indexed assignment (`[]=`).
/// We intentionally only handle `[]=` (not all setter methods like `foo.bar=`)
/// because the previous broader fix caused corpus regressions.
fn is_write_or_indexed_assign(node: &ruby_prism::Node<'_>) -> bool {
    node.as_local_variable_write_node().is_some()
        || node.as_instance_variable_write_node().is_some()
        || node.as_class_variable_write_node().is_some()
        || node.as_global_variable_write_node().is_some()
        || node.as_constant_write_node().is_some()
        || is_indexed_assign(node)
}

/// Check if a node is an indexed assignment (`obj[key] = val`), which Prism
/// parses as a `CallNode` with method name `[]=`.
fn is_indexed_assign(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(call) = node.as_call_node() {
        call.name().as_slice() == b"[]="
    } else {
        false
    }
}

/// Check if a condition is "complex" (not a simple variable/constant/method call).
fn is_complex_condition(node: &ruby_prism::Node<'_>) -> bool {
    // Simple: variables, constants, `defined?`, `yield`
    if node.as_local_variable_read_node().is_some()
        || node.as_instance_variable_read_node().is_some()
        || node.as_class_variable_read_node().is_some()
        || node.as_global_variable_read_node().is_some()
        || node.as_constant_read_node().is_some()
        || node.as_constant_path_node().is_some()
        || node.as_defined_node().is_some()
        || node.as_yield_node().is_some()
    {
        return false;
    }
    // Method calls without operators are simple
    if let Some(call) = node.as_call_node() {
        if call
            .block()
            .and_then(|block| block.as_block_node())
            .is_some()
        {
            return true;
        }
        let name = call.name().as_slice();
        // Operator methods (except []) are complex
        if !name[0].is_ascii_alphabetic() && name[0] != b'_' && name != b"[]" {
            return true;
        }
        return false;
    }
    // Everything else is complex (and, or, binary ops, etc.)
    true
}

/// Mirror RuboCop's `complex_condition?`: unwrap parentheses and check the
/// inner expression. RuboCop checks `begin_type?` (its parenthesized wrapper)
/// and recurses into children before testing complexity.
fn complex_condition(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(paren) = node.as_parentheses_node() {
        let body = match paren.body() {
            Some(b) => b,
            None => return false,
        };
        if let Some(stmts) = body.as_statements_node() {
            return stmts.body().iter().any(|child| complex_condition(&child));
        }
        return complex_condition(&body);
    }
    is_complex_condition(node)
}

impl Cop for TernaryParentheses {
    fn name(&self) -> &'static str {
        "Style/TernaryParentheses"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            CALL_NODE,
            CLASS_VARIABLE_READ_NODE,
            CLASS_VARIABLE_WRITE_NODE,
            CONSTANT_PATH_NODE,
            CONSTANT_READ_NODE,
            CONSTANT_WRITE_NODE,
            FALSE_NODE,
            GLOBAL_VARIABLE_READ_NODE,
            GLOBAL_VARIABLE_WRITE_NODE,
            IF_NODE,
            INSTANCE_VARIABLE_READ_NODE,
            INSTANCE_VARIABLE_WRITE_NODE,
            LOCAL_VARIABLE_READ_NODE,
            LOCAL_VARIABLE_WRITE_NODE,
            NIL_NODE,
            PARENTHESES_NODE,
            SELF_NODE,
            STATEMENTS_NODE,
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
        let enforced_style = config.get_str("EnforcedStyle", "require_no_parentheses");
        let allow_safe = config.get_bool("AllowSafeAssignment", true);
        let if_node = match node.as_if_node() {
            Some(n) => n,
            None => return,
        };

        // Ternary has no if_keyword_loc
        if if_node.if_keyword_loc().is_some() {
            return;
        }

        let predicate = if_node.predicate();
        let is_parenthesized = predicate.as_parentheses_node().is_some();

        // AllowSafeAssignment: skip if condition is a parenthesized assignment
        if allow_safe && is_parenthesized {
            if let Some(paren) = predicate.as_parentheses_node() {
                if is_ternary_safe_assignment(&paren) {
                    return;
                }
            }
        }

        if is_parenthesized {
            if let Some(paren) = predicate.as_parentheses_node() {
                if is_parenthesized_one_line_pattern_matching(&paren) {
                    return;
                }
            }
        }

        match enforced_style {
            "require_parentheses" => {
                if !is_parenthesized {
                    let loc = predicate.location();
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Use parentheses for ternary conditions.".to_string(),
                    ));
                }
            }
            "require_parentheses_when_complex" => {
                let is_complex = complex_condition(&predicate);
                if is_complex && !is_parenthesized {
                    let loc = predicate.location();
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    diagnostics.push(
                        self.diagnostic(
                            source,
                            line,
                            column,
                            "Use parentheses for ternary expressions with complex conditions."
                                .to_string(),
                        ),
                    );
                } else if !is_complex && is_parenthesized {
                    let paren = predicate.as_parentheses_node().unwrap();
                    let open_loc = paren.opening_loc();
                    let (line, column) = source.offset_to_line_col(open_loc.start_offset());
                    diagnostics.push(
                        self.diagnostic(
                            source,
                            line,
                            column,
                            "Only use parentheses for ternary expressions with complex conditions."
                                .to_string(),
                        ),
                    );
                }
            }
            _ => {
                // "require_no_parentheses" (default)
                if is_parenthesized {
                    let paren = predicate.as_parentheses_node().unwrap();
                    let open_loc = paren.opening_loc();
                    let (line, column) = source.offset_to_line_col(open_loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Ternary conditions should not be wrapped in parentheses.".to_string(),
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{run_cop_full, run_cop_full_with_config};

    crate::cop_fixture_tests!(TernaryParentheses, "cops/style/ternary_parentheses");

    #[test]
    fn require_parentheses_flags_missing() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("require_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        // No parens should be flagged
        let source = b"x = foo? ? 'a' : 'b'\n";
        let diags = run_cop_full_with_config(&TernaryParentheses, source, config.clone());
        assert_eq!(
            diags.len(),
            1,
            "Should flag missing parens with require_parentheses"
        );
        assert!(diags[0].message.contains("Use parentheses"));

        // With parens should be OK
        let source2 = b"x = (foo?) ? 'a' : 'b'\n";
        let diags2 = run_cop_full_with_config(&TernaryParentheses, source2, config);
        assert!(
            diags2.is_empty(),
            "Should allow parens with require_parentheses"
        );
    }

    #[test]
    fn allow_safe_assignment_in_ternary() {
        // Default AllowSafeAssignment is true, so (x = y) ? a : b should be allowed
        let source = b"(x = y) ? 'a' : 'b'\n";
        let diags = run_cop_full(&TernaryParentheses, source);
        assert!(diags.is_empty(), "Should allow safe assignment parens");
    }

    #[test]
    fn defined_is_not_complex() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("require_parentheses_when_complex".into()),
            )]),
            ..CopConfig::default()
        };
        // defined? is non-complex — should not require parens
        let source = b"x = defined?(Foo) ? Foo : nil\n";
        let diags = run_cop_full_with_config(&TernaryParentheses, source, config.clone());
        assert!(
            diags.is_empty(),
            "defined? should not be considered complex: {:?}",
            diags
        );

        // yield is non-complex
        let source2 = b"x = yield ? 1 : 0\n";
        let diags2 = run_cop_full_with_config(&TernaryParentheses, source2, config);
        assert!(
            diags2.is_empty(),
            "yield should not be considered complex: {:?}",
            diags2
        );
    }

    #[test]
    fn literal_conditions_are_complex_when_style_requires_parentheses() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("require_parentheses_when_complex".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"true ? 1 : 0\nfalse ? 1 : 0\nnil ? 1 : 0\nself ? 1 : 0\n";
        let diags = run_cop_full_with_config(&TernaryParentheses, source, config);

        assert_eq!(
            diags.len(),
            4,
            "literal predicates should be considered complex: {:?}",
            diags
        );
        assert_eq!(diags[0].location.line, 1);
        assert_eq!(diags[1].location.line, 2);
        assert_eq!(diags[2].location.line, 3);
        assert_eq!(diags[3].location.line, 4);
        assert!(
            diags
                .iter()
                .all(|diag| diag.message.contains("Use parentheses")),
            "expected complex-condition message for literal predicates: {:?}",
            diags
        );
    }

    #[test]
    fn disallow_safe_assignment() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([("AllowSafeAssignment".into(), serde_yml::Value::Bool(false))]),
            ..CopConfig::default()
        };
        let source = b"(x = y) ? 'a' : 'b'\n";
        let diags = run_cop_full_with_config(&TernaryParentheses, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag safe assignment parens when disallowed"
        );
    }

    #[test]
    fn allows_parenthesized_one_line_pattern_matching() {
        let source = b"(descriptor in Element[slot:]) ? slot : nil\n";
        let diags = run_cop_full(&TernaryParentheses, source);
        assert!(
            diags.is_empty(),
            "Should allow parenthesized one-line pattern matching: {:?}",
            diags
        );
    }

    #[test]
    fn require_parentheses_when_complex_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &TernaryParentheses,
            include_bytes!(
                "../../../tests/fixtures/cops/style/ternary_parentheses/require_parentheses_when_complex_offense.rb"
            ),
            {
                let mut options = std::collections::HashMap::new();
                options.insert(
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("require_parentheses_when_complex".into()),
                );
                CopConfig {
                    options,
                    ..CopConfig::default()
                }
            },
        );
    }

    #[test]
    fn require_parentheses_when_complex_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &TernaryParentheses,
            include_bytes!(
                "../../../tests/fixtures/cops/style/ternary_parentheses/require_parentheses_when_complex_no_offense.rb"
            ),
            {
                let mut options = std::collections::HashMap::new();
                options.insert(
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("require_parentheses_when_complex".into()),
                );
                CopConfig {
                    options,
                    ..CopConfig::default()
                }
            },
        );
    }
}
