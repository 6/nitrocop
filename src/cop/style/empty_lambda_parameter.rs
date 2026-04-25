use crate::cop::shared::node_type::{BLOCK_PARAMETERS_NODE, LAMBDA_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

pub struct EmptyLambdaParameter;

impl Cop for EmptyLambdaParameter {
    fn name(&self) -> &'static str {
        "Style/EmptyLambdaParameter"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[BLOCK_PARAMETERS_NODE, LAMBDA_NODE]
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        // Check LambdaNode for empty parameters: -> () {}
        let lambda_node = match node.as_lambda_node() {
            Some(l) => l,
            None => return,
        };

        // Check if operator is -> (stabby lambda)
        let operator_loc = lambda_node.operator_loc();
        if operator_loc.as_slice() != b"->" {
            return;
        }

        // Check parameters
        let params = match lambda_node.parameters() {
            Some(p) => p,
            None => return,
        };

        // For lambdas, parameters can be a BlockParametersNode
        // -> () {} would have a BlockParametersNode with opening_loc "(" and empty params
        let bp = match params.as_block_parameters_node() {
            Some(bp) => bp,
            None => return,
        };

        // Must have parentheses as opening/closing
        let opening_loc = match bp.opening_loc() {
            Some(loc) => loc,
            None => return,
        };

        if opening_loc.as_slice() != b"(" {
            return;
        }

        // Parameters must be empty
        if let Some(inner_params) = bp.parameters() {
            let has_params = !inner_params.requireds().is_empty()
                || !inner_params.optionals().is_empty()
                || inner_params.rest().is_some()
                || !inner_params.posts().is_empty()
                || !inner_params.keywords().is_empty()
                || inner_params.keyword_rest().is_some()
                || inner_params.block().is_some();
            if has_params {
                return;
            }
        }

        let (line, column) = source.offset_to_line_col(opening_loc.start_offset());
        let mut diag = self.diagnostic(
            source,
            line,
            column,
            "Omit parentheses for the empty lambda parameters.".to_string(),
        );
        if let Some(corr) = corrections {
            // Remove the entire BlockParametersNode source range — i.e. `()`.
            // Matches RuboCop's `corrector.remove(node.parameters.source_range)`.
            // Any extra whitespace left between `->` and `{` is cleaned up by
            // Layout cops on a later iteration.
            let bp_loc = bp.location();
            corr.push(crate::correction::Correction {
                start: bp_loc.start_offset(),
                end: bp_loc.end_offset(),
                replacement: String::new(),
                cop_name: self.name(),
                cop_index: 0,
            });
            diag.corrected = true;
        }
        diagnostics.push(diag);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(EmptyLambdaParameter, "cops/style/empty_lambda_parameter");
    crate::cop_autocorrect_fixture_tests!(
        EmptyLambdaParameter,
        "cops/style/empty_lambda_parameter"
    );
}
