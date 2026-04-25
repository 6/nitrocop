use crate::cop::shared::node_type::{CALL_NODE, DEF_NODE, LAMBDA_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Checks for redundant `return` in terminal position of `def`, `defs`,
/// `define_method`/`define_singleton_method` blocks, `lambda` blocks,
/// and stabby lambda (`->`).
///
/// Handles branching (if/unless/case/case-in), begin/rescue, nested control flow,
/// and rescue modifier (`return expr rescue fallback`).
///
/// Supports both `case/when` (CaseNode) and `case/in` pattern matching
/// (CaseMatchNode) for detecting redundant returns in branch terminal positions.
///
/// Skips ternary expressions (`a ? return : raise`) since RuboCop does not
/// flag `return` inside ternary branches. Also skips checking the main body
/// of `begin/rescue/else` when an else clause is present, since the else
/// clause determines the return value (not the main body).
pub struct RedundantReturn;

impl Cop for RedundantReturn {
    fn name(&self) -> &'static str {
        "Style/RedundantReturn"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[DEF_NODE, CALL_NODE, LAMBDA_NODE]
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
        let allow_multiple = config.get_bool("AllowMultipleReturnValues", false);

        // DefNode: check the method body
        if let Some(def_node) = node.as_def_node() {
            if let Some(body) = def_node.body() {
                check_terminal(
                    self,
                    source,
                    &body,
                    allow_multiple,
                    diagnostics,
                    &mut corrections,
                );
            }
            return;
        }

        // CallNode: check blocks on define_method, define_singleton_method, lambda
        if let Some(call_node) = node.as_call_node() {
            let name = call_node.name();
            let name_str = std::str::from_utf8(name.as_slice()).unwrap_or("");
            match name_str {
                "define_method" | "define_singleton_method" | "lambda" => {
                    if let Some(block) = call_node.block() {
                        if let Some(block_node) = block.as_block_node() {
                            if let Some(body) = block_node.body() {
                                check_terminal(
                                    self,
                                    source,
                                    &body,
                                    allow_multiple,
                                    diagnostics,
                                    &mut corrections,
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
            return;
        }

        // LambdaNode: check stabby lambda body (-> { ... })
        if let Some(lambda_node) = node.as_lambda_node() {
            if let Some(body) = lambda_node.body() {
                check_terminal(
                    self,
                    source,
                    &body,
                    allow_multiple,
                    diagnostics,
                    &mut corrections,
                );
            }
        }
    }
}

/// Recursively check terminal positions for redundant `return` statements.
/// A terminal position is the last expression that would be implicitly returned.
fn check_terminal(
    cop: &RedundantReturn,
    source: &SourceFile,
    node: &ruby_prism::Node<'_>,
    allow_multiple: bool,
    diagnostics: &mut Vec<Diagnostic>,
    corrections: &mut Option<&mut Vec<crate::correction::Correction>>,
) {
    // StatementsNode: check the last statement
    if let Some(stmts) = node.as_statements_node() {
        if let Some(last) = stmts.body().last() {
            check_terminal(cop, source, &last, allow_multiple, diagnostics, corrections);
        }
        return;
    }

    // ReturnNode: this is a redundant return in terminal position
    if let Some(ret_node) = node.as_return_node() {
        if allow_multiple {
            let arg_count = ret_node.arguments().map_or(0, |a| a.arguments().len());
            if arg_count > 1 {
                return;
            }
        }
        let loc = node.location();
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        let mut diag = cop.diagnostic(
            source,
            line,
            column,
            "Redundant `return` detected.".to_string(),
        );
        if let Some(corr) = corrections.as_mut() {
            if let Some(replacement) = autocorrect_replacement(source, &ret_node) {
                corr.push(crate::correction::Correction {
                    start: loc.start_offset(),
                    end: loc.end_offset(),
                    replacement,
                    cop_name: cop.name(),
                    cop_index: 0,
                });
                diag.corrected = true;
            }
        }
        diagnostics.push(diag);
        return;
    }

    // RescueModifierNode: `return expr rescue fallback` — check the inner expression
    if let Some(rescue_mod) = node.as_rescue_modifier_node() {
        check_terminal(
            cop,
            source,
            &rescue_mod.expression(),
            allow_multiple,
            diagnostics,
            corrections,
        );
        return;
    }

    // IfNode: check terminal position in each branch (skip ternary expressions)
    if let Some(if_node) = node.as_if_node() {
        // Ternary expressions (a ? b : c) have no if_keyword_loc
        if if_node.if_keyword_loc().is_none() {
            return;
        }
        if let Some(stmts) = if_node.statements() {
            check_terminal_stmts(
                cop,
                source,
                &stmts,
                allow_multiple,
                diagnostics,
                corrections,
            );
        }
        if let Some(subsequent) = if_node.subsequent() {
            if let Some(elsif) = subsequent.as_if_node() {
                check_terminal(
                    cop,
                    source,
                    &elsif.as_node(),
                    allow_multiple,
                    diagnostics,
                    corrections,
                );
            } else if let Some(else_node) = subsequent.as_else_node() {
                if let Some(stmts) = else_node.statements() {
                    check_terminal_stmts(
                        cop,
                        source,
                        &stmts,
                        allow_multiple,
                        diagnostics,
                        corrections,
                    );
                }
            }
        }
        return;
    }

    // UnlessNode: check terminal position in each branch
    if let Some(unless_node) = node.as_unless_node() {
        if let Some(stmts) = unless_node.statements() {
            check_terminal_stmts(
                cop,
                source,
                &stmts,
                allow_multiple,
                diagnostics,
                corrections,
            );
        }
        if let Some(else_clause) = unless_node.else_clause() {
            if let Some(stmts) = else_clause.statements() {
                check_terminal_stmts(
                    cop,
                    source,
                    &stmts,
                    allow_multiple,
                    diagnostics,
                    corrections,
                );
            }
        }
        return;
    }

    // CaseNode: check terminal position in each when/else branch
    if let Some(case_node) = node.as_case_node() {
        for condition in case_node.conditions().iter() {
            if let Some(when_node) = condition.as_when_node() {
                if let Some(stmts) = when_node.statements() {
                    check_terminal_stmts(
                        cop,
                        source,
                        &stmts,
                        allow_multiple,
                        diagnostics,
                        corrections,
                    );
                }
            }
        }
        if let Some(else_clause) = case_node.else_clause() {
            if let Some(stmts) = else_clause.statements() {
                check_terminal_stmts(
                    cop,
                    source,
                    &stmts,
                    allow_multiple,
                    diagnostics,
                    corrections,
                );
            }
        }
        return;
    }

    // CaseMatchNode: check terminal position in each in/else branch (pattern matching)
    if let Some(case_match_node) = node.as_case_match_node() {
        for condition in case_match_node.conditions().iter() {
            if let Some(in_node) = condition.as_in_node() {
                if let Some(stmts) = in_node.statements() {
                    check_terminal_stmts(
                        cop,
                        source,
                        &stmts,
                        allow_multiple,
                        diagnostics,
                        corrections,
                    );
                }
            }
        }
        if let Some(else_clause) = case_match_node.else_clause() {
            if let Some(stmts) = else_clause.statements() {
                check_terminal_stmts(
                    cop,
                    source,
                    &stmts,
                    allow_multiple,
                    diagnostics,
                    corrections,
                );
            }
        }
        return;
    }

    // BeginNode: check statements body and rescue clauses
    if let Some(begin_node) = node.as_begin_node() {
        let has_rescue = begin_node.rescue_clause().is_some();
        let has_else = begin_node.else_clause().is_some();

        // Only check main body if there's no rescue+else combination.
        // When rescue has an else clause, the else clause's value is the
        // return value (not the main body), so returns in the main body
        // are early exits, not redundant.
        if !has_rescue || !has_else {
            if let Some(stmts) = begin_node.statements() {
                check_terminal_stmts(
                    cop,
                    source,
                    &stmts,
                    allow_multiple,
                    diagnostics,
                    corrections,
                );
            }
        }
        // Check rescue clauses
        if let Some(rescue) = begin_node.rescue_clause() {
            check_rescue_terminal(
                cop,
                source,
                &rescue,
                allow_multiple,
                diagnostics,
                corrections,
            );
        }
        // Check else clause on begin/rescue/else
        if let Some(else_clause) = begin_node.else_clause() {
            if let Some(stmts) = else_clause.statements() {
                check_terminal_stmts(
                    cop,
                    source,
                    &stmts,
                    allow_multiple,
                    diagnostics,
                    corrections,
                );
            }
        }
        return;
    }

    // RescueNode (implicit rescue on def body): check each rescue clause
    if let Some(rescue_node) = node.as_rescue_node() {
        // The rescue node's own statements
        if let Some(stmts) = rescue_node.statements() {
            check_terminal_stmts(
                cop,
                source,
                &stmts,
                allow_multiple,
                diagnostics,
                corrections,
            );
        }
        // Subsequent rescue clauses
        if let Some(subsequent) = rescue_node.subsequent() {
            check_rescue_terminal(
                cop,
                source,
                &subsequent,
                allow_multiple,
                diagnostics,
                corrections,
            );
        }
    }
}

/// Check the last statement in a StatementsNode as a terminal position.
fn check_terminal_stmts(
    cop: &RedundantReturn,
    source: &SourceFile,
    stmts: &ruby_prism::StatementsNode<'_>,
    allow_multiple: bool,
    diagnostics: &mut Vec<Diagnostic>,
    corrections: &mut Option<&mut Vec<crate::correction::Correction>>,
) {
    if let Some(last) = stmts.body().last() {
        check_terminal(cop, source, &last, allow_multiple, diagnostics, corrections);
    }
}

/// Recursively check rescue clause chains for redundant returns.
fn check_rescue_terminal(
    cop: &RedundantReturn,
    source: &SourceFile,
    rescue: &ruby_prism::RescueNode<'_>,
    allow_multiple: bool,
    diagnostics: &mut Vec<Diagnostic>,
    corrections: &mut Option<&mut Vec<crate::correction::Correction>>,
) {
    if let Some(stmts) = rescue.statements() {
        check_terminal_stmts(
            cop,
            source,
            &stmts,
            allow_multiple,
            diagnostics,
            corrections,
        );
    }
    if let Some(subsequent) = rescue.subsequent() {
        check_rescue_terminal(
            cop,
            source,
            &subsequent,
            allow_multiple,
            diagnostics,
            corrections,
        );
    }
}

/// Build the autocorrect replacement text for a redundant `return` node.
///
/// Mirrors RuboCop's `Style/RedundantReturn` autocorrect:
/// - `return`            → `nil`
/// - `return foo`        → `foo` (single non-splat argument)
/// - `return *foo`       → `[*foo]` (single splat argument, wrapped to keep semantics)
/// - `return foo, bar`   → `[foo, bar]` (multiple arguments)
///
/// Returns `None` if the byte range cannot be sliced as UTF-8 (defensive — Ruby
/// source is UTF-8 in practice).
fn autocorrect_replacement(
    source: &SourceFile,
    ret_node: &ruby_prism::ReturnNode<'_>,
) -> Option<String> {
    let args = match ret_node.arguments() {
        Some(a) => a,
        None => return Some("nil".to_string()),
    };
    let arg_list: Vec<_> = args.arguments().iter().collect();
    match arg_list.len() {
        0 => Some("nil".to_string()),
        1 if arg_list[0].as_splat_node().is_none() => {
            let loc = arg_list[0].location();
            source
                .try_byte_slice(loc.start_offset(), loc.end_offset())
                .map(|s| s.to_string())
        }
        _ => {
            let first = arg_list.first().unwrap().location().start_offset();
            let last = arg_list.last().unwrap().location().end_offset();
            source
                .try_byte_slice(first, last)
                .map(|s| format!("[{}]", s))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{run_cop_full, run_cop_full_with_config};

    crate::cop_fixture_tests!(RedundantReturn, "cops/style/redundant_return");
    crate::cop_autocorrect_fixture_tests!(RedundantReturn, "cops/style/redundant_return");

    #[test]
    fn allow_multiple_return_values() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "AllowMultipleReturnValues".into(),
                serde_yml::Value::Bool(true),
            )]),
            ..CopConfig::default()
        };
        // `return x, y` should be allowed when AllowMultipleReturnValues is true
        let source = b"def foo\n  return x, y\nend\n";
        let diags = run_cop_full_with_config(&RedundantReturn, source, config);
        assert!(
            diags.is_empty(),
            "Should allow multiple return values when configured"
        );
    }

    #[test]
    fn disallow_multiple_return_values_by_default() {
        // `return x, y` should be flagged by default
        let source = b"def foo\n  return x, y\nend\n";
        let diags = run_cop_full(&RedundantReturn, source);
        assert_eq!(
            diags.len(),
            1,
            "Should flag multiple return values by default"
        );
    }

    #[test]
    fn allow_multiple_still_flags_single_return() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "AllowMultipleReturnValues".into(),
                serde_yml::Value::Bool(true),
            )]),
            ..CopConfig::default()
        };
        // `return x` should still be flagged even with AllowMultipleReturnValues
        let source = b"def foo\n  return x\nend\n";
        let diags = run_cop_full_with_config(&RedundantReturn, source, config);
        assert_eq!(diags.len(), 1, "Single return should still be flagged");
    }
}
