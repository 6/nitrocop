use crate::cop::shared::node_type::DEF_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Verifies that method definitions use the correct parentheses style.
///
/// Variant behavior diverges from the default in two important RuboCop quirks:
/// `require_no_parentheses` and `require_no_parentheses_except_multiline` still
/// register `def foo()` as an offense even though there are no arguments, but
/// they must keep parentheses for endless defs and any rest/kwrest/forwarding
/// argument list where removing them would be a syntax error.
///
/// Fixed variant divergence:
/// - `**nil` (`NoKeywordsParameterNode`) does NOT force parentheses, unlike
///   `**opts`/`**`/`...`. RuboCop's `anonymous_arguments?` checks for `:kwrestarg`
///   but not `:kwnilarg`, so `def foo(**nil)` should be flagged. (FN fix)
/// - For `require_no_parentheses_except_multiline`, the multiline check must
///   consider the lparen..rparen span even when there are no actual parameters.
///   `def foo(\n)` has multiline args per RuboCop and is not an offense. (FP fix)
pub struct MethodDefParentheses;

impl Cop for MethodDefParentheses {
    fn name(&self) -> &'static str {
        "Style/MethodDefParentheses"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[DEF_NODE]
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
        let enforced_style = config.get_str("EnforcedStyle", "require_parentheses");

        let def_node = match node.as_def_node() {
            Some(d) => d,
            None => return,
        };

        let params = def_node.parameters();
        let has_actual_parameters = params
            .as_ref()
            .is_some_and(|params| has_actual_parameters(params));

        let has_parens = def_node.lparen_loc().is_some();

        match enforced_style {
            "require_parentheses" if has_actual_parameters && !has_parens => {
                let params_loc = params
                    .expect("defs with actual parameters always have a parameters node")
                    .location();
                let (line, column) = source.offset_to_line_col(params_loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Use `def` with parentheses when there are parameters.".to_string(),
                ));
            }
            "require_no_parentheses"
                if has_parens && !is_forced_parentheses(&def_node, params.as_ref()) =>
            {
                let start = def_node
                    .lparen_loc()
                    .expect("defs with parentheses always have a left paren")
                    .start_offset();
                let (line, column) = source.offset_to_line_col(start);
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Use `def` without parentheses.".to_string(),
                ));
            }
            "require_no_parentheses_except_multiline" => {
                // RuboCop's `args.multiline?` checks the arguments node which
                // includes parentheses. When parens are present, check the
                // lparen..rparen span — even for empty parens like `def foo(\n)`,
                // RuboCop considers the args node multiline if parens span lines.
                let is_multiline = if has_parens {
                    let start = def_node.lparen_loc().unwrap().start_offset();
                    let end = def_node.rparen_loc().unwrap().end_offset();
                    source.byte_slice(start, end, "").contains('\n')
                } else {
                    match params.as_ref() {
                        Some(params) if has_actual_parameters => source
                            .byte_slice(
                                params.location().start_offset(),
                                params.location().end_offset(),
                                "",
                            )
                            .contains('\n'),
                        _ => false,
                    }
                };

                if is_multiline && !has_parens {
                    // Multiline args need parentheses
                    let params_loc = params
                        .expect(
                            "multiline defs with actual parameters always have a parameters node",
                        )
                        .location();
                    let (line, column) = source.offset_to_line_col(params_loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Use `def` with parentheses when there are parameters.".to_string(),
                    ));
                } else if !is_multiline
                    && has_parens
                    && !is_forced_parentheses(&def_node, params.as_ref())
                {
                    // Single-line args should not have parentheses (unless forced)
                    let start = def_node
                        .lparen_loc()
                        .expect("defs with parentheses always have a left paren")
                        .start_offset();
                    let (line, column) = source.offset_to_line_col(start);
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Use `def` without parentheses.".to_string(),
                    ));
                }
            }
            _ => {}
        }
    }
}

/// Checks if parentheses are syntactically required and cannot be removed.
/// Matches RuboCop's `forced_parentheses?` logic: endless methods, forwarding
/// parameters (`...`), any rest arg (`*`/`*args`), any keyword rest (`**`/`**opts`),
/// and anonymous block forwarding (`&`).
fn is_forced_parentheses(
    def_node: &ruby_prism::DefNode<'_>,
    params: Option<&ruby_prism::ParametersNode<'_>>,
) -> bool {
    // Endless method (def foo(x) = ...)
    if def_node.equal_loc().is_some() {
        return true;
    }

    let Some(params) = params else {
        return false;
    };

    // Any rest arg (*args or *)
    if params.rest().is_some() {
        return true;
    }
    // Any keyword rest (**opts, **) or forwarding parameter (...).
    // **nil (NoKeywordsParameterNode) does NOT force parentheses — `def foo **nil`
    // is valid syntax. RuboCop's `anonymous_arguments?` checks for :kwrestarg but
    // not :kwnilarg, so **nil is not considered forced.
    if let Some(kw_rest) = params.keyword_rest() {
        if kw_rest.as_no_keywords_parameter_node().is_none() {
            return true;
        }
    }
    // Anonymous block forwarding (&)
    if let Some(block) = params.block() {
        if block.name().is_none() {
            return true;
        }
    }
    false
}

fn has_actual_parameters(params: &ruby_prism::ParametersNode<'_>) -> bool {
    !(params.requireds().is_empty()
        && params.optionals().is_empty()
        && params.rest().is_none()
        && params.posts().is_empty()
        && params.keywords().is_empty()
        && params.keyword_rest().is_none()
        && params.block().is_none())
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(MethodDefParentheses, "cops/style/method_def_parentheses");

    fn style_config(style: &str) -> CopConfig {
        let mut opts = std::collections::HashMap::new();
        opts.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String(style.to_string()),
        );
        CopConfig {
            options: opts,
            ..CopConfig::default()
        }
    }

    #[test]
    fn require_no_parentheses_except_multiline_offense() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &MethodDefParentheses,
            include_bytes!(
                "../../../tests/fixtures/cops/style/method_def_parentheses/require_no_parentheses_except_multiline_offense.rb"
            ),
            style_config("require_no_parentheses_except_multiline"),
        );
    }

    #[test]
    fn require_no_parentheses_except_multiline_no_offense() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &MethodDefParentheses,
            include_bytes!(
                "../../../tests/fixtures/cops/style/method_def_parentheses/require_no_parentheses_except_multiline_no_offense.rb"
            ),
            style_config("require_no_parentheses_except_multiline"),
        );
    }

    #[test]
    fn require_no_parentheses_offense() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &MethodDefParentheses,
            include_bytes!(
                "../../../tests/fixtures/cops/style/method_def_parentheses/require_no_parentheses_offense.rb"
            ),
            style_config("require_no_parentheses"),
        );
    }

    #[test]
    fn require_no_parentheses_no_offense() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &MethodDefParentheses,
            include_bytes!(
                "../../../tests/fixtures/cops/style/method_def_parentheses/require_no_parentheses_no_offense.rb"
            ),
            style_config("require_no_parentheses"),
        );
    }
}
