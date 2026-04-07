use crate::cop::shared::node_type::{BLOCK_NODE, CALL_NODE, LAMBDA_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// ## Variant FP Fix (literal style)
///
/// With `EnforcedStyle: literal`, RuboCop's Style/Lambda only fires for
/// `on_block`/`on_lambda` callbacks, so it never sees bare `lambda` calls
/// without an attached block. Such calls are references to the lambda method
/// (e.g., `b = lambda` inside a block), not lambda expressions, and should
/// not be flagged.
///
/// Example of cases that should NOT be flagged under literal style:
/// - `1.times { b = lambda }` — lambda without a block, just a method reference
/// - `foo(lambda)` — lambda passed as argument without a block
///
/// The fix checks `call.block().is_none()` in the literal style branch to
/// skip these bare lambda calls.
///
/// ## Original Investigation (2026-03-11)
///
/// Corpus oracle reported FP=2, FN=2.
///
/// FP=2: Puppet passes lambdas as `lambda() { ... }` when probing callable
/// signatures. RuboCop keys on the selector source being exactly `lambda`, so
/// it ignores calls with an explicit empty arglist. Nitrocop previously flagged
/// those forms because it treated every bare `lambda` call with a block as a
/// style candidate.
///
/// FN=2: Bridgetown still reports two missing multiline literal cases in a
/// `rubylayout.rb` front-matter template. A generic `render html->{ <<~HTML ... }`
/// reproduction already matches RuboCop in fixtures, which suggests the
/// remaining corpus misses are likely file/context-specific rather than this
/// cop's core selector logic.
pub struct Lambda;

impl Cop for Lambda {
    fn name(&self) -> &'static str {
        "Style/Lambda"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[BLOCK_NODE, CALL_NODE, LAMBDA_NODE]
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
        let style = config.get_str("EnforcedStyle", "line_count_dependent");

        // Check -> (lambda literal) nodes
        if let Some(lambda_node) = node.as_lambda_node() {
            self.check_lambda_literal(source, &lambda_node, style, diagnostics);
            return;
        }

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        // Only bare `lambda` calls (no receiver)
        if call.receiver().is_some() {
            return;
        }

        if call.name().as_slice() != b"lambda" {
            return;
        }

        // RuboCop only considers block nodes whose selector source is exactly
        // `lambda`, so explicit empty-arg forms like `lambda() { ... }` are
        // ignored.
        if call.closing_loc().is_some() {
            return;
        }

        self.check_lambda_method(source, &call, style, diagnostics);
    }
}

impl Lambda {
    fn check_lambda_literal(
        &self,
        source: &SourceFile,
        lambda_node: &ruby_prism::LambdaNode<'_>,
        style: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let loc = lambda_node.operator_loc();
        let (start_line, _) = source.offset_to_line_col(lambda_node.location().start_offset());
        let end_off = lambda_node
            .location()
            .end_offset()
            .saturating_sub(1)
            .max(lambda_node.location().start_offset());
        let (end_line, _) = source.offset_to_line_col(end_off);
        let is_multiline = start_line != end_line;

        match style {
            "lambda" => {
                // Always flag `->` — use `lambda` instead
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Use the `lambda` method for all lambdas.".to_string(),
                ));
            }
            "literal" => {
                // `->` is preferred — no offense
            }
            _ => {
                // "line_count_dependent" (default):
                // Single-line `-> { }` is correct.
                // Multi-line `->() do ... end` should use `lambda` instead.
                if is_multiline {
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Use the `lambda` method for multiline lambdas.".to_string(),
                    ));
                }
            }
        }
    }

    fn check_lambda_method(
        &self,
        source: &SourceFile,
        call: &ruby_prism::CallNode<'_>,
        style: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        match style {
            "literal" => {
                // RuboCop's Style/Lambda only fires for on_block (and
                // on_lambda/on_numblock/on_itblock), so it never sees bare
                // `lambda` calls without an attached block. Such calls are just
                // references to the lambda method (e.g., `b = lambda` inside a
                // block) and should not be flagged.
                if call.block().is_none() {
                    return;
                }
                let loc = call.message_loc().unwrap_or_else(|| call.location());
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Use the `-> {}` lambda literal syntax for all lambdas.".to_string(),
                ));
            }
            "lambda" => {
                // Never flag `lambda` — it's preferred
            }
            _ => {
                // "line_count_dependent" (default): only flag single-line `lambda`
                let block = match call.block() {
                    Some(b) => b,
                    None => return,
                };
                let block_node = match block.as_block_node() {
                    Some(bn) => bn,
                    None => return,
                };

                let (start_line, _) =
                    source.offset_to_line_col(block_node.location().start_offset());
                let (end_line, _) = source.offset_to_line_col(
                    block_node
                        .location()
                        .end_offset()
                        .saturating_sub(1)
                        .max(block_node.location().start_offset()),
                );

                if start_line == end_line {
                    let loc = call.message_loc().unwrap_or_else(|| call.location());
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    diagnostics.push(
                        self.diagnostic(
                            source,
                            line,
                            column,
                            "Use the `-> {}` lambda literal syntax for single-line lambdas."
                                .to_string(),
                        ),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{run_cop_full, run_cop_full_with_config};

    crate::cop_fixture_tests!(Lambda, "cops/style/lambda");

    #[test]
    fn lambda_with_receiver_is_ignored() {
        let source = b"obj.lambda { |x| x }\n";
        let diags = run_cop_full(&Lambda, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn literal_no_offense_bare_lambda() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("literal".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"b = lambda\n";
        let diags = run_cop_full_with_config(&Lambda, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }
}
