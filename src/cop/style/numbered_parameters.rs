use crate::cop::shared::node_type::{
    BLOCK_NODE, CALL_NODE, FORWARDING_SUPER_NODE, LAMBDA_NODE, NUMBERED_PARAMETERS_NODE, SUPER_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Also handles `LambdaNode` (`-> do ... end` / `-> { ... }`) with numbered
/// parameters, not just method-call blocks. RuboCop's `on_numblock` fires for
/// both block types.
///
/// ## Variant Fix (2026-04-07)
///
/// The `disallow` style had a false negative: `super { [_1.name, _1] }`
/// was not flagged because `SuperNode` and `ForwardingSuperNode` were not
/// handled. Added `SUPER_NODE` and `FORWARDING_SUPER_NODE` to
/// `interested_node_types` and corresponding handling in `check_node` to detect
/// numbered parameters in `super { }` blocks, matching RuboCop's `on_numblock`.
pub struct NumberedParameters;

impl Cop for NumberedParameters {
    fn name(&self) -> &'static str {
        "Style/NumberedParameters"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            BLOCK_NODE,
            CALL_NODE,
            FORWARDING_SUPER_NODE,
            LAMBDA_NODE,
            NUMBERED_PARAMETERS_NODE,
            SUPER_NODE,
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
        let style = config.get_str("EnforcedStyle", "allow_single_line");

        // Handle LambdaNode (-> do...end / -> {...}) with numbered parameters.
        if let Some(lambda) = node.as_lambda_node() {
            let params = match lambda.parameters() {
                Some(p) => p,
                None => return,
            };
            if params.as_numbered_parameters_node().is_none() {
                return;
            }

            let loc = lambda.location();
            let (start_line, column) = source.offset_to_line_col(loc.start_offset());

            if style == "disallow" {
                diagnostics.push(self.diagnostic(
                    source,
                    start_line,
                    column,
                    "Avoid using numbered parameters.".to_string(),
                ));
            } else if style == "allow_single_line" {
                let (end_line, _) = source.offset_to_line_col(loc.end_offset().saturating_sub(1));
                if start_line != end_line {
                    diagnostics.push(self.diagnostic(
                        source,
                        start_line,
                        column,
                        "Avoid using numbered parameters for multi-line blocks.".to_string(),
                    ));
                }
            }
            return;
        }

        // Handle SuperNode (super { ... }) with numbered parameters.
        if let Some(super_node) = node.as_super_node() {
            let block = match super_node.block().and_then(|block| block.as_block_node()) {
                Some(b) => b,
                None => return,
            };
            let params = match block.parameters() {
                Some(p) => p,
                None => return,
            };
            if params.as_numbered_parameters_node().is_none() {
                return;
            }

            let loc = super_node.location();
            let (start_line, column) = source.offset_to_line_col(loc.start_offset());

            if style == "disallow" {
                diagnostics.push(self.diagnostic(
                    source,
                    start_line,
                    column,
                    "Avoid using numbered parameters.".to_string(),
                ));
            } else if style == "allow_single_line" {
                let (end_line, _) = source.offset_to_line_col(loc.end_offset().saturating_sub(1));
                if start_line != end_line {
                    diagnostics.push(self.diagnostic(
                        source,
                        start_line,
                        column,
                        "Avoid using numbered parameters for multi-line blocks.".to_string(),
                    ));
                }
            }
            return;
        }

        // Handle ForwardingSuperNode (super without args) with numbered parameters.
        if let Some(forwarding_super) = node.as_forwarding_super_node() {
            let block = match forwarding_super.block() {
                Some(b) => b,
                None => return,
            };
            let params = match block.parameters() {
                Some(p) => p,
                None => return,
            };
            if params.as_numbered_parameters_node().is_none() {
                return;
            }

            let loc = forwarding_super.location();
            let (start_line, column) = source.offset_to_line_col(loc.start_offset());

            if style == "disallow" {
                diagnostics.push(self.diagnostic(
                    source,
                    start_line,
                    column,
                    "Avoid using numbered parameters.".to_string(),
                ));
            } else if style == "allow_single_line" {
                let (end_line, _) = source.offset_to_line_col(loc.end_offset().saturating_sub(1));
                if start_line != end_line {
                    diagnostics.push(self.diagnostic(
                        source,
                        start_line,
                        column,
                        "Avoid using numbered parameters for multi-line blocks.".to_string(),
                    ));
                }
            }
            return;
        }

        // Check for method-call blocks that use numbered parameters.
        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let block = match call.block() {
            Some(b) => b,
            None => return,
        };

        let block_node = match block.as_block_node() {
            Some(b) => b,
            None => return,
        };

        // In Prism, blocks with numbered params have parameters() set to a
        // NumberedParametersNode. Blocks with explicit params have BlockParametersNode.
        // Only proceed if parameters is a NumberedParametersNode — this is the
        // authoritative way to detect numbered parameter usage via the AST,
        // avoiding false positives from string matching _1.._9 in comments,
        // strings, or variable names like _1_foo.
        let params = match block_node.parameters() {
            Some(p) => p,
            None => return,
        };

        if params.as_numbered_parameters_node().is_none() {
            return;
        }

        if style == "disallow" {
            let loc = node.location();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                "Avoid using numbered parameters.".to_string(),
            ));
        }

        if style == "allow_single_line" {
            // Flag if multi-line block
            let block_loc = block_node.location();
            let (start_line, _) = source.offset_to_line_col(block_loc.start_offset());
            let (end_line, _) = source.offset_to_line_col(block_loc.end_offset().saturating_sub(1));
            if start_line != end_line {
                let loc = node.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Avoid using numbered parameters for multi-line blocks.".to_string(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(NumberedParameters, "cops/style/numbered_parameters");
}
