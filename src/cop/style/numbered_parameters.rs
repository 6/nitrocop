use crate::cop::shared::node_type::{
    BLOCK_NODE, CALL_NODE, FORWARDING_SUPER_NODE, LAMBDA_NODE, NUMBERED_PARAMETERS_NODE, SUPER_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Mirrors RuboCop's `on_numblock` across all Prism shapes that can carry
/// numbered-parameter blocks.
///
/// ## Investigation findings (2026-04-08)
///
/// Variant `EnforcedStyle: disallow` had a false negative on
/// `super { [_1.name, _1] }` from `rom-rb/rom`. Prism represents no-argument
/// `super` with a block as `ForwardingSuperNode` and argument-bearing `super(...)`
/// with a block as `SuperNode`, not `CallNode`, so the previous implementation
/// never reached its numbered-parameter check for either form. The default
/// `allow_single_line` style stayed clean because the missed corpus case was a
/// single-line block.
///
/// Fix: inspect `SuperNode` and `ForwardingSuperNode` blocks the same way as
/// method-call blocks, while keeping the existing multiline-only behavior for
/// the default style.
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
            self.check_block_style(
                source,
                loc.start_offset(),
                loc.start_offset(),
                loc.end_offset(),
                style,
                diagnostics,
            );
            return;
        }

        if let Some(super_node) = node.as_super_node() {
            let block = match super_node.block() {
                Some(b) => b,
                None => return,
            };
            let block_node = match block.as_block_node() {
                Some(b) => b,
                None => return,
            };
            self.check_numbered_block(source, node, &block_node, style, diagnostics);
            return;
        }

        if let Some(fwd_super) = node.as_forwarding_super_node() {
            let block_node = match fwd_super.block() {
                Some(b) => b,
                None => return,
            };
            self.check_numbered_block(source, node, &block_node, style, diagnostics);
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

        self.check_numbered_block(source, node, &block_node, style, diagnostics);
    }
}

impl NumberedParameters {
    fn check_numbered_block(
        &self,
        source: &SourceFile,
        offense_node: &ruby_prism::Node<'_>,
        block_node: &ruby_prism::BlockNode<'_>,
        style: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
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

        let report_loc = offense_node.location();
        let block_loc = block_node.location();
        self.check_block_style(
            source,
            report_loc.start_offset(),
            block_loc.start_offset(),
            block_loc.end_offset(),
            style,
            diagnostics,
        );
    }

    fn check_block_style(
        &self,
        source: &SourceFile,
        report_start_offset: usize,
        block_start_offset: usize,
        block_end_offset: usize,
        style: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let (line, column) = source.offset_to_line_col(report_start_offset);

        if style == "disallow" {
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                "Avoid using numbered parameters.".to_string(),
            ));
            return;
        }

        if style == "allow_single_line" {
            let (start_line, _) = source.offset_to_line_col(block_start_offset);
            let (end_line, _) = source.offset_to_line_col(block_end_offset.saturating_sub(1));
            if start_line != end_line {
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

    fn disallow_config() -> crate::cop::CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("disallow".to_string()),
        );
        crate::cop::CopConfig {
            options,
            ..crate::cop::CopConfig::default()
        }
    }

    #[test]
    fn disallow_style_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &NumberedParameters,
            include_bytes!(
                "../../../tests/fixtures/cops/style/numbered_parameters/offense.disallow.rb"
            ),
            disallow_config(),
        );
    }
}
