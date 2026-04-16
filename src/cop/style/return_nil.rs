use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Corpus investigation (2026-03-27):
/// - FN=7: plain `proc do ... end` blocks were being suppressed as iterator blocks.
///   RuboCop only suppresses ancestors that are regular chained sends with block
///   arguments; bare `proc` has no receiver, so `return nil` inside those blocks
///   remains an offense.
/// - FN=4: safe-navigation block calls like `messages&.each do |message| ... end`
///   were also being suppressed. RuboCop's iterator check only matches regular
///   `send`, not `csend`, so `&.` must not trigger the suppression.
/// - FP=1: `begin ... rescue ... return nil end.tap { |x| ... }` was flagged because
///   the visitor only pushed block context after walking the call receiver. RuboCop's
///   ancestor walk still sees the attached block for returns inside the receiver tree.
///   Fix: push block context before visiting receiver/arguments/body, keep `lambda`
///   as a scope boundary, and only treat regular dot calls with receivers as chained
///   sends.
/// - FN=2 under `EnforcedStyle: return_nil`: nested bare returns inside another
///   return expression, such as `return redirect(...) && return`, were skipped
///   because `visit_return_node` never recursed into the return arguments.
///   RuboCop still walks that subtree and flags the inner bare `return`, so we
///   must always traverse return arguments even when the current return itself
///   is already in the correct style or suppressed as an iterator-block exit.
pub struct ReturnNil;

impl Cop for ReturnNil {
    fn name(&self) -> &'static str {
        "Style/ReturnNil"
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &crate::parse::codemap::CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let enforced_style = config.get_str("EnforcedStyle", "return");
        let mut visitor = ReturnNilVisitor {
            cop: self,
            source,
            enforced_style,
            diagnostics: Vec::new(),
            block_stack: Vec::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

/// Tracks block context to determine whether a `return` is inside an iterator block.
#[derive(Clone)]
struct BlockContext {
    has_args: bool,
    is_chained_send: bool,
    is_define_method: bool,
}

struct ReturnNilVisitor<'a, 'src> {
    cop: &'a ReturnNil,
    source: &'src SourceFile,
    enforced_style: &'a str,
    diagnostics: Vec<Diagnostic>,
    block_stack: Vec<BlockContext>,
}

impl ReturnNilVisitor<'_, '_> {
    /// Check if `return` is inside an iterator block (chained send with args).
    /// Mirrors RuboCop's ancestor walk in `on_return`:
    /// - If we hit a define_method block → stop (it creates its own scope)
    /// - If block has no args → skip, keep looking outward
    /// - If block has args and is a regular chained send → suppress
    fn inside_iterator_block(&self) -> bool {
        for ctx in self.block_stack.iter().rev() {
            if ctx.is_define_method {
                return false;
            }
            if !ctx.has_args {
                continue;
            }
            if ctx.is_chained_send {
                return true;
            }
        }
        false
    }
}

fn is_regular_chained_send(node: &ruby_prism::CallNode<'_>) -> bool {
    node.receiver().is_some()
        && node
            .call_operator_loc()
            .is_none_or(|op: ruby_prism::Location<'_>| op.as_slice() != b"&.")
}

impl<'pr> Visit<'pr> for ReturnNilVisitor<'_, '_> {
    fn visit_return_node(&mut self, node: &ruby_prism::ReturnNode<'pr>) {
        // RuboCop suppresses the offense when `return` is inside an iterator block
        // to avoid double-reporting with Lint/NonLocalExitFromIterator.
        if !self.inside_iterator_block() {
            match self.enforced_style {
                "return" => {
                    // Flag `return nil` — prefer `return`
                    if let Some(args) = node.arguments() {
                        let arg_list: Vec<_> = args.arguments().iter().collect();
                        if arg_list.len() == 1 && arg_list[0].as_nil_node().is_some() {
                            let loc = node.location();
                            let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                            self.diagnostics.push(self.cop.diagnostic(
                                self.source,
                                line,
                                column,
                                "Use `return` instead of `return nil`.".to_string(),
                            ));
                        }
                    }
                }
                "return_nil" => {
                    // Flag bare `return` — prefer `return nil`
                    if node.arguments().is_none() {
                        let loc = node.location();
                        let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                        self.diagnostics.push(self.cop.diagnostic(
                            self.source,
                            line,
                            column,
                            "Use `return nil` instead of `return`.".to_string(),
                        ));
                    }
                }
                _ => {}
            }
        }

        if let Some(args) = node.arguments() {
            self.visit(&args.as_node());
        }
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if let Some(block) = node.block() {
            if let Some(block_node) = block.as_block_node() {
                let method_name = node.name().as_slice();
                let has_args = block_node.parameters().is_some();
                let is_chained_send = is_regular_chained_send(node);
                let is_define_method =
                    method_name == b"define_method" || method_name == b"define_singleton_method";

                // `lambda do...end` creates its own scope (like stabby `-> {}`).
                // In Prism, method-style `lambda` is a CallNode, not LambdaNode.
                // Save and restore the block stack to isolate the lambda scope.
                if method_name == b"lambda" && node.receiver().is_none() {
                    let saved = std::mem::take(&mut self.block_stack);
                    if let Some(recv) = node.receiver() {
                        self.visit(&recv);
                    }
                    if let Some(args) = node.arguments() {
                        self.visit(&args.as_node());
                    }
                    if let Some(body) = block_node.body() {
                        self.visit(&body);
                    }
                    self.block_stack = saved;
                    return;
                }

                // RuboCop's ancestor walk sees the block attached to this call even
                // when `return nil` appears in the receiver subtree, so keep this
                // context active while visiting receiver, arguments, and body.
                self.block_stack.push(BlockContext {
                    has_args,
                    is_chained_send,
                    is_define_method,
                });
                if let Some(recv) = node.receiver() {
                    self.visit(&recv);
                }
                if let Some(args) = node.arguments() {
                    self.visit(&args.as_node());
                }
                if let Some(body) = block_node.body() {
                    self.visit(&body);
                }
                self.block_stack.pop();
                return;
            } else {
                // BlockArgumentNode (&block) — handled after receiver/arguments
            }
        }

        if let Some(recv) = node.receiver() {
            self.visit(&recv);
        }
        if let Some(args) = node.arguments() {
            self.visit(&args.as_node());
        }
        if let Some(block) = node.block() {
            self.visit(&block);
        }
    }

    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        // Standalone block (not attached to a call — handled via visit_call_node above)
        let has_args = node.parameters().is_some();
        self.block_stack.push(BlockContext {
            has_args,
            is_chained_send: false,
            is_define_method: false,
        });
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.block_stack.pop();
    }

    // Don't recurse into nested def/class/module/lambda (they create their own scope)
    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        // Reset block stack inside method definitions — they create a new scope
        let saved = std::mem::take(&mut self.block_stack);
        ruby_prism::visit_def_node(self, node);
        self.block_stack = saved;
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        let saved = std::mem::take(&mut self.block_stack);
        ruby_prism::visit_lambda_node(self, node);
        self.block_stack = saved;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(ReturnNil, "cops/style/return_nil");
    crate::cop_variant_fixture_tests!(ReturnNil, "cops/style/return_nil", return_nil);
}
