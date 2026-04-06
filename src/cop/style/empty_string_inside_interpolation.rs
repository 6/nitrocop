use crate::cop::shared::node_type::EMBEDDED_STATEMENTS_NODE;
use crate::cop::shared::util;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Prism exposes every `#{...}` body as an `EmbeddedStatementsNode`, including
/// double-quoted strings, backticks, regexps, and symbols. RuboCop's default
/// style only inspects the top-level conditional in the interpolation body, but
/// `EnforcedStyle=ternary` walks descendant modifier `if`/`unless` nodes and
/// reports the interpolation opener itself. The previous port reused the
/// top-level logic for `ternary`, which missed nested modifier conditionals in
/// multi-statement interpolations and reported multiline offenses on the inner
/// `if` line instead of the `#{` opener.
pub struct EmptyStringInsideInterpolation;

impl Cop for EmptyStringInsideInterpolation {
    fn name(&self) -> &'static str {
        "Style/EmptyStringInsideInterpolation"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[EMBEDDED_STATEMENTS_NODE]
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
        let enforced_style = config.get_str("EnforcedStyle", "trailing_conditional");

        let embedded = if let Some(node) = node.as_embedded_statements_node() {
            node
        } else {
            return;
        };

        let Some(statements) = embedded.statements() else {
            return;
        };

        match enforced_style {
            "trailing_conditional" => {
                let stmt_list: Vec<_> = statements.body().iter().collect();
                if stmt_list.len() != 1 {
                    return;
                }

                if let Some(if_node) = stmt_list[0].as_if_node() {
                    if branch_is_empty(if_node.statements())
                        || else_branch_is_empty(if_node.subsequent())
                    {
                        add_diagnostic(self, source, &stmt_list[0], diagnostics, MSG_TERNARY);
                    }
                } else if let Some(unless_node) = stmt_list[0].as_unless_node() {
                    if branch_is_empty(unless_node.statements())
                        || branch_is_empty(
                            unless_node.else_clause().and_then(|node| node.statements()),
                        )
                    {
                        add_diagnostic(self, source, &stmt_list[0], diagnostics, MSG_TERNARY);
                    }
                }
            }
            "ternary" => {
                // RuboCop's each_child_node(:if) only checks direct children
                // of the interpolation, not all descendants. Match that by
                // iterating the statements body directly.
                let embedded_node = embedded.as_node();
                for stmt in statements.body().iter() {
                    let is_modifier = stmt
                        .as_if_node()
                        .map_or(false, |n| util::is_modifier_if(&n))
                        || stmt
                            .as_unless_node()
                            .map_or(false, |n| util::is_modifier_unless(&n));
                    if is_modifier {
                        add_diagnostic(
                            self,
                            source,
                            &embedded_node,
                            diagnostics,
                            MSG_TRAILING_CONDITIONAL,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

const MSG_TRAILING_CONDITIONAL: &str = "Do not use trailing conditionals in string interpolation.";
const MSG_TERNARY: &str = "Do not return empty strings in string interpolation.";

fn add_diagnostic(
    cop: &EmptyStringInsideInterpolation,
    source: &SourceFile,
    node: &ruby_prism::Node<'_>,
    diagnostics: &mut Vec<Diagnostic>,
    message: &str,
) {
    let loc = node.location();
    let (line, column) = source.offset_to_line_col(loc.start_offset());
    diagnostics.push(cop.diagnostic(source, line, column, message.to_string()));
}

fn branch_is_empty(branch: Option<ruby_prism::StatementsNode<'_>>) -> bool {
    let Some(statements) = branch else {
        return false;
    };

    let body: Vec<_> = statements.body().iter().collect();
    body.len() == 1 && is_empty_string_or_nil(&body[0])
}

fn else_branch_is_empty(branch: Option<ruby_prism::Node<'_>>) -> bool {
    branch
        .and_then(|node| node.as_else_node())
        .is_some_and(|else_node| branch_is_empty(else_node.statements()))
}

fn is_empty_string_or_nil(node: &ruby_prism::Node<'_>) -> bool {
    if node.as_nil_node().is_some() {
        return true;
    }
    if let Some(string_node) = node.as_string_node() {
        return string_node.content_loc().as_slice().is_empty();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yml::Value;

    crate::cop_fixture_tests!(
        EmptyStringInsideInterpolation,
        "cops/style/empty_string_inside_interpolation"
    );

    fn ternary_config() -> CopConfig {
        let mut config = CopConfig::default();
        config.options.insert(
            "EnforcedStyle".to_string(),
            Value::String("ternary".to_string()),
        );
        config
    }

    #[test]
    fn ternary_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &EmptyStringInsideInterpolation,
            include_bytes!(
                "../../../tests/fixtures/cops/style/empty_string_inside_interpolation/ternary_offense.rb"
            ),
            ternary_config(),
        );
    }

    #[test]
    fn ternary_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &EmptyStringInsideInterpolation,
            include_bytes!(
                "../../../tests/fixtures/cops/style/empty_string_inside_interpolation/ternary_no_offense.rb"
            ),
            ternary_config(),
        );
    }
}
