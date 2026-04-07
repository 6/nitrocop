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
            let params = def_node.parameters().unwrap();
            let loc = params.location();
            let is_multiline = source
                .byte_slice(loc.start_offset(), loc.end_offset(), "")
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
}
