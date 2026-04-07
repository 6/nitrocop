use crate::cop::shared::node_type::DEF_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Verifies that method definitions use the correct parentheses style.
///
/// For `require_no_parentheses_except_multiline`: only requires parentheses when
/// method definition arguments span multiple lines. Single-line arguments without
/// parentheses are accepted (per RuboCop behavior where `args.multiline?` determines
/// whether parentheses are required).
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

        // Only apply to methods with parameters
        let params = match def_node.parameters() {
            Some(p) => p,
            None => return,
        };

        // Check if there are actual parameters
        if params.requireds().is_empty()
            && params.optionals().is_empty()
            && params.rest().is_none()
            && params.posts().is_empty()
            && params.keywords().is_empty()
            && params.keyword_rest().is_none()
            && params.block().is_none()
        {
            return;
        }

        let has_parens = def_node.lparen_loc().is_some();

        // For require_no_parentheses_except_multiline: only require parens if args span multiple lines
        if enforced_style == "require_no_parentheses_except_multiline" && !has_parens {
            let is_multiline = source
                .byte_slice(
                    params.location().start_offset(),
                    params.location().end_offset(),
                    "",
                )
                .contains('\n');
            if !is_multiline {
                return;
            }
        }

        match enforced_style {
            "require_parentheses" | "require_no_parentheses_except_multiline" if !has_parens => {
                // RuboCop points at the arguments (parameters), not the `def` keyword
                let params_loc = params.location();
                let (line, column) = source.offset_to_line_col(params_loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Use `def` with parentheses when there are parameters.".to_string(),
                ));
            }
            "require_no_parentheses" if has_parens => {
                // RuboCop points at the args node including parens — use lparen_loc
                let start = def_node
                    .lparen_loc()
                    .map_or_else(|| params.location().start_offset(), |lp| lp.start_offset());
                let (line, column) = source.offset_to_line_col(start);
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Use `def` without parentheses.".to_string(),
                ));
            }
            _ => {}
        }
    }
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
